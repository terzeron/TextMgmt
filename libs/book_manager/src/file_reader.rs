//! 파일 종류 판별과 기본 `FileReader`.
//!
//! `DefaultFileReader`는 `loader` crate 없이 메타데이터 문서만 만든다: 카테고리, 제목, 저자, 크기, 시각, 형식.
//! 본문 요약은 txt만 앞부분을 담고, 쪽수·줄 수·ISBN·PDF/EPUB/DOC 본문은 채우지 않는다.
//! 운영에서는 `file_indexer::record_builder::read_file`을 감싼 `FileReader`를 `BookManagerDeps::with_file_reader`로 꽂아야 한다.

use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use file_indexer::env_config::Config;
use file_indexer::record_builder;
use serde_json::{Value, json};

use super::path_validator::relative_to;
use super::path_validator::{declared_type, detect_file_type, is_indexable_type};
use super::{Docs, FileReader};
use file_indexer::file_time::iso;
/// 요약에 담는 본문 길이(문자).
const SUMMARY_CHARS: usize = 3500;

fn local_naive(time: std::time::SystemTime) -> NaiveDateTime {
    chrono::DateTime::<Local>::from(time).naive_local()
}

/// 기본 `FileReader`. 설명은 모듈 머리말을 본다.
pub struct DefaultFileReader {
    path_prefix: PathBuf,
}

/// `loader` 구현을 사용해 파일 본문과 페이지·행수 metadata까지 만드는 reader.
pub struct LoaderFileReader {
    config: Config,
    worker_executable: PathBuf,
}

impl LoaderFileReader {
    pub fn new(book_prefix: PathBuf, comics_prefix: PathBuf) -> Self {
        Self { config: Config::new(book_prefix, comics_prefix), worker_executable: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("backend")) }
    }

    pub fn with_worker_executable(mut self, executable: PathBuf) -> Self {
        self.worker_executable = executable;
        self
    }

    fn read(&self, path: &Path, path_prefix: Option<&Path>, skip_text: bool) -> Docs {
        let outcome = file_indexer::child_process_runner::read_file_with_child_stages(&self.worker_executable, &self.config, path, path_prefix, skip_text);
        for message in outcome.messages {
            log::warn!("loader failed to read '{}': {message}", path.display());
        }
        outcome.item.into_iter().collect()
    }
}

impl FileReader for LoaderFileReader {
    fn read_file(&self, path: &Path, path_prefix: Option<&Path>) -> Docs {
        self.read(path, path_prefix, false)
    }

    fn read_file_with_options(&self, path: &Path, path_prefix: Option<&Path>, skip_text: bool) -> Docs {
        self.read(path, path_prefix, skip_text)
    }

    fn try_read_file_with_timeout(self: Arc<Self>, path: PathBuf, path_prefix: Option<PathBuf>, skip_text: bool, timeout: Duration) -> Result<Docs, super::ReadFileError> {
        let outcome =
            file_indexer::child_process_runner::read_file_in_child(&self.worker_executable, &self.config, &path, path_prefix.as_deref(), skip_text, timeout).map_err(|error| match error {
                file_indexer::child_process_runner::ChildReadError::Timeout(error) => super::ReadFileError::Timeout(error.to_string()),
                file_indexer::child_process_runner::ChildReadError::Other(error) => super::ReadFileError::Other(error),
            })?;
        for message in outcome.messages {
            log::warn!("loader failed to read '{}': {message}", path.display());
        }
        Ok(outcome.item.into_iter().collect())
    }
}

impl DefaultFileReader {
    pub fn new(path_prefix: PathBuf) -> Self {
        Self { path_prefix }
    }

