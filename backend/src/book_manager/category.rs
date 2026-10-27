//! 카테고리 이름 변경·삭제. Python `rename_category`, `preview_delete_category`, `delete_category`에 대응한다.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::BookManager;

fn failures_of(es_result: &Value) -> Option<&Value> {
    es_result.get("failures").filter(|f| f.as_array().is_some_and(|a| !a.is_empty()) || f.as_str().is_some_and(|s| !s.is_empty()))
}

fn failures_text(failures: &Value) -> String {
    match failures.as_array() {
        Some(list) => format!("[{}]", list.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), |s| format!("'{s}'"))).collect::<Vec<_>>().join(", ")),
        None => failures.to_string(),
    }
}

impl BookManager {
    pub async fn rename_category(&self, old_category: &str, new_category: &str) -> (Value, Option<String>) {
        log::debug!("# rename_category(old='{old_category}', new='{new_category}')");
        let fail = |message: String| (json!({}), Some(message));

        // 입력 검증
        if old_category.is_empty() || new_category.is_empty() {
            return fail("카테고리 이름이 비어있습니다".to_string());
        }
        if old_category == new_category {
            return fail("이전 카테고리와 새 카테고리가 동일합니다".to_string());
        }
        if old_category.contains("..") || new_category.contains("..") {
            return fail("카테고리 이름에 '..'는 사용할 수 없습니다".to_string());
        }
        // '_root'는 최상위 파일을 묶어 부르는 이름일 뿐 디렉터리가 아니다.
        if old_category == Self::ROOT_CATEGORY {
            return fail("최상위 디렉토리는 이름을 변경할 수 없습니다".to_string());
        }
        if new_category == Self::ROOT_CATEGORY {
            return fail("'_root'는 최상위 디렉토리를 가리키는 예약된 이름이라 사용할 수 없습니다".to_string());
        }

        // 경로 검증(Path Traversal 방지)
        for category in [old_category, new_category] {
            if !self.resolve_path(&self.path_prefix.join(category)).is_ok_and(|p| self.resolved_root().is_ok_and(|root| p.starts_with(root))) {
                return fail(format!("잘못된 경로입니다: {category}"));
            }
        }

        // 디렉터리 rename은 하위 카테고리까지 옮기므로 양쪽 모두 하위 문서를 포함해 센다.
        let counts = match self.es.count_by_categories(&[old_category.to_string(), new_category.to_string()], true) {
            Ok(counts) => counts,
            Err(e) => return fail(format!("ES 조회 실패: {e}")),
        };
        if counts.get(old_category).copied().unwrap_or(0) == 0 {
            return fail(format!("카테고리 '{old_category}'에 문서가 없습니다"));
        }
        let new_count = counts.get(new_category).copied().unwrap_or(0);
        if new_count > 0 {
            return fail(format!("대상 카테고리 '{new_category}'에 이미 {new_count}개의 문서가 존재합니다"));
        }

        // 파일시스템 디렉터리 이름 변경
        let old_dir = self.path_prefix.join(old_category);
        let new_dir = self.path_prefix.join(new_category);
        let mut fs_renamed = false;
        if self.fs.metadata(&old_dir).is_ok_and(|m| m.is_dir()) {
            if self.path_exists(&new_dir) {
                return fail(format!("대상 디렉토리가 이미 존재합니다: {}", new_dir.display()));
            }
            let renamed = match new_dir.parent() {
                Some(parent) => std::fs::create_dir_all(parent),
                None => Ok(()),
            }
            .and_then(|()| self.fs.rename(&old_dir, &new_dir));
            if let Err(e) = renamed {
                return fail(format!("디렉토리 이름 변경 실패: {e}"));
            }
            fs_renamed = true;
        } else {
            log::warn!("rename_category: 디렉토리 없음 '{}', ES만 갱신", old_dir.display());
        }

        // ES update_by_query 실행. 실패하면 FS를 되돌린다.
        let es_result = match self.es.rename_category(old_category, new_category) {
            Ok(result) => result,
            Err(e) => {
                if fs_renamed && let Err(rollback_err) = self.fs.rename(&new_dir, &old_dir) {
                    log::error!("rename_category: FS 롤백 실패: {rollback_err}");
                    return fail(format!("ES 업데이트 실패: {e}. 경고: FS 롤백도 실패하여 수동 복구 필요 ('{}' → '{}')", new_dir.display(), old_dir.display()));
                }
                return fail(format!("ES 업데이트 실패: {e}"));
            }
        };

        if let Some(failures) = failures_of(&es_result) {
            // 부분 실패 시 FS 롤백
            if fs_renamed && let Err(rollback_err) = self.fs.rename(&new_dir, &old_dir) {
                log::error!("rename_category: FS 롤백 실패: {rollback_err}");
                return fail(format!("ES 부분 실패: {}. 경고: FS 롤백도 실패하여 수동 복구 필요", failures_text(failures)));
            }
            return fail(format!("ES 업데이트 부분 실패: {}", failures_text(failures)));
        }

        (json!({"old_category": old_category, "new_category": new_category, "updated_count": es_result["updated"], "fs_renamed": fs_renamed}), None)
    }

