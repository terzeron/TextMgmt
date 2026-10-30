//! OOXML(DOCX 계열) 검사. 구조(필수 부분, XML 파싱, 선언된 부분의 존재)를 본다.
//! 압축 해제는 `epub::security::read_limited`로 상한에서 멈춘다.

use std::collections::HashSet;
use std::path::Path;

use corpus_common::reader::open_zip;
use scan_common::{Category, FileReport, Severity};

use crate::epub::security::{Limits, read_limited};

/// 이게 없으면 Word 문서가 아니다.
const REQUIRED_PARTS: [&str; 3] = ["[Content_Types].xml", "_rels/.rels", "word/document.xml"];

/// 부분 하나를 상한 안에서 읽는다. 상한을 넘으면 `None`이고 미완료로 남긴다. 읽을 수 없으면 `None`이다(해제 실패는 빠른 점검이 이미 알렸다).
fn read_part(zip: &mut corpus_common::reader::Zip, index: usize, name: &str, limits: &Limits, report: &mut FileReport) -> Option<Vec<u8>> {
    let data = read_limited(zip, index, limits.max_member).ok()?;
    if data.len() as u64 > limits.max_member {
        report.mark_incomplete(&format!("{name}: part larger than max inspection size"));
        return None;
    }
    Some(data)
}

/// DOCX의 구조를 검사해 `report`에 더한다. ZIP으로 열리지 않으면 아무것도 하지 않는다(빠른 점검이 `NOT_ZIP`을 낸다).
pub fn inspect_structure(path: &Path, limits: &Limits, report: &mut FileReport) {
    let Some(mut zip) = open_zip(path) else { return };
    let names: Vec<String> = zip.file_names().map(|r| r.map(|c| c.into_owned()).unwrap_or_else(|_| "<invalid-name>".into())).collect();
    let set: HashSet<&str> = names.iter().map(String::as_str).collect();
    for part in REQUIRED_PARTS {
        if !set.contains(part) {
            report.push(Category::Render, Severity::High, "OOXML_NOT_DOCX", part, "필수 부분이 없다");
        }
    }
    for part in REQUIRED_PARTS {
        let Some(index) = names.iter().position(|n| n == part) else { continue };
        let Some(data) = read_part(&mut zip, index, part, limits, report) else { continue };
        let text = String::from_utf8_lossy(&data);
        let parsed = roxmltree::Document::parse_with_options(text.strip_prefix('\u{feff}').unwrap_or(&text), roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() });
        match parsed {
            Err(e) => report.push(Category::Render, Severity::High, "MALFORMED_MARKUP", part, &format!("XML이 올바르지 않다: {e}")),
            Ok(doc) if part == "[Content_Types].xml" => {
                for node in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Override") {
                    let Some(declared) = node.attribute("PartName") else { continue };
                    let declared = declared.trim_start_matches('/');
                    if !set.contains(declared) {
                        report.push(Category::Render, Severity::Medium, "OOXML_PART_MISSING", declared, "[Content_Types].xml이 선언한 부분이 ZIP에 없다");
                    }
                }
            }
            Ok(_) => {}
        };
    }
}
