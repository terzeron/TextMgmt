//! 책 이동·삭제·단일 파일 색인. Python `update_book`, `delete_book`, `delete_file`, `index_single_file`에 대응한다.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use serde_json::{Value, json};

use super::BookManager;
use super::util::relative_to;
use crate::es_manager::Docs;

/// 이동 뒤 ES 갱신 결과.
enum Outcome {
    Done,
    /// `es.delete`가 `false`를 돌려줬다.
    DeleteRefused,
    ReindexFailed(String),
}

impl BookManager {
    /// `Path.exists()`.
    pub(crate) fn path_exists(&self, path: &Path) -> bool {
        self.fs.metadata(path).is_ok()
    }

    /// 책 루트 기준 상대 경로. 그대로 못 떼면 풀어 쓴 경로로 시도한다.
    pub(crate) fn relative_to_root(&self, path: &Path) -> Option<String> {
        relative_to(path, &self.path_prefix).or_else(|| {
            let (resolved, root) = (self.resolve_path(path).ok()?, self.resolved_root().ok()?);
            relative_to(&resolved, &root)
        })
    }

    /// 파일 이동 롤백: 새 위치에서 원래 위치로 되돌린다.
    fn rollback_move(&self, new_full_path: &Path, file_path: &Path, book_id: i64) -> io::Result<()> {
        self.fs.rename(new_full_path, file_path).inspect_err(|e| log::error!("update_book: rollback failed for book_id={book_id}: {e}"))
    }

    /// 기존 ES 문서를 되살린다. 실패 사유(없으면 성공)를 돌려준다.
    fn restore_old_doc(&self, book_id: i64, old_doc: &Value) -> Result<(), String> {
        let docs = Docs::from([(book_id, old_doc.clone())]);
        match self.add_book_raw(&docs) {
            Ok((restored, None)) if restored == Some(book_id) => Ok(()),
            Ok((_, error)) => Err(error.unwrap_or_else(|| "None".to_string())),
            Err(e) => Err(e),
        }
    }

    /// 새 위치에서 문서를 다시 만들어 색인한다. `Ok(None)`이면 성공이다. `Err`는 예외, `Ok(Some)`은 색인 실패 사유다.
    fn reindex_moved_file(&self, book_id: i64, new_full_path: &Path, new_relative: &str, fields: &[(&str, &str)]) -> Result<Option<String>, String> {
        let reader = self.reader();
        let prefix = self.path_prefix.clone();
        let target = new_full_path.to_path_buf();
        let data = catch_unwind(AssertUnwindSafe(|| reader.read_file(&target, Some(&prefix)))).map_err(|_| format!("파일 읽기 중 오류: {new_relative}"))?;
        if data.len() != 1 {
            return Ok(Some(format!("지원하지 않는 파일 형식입니다: {new_relative}")));
        }
        let mut new_doc = data.into_values().next().unwrap_or(Value::Null);
        for (key, value) in fields {
            new_doc[*key] = json!(value);
        }
        self.es.delete_by_file_paths(&[new_relative.to_string()], Some(&[book_id])).map_err(|e| e.to_string())?;
        let (reindexed, error) = self.add_book_raw(&Docs::from([(book_id, new_doc)]))?;
        if reindexed == Some(book_id) && error.is_none() {
            return Ok(None);
        }
        Ok(Some(error.unwrap_or_else(|| "None".to_string())))
    }

    pub async fn update_book(&self, book_id: i64, new_category: &str, new_title: &str, new_author: &str, new_path: &Path, new_type: &str) -> (String, Option<String>) {
        self.update_book_with(book_id, new_category, new_title, new_author, new_path, new_type, false).await
    }

    /// 책의 카테고리·제목·저자·경로를 바꾼다. 파일을 옮기고 ES 문서를 지웠다가 다시 넣으며, 실패하면 되돌린다.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_book_with(&self, book_id: i64, new_category: &str, new_title: &str, new_author: &str, new_path: &Path, new_type: &str, force: bool) -> (String, Option<String>) {
        log::debug!(
            "# update_book(book_id={book_id}, new_category='{new_category}', new_title='{new_title}', new_author='{new_author}', new_path='{}', new_file_type='{new_type}', force={force})",
            new_path.display()
        );
        // 경로 탈출 방지: path_prefix 외부 이동 금지
        if !matches!(self.is_inside_root(new_path), Ok(true)) {
            return ("Error".to_string(), Some("잘못된 경로입니다".to_string()));
        }
        let doc = match self.find_doc(book_id) {
            Ok(Some(doc)) => doc,
            Ok(None) => return ("Error".to_string(), Some(format!("can't update book information of '{book_id}' in ElasticSearch, no such a book"))),
            Err(e) => return ("Error".to_string(), Some(e)),
        };
        let book = self.make_book(book_id, &doc, 0.0);
        let file_path = book.file_path;
        let new_full_path = new_path;

