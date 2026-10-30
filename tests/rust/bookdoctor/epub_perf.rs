//! EPUB 성능 최적화 테스트: 결과는 그대로이고 낭비만 줄었는지 센다.

mod common;

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use bookdoctor::epub::security::{DECODE_CALLS, Limits, scan};
use scan_common::{FileReport, Format};

/// `DECODE_CALLS`는 프로세스 전체 카운터라서 읽는 테스트끼리 순서를 세운다.
static SERIAL: Mutex<()> = Mutex::new(());

fn scan_counting(path: &Path) -> (FileReport, usize) {
    let before = DECODE_CALLS.load(Ordering::SeqCst);
    let mut r = FileReport::new(path, Some(Format::Epub));
    scan(path, &Limits::default(), &mut r);
    (r, DECODE_CALLS.load(Ordering::SeqCst) - before)
}

fn binary(len: usize, seed: u8) -> Vec<u8> {
    // UTF-8로 유효하지 않은 바이트가 많이 든 데이터(이미지·폰트처럼).
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) | 0x80).collect()
}

#[test]
fn text_members_are_decoded() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    common::epub_with_chapter(&p, &common::xhtml("<p>본문</p>"));
    let (_, calls) = scan_counting(&p);
    assert!(calls >= 2, "xhtml와 opf는 변환해서 검사한다: {calls}");
}

#[test]
fn non_text_members_are_not_decoded() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = tempfile::tempdir().unwrap();
    let plain = d.path().join("plain.epub");
    common::epub_with_chapter(&plain, &common::xhtml("<p>본문</p>"));
    let (_, base_calls) = scan_counting(&plain);

    let with_media = d.path().join("media.epub");
    let jpg = binary(1 << 20, 1);
    let png = binary(1 << 19, 2);
    let ttf = binary(1 << 18, 3);
    common::epub_with_files(&with_media, &common::clean_opf(), &common::xhtml("<p>본문</p>"), &[("OEBPS/a.jpg", &jpg), ("OEBPS/b.png", &png), ("OEBPS/c.ttf", &ttf), ("OEBPS/d.bin", &jpg)]);
    let (_, media_calls) = scan_counting(&with_media);
    assert_eq!(media_calls, base_calls, "이미지·폰트·알 수 없는 확장자 멤버는 UTF-8 변환 대상이 아니다");
}

#[test]
fn binary_member_checks_still_run_without_decoding() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("disguised.epub");
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    common::epub_with_files(&p, &common::clean_opf(), &common::xhtml("<p>본문</p>"), &[("OEBPS/cover.jpg", &pe)]);
    let (r, _) = scan_counting(&p);
    let f = r.findings.iter().find(|f| f.code == "PE_EXECUTABLE_CONTENT").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!(f.severity, scan_common::Severity::Critical);
}

#[test]
fn txt_members_are_not_decoded_either_because_other_kind_has_no_text_rules() {
    // Kind::Other의 txt에는 텍스트 규칙이 없다(scan_text가 아무것도 하지 않는다). 변환해도 결과가 달라지지 않는다.
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = tempfile::tempdir().unwrap();
    let plain = d.path().join("plain.epub");
    common::epub_with_chapter(&plain, &common::xhtml("<p>본문</p>"));
    let (_, base_calls) = scan_counting(&plain);
    let with_txt = d.path().join("txt.epub");
    common::epub_with_files(&with_txt, &common::clean_opf(), &common::xhtml("<p>본문</p>"), &[("OEBPS/notes.txt", "소림사 장로\n".as_bytes())]);
    let (r, txt_calls) = scan_counting(&with_txt);
    assert_eq!(txt_calls, base_calls);
    assert!(!r.findings.iter().any(|f| f.location.contains("notes.txt")), "{:?}", r.findings);
}
