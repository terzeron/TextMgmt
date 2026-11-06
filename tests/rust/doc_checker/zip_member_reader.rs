//! `doc_checker::zip_member_reader` 단위 테스트. 비텍스트 멤버를 UTF-8로 변환하지 않는지 `decode_calls`로 센다.
//! `decode_text`·`read_limited`처럼 `pub(crate)`인 항목은 소스 파일 안의 `#[cfg(test)] mod tests`가 검증한다.

#[path = "../shared/doc_support/mod.rs"]
mod support;
use crate::support::shared_fixtures::binary;
use doc_checker::epub::security_checker::scan as scan_into;
use doc_checker::zip_member_reader::{Limits, decode_calls};
use doc_checker::{FileReport, Format};
use std::path::Path;

fn scan_counting(path: &Path) -> (FileReport, usize) {
    let before = decode_calls();
    let mut r = FileReport::new(path, Some(Format::Epub));
    scan_into(path, &Limits::default(), &mut r);
    (r, decode_calls() - before)
}

#[test]
fn text_members_are_decoded() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    support::epub_with_chapter(&p, &support::xhtml("<p>본문</p>"));
    let (_, calls) = scan_counting(&p);
    assert!(calls >= 2, "xhtml와 opf는 변환해서 검사한다: {calls}");
}

#[test]
fn non_text_members_are_not_decoded() {
    let d = tempfile::tempdir().unwrap();
    let plain = d.path().join("plain.epub");
    support::epub_with_chapter(&plain, &support::xhtml("<p>본문</p>"));
    let (_, base_calls) = scan_counting(&plain);

    let with_media = d.path().join("media.epub");
    let jpg = binary(1 << 20, 1);
    let png = binary(1 << 19, 2);
    let ttf = binary(1 << 18, 3);
    support::epub_with_files(&with_media, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/a.jpg", &jpg), ("OEBPS/b.png", &png), ("OEBPS/c.ttf", &ttf), ("OEBPS/d.bin", &jpg)]);
    let (_, media_calls) = scan_counting(&with_media);
    assert_eq!(media_calls, base_calls, "이미지·폰트·알 수 없는 확장자 멤버는 UTF-8 변환 대상이 아니다");
}

#[test]
fn binary_member_checks_still_run_without_decoding() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("disguised.epub");
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    support::epub_with_files(&p, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/cover.jpg", &pe)]);
    let (r, _) = scan_counting(&p);
    let f = r.findings.iter().find(|f| f.code == "PE_EXECUTABLE_CONTENT").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!(f.severity, doc_checker::Severity::Critical);
}

#[test]
fn txt_members_are_not_decoded_either_because_other_kind_has_no_text_rules() {
    // Kind::Other의 txt에는 텍스트 규칙이 없다(scan_text가 아무것도 하지 않는다). 변환해도 결과가 달라지지 않는다.
    let d = tempfile::tempdir().unwrap();
    let plain = d.path().join("plain.epub");
    support::epub_with_chapter(&plain, &support::xhtml("<p>본문</p>"));
    let (_, base_calls) = scan_counting(&plain);
    let with_txt = d.path().join("txt.epub");
    support::epub_with_files(&with_txt, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/notes.txt", "소림사 장로\n".as_bytes())]);
    let (r, txt_calls) = scan_counting(&with_txt);
    assert_eq!(txt_calls, base_calls);
    assert!(!r.findings.iter().any(|f| f.location.contains("notes.txt")), "{:?}", r.findings);
}
