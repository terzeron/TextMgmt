//! HWPX 검사. ZIP 안의 OWPML 패키지(`mimetype`, `Contents/content.hpf`, `Contents/section*.xml`, `BinData/`)를 본다.
//! 구조와 내장 실행 파일·OLE, XML 공격을 찾는다. 스크립트·매크로의 저장 위치를 확인하지 못해 스크립트 탐지는 하지 않는다(docs/bookdoctor-hwpx-notes.md).

use std::collections::HashSet;
use std::path::Path;

use corpus_common::reader::open_zip;
use corpus_common::util::unquote;
use scan_common::{Category, FileReport, Severity};

use super::ooxml::{check_xml_attacks, is_xml_part, parse, preview, push_once, read_xml_part};
use crate::epub::security::{Limits, read_limited};

const MANIFEST: &str = "Contents/content.hpf";
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
/// 검사하는 XML 부분의 상한(개수).
const MAX_PARTS_SCANNED: usize = 256;

/// 매니페스트가 가리키는 섹션 파일이 ZIP에 있는지 본다. `href`는 루트 기준이 관례지만 `Contents/` 기준도 받아들인다.
fn check_manifest(text: &str, set: &HashSet<&str>, report: &mut FileReport) {
    let Some(doc) = parse(text) else {
        report.push(Category::Render, Severity::High, "MALFORMED_MARKUP", MANIFEST, "XML이 올바르지 않다");
        return;
    };
    for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "item") {
        let href = unquote(item.attribute("href").unwrap_or(""));
        let base = href.rsplit('/').next().unwrap_or("");
        if href.contains("://") || !(base.starts_with("section") && base.ends_with(".xml")) {
            continue;
        }
        let rooted = format!("Contents/{href}");
        if !set.contains(href.as_str()) && !set.contains(rooted.as_str()) {
            push_render(report, Severity::High, "HWPX_SECTION_MISSING", &href, "매니페스트가 가리키는 섹션이 ZIP에 없다");
        }
    }
}

fn push_render(report: &mut FileReport, severity: Severity, code: &str, location: &str, detail: &str) {
    if !report.findings.iter().any(|f| f.code == code && f.location == location) {
        report.push(Category::Render, severity, code, &preview(location), detail);
    }
}

/// HWPX 하나를 검사해 `report`에 더한다. ZIP으로 열리지 않으면 아무것도 하지 않는다(빠른 점검이 `NOT_ZIP`을 낸다).
pub fn inspect(path: &Path, limits: &Limits, report: &mut FileReport) {
    let Some(mut zip) = open_zip(path) else { return };
    let names: Vec<String> = zip.file_names().map(|r| r.map(|c| c.into_owned()).unwrap_or_else(|_| "<invalid-name>".into())).collect();
    let set: HashSet<&str> = names.iter().map(String::as_str).collect();

    // mimetype은 없어도 허용한다. 있는데 hwp·owpml이 아니면 다른 종류의 파일이다.
    if let Some(i) = names.iter().position(|n| n == "mimetype")
        && let Ok(data) = read_limited(&mut zip, i, 256)
    {
        let lower = String::from_utf8_lossy(&data).to_lowercase();
        if !(lower.contains("hwp") || lower.contains("owpml")) {
            push_render(report, Severity::Medium, "HWPX_BAD_MIMETYPE", "mimetype", "mimetype에 hwp·owpml이 없다");
        }
    }
    if !set.contains(MANIFEST) {
        push_render(report, Severity::High, "HWPX_NO_MANIFEST", MANIFEST, "패키지 매니페스트가 없다");
    }

    // BinData: 내장 OLE 객체. 실행 파일은 빠른 점검이 이름·매직으로 이미 본다.
    for (i, name) in names.iter().enumerate().filter(|(_, n)| n.to_lowercase().starts_with("bindata/")) {
        let ole_name = name.to_lowercase().ends_with(".ole");
        let ole_magic = !ole_name && read_limited(&mut zip, i, OLE_MAGIC.len() as u64).is_ok_and(|d| d.starts_with(&OLE_MAGIC));
        if ole_name || ole_magic {
            push_once(report, Severity::High, "HWPX_OLE_OBJECT", &preview(name), "Embedded OLE object");
        }
    }

    // XML 부분: 파싱, 매니페스트 확인, DTD·엔티티 공격. 읽기는 남은 예산에서 멈춘다.
    let mut spent = 0u64;
    let mut scanned = 0usize;
    for (index, name) in names.iter().enumerate() {
        // 이름의 대소문자와 디렉터리를 가리지 않고 모든 XML 부분을 본다(OPC 부분 이름은 대소문자를 가리지 않는다).
        if !is_xml_part(name) {
            continue;
        }
        scanned += 1;
        if scanned > MAX_PARTS_SCANNED {
            report.mark_incomplete(&format!("scan stopped after {MAX_PARTS_SCANNED} XML parts"));
            break;
        }
        let Some(text) = read_xml_part(&mut zip, index, name, limits, &mut spent, report) else { continue };
        check_xml_attacks(&text, name, report);
        if name == MANIFEST {
            check_manifest(&text, &set, report);
        } else if parse(&text).is_none() {
            push_render(report, Severity::High, "MALFORMED_MARKUP", name, "XML이 올바르지 않다");
        }
    }
}
