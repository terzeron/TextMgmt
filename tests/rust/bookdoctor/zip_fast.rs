//! 범용 ZIP 빠른 점검 테스트: 열 수 있는지, 멤버를 풀 수 있는지, 이름·압축비·내장 실행 파일을 본다.

mod common;

use std::fs;
use std::path::Path;

use bookdoctor::cli::{Options, parse, print_usage, run};
use bookdoctor::scan::scan_file;
use scan_common::{Category, FileReport, Format, Severity};

fn scan(path: &Path) -> FileReport {
    scan_file(path, &Options::default())
}

fn deep(path: &Path) -> FileReport {
    scan_file(path, &Options { zip_deep: true, ..Options::default() })
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a scan_common::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

fn text(n: usize) -> Vec<u8> {
    "소림사 장로가 강호에서 내공을 운기조식하며 화산파 검법을 수련했다.\n".repeat(n).into_bytes()
}

#[test]
fn valid_zip_has_no_findings() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("ok.zip");
    common::zip_of(&p, &[("a.txt", &text(50)), ("dir/b.txt", &text(10))]);
    let r = scan(&p);
    assert_eq!(r.format, Some(Format::Zip));
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn plain_text_with_zip_extension_is_not_a_zip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("fake.zip");
    fs::write(&p, text(20)).unwrap();
    let f = scan(&p);
    let f = finding(&f, "NOT_ZIP");
    assert_eq!((f.category, f.severity), (Category::Defect, Severity::High));
}

#[test]
fn entry_with_a_corrupt_deflate_stream_is_found_by_the_probe_without_deep() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("bad.zip");
    common::zip_of(&p, &[("good.txt", &text(30)), ("bad.txt", &text(400))]);
    let mut b = fs::read(&p).unwrap();
    let at = common::data_offset(&b, "bad.txt");
    b[at] = 0xFF; // BFINAL=1, BTYPE=11(예약): 풀 수 없는 deflate 블록
    fs::write(&p, b).unwrap();
    let r = scan(&p);
    let f = finding(&r, "ZIP_ENTRY_UNREADABLE");
    assert_eq!((f.category, f.severity, f.location.as_str()), (Category::Defect, Severity::High, "bad.txt"));
    assert!(!r.findings.iter().any(|f| f.location == "good.txt"));
}

#[test]
fn crc_mismatch_beyond_the_probe_is_only_found_with_zip_deep() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("crc.zip");
    let body = vec![b'x'; 8192];
    common::zip_stored(&p, &[("data.bin", &body)]);
    let mut b = fs::read(&p).unwrap();
    let at = common::data_offset(&b, "data.bin");
    b[at + 8191] ^= 0xFF; // 마지막 바이트: 앞 4096바이트 시험 밖
    fs::write(&p, b).unwrap();
    assert!(!scan(&p).has("ZIP_CRC_MISMATCH") && !scan(&p).has("ZIP_ENTRY_UNREADABLE"), "probe must stay shallow");
    let f = deep(&p);
    let f = finding(&f, "ZIP_CRC_MISMATCH");
    assert_eq!((f.category, f.severity, f.location.as_str()), (Category::Defect, Severity::High, "data.bin"));
}

#[test]
fn small_member_crc_mismatch_is_found_even_without_deep() {
    // 4096바이트 이하는 시험이 끝까지 읽으므로 CRC도 확인된다.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("small.zip");
    common::zip_stored(&p, &[("s.bin", &[b'y'; 100])]);
    let mut b = fs::read(&p).unwrap();
    let at = common::data_offset(&b, "s.bin");
    b[at + 50] ^= 0xFF;
    fs::write(&p, b).unwrap();
    assert!(scan(&p).has("ZIP_CRC_MISMATCH"));
}

#[test]
fn zip_slip_is_critical_and_named_after_the_member() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slip.zip");
    common::zip_of(&p, &[("../../evil.sh", b"echo hi\n"), ("ok.txt", b"ok")]);
    let r = scan(&p);
    let f = finding(&r, "ZIP_PATH_TRAVERSAL");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::Critical));
    assert!(f.location.contains("evil.sh"));
}

#[test]
fn highly_compressed_member_is_flagged_without_reading_it_all() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("bomb.zip");
    common::zip_of(&p, &[("zeros.bin", &vec![0u8; 8 * 1024 * 1024])]);
    let r = scan(&p);
    assert_eq!(finding(&r, "HIGH_COMPRESSION_RATIO").severity, Severity::Medium);
}

