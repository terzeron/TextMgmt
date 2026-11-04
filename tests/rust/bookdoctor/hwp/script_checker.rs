//! `bookdoctor::hwp::script_checker` 단위 테스트.

#[path = "../support/mod.rs"]
mod support;

#[path = "../ole_common.rs"]
mod ole_common;
use crate::ole_common::{Cfb, make_cfb};
use bookdoctor::{FileReport};
use crate::support::shared_fixtures::{JSCRIPT_VERSION, default_jscript, file_header, finding, raw_deflate, scan, wchars, zlib};

fn hwp_cfb(extra: &[(&str, &[u8])], flags: u32) -> Cfb {
    let header = file_header(flags);
    let mut streams: Vec<(&str, &[u8])> = vec![("FileHeader", &header)];
    streams.extend_from_slice(extra);
    make_cfb(&streams)
}

fn scan_cfb(name: &str, cfb: &Cfb) -> FileReport {
    scan(name, &cfb.bytes)
}

fn scan_scripts(flags: u32, script: &[u8]) -> FileReport {
    scan_cfb("a.hwp", &hwp_cfb(&[("Scripts/JScriptVersion", &JSCRIPT_VERSION[..]), ("Scripts/DefaultJScript", script)], flags))
}

#[test]
fn no_scripts_storage_and_no_flag_is_clean() {
    let r = scan_cfb("a.hwp", &hwp_cfb(&[], 0));
    assert!(!r.has("HWP_SCRIPT") && !r.has("HWP_SCRIPT_FLAG_MISMATCH"), "{:?}", r.findings);
}

#[test]
fn an_empty_scripts_storage_of_an_ordinary_document_is_clean() {
    // 한글은 스크립트가 없어도 `Scripts` 저장소(빈 DefaultJScript)를 항상 쓴다. 실제 건강한 HWP5(EPUB 안에서 발견)의 두 스트림을 그대로 쓴다.
    const REAL_JSCRIPT_VERSION: [u8; 13] = [0x63, 0x64, 0x80, 0x00, 0x00, 0xf7, 0xdf, 0x88, 0xa9, 0x08, 0x00, 0x00, 0x00];
    const REAL_DEFAULT_JSCRIPT: [u8; 16] = [0x63, 0x60, 0x40, 0x05, 0xff, 0x81, 0x00, 0x00, 0x6e, 0xbb, 0x6e, 0xd1, 0x14, 0x00, 0x00, 0x00];
    let real = scan_cfb("a.hwp", &hwp_cfb(&[("Scripts/JScriptVersion", &REAL_JSCRIPT_VERSION[..]), ("Scripts/DefaultJScript", &REAL_DEFAULT_JSCRIPT[..])], 0x1));
    assert!(real.findings.is_empty() && !real.incomplete, "{:?} {:?}", real.findings, real.reasons);
    // 압축하지 않은 빈 표와 압축한 빈 표도 같다.
    let empty = default_jscript(["", "", "", ""]);
    for (flags, bytes) in [(0u32, empty.clone()), (0x1, raw_deflate(&empty)), (0x1, zlib(&empty))] {
        let r = scan_scripts(flags, &bytes);
        assert!(r.findings.is_empty() && !r.incomplete, "flags {flags:#x}: {:?} {:?}", r.findings, r.reasons);
    }
    // 공백뿐인 스크립트도 스크립트가 아니다.
    assert!(scan_scripts(0, &default_jscript(["  ", "\n\t", "", ""])).findings.is_empty());
}

#[test]
fn the_script_flag_makes_the_storage_high_even_when_empty() {
    let r = scan_cfb("a.hwp", &hwp_cfb(&[("Scripts/JScriptVersion", &JSCRIPT_VERSION[..])], 0x8));
    assert_eq!(finding(&r, "HWP_SCRIPT").severity, bookdoctor::Severity::High);
    assert!(!r.has("HWP_SCRIPT_FLAG_MISMATCH"), "{:?}", r.findings);
    // 실제 스크립트 소스가 있으면 비트가 없어도 High다.
    let r = scan_scripts(0, &default_jscript(["", "var a = 1;", "", ""]));
    assert_eq!(finding(&r, "HWP_SCRIPT").severity, bookdoctor::Severity::High);
    assert!(!r.has("HWP_SCRIPT_FLAG_MISMATCH"), "storage without the flag is what Hangul writes: {:?}", r.findings);
}

#[test]
fn a_script_bit_without_a_storage_is_a_mismatch() {
    let r = scan_cfb("a.hwp", &hwp_cfb(&[], 0x8));
    let f = finding(&r, "HWP_SCRIPT_FLAG_MISMATCH");
    assert_eq!((f.severity, f.category), (bookdoctor::Severity::Medium, bookdoctor::Category::Defect));
    assert!(!r.has("HWP_SCRIPT"));
}