    /// 삭제 대상 카테고리의 디스크 경로를 검증해 돌려준다.
    fn resolve_deletable_category_dir(&self, category: &str, reject_library_root: bool) -> Result<PathBuf, String> {
        let root = self.resolved_root().unwrap_or_else(|_| self.path_prefix.clone());
        if category.is_empty() {
            return Err("카테고리 이름이 비어있습니다".to_string());
        }
        if category.contains("..") {
            return Err("카테고리 이름에 '..'는 사용할 수 없습니다".to_string());
        }
        // '_root'를 지우면 최상위 파일의 ES 문서만 사라지고 파일은 그대로 남아 불일치가 생긴다.
        if category == Self::ROOT_CATEGORY {
            return Err("최상위 디렉토리는 삭제할 수 없습니다".to_string());
        }
        let dir_check = self.resolve_path(&self.path_prefix.join(category)).map_err(|_| format!("잘못된 경로입니다: {category}"))?;
        if !dir_check.starts_with(&root) {
            return Err(format!("잘못된 경로입니다: {category}"));
        }
        // '.'처럼 책 루트 자신으로 풀리는 이름은 라이브러리 전체를 가리킨다.
        if reject_library_root && dir_check == root {
            return Err(format!("잘못된 경로입니다: {category}"));
        }
        Ok(dir_check)
    }

    /// 삭제 확인 모달용으로, 지워질 하위 디렉터리 수·디스크 파일 수·ES 문서 수를 센다. 아무것도 바꾸지 않는다.
    pub async fn preview_delete_category(&self, category: &str) -> (Value, Option<String>) {
        let dir_check = match self.resolve_deletable_category_dir(category, true) {
            Ok(dir) => dir,
            Err(e) => return (json!({}), Some(e)),
        };
        let (mut directory_count, mut file_count) = (0i64, 0i64);
        if dir_check.is_dir() {
            let mut stack = vec![dir_check];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = self.fs.read_dir(&dir) else { continue };
                for entry in entries {
                    if Self::is_dir_nofollow(&entry) {
                        directory_count += 1;
                        stack.push(entry);
                    } else {
                        // 디렉터리 링크는 따라가지 않고 파일 하나로 센다(rmtree도 링크 대상은 지우지 않는다).
                        file_count += 1;
                    }
                }
            }
        }
        match self.es.count_by_category(category, true) {
            Ok(es_count) => (json!({"category": category, "directory_count": directory_count, "file_count": file_count, "es_count": es_count}), None),
            Err(e) => (json!({}), Some(format!("ES 조회 실패: {e}"))),
        }
    }

    /// 카테고리의 모든 문서를 일괄 삭제한다. `delete_files`가 거짓이면 ES 문서만 지우고 파일은 남긴다.
    /// 참이면 디렉터리 트리도 지운다(디스크를 먼저 지워서, 실패하면 ES 문서가 남아 다시 시도할 수 있다).
    pub async fn delete_category(&self, category: &str, delete_files: bool) -> (Value, Option<String>) {
        log::debug!("# delete_category(category='{category}')");
        let dir_check = match self.resolve_deletable_category_dir(category, delete_files) {
            Ok(dir) => dir,
            Err(e) => return (json!({}), Some(e)),
        };

        // 하위 카테고리 포함 문서 수. 파일까지 지우는 경우 ES 문서가 없어도 디스크 디렉터리가 있으면 지운다.
        let doc_count = match self.es.count_by_category(category, true) {
            Ok(n) => n,
            Err(e) => return (json!({}), Some(format!("ES 조회 실패: {e}"))),
        };
        if doc_count == 0 && !(delete_files && dir_check.is_dir()) {
            return (json!({}), Some(format!("카테고리 '{category}'에 문서가 없습니다")));
        }

        let mut fs_deleted = false;
        if delete_files && dir_check.is_dir() {
            if let Err(e) = self.fs.remove_dir_all(&dir_check) {
                log::warn!("delete_category: 디렉토리 삭제 실패 '{}': {e}", dir_check.display());
                return (json!({}), Some(format!("디렉토리 삭제 실패: {e}")));
            }
            fs_deleted = true;
        }

        let es_result = match self.es.delete_by_category(category, true) {
            Ok(result) => result,
            Err(e) => return (json!({}), Some(format!("ES 삭제 실패: {e}"))),
        };
        if let Some(failures) = failures_of(&es_result) {
            return (json!({}), Some(format!("ES 삭제 부분 실패: {}", failures_text(failures))));
        }

        let mut result = json!({"category": category, "deleted_count": es_result["deleted"]});
        if delete_files {
            result["fs_deleted"] = json!(fs_deleted);
        }
        (result, None)
    }
}
