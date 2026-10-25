//! PDF 편집: 원본 번호로 선택하고 백업을 보존한 뒤 파일과 색인을 갱신한다.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lopdf::{Document, Object, ObjectId, dictionary};
use serde_json::{Value, json};
use sysdeps::FsOps;

use super::util::{strerror, unique_token};
use super::{BookManager, Docs};

mod images;
use images::Edits;

struct EditResult {
    remaining: usize,
    backup: String,
    backup_created: bool,
    cropped: usize,
    split: usize,
    inserted: usize,
}

impl BookManager {
    pub async fn preview_insert_image(&self, url: &str) -> (Option<Vec<u8>>, Option<String>) {
        let url = url.trim().to_string();
        if url.is_empty() {
            return (None, Some("이미지 URL이 비어 있습니다".into()));
        }
        let (_, fetcher) = self.pdf_edit_settings();
        match tokio::task::spawn_blocking(move || images::download_image(fetcher.as_ref(), &url)).await {
            Ok(Ok(jpeg)) => (Some(jpeg), None),
            Ok(Err(error)) => (None, Some(error)),
            Err(error) => {
                log::error!("preview_insert_image: {error}");
                (None, Some("이미지를 가져오지 못했습니다".into()))
            }
        }
    }

    pub async fn preview_insert_upload(&self, data: &[u8]) -> (Option<Vec<u8>>, Option<String>) {
        let data = data.to_vec();
        match tokio::task::spawn_blocking(move || images::normalize_upload(&data)).await {
            Ok(Ok(jpeg)) => (Some(jpeg), None),
            Ok(Err(error)) => (None, Some(error)),
            Err(error) => {
                log::error!("preview_insert_upload: {error}");
                (None, Some("이미지를 읽지 못했습니다".into()))
            }
        }
    }

    /// Python `delete_pdf_pages`. 페이지 번호와 순서는 편집 전 문서를 기준으로 한다.
    pub async fn delete_pdf_pages(
        &self,
        book_id: i64,
        pages: &[i64],
        crops: Option<&[Value]>,
        splits: Option<&[Value]>,
        inserts: Option<&[Value]>,
        order: Option<&[i64]>,
    ) -> (Option<Value>, Option<String>) {
        match self.edit_pdf(book_id, pages, crops, splits, inserts, order).await {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        }
    }

    async fn edit_pdf(&self, book_id: i64, pages: &[i64], crops: Option<&[Value]>, splits: Option<&[Value]>, inserts: Option<&[Value]>, order: Option<&[i64]>) -> Result<Value, String> {
        let edits = Edits::parse(pages, crops, splits, inserts, order)?;
        let old = self.find_doc_async(book_id).await?.ok_or_else(|| format!("Book not found: {book_id}"))?;
        let path = self.make_book(book_id, &old, 0.0).file_path;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf")) {
            return Err(format!("Not a PDF file: {name}"));
        }
        if !self.fs.is_file(&path).map_err(io_error)? {
            return Err(format!("File not found: {name}"));
        }
        let (trash_name, fetcher) = self.pdf_edit_settings();
        let (fs, source, trash) = (self.fs.clone(), path.clone(), self.path_prefix.join(trash_name));
        let result = tokio::task::spawn_blocking(move || edit_file(fs, &source, &trash, &edits, fetcher.as_ref())).await.map_err(|e| e.to_string())??;

        // 파일 교체가 끝나면 색인 실패와 무관하게 이전 미리보기를 폐기한다.
        self.pdf_cache().clear();
        self.page_count_cache().clear();
        let _ = self.fs.remove_file(&self.path_prefix.join(".preview_cache").join(format!("{book_id}.pdf")));
        for (directory, prefix, suffix) in [(".cover_cache", format!("{book_id}.v"), ".jpg"), (".preview_cache", format!("{book_id}_"), ".pdf")] {
            if let Ok(entries) = std::fs::read_dir(self.path_prefix.join(directory)) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with(&prefix) && name.ends_with(suffix) {
                        let _ = self.fs.remove_file(&entry.path());
                    }
                }
            }
        }
        let (reader, root, es, remaining) = (self.reader(), self.path_prefix.clone(), self.es.clone(), result.remaining);
        let data = tokio::task::spawn_blocking(move || -> Result<Docs, String> {
            let docs = reader.try_read_file(&path, Some(&root), false).map_err(|e| format!("{e:?}"))?;
            if docs.len() != 1 {
                return Err("재색인할 문서를 읽지 못했습니다".into());
            }
            let mut doc = docs.into_values().next().unwrap();
            if !doc.is_object() {
                return Err("재색인할 문서를 읽지 못했습니다".into());
            }
            for key in ["category", "title", "author", "file_path", "file_type"] {
                doc[key] = old.get(key).cloned().unwrap_or_else(|| json!(if key == "file_type" { "pdf" } else { "" }));
            }
            doc["page_count"] = json!(remaining);
            doc["file_size"] = json!(std::fs::metadata(&path).map_err(io_error)?.len());
            if !es.delete(book_id).map_err(|e| e.to_string())? {
                return Err("ES 문서 삭제 실패".into());
            }
            Ok(Docs::from([(book_id, doc)]))
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|v| v);
        let reindex = match data {
            Err(error) => Err(error),
            Ok(data) => {
                let (id, error) = self.add_book(&data).await;
                if id == Some(book_id) && error.is_none() { Ok(()) } else { Err(error.unwrap_or_else(|| "ES 재색인 실패".into())) }
            }
        };
        if let Err(error) = reindex {
            return Err(format!("페이지는 삭제되었으나 색인 갱신에 실패했습니다. 원본은 {} 에 있습니다: {error}", result.backup));
        }
        self.clear_mismatch_cache();
        Ok(
            json!({"remaining_pages":result.remaining, "cropped_pages":result.cropped, "split_pages":result.split, "inserted_pages":result.inserted, "backup":result.backup, "backup_created":result.backup_created}),
        )
    }
}

