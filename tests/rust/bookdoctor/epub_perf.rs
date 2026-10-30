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

// --- 렌더링 단계의 중복 압축 해제 제거 ---

use std::collections::HashMap;

use bookdoctor::cli::Options;
use bookdoctor::epub::render::{FULL_READS, inspect_with, inspect_with_verified};
use bookdoctor::epub::scan_epub_file;
use bookdoctor::epub::security::scan_verified;

/// 같은 보안 단계 결과 위에서 렌더링을 새 방식(검증된 멤버 건너뜀)과 옛 방식으로 돌려 결과를 견준다.
fn render_both_ways(path: &Path, limits: &Limits) -> (FileReport, FileReport, HashMap<usize, u64>) {
    let mut base = FileReport::new(path, Some(Format::Epub));
    let (_, verified) = scan_verified(path, limits, &mut base);
    let mut new = base.clone();
    inspect_with_verified(path, &mut new, limits, &verified);
    let mut old = base.clone();
    inspect_with(path, &mut old, limits);
    (new, old, verified)
}

fn assert_same(new: &FileReport, old: &FileReport) {
    assert_eq!(new.findings, old.findings);
    assert_eq!(new.reasons, old.reasons);
    assert_eq!(new.incomplete, old.incomplete);
}

fn epub_with_media(path: &Path) {
    let jpg = binary(300_000, 1);
    let png = binary(200_000, 2);
    common::epub_with_files(path, &common::clean_opf(), &common::xhtml("<p>본문</p>"), &[("OEBPS/a.jpg", &jpg), ("OEBPS/b.png", &png), ("OEBPS/c.ttf", &jpg)]);
}

#[test]
fn render_skips_members_the_security_pass_already_read_to_the_end() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = tempfile::tempdir().unwrap();
    let plain = d.path().join("plain.epub");
    common::epub_with_chapter(&plain, &common::xhtml("<p>본문</p>"));
    let media = d.path().join("media.epub");
    epub_with_media(&media);
    let count = |p: &Path| {
        let before = FULL_READS.load(Ordering::SeqCst);
        scan_epub_file(p, &Options::default(), &mut FileReport::new(p, Some(Format::Epub)));
        FULL_READS.load(Ordering::SeqCst) - before
    };
    assert_eq!(count(&media), count(&plain), "이미지·폰트 멤버는 보안 단계가 이미 끝까지 읽었으므로 렌더링이 다시 읽지 않는다");
}

#[test]
fn skipping_verified_members_does_not_change_render_results() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("media.epub");
    epub_with_media(&p);
    let (new, old, verified) = render_both_ways(&p, &Limits::default());
    assert_same(&new, &old);
    assert!(verified.len() >= 3, "{verified:?}");
}

#[test]
fn corrupt_member_is_still_reported_by_the_render_pass() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("crc.epub");
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let jpg: Vec<u8> = (0..50_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    common::epub_with_files(&p, &common::clean_opf(), &common::xhtml("<p>본문</p>"), &[("OEBPS/a.jpg", &jpg)]);
    // 압축되지 않는 데이터라 본문 한가운데를 깨뜨려도 스트림은 풀리고 CRC만 틀린다.
    let mut b = std::fs::read(&p).unwrap();
    let at = common::data_offset(&b, "OEBPS/a.jpg");
    b[at + 20_000] ^= 0xFF;
    std::fs::write(&p, b).unwrap();
    let (new, old, verified) = render_both_ways(&p, &Limits::default());
    assert_same(&new, &old);
    assert!(new.has("ZIP_CRC_MISMATCH"), "{:?}", new.findings);
    // 손상 멤버는 보안 단계의 읽기가 실패하므로 검증 목록에 없다(그래서 렌더링이 읽어 보고한다).
    let names = ["mimetype", "META-INF/container.xml", "OEBPS/content.opf", "OEBPS/ch1.xhtml", "OEBPS/a.jpg"];
    assert!(!verified.contains_key(&names.iter().position(|n| *n == "OEBPS/a.jpg").unwrap()), "{verified:?}");
}

#[test]
fn budget_exhaustion_is_judged_the_same_with_or_without_skipping() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("big.epub");
    epub_with_media(&p);
    for max_total in [200_000u64, 450_000, 600_000, 2_000_000] {
        let limits = Limits { max_total, ..Limits::default() };
        let (new, old, _) = render_both_ways(&p, &limits);
        assert_same(&new, &old);
    }
}

