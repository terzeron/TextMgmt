//! EPUB OPF 메타데이터(title, author, subject, description) 읽기. `epubveri` 보고서가 출판 정보를 채울 때 쓴다.
//!
//! OPF는 신뢰할 수 없는 입력으로 다룬다: DTD를 읽지 않고(엔티티 폭탄·외부 엔티티 차단), 크기 상한을 두고,
//! 깨진 XML은 태그 구조를 복구해 한 번 더 읽는다.

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};
use xml_recoverer::{close_open_tags, strip_doctype, strip_unknown_entities};

/// 읽을 OPF 크기 상한.
pub const MAX_OPF_BYTES: usize = 8 * 1024 * 1024;
/// 이보다 멤버가 많은 ZIP은 읽지 않는다.
const MAX_ZIP_ENTRIES: usize = 100_000;

/// EPUB 의 OPF 에서 읽은 메타데이터.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpubMetadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub description: String,
}

impl PartialEq<Value> for EpubMetadata {
    fn eq(&self, other: &Value) -> bool {
        *other == json!({"title": self.title, "author": self.author, "subject": self.subject, "description": self.description})
    }
}

fn parse_opf(xml: &str, meta: &mut EpubMetadata) -> bool {
    // DTD 는 읽지 않는다. 엔티티 폭탄(CWE-776)과 외부 엔티티(CWE-611)를 둘 다 이 한 줄로 막는다.
    let strict = roxmltree::Document::parse(xml);
    let cleaned;
    let recovered;
    let doc = match strict {
        Ok(d) => d,
        Err(_) => {
            // 복구: DOCTYPE 과 알 수 없는 엔티티 참조를 걷어내고 한 번 더 읽는다. 엔티티 내용은 읽지 않는다.
            cleaned = strip_unknown_entities(&strip_doctype(xml));
            match roxmltree::Document::parse(&cleaned) {
                Ok(d) => d,
                Err(_) => {
                    recovered = close_open_tags(&cleaned);
                    match roxmltree::Document::parse(&recovered) {
                        Ok(d) => d,
                        Err(_) => return false,
                    }
                }
            }
        }
    };
    for node in doc.descendants().filter(|n| n.is_element()) {
        let tag = node.tag_name().name().to_lowercase();
        let text = node.first_child().filter(|c| c.is_text()).and_then(|c| c.text()).filter(|t| !t.is_empty());
        let Some(text) = text else { continue };
        let text = text.trim();
        match tag.as_str() {
            "title" if meta.title.is_empty() => meta.title = text.to_string(),
            "creator" | "author" if meta.author.is_empty() => meta.author = text.to_string(),
            "subject" => {
                if meta.subject.is_empty() {
                    meta.subject = text.to_string();
                } else {
                    meta.subject = format!("{} > {text}", meta.subject);
                }
            }
            "description" if meta.description.is_empty() => meta.description = text.to_string(),
            _ => {}
        }
    }
    true
}

/// EPUB 파일 내부의 `.opf` 메타데이터(title, author, subject, description)를 읽는다. 못 읽으면 빈 값이다.
pub fn inspect_epub_metadata(path: &Path) -> EpubMetadata {
    let meta = EpubMetadata::default();
    let Some(mut zip) = common::zip_reader::open_zip(path) else { return meta };
    if zip.len() > MAX_ZIP_ENTRIES {
        return meta;
    }
    let Some(index) = zip.file_names().enumerate().find(|(_, n)| n.as_ref().is_ok_and(|n| n.ends_with(".opf"))).map(|(i, _)| i) else { return meta };
    let mut raw = Vec::new();
    match zip.by_index(index) {
        Ok(entry) => {
            if entry.size() > MAX_OPF_BYTES as u64 {
                log::warn!("OPF too large, skipping metadata ({} bytes)", entry.size());
                return meta;
            }
            // 선언 크기를 속인 항목도 상한 이상은 읽지 않는다.
            if entry.take(MAX_OPF_BYTES as u64 + 1).read_to_end(&mut raw).is_err() || raw.len() > MAX_OPF_BYTES {
                return meta;
            }
        }
        Err(_) => return meta,
    }
    let text = String::from_utf8_lossy(&raw);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut found = EpubMetadata::default();
    if parse_opf(text, &mut found) { found } else { meta }
}
