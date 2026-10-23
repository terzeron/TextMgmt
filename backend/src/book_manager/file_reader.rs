//! 파일 종류 판별과 기본 `FileReader`.
//!
//! `DefaultFileReader`는 `loader` crate 없이 메타데이터 문서만 만든다: 카테고리, 제목, 저자, 크기, 시각, 형식.
//! 본문 요약은 txt만 앞부분을 담고, 쪽수·줄 수·ISBN·PDF/EPUB/DOC 본문은 채우지 않는다.
//! 운영에서는 `loader::record::read_file`을 감싼 `FileReader`를 `BookManagerDeps::with_file_reader`로 꽂아야 한다.

use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde_json::{Value, json};
use sysdeps::FsOps;

use super::paths::{declared_type, is_indexable_type};
use super::util::{iso, relative_to};
use super::{Docs, FileReader};

/// ZIP 기반으로 선언된 형식. 선언이 이 안이면 내용물을 열어보지 않는다.
const ZIP_BASED_TYPES: [&str; 3] = ["epub", "cbz", "docx"];
const IMAGE_SUFFIXES: [&str; 6] = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp"];
/// 요약에 담는 본문 길이(문자).
const SUMMARY_CHARS: usize = 3500;

/// ZIP 컨테이너의 실제 종류를 내용물로 판별한다. `Loader._zip_container_type`.
fn zip_container_type<R: Read + Seek>(reader: R) -> Option<&'static str> {
    let mut zip = zip::ZipArchive::new(reader).ok()?;
    let names = super::epub::zip_names(&mut zip);
    if names.iter().any(|n| n.ends_with(".opf")) || names.iter().any(|n| n == "META-INF/container.xml") {
        return Some("epub");
    }
    if names.iter().any(|n| n.starts_with("word/")) {
        return Some("docx");
    }
    let entries: Vec<&String> = names.iter().filter(|n| !n.ends_with('/')).collect();
    if !entries.is_empty() && entries.iter().all(|n| IMAGE_SUFFIXES.iter().any(|s| n.to_lowercase().ends_with(s))) {
        return Some("cbz");
    }
    None
}

/// 매직바이트로 실제 형식을 판별한다. 판별 불가면 `None`. `Loader.detect_file_type`.
pub fn detect_file_type(fs: &dyn FsOps, path: &Path, declared: &str) -> Option<String> {
    let mut file = fs.open(path).ok()?;
    let mut head = [0u8; 4];
    file.read_exact(&mut head).ok()?;
    if &head == b"%PDF" {
        return Some("pdf".to_string());
    }
    if &head == b"PK\x03\x04" {
        if ZIP_BASED_TYPES.contains(&declared) {
            return Some(declared.to_string());
        }
        file.seek(SeekFrom::Start(0)).ok()?;
        return zip_container_type(file).map(str::to_string);
    }
    None
}

/// `[저자] 제목` 꼴이면 둘로 나눈다. `loader::record::split_author_title`와 같다.
fn split_author_title(stem: &str) -> (String, String) {
    if let Some(rest) = stem.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        let author = &rest[..end];
        let title = rest[end + 1..].trim_start_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c));
        if !author.is_empty() && !title.is_empty() && !title.contains('\n') && !author.contains('\n') {
            return (author.to_string(), title.to_string());
        }
    }
    (String::new(), stem.to_string())
}

fn local_naive(time: std::time::SystemTime) -> NaiveDateTime {
    chrono::DateTime::<Local>::from(time).naive_local()
}

/// 기본 `FileReader`. 설명은 모듈 머리말을 본다.
pub struct DefaultFileReader {
    path_prefix: PathBuf,
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
        let (author, title) = split_author_title(&stem);
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