#[test]
fn suspicious_script_is_critical_in_any_part() {
    let evil = r#"var s = new ActiveXObject("WScript.Shell"); s.Run("cmd.exe /c calc");"#;
    for parts in [["", evil, "", ""], ["", "", evil, ""], ["", "", "", evil], [evil, "", "", ""]] {
        let r = scan_scripts(0x8, &default_jscript(parts));
        assert_eq!(finding(&r, "HWP_SCRIPT_SUSPICIOUS").severity, bookdoctor::Severity::Critical, "{parts:?}");
    }
    let r = scan_scripts(0x8, &default_jscript(["", "var x = EVAL ( payload ) ; new Function('a')", "", ""]));
    assert!(r.has("HWP_SCRIPT_SUSPICIOUS"), "{:?}", r.findings);
}

#[test]
fn benign_script_is_high_not_critical() {
    let r = scan_scripts(0x8, &default_jscript(["", "function f() { return 1 + 1; } // a shell of a function", "", ""]));
    assert_eq!(finding(&r, "HWP_SCRIPT").severity, bookdoctor::Severity::High);
    assert!(!r.has("HWP_SCRIPT_SUSPICIOUS") && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn malformed_script_is_still_high_incomplete_and_never_panics() {
    let mut huge = Vec::new();
    huge.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    huge.extend_from_slice(&wchars("short"));
    let mut missing_end = default_jscript(["", "x", "", ""]);
    missing_end.truncate(missing_end.len() - 4);
    let mut wrong_end = default_jscript(["", "x", "", ""]);
    let n = wrong_end.len();
    wrong_end[n - 4..].copy_from_slice(&7u32.to_le_bytes());
    for (name, bytes) in [("huge length", huge), ("missing end flag", missing_end), ("wrong end flag", wrong_end), ("empty", Vec::new()), ("three bytes", vec![1, 2, 3])] {
        let r = scan_scripts(0x8, &bytes);
        assert!(r.has("HWP_SCRIPT") && r.incomplete, "{name}: {:?} {:?}", r.findings, r.reasons);
    }
    // 구조가 깨져도 원시 바이트에서 키워드를 찾는다.
    let mut evil = Vec::new();
    evil.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    evil.extend_from_slice(&wchars("WScript.Shell"));
    assert!(scan_scripts(0x8, &evil).has("HWP_SCRIPT_SUSPICIOUS"));
    // 임의 바이트로 열어도 panic하지 않는다.
    let mut seed = 0x1234_5678u32;
    for _ in 0..200 {
        let bytes: Vec<u8> = (0..64).map(|_| { seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); (seed >> 24) as u8 }).collect();
        let r = scan_scripts(0x8, &bytes);
        assert!(!r.reasons.iter().any(|m| m.contains("panic")), "{:?}", r.reasons);
    }
}

#[test]
fn a_script_stream_over_the_limit_is_incomplete() {
    let big = default_jscript(["", &"a".repeat(3 * 1024 * 1024), "", ""]);
    assert!(big.len() > 4 * 1024 * 1024);
    let r = scan_scripts(0x8, &big);
    assert!(r.has("HWP_SCRIPT") && r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn compressed_script_is_read() {
    let evil = r#"new ActiveXObject("WScript.Shell").Run("cmd.exe");"#;
    let plain = default_jscript(["", evil, "", ""]);
    // 압축 비트(0x1)가 서 있고 raw deflate나 zlib으로 압축된 스크립트다.
    for (name, bytes) in [("raw", raw_deflate(&plain)), ("zlib", zlib(&plain))] {
        let r = scan_scripts(0x1 | 0x8, &bytes);
        assert!(r.has("HWP_SCRIPT_SUSPICIOUS"), "{name}: {:?} {:?}", r.findings, r.reasons);
        assert!(!r.incomplete, "{name}: {:?}", r.reasons);
    }
    // 압축 비트가 서 있어도 실제로는 압축하지 않은 스크립트(표 2는 Scripts를 압축 대상으로 표시하지 않는다).
    assert!(scan_scripts(0x1 | 0x8, &plain).has("HWP_SCRIPT_SUSPICIOUS"));
    // 압축 비트가 없는데 압축된 스크립트는 구조를 못 읽으므로 원시 검사와 미완료다.
    assert!(scan_scripts(0x8, &raw_deflate(&plain)).incomplete);
}

#[test]
fn a_script_that_decompresses_past_the_limit_is_incomplete() {
    let bomb = raw_deflate(&default_jscript(["", &"a".repeat(6 * 1024 * 1024), "", ""]));
    assert!(bomb.len() < 64 * 1024);
    let r = scan_scripts(0x1 | 0x8, &bomb);
    assert!(r.has("HWP_SCRIPT") && r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    assert!(r.reasons.iter().any(|m| m.contains("decompresses beyond")), "{:?}", r.reasons);
}

#[test]
fn every_stream_under_scripts_is_scanned_not_only_defaultjscript() {
    let evil = wchars("var s = new ActiveXObject('WScript.Shell');");
    let cfb = hwp_cfb(&[("Scripts/JScriptVersion", &JSCRIPT_VERSION[..]), ("Scripts/Extra", &evil[..])], 0x8);
    let r = scan_cfb("a.hwp", &cfb);
    let f = finding(&r, "HWP_SCRIPT_SUSPICIOUS");
    assert!(f.location.contains("Extra"), "{}", f.location);
    // 압축되어 있어도 같다.
    let cfb = hwp_cfb(&[("Scripts/Extra", &raw_deflate(&evil)[..])], 0x1 | 0x8);
    assert!(scan_cfb("b.hwp", &cfb).has("HWP_SCRIPT_SUSPICIOUS"));
}

#[test]
fn defaultjscript_is_scanned_even_when_many_other_streams_come_first() {
    let evil = default_jscript(["", r#"new ActiveXObject("WScript.Shell")"#, "", ""]);
    let junk = [1u8, 2, 3];
    let names: Vec<String> = (0..60).map(|i| format!("Scripts/S{i:02}")).collect();
    let mut streams: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &junk[..])).collect();
    streams.push(("Scripts/DefaultJScript", &evil[..]));
    let r = scan_cfb("a.hwp", &hwp_cfb(&streams, 0x8));
    assert!(r.has("HWP_SCRIPT_SUSPICIOUS"), "DefaultJScript past the stream cap was skipped: {:?} {:?}", r.findings, r.reasons);
    // 상한 때문에 건너뛴 스트림이 있으면 깨끗하다고 하지 않는다.
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("Scripts")), "{:?}", r.reasons);
}

#[test]
fn data_after_a_valid_script_table_is_still_scanned() {
    // 표 9 구조를 지킨 앞부분 뒤에 붙은 바이트도 키워드를 찾는다. 읽는 쪽이 어디까지 읽는지 알 수 없다.
    let mut bytes = default_jscript(["", "var a = 1;", "", ""]);
    bytes.extend_from_slice(&wchars("new ActiveXObject('WScript.Shell')"));
    let r = scan_scripts(0x8, &bytes);
    assert!(r.has("HWP_SCRIPT_SUSPICIOUS"), "{:?} {:?}", r.findings, r.reasons);
    // 압축 비트가 서 있어도 같다: 원본의 뒷부분과 푼 내용을 모두 본다.
    let mut compressed_flag = default_jscript(["", "var a = 1;", "", ""]);
    compressed_flag.extend_from_slice(&wchars("WScript.Shell"));
    assert!(scan_scripts(0x1 | 0x8, &compressed_flag).has("HWP_SCRIPT_SUSPICIOUS"));
}

#[test]
fn streams_skipped_by_the_cap_or_hidden_behind_the_version_name_are_not_proof_of_an_empty_scripts_storage() {
    // 스트림 상한(32)에 잘려 못 본 스트림이 있으면 비어 있음을 증명하지 못한다.
    let empty = default_jscript(["", "", "", ""]);
    let names: Vec<String> = (0..40).map(|_| "Scripts/JScriptVersion".to_string()).collect();
    let mut streams: Vec<(&str, &[u8])> = vec![("Scripts/DefaultJScript", &empty[..])];
    streams.extend(names.iter().map(|n| (n.as_str(), &JSCRIPT_VERSION[..])));
    let r = scan_cfb("a.hwp", &hwp_cfb(&streams, 0));
    assert!(r.has("HWP_SCRIPT") && r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    // `JScriptVersion`이라는 이름의 예외는 최상위 `Scripts/JScriptVersion`에만 적용한다.
    let r = scan_cfb("b.hwp", &hwp_cfb(&[("Scripts/DefaultJScript", &empty[..]), ("Scripts/Sub/JScriptVersion", &[1u8, 2, 3, 4, 5, 6, 7, 8, 9][..])], 0));
    assert!(r.has("HWP_SCRIPT"), "a nested stream named JScriptVersion must not be exempt: {:?}", r.findings);
    // 정상 구조(최상위 버전 스트림 하나)는 여전히 깨끗하다.
    let r = scan_cfb("c.hwp", &hwp_cfb(&[("Scripts/DefaultJScript", &empty[..]), ("Scripts/JScriptVersion", &JSCRIPT_VERSION[..])], 0));
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}