        // 대상 경로에 다른 파일이 이미 있는지 확인
        if self.path_exists(new_full_path) {
            let is_same_file = self.path_exists(&file_path) && self.fs.same_file(&file_path, new_full_path).unwrap_or(false);
            if !is_same_file {
                if !force {
                    let relative = self.relative_to_root(new_full_path).unwrap_or_else(|| new_full_path.display().to_string());
                    return ("Error".to_string(), Some(format!("CONFLICT:대상 경로에 파일이 이미 존재합니다: {relative}")));
                }
                log::warn!("update_book: force overwriting existing file '{}' (book_id={book_id})", new_full_path.display());
            }
        }

        let moved = match new_full_path.parent() {
            Some(parent) => std::fs::create_dir_all(parent),
            None => Ok(()),
        }
        .and_then(|()| self.fs.rename(&file_path, new_full_path));
        if let Err(e) = moved {
            return ("Error".to_string(), Some(format!("can't move '{}' to '{}', {e}", file_path.display(), new_full_path.display())));
        }

        // 이동한 파일로 ES 문서를 다시 만들어 파생 메타데이터를 맞춘다.
        let Some(new_relative) = self.relative_to_root(new_full_path) else {
            let _ = self.rollback_move(new_full_path, &file_path, book_id);
            return ("Error".to_string(), Some("잘못된 경로입니다".to_string()));
        };
        let old_doc = doc;
        let mut old_doc_deleted = false;
        let outcome: Result<Outcome, String> = (|| {
            match self.es.delete(book_id) {
                Ok(true) => {}
                Ok(false) => {
                    log::error!("update_book: ES delete failed for book_id={book_id}, rolling back file move");
                    return Ok(Outcome::DeleteRefused);
                }
                Err(e) => return Err(e.to_string()),
            }
            old_doc_deleted = true;
            let fields = [("category", new_category), ("title", new_title), ("author", new_author), ("file_path", new_relative.as_str()), ("file_type", new_type)];
            Ok(match self.reindex_moved_file(book_id, new_full_path, &new_relative, &fields)? {
                None => Outcome::Done,
                Some(reason) => Outcome::ReindexFailed(reason),
            })
        })();