#[test]
fn forged_small_declared_size_does_not_make_deep_scan_read_everything() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("forged.zip");
    common::zip_of(&p, &[("zeros.bin", &vec![0u8; 32 * 1024 * 1024])]);
    let mut b = fs::read(&p).unwrap();
    // 중앙 디렉터리의 비압축 크기(오프셋 24)를 10으로 속인다.
    let central = (0..b.len() - 46).find(|&i| &b[i..i + 4] == b"PK\x01\x02").unwrap();
    b[central + 24..central + 28].copy_from_slice(&10u32.to_le_bytes());
    fs::write(&p, b).unwrap();
    let r = scan_file(&p, &Options { zip_deep: true, max_total_mb: 2, ..Options::default() });
    // 읽기는 상한에서 멈춘다: 끝까지 풀었다고 통과시키지 않고 미완료나 압축비 지표로 남긴다.
    assert!(r.incomplete || r.has("HIGH_COMPRESSION_RATIO") || r.has("ZIP_ENTRY_UNREADABLE") || r.has("ZIP_CRC_MISMATCH"), "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn directory_over_the_size_limit_is_incomplete_and_not_opened() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("hugedir.zip");
    common::zip_of(&p, &[("a.txt", b"a")]);
    let mut b = fs::read(&p).unwrap();
    let eocd = (0..=b.len() - 22).rev().find(|&i| &b[i..i + 4] == b"PK\x05\x06").unwrap();
    b[eocd + 12..eocd + 16].copy_from_slice(&0x2000_0000u32.to_le_bytes()); // 512MiB
    fs::write(&p, b).unwrap();
    let r = scan(&p);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("Central directory too large")), "{:?}", r.reasons);
    assert!(r.findings.is_empty());
}

#[test]
fn nested_zip_is_flagged_but_not_recursed() {
    let d = tempfile::tempdir().unwrap();
    let inner = d.path().join("inner.zip");
    common::zip_of(&inner, &[("../../deep-evil", b"x")]);
    let p = d.path().join("outer.zip");
    common::zip_of(&p, &[("inner.dat", &fs::read(&inner).unwrap())]);
    let r = scan(&p);
    assert_eq!(finding(&r, "NESTED_ARCHIVE").severity, Severity::Medium);
    assert!(!r.has("ZIP_PATH_TRAVERSAL"), "nested archive must not be recursed in P1");
}

#[test]
fn executable_member_is_flagged_by_name_and_by_magic() {
    let d = tempfile::tempdir().unwrap();
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let p = d.path().join("exe.zip");
    common::zip_of(&p, &[("setup.exe", b"not really"), ("picture.dat", &pe), ("readme.txt", b"hi")]);
    let r = scan(&p);
    let locs: Vec<&str> = r.findings.iter().filter(|f| f.code == "EMBEDDED_EXECUTABLE").map(|f| f.location.as_str()).collect();
    assert!(locs.contains(&"setup.exe") && locs.contains(&"picture.dat") && !locs.contains(&"readme.txt"), "{locs:?}");
    assert_eq!(finding(&r, "EMBEDDED_EXECUTABLE").severity, Severity::High);
}

#[test]
fn encrypted_member_is_incomplete_not_clean() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("enc.zip");
    common::zip_of(&p, &[("secret.txt", &text(20)), ("open.txt", &text(5))]);
    common::mark_encrypted(&p, "secret.txt");
    let r = scan(&p);
    assert!(r.incomplete);
    assert!(r.reasons.iter().any(|x| x.contains("secret.txt") && FileReport::is_encrypted_reason(x)), "{:?}", r.reasons);
}

#[test]
fn member_name_with_escape_never_reaches_the_terminal_raw() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("esc.zip");
    common::zip_of(&p, &[("a\u{1b}[2J.exe", b"x")]);
    let mut out = Vec::new();
    run(&["--min-severity".into(), "low".into(), "--color".into(), "never".into(), p.to_string_lossy().into_owned()], &mut out);
    assert!(!String::from_utf8(out).unwrap().contains('\u{1b}'));
    let mut json = Vec::new();
    run(&["--json".into(), "--min-severity".into(), "low".into(), p.to_string_lossy().into_owned()], &mut json);
    let json = String::from_utf8(json).unwrap();
    assert!(!json.contains("\\u001b") && !json.contains('\u{1b}'), "{json}");
}

#[test]
fn zip_deep_option_is_parsed_and_documented() {
    assert!(!parse(&["a.zip".to_string()]).unwrap().zip_deep);
    assert!(parse(&["--zip-deep".to_string(), "a.zip".to_string()]).unwrap().zip_deep);
    let mut out = Vec::new();
    print_usage(&mut out).unwrap();
    let usage = String::from_utf8(out).unwrap();
    assert!(usage.contains("--zip-deep") && usage.contains("zip"), "{usage}");
}
