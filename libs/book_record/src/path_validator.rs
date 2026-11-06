//! 파일시스템 상태와 무관한 경로·파일 종류 규칙: href·ZIP 항목 경로 정리, 상대 경로, 최상위 카테고리 판정,
//! 확장자와 매직바이트로 보는 색인 대상 판정. 책 루트에 맞춘 검증은 `book_manager`가 맡는다.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::OnceLock;

use common::url_parser::unquote;
use common::zip_reader::zip_entry_name;
use sysdeps::FsOps;

/// 책 루트 자체를 가리키는 카테고리 이름.
pub const ROOT_CATEGORY: &str = "_root";

/// 색인 대상 확장자.
pub const INDEXABLE_FILE_TYPES: [&str; 17] = ["txt", "epub", "pdf", "docx", "doc", "hwp", "rtf", "html", "jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg", "cbz"];

fn indexable_set() -> &'static BTreeSet<&'static str> {
    static SET: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| INDEXABLE_FILE_TYPES.iter().copied().collect())
}

/// 이름만 보고 판정한 색인 대상 확장자인가.
pub fn is_indexable_type(file_type: &str) -> bool {
    indexable_set().contains(file_type)
}

/// 확장자 소문자(점 없음). 확장자가 없으면 빈 문자열.
pub fn declared_type(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, ext)) => ext.to_lowercase(),
        None => String::new(),
    }
}

/// 퍼센트 인코딩을 풀고 `#` 조각과 `?` 질의를 뗀 href.
pub fn clean_href(href: &str) -> String {
    let decoded = unquote(href);
    let no_fragment = decoded.split('#').next().unwrap_or("");
    no_fragment.split('?').next().unwrap_or("").to_string()
}

/// `dir` 기준 `href`의 ZIP 항목 이름. 압축 파일 밖을 가리키면 빈 문자열이라 어떤 항목과도 맞지 않는다.
pub fn zip_path(dir: &str, href: &str) -> String {
    zip_entry_name(Path::new(dir).join(href)).unwrap_or_default()
}

/// 상대 경로 문자열. 접두사를 못 떼면 `None`.
pub fn relative_to(path: &Path, prefix: &Path) -> Option<String> {
    path.strip_prefix(prefix).ok().map(|p| p.to_string_lossy().into_owned())
}

/// `_is_top_level_target_category`.
pub fn is_top_level_target_category(category: &str) -> bool {
    !category.is_empty() && category != ROOT_CATEGORY && !category.contains('/')
}

/// 심볼릭 링크를 따라가지 않고 보통 파일인가.
pub fn is_regular_file_nofollow(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// 심볼릭 링크를 따라가지 않고 디렉터리인가.
pub fn is_dir_nofollow(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// 파일 이름 조각.
pub fn file_name_of(path: &Path) -> OsString {
    path.file_name().map(|n| n.to_os_string()).unwrap_or_default()
}

/// 이 확장자 멤버만 든 ZIP은 만화(cbz)다.
const IMAGE_SUFFIXES: [&str; 6] = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp"];
/// ZIP 기반으로 선언된 형식. 선언이 이 안이면 내용물을 열어보지 않는다.
const ZIP_BASED_TYPES: [&str; 3] = ["epub", "cbz", "docx"];

/// ZIP 컨테이너의 실제 종류를 내용물로 판별한다.
fn zip_container_type<R: Read + Seek>(reader: R) -> Option<&'static str> {
    let mut zip = zip::ZipArchive::new(reader).ok()?;
    let names = common::zip_reader::zip_names(&mut zip);
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
