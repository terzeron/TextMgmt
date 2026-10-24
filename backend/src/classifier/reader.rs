//! 파일에서 본문과 메타데이터를 읽는다. Python `backend/classifier/reader.py` 대응.
//!
//! 본문 샘플 읽기, 인코딩 판별, ZIP 열기는 `corpus_common` 에 이미 있어 그대로 쓴다. 여기에는 그 crate 에 없는
//! `read_epub_publisher` 만 새로 둔다. EPUB(ZIP)은 신뢰할 수 없는 입력이라 항목 수와 OPF 크기에 상한을 둔다.

use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

pub use corpus_common::reader::{TEXT_HEAD_CHARS, TEXT_TAIL_CHARS, decode_best, korean_likeness, read_document_text, read_epub_text, read_txt_text};
use corpus_common::reader::{MAX_MEMBER_BYTES, open_zip};

/// 훑어 볼 ZIP 항목 수 상한. 정상 EPUB 은 수백 개다.
pub const MAX_ZIP_ENTRIES: usize = 100_000;

static DC_PUBLISHER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<dc:publisher[^>]*>(.*?)</dc:publisher>").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

/// EPUB 의 `dc:publisher` 를 읽는다. 본문은 안 읽고 OPF 하나만 푼다. 못 읽으면 빈 문자열이다.
pub fn read_epub_publisher(path: &Path) -> String {
    let Some(mut zip) = open_zip(path) else {
        log::debug!("OPF 를 읽지 못했다: ZIP 이 아니다");
        return String::new();
    };
    if zip.len() > MAX_ZIP_ENTRIES {
        log::debug!("OPF 를 읽지 못했다: 항목이 너무 많다 ({})", zip.len());
        return String::new();
    }
    let opf = zip.file_names().enumerate().find(|(_, n)| n.as_ref().is_ok_and(|n| n.to_lowercase().ends_with(".opf"))).map(|(i, _)| i);
    let Some(index) = opf else { return String::new() };
    let mut raw = Vec::new();
    match zip.by_index(index) {
        // 선언 크기가 상한을 넘으면 읽지 않는다. 그보다 작게 선언하고 더 풀리는 경우는 take 가 막는다.
        Ok(entry) if entry.size() <= MAX_MEMBER_BYTES => {
            if entry.take(MAX_MEMBER_BYTES).read_to_end(&mut raw).is_err() {
                return String::new();
            }
        }
        _ => return String::new(),
    }
    let xml = String::from_utf8_lossy(&raw);
    let values: Vec<String> = DC_PUBLISHER.captures_iter(&xml).map(|m| TAG.replace_all(&m[1], "").trim().to_string()).filter(|v| !v.is_empty()).collect();
    values.join(" ")
}
