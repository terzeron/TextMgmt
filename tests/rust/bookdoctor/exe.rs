//! 실행 파일 이중 확인 테스트: 확장자와 매직바이트가 모두 맞아야 "실행 파일 확정"이다.

use std::path::Path;

use bookdoctor::cli::{Options, run};
use bookdoctor::exe::{EXEC_EXTENSIONS, is_exec_extension};
use bookdoctor::scan::{dispatch_format, scan_file};
use scan_common::sniff::Sniffed;
use scan_common::{Category, FileReport, Format, Severity};

/// `MZ` 머리, e_lfanew=`offset`, 그 자리의 `PE\0\0`. 파일 길이는 `len`이다.
fn pe_bytes(offset: u32, len: usize) -> Vec<u8> {
    let mut b = vec![0u8; len];
    b[0] = b'M';
    b[1] = b'Z';
    b[0x3C..0x40].copy_from_slice(&offset.to_le_bytes());
    let at = offset as usize;
    b[at..at + 4].copy_from_slice(b"PE\0\0");
    b
}

fn min_pe() -> Vec<u8> {
    pe_bytes(0x40, 0x100)
}

fn elf() -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..4].copy_from_slice(b"\x7fELF");
    b
}

fn macho() -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..4].copy_from_slice(&[0xCF, 0xFA, 0xED, 0xFE]);
    b
}

fn scan_named(name: &str, bytes: &[u8]) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    std::fs::write(&p, bytes).unwrap();
    scan_file(&p, &Options::default())
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a scan_common::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing in {:?}", r.findings))
}

#[test]
fn exe_with_pe_header_is_confirmed() {
    let r = scan_named("a.exe", &min_pe());
    assert_eq!(r.format, Some(Format::Executable));
    let f = finding(&r, "EXECUTABLE_FILE");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert!(f.detail.contains("PE"), "{}", f.detail);
    assert!(!r.has("EXECUTABLE_EXTENSION_NO_MAGIC"));
}

#[test]
fn elf_so_is_confirmed() {
    let r = scan_named("libx.so", &elf());
    assert!(finding(&r, "EXECUTABLE_FILE").detail.contains("ELF"));
}

#[test]
fn macho_dylib_is_confirmed() {
    let r = scan_named("libx.dylib", &macho());
    assert!(finding(&r, "EXECUTABLE_FILE").detail.contains("Mach-O"));
}

#[test]
fn exe_extension_with_text_content_is_not_confirmed() {
    let r = scan_named("a.exe", "그냥 글이다\n".repeat(20).as_bytes());
    assert_eq!(r.format, Some(Format::Executable));
    assert!(!r.has("EXECUTABLE_FILE"));
    let f = finding(&r, "EXECUTABLE_EXTENSION_NO_MAGIC");
    assert_eq!((f.category, f.severity), (Category::Defect, Severity::Low));
}

#[test]
fn mz_without_pe_header_is_not_confirmed() {
    let mut b = b"MZ this is only text that starts with MZ".to_vec();
    b.resize(0x100, b' ');
    let r = scan_named("a.exe", &b);
    assert!(!r.has("EXECUTABLE_FILE") && r.has("EXECUTABLE_EXTENSION_NO_MAGIC"));
}

#[test]
fn pe_with_e_lfanew_beyond_the_head_window_is_confirmed() {
    let r = scan_named("big.dll", &pe_bytes(70_000, 70_100));
    assert!(r.has("EXECUTABLE_FILE"), "{:?}", r.findings);
}

#[test]
fn extension_match_is_case_insensitive() {
    assert!(scan_named("A.EXE", &min_pe()).has("EXECUTABLE_FILE"));
    assert!(is_exec_extension("ExE") && is_exec_extension("SO"));
}

#[test]
fn any_executable_magic_counts_for_any_executable_extension() {
    // 확장자가 .so인데 PE 내용이어도 실행 파일은 맞다(종류는 상세에 그대로 적는다).
    let r = scan_named("odd.so", &min_pe());
    assert!(finding(&r, "EXECUTABLE_FILE").detail.contains("PE"));
}