        match outcome {
            Ok(Outcome::Done) => {
                self.clear_mismatch_cache();
                ("Ok".to_string(), None)
            }
            Ok(Outcome::DeleteRefused) => match self.rollback_move(new_full_path, &file_path, book_id) {
                Err(rollback_err) => ("Error".to_string(), Some(format!("ES 문서 삭제 실패, 파일 롤백도 실패: {rollback_err}"))),
                Ok(()) => ("Error".to_string(), Some(format!("ES 문서 삭제 실패, 파일 롤백 완료: book_id={book_id}"))),
            },
            Ok(Outcome::ReindexFailed(reindex_error)) => {
                log::error!("update_book: ES reindex failed for book_id={book_id}, rolling back file move: {reindex_error}");
                if let Err(rollback_err) = self.rollback_move(new_full_path, &file_path, book_id) {
                    return ("Error".to_string(), Some(format!("ES 재색인 실패, 파일 롤백도 실패: ES={reindex_error}, rollback={rollback_err}")));
                }
                match self.restore_old_doc(book_id, &old_doc) {
                    Err(restore_err) => ("Error".to_string(), Some(format!("ES 재색인 실패, 파일 롤백 완료, 기존 ES 문서 복구 실패: {restore_err}"))),
                    Ok(()) => ("Error".to_string(), Some(format!("ES 재색인 실패, 파일 롤백 및 기존 ES 문서 복구 완료: {reindex_error}"))),
                }
            }
            Err(e) => {
                log::error!("update_book: ES reindex exception for book_id={book_id}: {e}, rolling back file move");
                if let Err(rollback_err) = self.rollback_move(new_full_path, &file_path, book_id) {
                    return ("Error".to_string(), Some(format!("ES 재색인 예외, 파일 롤백 실패: ES={e}, rollback={rollback_err}")));
                }
                if old_doc_deleted {
                    return match self.restore_old_doc(book_id, &old_doc) {
                        Err(restore_err) => ("Error".to_string(), Some(format!("ES 재색인 예외, 파일 롤백 완료, 기존 ES 문서 복구 실패: {restore_err}"))),
                        Ok(()) => ("Error".to_string(), Some(format!("ES 재색인 예외, 파일 롤백 및 기존 ES 문서 복구 완료: {e}"))),
                    };
                }
                ("Error".to_string(), Some(format!("ES 문서 삭제 예외, 파일 롤백 완료: {e}")))
            }
        }
    }

    pub async fn delete_book(&self, book_id: i64) -> (String, Option<String>) {
        log::debug!("# delete_book(book_id={book_id})");
        let doc = match self.find_doc(book_id) {
            Ok(Some(doc)) => doc,
            Ok(None) => return ("Ok".to_string(), None),
            Err(e) => return ("Error".to_string(), Some(e)),
        };
        let mut warning_message = None;

        // 파일 삭제
        let file_path = self.make_book(book_id, &doc, 0.0).file_path;
        match self.fs.remove_file(&file_path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                log::warn!("File already deleted for book_id={book_id}: {e}");
                warning_message = Some(format!("파일이 이미 삭제되었습니다: {e}"));
            }
            Err(e) => return ("Error".to_string(), Some(format!("can't delete a book with '{book_id}', {e}"))),
        }

        // ES 문서 삭제
        match self.es.delete(book_id) {
            Ok(true) => match warning_message {
                Some(message) => ("Warning".to_string(), Some(message)),
                None => ("Ok".to_string(), None),
            },
            Ok(false) => ("Error".to_string(), Some(format!("can't delete book information of '{book_id}' from ElasticSearch"))),
            Err(e) => ("Error".to_string(), Some(format!("can't delete book information of '{book_id}' from ElasticSearch: {e}"))),
        }
    }

    /// 파일시스템의 파일을 읽어 ES에 적재한다. 같은 경로의 기존 문서는 지운다.
    pub async fn index_single_file(&self, file_path: &str) -> (Option<i64>, Option<String>) {
        self.index_single_file_with(file_path, true).await
    }

    pub async fn index_single_file_with(&self, file_path: &str, clean_existing: bool) -> (Option<i64>, Option<String>) {
        let abs_path = match self.resolve_path(&self.path_prefix.join(file_path)) {
            Ok(p) if matches!(self.is_inside_root(&p), Ok(true)) => p,
            _ => return (None, Some("잘못된 경로입니다".to_string())),
        };
        if !self.fs.is_file(&abs_path).unwrap_or(false) {
            return (None, Some(format!("파일을 찾을 수 없습니다: {file_path}")));
        }
        let data = self.reader().read_file(&abs_path, None);
        if data.is_empty() {
            return (None, Some(format!("지원하지 않는 파일 형식입니다: {file_path}")));
        }
        if clean_existing {
            let file_paths: Vec<String> = data.values().filter_map(|doc| doc.get("file_path").and_then(Value::as_str).map(str::to_string)).collect();
            let doc_ids: Vec<i64> = data.keys().copied().collect();
            if let Err(e) = self.es.delete_by_file_paths(&file_paths, Some(&doc_ids)) {
                return (None, Some(format!("기존 ES 문서 정리 실패: {e}")));
            }
        }
        let (book_id, error) = self.add_book(&data).await;
        if error.is_none() {
            self.clear_mismatch_cache();
        }
        (book_id, error)
    }

    /// 파일시스템에서 파일 하나를 지운다.
    pub async fn delete_file(&self, file_path: &str) -> (String, Option<String>) {
        let abs_path = match self.resolve_path(&self.path_prefix.join(file_path)) {
            Ok(p) if matches!(self.is_inside_root(&p), Ok(true)) => p,
            _ => return ("Error".to_string(), Some("잘못된 경로입니다".to_string())),
        };
        if !self.fs.is_file(&abs_path).unwrap_or(false) {
            return ("Error".to_string(), Some(format!("파일을 찾을 수 없습니다: {file_path}")));
        }
        match self.fs.remove_file(&abs_path) {
            Ok(()) => {
                self.clear_mismatch_cache();
                ("Ok".to_string(), None)
            }
            Err(e) => ("Error".to_string(), Some(format!("파일 삭제 실패: {e}"))),
        }
    }
}