fn io_error(error: std::io::Error) -> String {
    format!("파일 처리 실패: {}", strerror(&error))
}

/// 임시 파일만 정리한다. 교체 전 원본과 확정한 백업은 실패해도 남긴다.
struct Parts(Vec<PathBuf>);
impl Drop for Parts {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn edit_file(fs: Arc<dyn FsOps>, path: &Path, trash: &Path, edits: &Edits, fetcher: &dyn crate::direct_cover_url::ImageFetcher) -> Result<EditResult, String> {
    let mut pdf = Document::load(path).map_err(|e| format!("PDF 페이지 삭제 실패: {e}"))?;
    let original = pdf.get_pages();
    edits.validate(original.len())?;
    flatten_pages(&mut pdf, &original.values().copied().collect::<Vec<_>>()).map_err(|e| format!("PDF 페이지 삭제 실패: {e}"))?;
    let selected = edits.apply(&mut pdf, fetcher)?;
    flatten_pages(&mut pdf, &selected).map_err(|e| format!("PDF 페이지 삭제 실패: {e}"))?;
    pdf.prune_objects();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let token = unique_token();
    let edit = path.with_file_name(format!(".{name}.{token}.edit.part"));
    let backup_part = trash.join(format!(".{name}.{token}.part"));
    let _parts = Parts(vec![edit.clone(), backup_part.clone()]);
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&edit).map_err(io_error)?;
    pdf.save_to(&mut output).map_err(io_error)?;
    std::fs::set_permissions(&edit, std::fs::metadata(path).map_err(io_error)?.permissions()).map_err(io_error)?;
    fs.sync_file(&output).map_err(io_error)?;
    std::fs::create_dir_all(trash).map_err(io_error)?;
    let backup = trash.join(name.as_ref());
    let mut backup_created = false;
    if !backup.try_exists().map_err(io_error)? {
        std::fs::copy(path, &backup_part).map_err(io_error)?;
        let saved = std::fs::File::open(&backup_part).map_err(io_error)?;
        fs.sync_file(&saved).map_err(io_error)?;
        // hard_link은 기존 백업을 덮어쓰지 않고 완성된 파일을 게시한다.
        match std::fs::hard_link(&backup_part, &backup) {
            Ok(()) => backup_created = true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_error(e)),
        }
    }
    fs.rename(&edit, path).map_err(io_error)?;
    Ok(EditResult {
        remaining: selected.len(),
        backup: format!("{}/{}", trash.file_name().unwrap_or_default().to_string_lossy(), name),
        backup_created,
        cropped: edits.crops.len(),
        split: edits.splits.len(),
        inserted: edits.inserts.len(),
    })
}

/// 부모 Pages를 바꿀 때 상속 속성을 각 페이지에 고정한다.
fn flatten_pages(pdf: &mut Document, selected: &[ObjectId]) -> lopdf::Result<()> {
    let root = pdf.catalog()?.get(b"Pages")?.as_reference()?;
    for &id in selected {
        let mut page = pdf.get_dictionary(id)?.clone();
        let mut parent = page.get(b"Parent").and_then(Object::as_reference).ok();
        let mut visited = BTreeSet::new();
        while let Some(id) = parent {
            if !visited.insert(id) {
                return Err(lopdf::Error::ReferenceCycle(id));
            }
            let ancestor = pdf.get_dictionary(id)?;
            for key in ["Resources", "MediaBox", "CropBox", "Rotate"] {
                if !page.has(key.as_bytes())
                    && let Ok(value) = ancestor.get(key.as_bytes())
                {
                    page.set(key, value.clone());
                }
            }
            parent = ancestor.get(b"Parent").and_then(Object::as_reference).ok();
        }
        page.set("Parent", root);
        pdf.objects.insert(id, Object::Dictionary(page));
    }
    let kids: Vec<Object> = selected.iter().copied().map(Object::Reference).collect();
    pdf.objects.insert(root, dictionary! {"Type"=>"Pages", "Count"=> selected.len() as i64, "Kids"=>kids}.into());
    Ok(())
}
