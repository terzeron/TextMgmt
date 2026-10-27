//! DOCX 읽기. 원본: `Loader.read_from_docx`(python-docx `Document(path).paragraphs`).
//!
//! 본문(`w:body`)의 바로 아래 문단만 센다(표 안 문단은 제외). 문단 글은 `w:r`과 `w:hyperlink`의 런 글을 이은 것이다.

use std::io::Read;
use std::path::Path;

use corpus_common::reader::open_zip;

use crate::TEXT_SIZE;
use crate::pytext::{head_chars, keep_word_chars};

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn is_w(node: roxmltree::Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(W_NS)
}

/// 런 하나의 글: `w:t`, `w:tab`, `w:br`(줄바꿈이면 `\n`), `w:cr`, `w:noBreakHyphen`, `w:ptab`.
fn run_text(run: roxmltree::Node) -> String {
    let mut out = String::new();
    for e in run.children().filter(|c| c.is_element() && c.tag_name().namespace() == Some(W_NS)) {
        match e.tag_name().name() {
            "t" => out.push_str(e.text().unwrap_or("")),
            "tab" | "ptab" => out.push('\t'),
            "br" => {
                let kind = e.attribute((W_NS, "type")).unwrap_or("textWrapping");
                if kind == "textWrapping" {
                    out.push('\n');
                }
            }
            "cr" => out.push('\n'),
            "noBreakHyphen" => out.push('-'),
            _ => {}
        }
    }
    out
}

/// 문단 글: 직계 `w:r`과 `w:hyperlink`(그 안의 `w:r`)를 문서 순서대로 잇는다.
pub fn paragraph_text(p: roxmltree::Node) -> String {
    let mut out = String::new();
    for e in p.children().filter(|c| c.is_element()) {
        if is_w(e, "r") {
            out.push_str(&run_text(e));
        } else if is_w(e, "hyperlink") {
            for r in e.children().filter(|c| is_w(*c, "r")) {
                out.push_str(&run_text(r));
            }
        }
    }
    out
}

fn read_member(zip: &mut corpus_common::reader::Zip, name: &str) -> Result<String, String> {
    let mut file = zip.by_name(name).map_err(|_| format!("KeyError: {name}"))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    String::from_utf8(buf.strip_prefix(b"\xef\xbb\xbf").map_or(buf.clone(), <[u8]>::to_vec)).map_err(|e| e.to_string())
}

/// 주 문서 파트 이름. `_rels/.rels`의 officeDocument 관계를 따르고, 없으면 `word/document.xml`.
fn main_part(zip: &mut corpus_common::reader::Zip) -> Result<String, String> {
    let rels = read_member(zip, "_rels/.rels")?;
    let doc = roxmltree::Document::parse(&rels).map_err(|e| format!("XML 해석 실패: {e}"))?;
    let target = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Relationship" && n.tag_name().namespace() == Some(REL_NS) && n.attribute("Type").is_some_and(|t| t.ends_with("/officeDocument")))
        .and_then(|n| n.attribute("Target"));
    Ok(target.map_or_else(|| "word/document.xml".to_owned(), |t| t.trim_start_matches('/').to_owned()))
}

/// DOCX 파일: (요약, 줄 수). 패키지를 열 수 없으면 `Err`.
pub fn read_from_docx(path: &Path) -> Result<(String, usize), String> {
    let mut zip = open_zip(path).ok_or("Package not found / bad zip file")?;
    let part = main_part(&mut zip)?;
    let xml = read_member(&mut zip, &part)?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| format!("XML 해석 실패: {e}"))?;
    let body = doc.root_element().children().find(|c| is_w(*c, "body")).ok_or("문서 본문(w:body)이 없다")?;

    let mut result = String::new();
    let mut total_text = String::new();
    for p in body.children().filter(|c| is_w(*c, "p")) {
        let text = paragraph_text(p);
        total_text.push_str(&text);
        total_text.push('\n');
        if result.chars().count() < TEXT_SIZE {
            result.push_str(&text);
        }
    }
    let line_count = if total_text.is_empty() { 0 } else { total_text.matches('\n').count() };
    let cleaned = keep_word_chars(&result);
    Ok((head_chars(&cleaned, TEXT_SIZE).to_owned(), line_count))
}