    fn read(&self, path: &Path, path_prefix: Option<&Path>, skip_text: bool) -> Docs {
        let mut out = Docs::new();
        if !path.is_file() {
            return out;
        }
        let Ok(meta) = std::fs::metadata(path) else { return out };
        let prefix = path_prefix.unwrap_or(&self.path_prefix);
        let Some(parent_rel) = relative_to(path.parent().unwrap_or(Path::new("")), prefix) else {
            log::warn!("'{}' is not in the subpath of '{}'", path.display(), prefix.display());
            return out;
        };
        let category = if parent_rel.is_empty() { "_root".to_string() } else { parent_rel };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (author, title) = record_builder::split_author_title(&stem);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut file_type = declared_type(&name);
        if let Some(detected) = detect_file_type(&sysdeps::RealFs, path, &file_type)
            && detected != file_type
        {
            file_type = detected;
        }
        if !is_indexable_type(&file_type) {
            return out;
        }
        let relative_path = relative_to(path, prefix).unwrap_or_default();
        let summary = if !skip_text && file_type == "txt" { read_summary(path) } else { String::new() };
        let created = meta.created().or_else(|_| meta.modified()).map(local_naive).unwrap_or_default();
        let doc: Value = json!({
            "category": category, "title": title, "author": author, "file_path": relative_path, "file_type": file_type,
            "file_size": meta.len(), "line_count": 0, "page_count": 0, "isbn": "", "summary": summary,
            "created_time": iso(created), "created_time_source": "birthtime", "updated_time": iso(Local::now().naive_local()),
        });
        out.insert(meta.ino() as i64, doc);
        out
    }
}

fn read_summary(path: &Path) -> String {
    let mut buf = Vec::new();
    let Ok(file) = std::fs::File::open(path) else { return String::new() };
    // 문자 3500개는 UTF-8에서 최대 14000바이트다.
    if file.take(SUMMARY_CHARS as u64 * 4).read_to_end(&mut buf).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&buf).chars().take(SUMMARY_CHARS).collect()
}

impl FileReader for DefaultFileReader {
    fn read_file(&self, path: &Path, path_prefix: Option<&Path>) -> Docs {
        self.read(path, path_prefix, false)
    }

    fn read_file_with_options(&self, path: &Path, path_prefix: Option<&Path>, skip_text: bool) -> Docs {
        self.read(path, path_prefix, skip_text)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn zip_with(path: &Path, names: &[&str]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for name in names {
            if let Some(dir) = name.strip_suffix('/') {
                zip.add_directory(dir, zip::write::SimpleFileOptions::default()).unwrap();
            } else {
                zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(b"x").unwrap();
            }
        }
        zip.finish().unwrap();
    }

    #[test]
    fn zip_contents_decide_the_real_type() {
        let d = tempfile::tempdir().unwrap();
        for (names, want) in [
            (&["word/document.xml"][..], Some("docx")),
            (&["img/", "img/a.JPG", "b.png"][..], Some("cbz")),
            (&["notes.txt"][..], None),
            (&["only/"][..], None),
            (&["OEBPS/content.opf"][..], Some("epub")),
        ] {
            let p = d.path().join("a.zip");
            zip_with(&p, names);
            assert_eq!(detect_file_type(&sysdeps::RealFs, &p, "zip").as_deref(), want, "{names:?}");
        }
    }

    #[test]
    fn default_reader_skips_directories_outside_paths_and_unindexed_types() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("books");
        std::fs::create_dir_all(root.join("cat")).unwrap();
        let reader = DefaultFileReader::new(root.clone());
        assert!(reader.read_file(&root.join("cat"), None).is_empty());
        let outside = d.path().join("out.txt");
        std::fs::write(&outside, "본문").unwrap();
        assert!(reader.read_file(&outside, None).is_empty());
        std::fs::write(root.join("cat/notes.md"), "본문").unwrap();
        assert!(reader.read_file(&root.join("cat/notes.md"), None).is_empty());
        std::fs::write(root.join("cat/a.txt"), "본문").unwrap();
        assert_eq!(reader.read_file(&root.join("cat/a.txt"), None).len(), 1);
    }

    #[test]
    fn child_reader_reports_worker_failures_and_messages() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("a.txt");
        std::fs::write(&book, "본문").unwrap();
        let reader = Arc::new(LoaderFileReader::new(d.path().into(), d.path().into()).with_worker_executable("/bin/false".into()));
        let error = reader.try_read_file_with_timeout(book.clone(), None, false, Duration::from_secs(10)).unwrap_err();
        assert!(matches!(error, super::super::ReadFileError::Other(_)), "{error:?}");

        let script = d.path().join("worker.sh");
        std::fs::write(&script, "#!/bin/sh\necho '{\"item\":null,\"messages\":[\"boom\"]}'\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let reader = Arc::new(LoaderFileReader::new(d.path().into(), d.path().into()).with_worker_executable(script));
        assert!(reader.try_read_file_with_timeout(book, None, false, Duration::from_secs(10)).unwrap().is_empty());
    }
}