#[test]
fn only_executable_extensions_are_executable_extensions() {
    for ext in ["exe", "dll", "sys", "scr", "ocx", "cpl", "drv", "efi", "so", "elf", "ko", "dylib", "bundle", "bin", "out"] {
        assert!(is_exec_extension(ext) && EXEC_EXTENSIONS.contains(&ext), "{ext}");
    }
    for ext in ["txt", "epub", "pdf", "zip", "doc", "hwp", ""] {
        assert!(!is_exec_extension(ext), "{ext}");
    }
}

#[test]
fn dispatch_sends_executable_extensions_to_the_executable_scanner() {
    assert_eq!(dispatch_format(Path::new("a.exe"), Sniffed::Pe), Some(Format::Executable));
    assert_eq!(dispatch_format(Path::new("a.so"), Sniffed::Unknown), Some(Format::Executable));
    // 기존 포맷은 바뀌지 않는다.
    assert_eq!(dispatch_format(Path::new("a.txt"), Sniffed::Pe), Some(Format::Txt));
    assert_eq!(dispatch_format(Path::new("a.pdf"), Sniffed::Pdf), Some(Format::Pdf));
}

#[test]
fn txt_with_executable_content_still_reports_disguised_executable() {
    let r = scan_named("note.txt", &min_pe());
    let f = finding(&r, "DISGUISED_EXECUTABLE");
    assert_eq!(f.severity, Severity::Critical);
    assert!(!r.has("EXECUTABLE_FILE"));
}

// --- CLI ---

fn run_dir(dir: &Path, extra: &[&str]) -> (i32, String) {
    let mut argv: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
    argv.push("-r".into());
    argv.push(dir.to_string_lossy().into_owned());
    let mut out = Vec::new();
    let code = run(&argv, &mut out);
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn executable_file_is_high_and_survives_the_default_filter() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.exe"), min_pe()).unwrap();
    let (code, out) = run_dir(d.path(), &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[HIGH] EXECUTABLE_FILE"), "{out}");
}

#[test]
fn summary_line_counts_confirmed_executables_separately() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.exe"), min_pe()).unwrap();
    std::fs::write(d.path().join("b.so"), elf()).unwrap();
    std::fs::write(d.path().join("c.dll"), b"not a binary at all\n".repeat(10)).unwrap();
    let (_, out) = run_dir(d.path(), &[]);
    assert!(out.contains("실행 파일 2건"), "{out}");
}

#[test]
fn summary_line_has_no_executable_part_without_executables() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "소림사 장로가 강호에서 내공을 운기조식했다.\n".repeat(10)).unwrap();
    let (_, out) = run_dir(d.path(), &[]);
    assert!(!out.contains("실행 파일"), "{out}");
}

#[test]
fn json_names_the_format_executable() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.exe"), min_pe()).unwrap();
    let (_, out) = run_dir(d.path(), &["--json"]);
    assert!(out.contains("\"format\":\"executable\""), "{out}");
}

#[test]
fn report_file_summary_counts_executables() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.exe"), min_pe()).unwrap();
    let report = d.path().join("r.json");
    let argv: Vec<String> = ["--report".to_string(), report.to_string_lossy().into_owned(), "-r".to_string(), d.path().to_string_lossy().into_owned()].into();
    assert_eq!(run(&argv, &mut Vec::new()), 1);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(v["summary"]["executables"], 1, "{v}");
}

#[test]
fn directory_walk_collects_executable_extensions_but_not_unknown_ones() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.exe"), min_pe()).unwrap();
    std::fs::write(d.path().join("readme.md"), b"x").unwrap();
    let (_, out) = run_dir(d.path(), &["--min-severity", "low"]);
    assert!(out.contains("a.exe") && !out.contains("readme.md"), "{out}");
}

#[test]
fn confirmed_executable_flag_is_set_only_for_confirmed_files() {
    assert!(scan_named("a.exe", &min_pe()).confirmed_executable);
    assert!(!scan_named("a.exe", b"plain text\n".repeat(30).as_slice()).confirmed_executable);
    assert!(!scan_named("a.txt", &min_pe()).confirmed_executable);
}

#[test]
fn executable_count_survives_a_filter_that_hides_the_finding() {
    // HIGH인 EXECUTABLE_FILE이 --min-severity critical로 숨겨져도 "실행 파일 N건"은 남아야 한다.
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.exe"), min_pe()).unwrap();
    let (code, out) = run_dir(d.path(), &["--min-severity", "critical"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("EXECUTABLE_FILE") && out.contains("실행 파일 1건"), "{out}");
}
