//! EPUB OPF 메타데이터(title, author, subject, description) 읽기. `epubveri` 보고서가 출판 정보를 채울 때 쓴다.
//!
//! OPF는 신뢰할 수 없는 입력으로 다룬다: DTD를 읽지 않고(엔티티 폭탄·외부 엔티티 차단), 크기 상한을 두고,
//! 깨진 XML은 태그 구조를 복구해 한 번 더 읽는다.

use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

/// 읽을 OPF 크기 상한.
pub const MAX_OPF_BYTES: usize = 8 * 1024 * 1024;
/// 이보다 멤버가 많은 ZIP은 읽지 않는다.
const MAX_ZIP_ENTRIES: usize = 100_000;

static DOCTYPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!DOCTYPE[^>\[]*(\[.*?\])?\s*>").expect("정규식"));
static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&[A-Za-z_][\w.-]*;").expect("정규식"));

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

/// 잘린 태그를 닫고 서로 맞지 않는 종료 태그를 앞선 열린 태그에 맞춘다.
fn recover_xml_structure(xml: &str) -> String {
    let mut output = String::with_capacity(xml.len() + 64);
    let mut open = Vec::<String>::new();
    let mut cursor = 0;
    while cursor < xml.len() {
        let Some(relative) = xml[cursor..].find('<') else { break };
        let start = cursor + relative;
        output.push_str(&xml[cursor..start]);

        let special_end = if xml[start..].starts_with("<!--") {
            Some(("-->", 4))
        } else if xml[start..].starts_with("<![CDATA[") {
            Some(("]]>", 9))
        } else if xml[start..].starts_with("<?") {
            Some(("?>", 2))
        } else {
            None
        };
        if let Some((terminator, prefix_len)) = special_end {
            if let Some(end) = xml[start + prefix_len..].find(terminator) {
                let end = start + prefix_len + end + terminator.len();
                output.push_str(&xml[start..end]);
                cursor = end;
                continue;
            }
            cursor = xml.len();
            break;
        }

        let bytes = xml.as_bytes();
        let mut quote = None;
        let mut end = None;
        for (index, byte) in bytes.iter().enumerate().skip(start + 1) {
            match (*byte, quote) {
                (b'\'' | b'"', None) => quote = Some(*byte),
                (b'\'' | b'"', Some(current)) if *byte == current => quote = None,
                (b'>', None) => {
                    end = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            cursor = xml.len();
            break;
        };
        let token = &xml[start..=end];
        let body = token[1..token.len() - 1].trim();
        if let Some(closing) = body.strip_prefix('/') {
            let name = closing.split_whitespace().next().unwrap_or("");
            if let Some(position) = open.iter().rposition(|tag| tag == name) {
                while open.len() > position + 1 {
                    if let Some(tag) = open.pop() {
                        output.push_str(&format!("</{tag}>"));
                    }
                }
                open.pop();
                output.push_str(token);
            }
        } else if body.starts_with('!') {
            // DTD는 읽지 않는다. 호출자가 제거하지 못한 선언도 파서로 넘기지 않는다.
        } else {
            output.push_str(token);
            if !body.ends_with('/') {
                let name = body.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
                if !name.is_empty() {
                    open.push(name.to_owned());
                }
            }
        }
        cursor = end + 1;
    }
    output.push_str(&xml[cursor..]);
    while let Some(tag) = open.pop() {
        output.push_str(&format!("</{tag}>"));
    }
    output
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
            let doctype = &*DOCTYPE_RE;
            let entity = &*ENTITY_RE;
            let no_doctype = doctype.replace_all(xml, "");
            cleaned =
                entity.replace_all(&no_doctype, |m: &regex::Captures| if matches!(&m[0], "&amp;" | "&lt;" | "&gt;" | "&quot;" | "&apos;") { m[0].to_string() } else { String::new() }).into_owned();
            match roxmltree::Document::parse(&cleaned) {
                Ok(d) => d,
                Err(_) => {
                    recovered = recover_xml_structure(&cleaned);
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
    let Some(mut zip) = sysdeps::zip_opener::open_zip(path) else { return meta };
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
