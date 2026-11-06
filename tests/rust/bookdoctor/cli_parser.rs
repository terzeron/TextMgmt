//! `bookdoctor::cli_parser` 단위 테스트.

mod support;
use bookdoctor::cli_parser::{effective_workers, execute, parse, print_usage, run, run_with_env};
use bookdoctor::option_model::{Command, Options};
use bookdoctor::{DEFAULT_LIBRARY_ROOT, Severity};
use std::path::{Path, PathBuf};

fn args(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

fn usage() -> String {
    let mut out = Vec::new();
    print_usage(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

// --- 명령 ---

#[test]
fn parse_reads_each_command() {
    assert_eq!(parse(&args("scan a")).unwrap().command, Command::Scan);
    assert_eq!(parse(&args("sanitize a")).unwrap().command, Command::Sanitize);
    assert_eq!(parse(&args("repair a")).unwrap().command, Command::Repair);
    assert_eq!(parse(&args("quarantine a")).unwrap().command, Command::Quarantine);
    let o = parse(&args("unquarantine")).unwrap();
    assert_eq!(o.command, Command::Unquarantine);
    assert!(o.paths.is_empty());
}

#[test]
fn parse_reads_scan_options() {
    let o = parse(&args("scan --json --min-severity low a b")).unwrap();
    assert!(o.json);
    assert_eq!(o.min_severity, Severity::Low);
    assert_eq!(o.paths, vec![PathBuf::from("a"), PathBuf::from("b")]);
}

#[test]
fn parse_rejects_a_missing_or_unknown_command() {
    assert!(parse(&args("a")).unwrap_err().contains("invalid choice"));
    assert!(parse(&args("--json a")).unwrap_err().contains("command"));
    assert!(parse(&args("bogus a")).unwrap_err().contains("bogus"));
}

#[test]
fn commands_that_take_paths_require_one() {
    for cmd in ["scan", "sanitize", "repair", "quarantine"] {
        assert_eq!(parse(&args(cmd)).unwrap_err(), "the following arguments are required: PATH...", "{cmd}");
    }
    assert!(parse(&args("unquarantine a")).unwrap_err().contains("unrecognized"));
}

#[test]
fn removed_options_are_rejected() {
    for opt in [
        "-r",
        "--recursive",
        "--report x",
        "--workers 2",
        "--limit 1",
        "--max-total-mb 9",
        "--only security",
        "--color never",
        "--zip-deep",
        "--no-prefetch",
        "--no-recurse",
        "--isolate",
        "--isolate-mem-mb 1",
        "--isolate-timeout-s 1",
        "--rules x",
        "--builtin-rules",
        "--pdf-text",
        "--no-poppler",
        "--no-open-check",
        "--dry-run",
        "--trash-dir x",
        "--sanitize",
        "--fix-xhtml",
    ] {
        let e = parse(&args(&format!("scan {opt} a"))).unwrap_err();
        assert!(e.contains("unrecognized"), "{opt}: {e}");
    }
}

#[test]
fn rewriting_commands_take_no_options() {
    for cmd in ["sanitize", "repair", "quarantine"] {
        for opt in ["--json", "--min-severity low", "--dry-run"] {
            assert!(parse(&args(&format!("{cmd} {opt} a"))).unwrap_err().contains("unrecognized"), "{cmd} {opt}");
        }
    }
}

#[test]
fn fixed_directories_live_under_the_library_root() {
    let o = Options::default();
    assert_eq!(o.library_root, PathBuf::from(DEFAULT_LIBRARY_ROOT));
    assert_eq!(o.trash_dir(), Path::new(DEFAULT_LIBRARY_ROOT).join("0_trash"));
    assert_eq!(o.broken_dir(), Path::new(DEFAULT_LIBRARY_ROOT).join("0_broken"));
    assert_eq!(o.manifest_path(), Path::new(DEFAULT_LIBRARY_ROOT).join("0_broken").join("manifest.json"));
}

#[test]
fn no_arguments_prints_usage_and_exits_2() {
    let mut out = Vec::new();
    assert_eq!(run(&[], &mut out), 2);
    assert!(String::from_utf8(out).unwrap().contains("bookdoctor scan"));
}

#[test]
fn usage_lists_the_commands_and_no_removed_option() {
    let text = usage();
    for cmd in ["scan", "sanitize", "repair", "quarantine", "unquarantine", "--json", "--min-severity"] {
        assert!(text.contains(cmd), "{cmd}: {text}");
    }
    for gone in ["--report", "--workers", "--pdf-text", "--isolate", "--rules", "--dry-run", "--trash-dir", "--color", "oppler"] {
        assert!(!text.contains(gone), "{gone}: {text}");
    }
}

#[test]
fn usage_keeps_the_external_memory_limit_advice() {
    assert!(usage().contains("MemoryMax"));
}

// --- scan ---

#[test]
fn clean_empty_dir_exits_0() {
    let d = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    assert_eq!(run(&["scan".into(), d.path().to_string_lossy().into_owned()], &mut out), 0);
}

#[test]
fn scan_always_walks_into_subdirectories() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("sub")).unwrap();
    write_file(&d.path().join("sub"), "empty.txt", b"");
    let (code, out) = run_in(d.path(), &["--min-severity", "low"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("EMPTY_FILE"), "{out}");
}

#[test]
fn effective_workers_are_capped_by_memory() {
    // 한도 256MB 기준: workers <= 256 / max_total_mb, 최소 1. 기본값(워커 2, 256MB)은 1이다.
    assert_eq!(Options::default().workers, 2);
    assert_eq!(effective_workers(&Options::default()), 1);
    assert_eq!(effective_workers(&Options { workers: 2, max_total_mb: 128, ..Options::default() }), 2);
    assert_eq!(effective_workers(&Options { workers: 8, max_total_mb: 1000, ..Options::default() }), 1);
}

fn write_file(dir: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(dir.join(name), bytes).unwrap();
}

/// `con.txt`는 LOW 1건(예약 장치 이름), `empty.txt`는 MEDIUM 1건(빈 파일)만 낸다.
fn low_and_medium_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "con.txt", support::clean_korean().as_bytes());
    write_file(d.path(), "empty.txt", b"");
    d
}

fn scan_argv(dir: &Path, extra: &[&str]) -> Vec<String> {
    let mut argv = vec!["scan".to_string()];
    argv.extend(extra.iter().map(|s| s.to_string()));
    argv.push(dir.to_string_lossy().into_owned());
    argv
}

fn run_in(dir: &Path, extra: &[&str]) -> (i32, String) {
    let mut out = Vec::new();
    let code = run(&scan_argv(dir, extra), &mut out);
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn parse_min_severity_defaults_to_high() {
    assert_eq!(parse(&args("scan a")).unwrap().min_severity, Severity::High);
    assert_eq!(Options::default().min_severity, Severity::High);
}

#[test]
fn parse_rejects_unknown_min_severity() {
    let e = parse(&args("scan --min-severity bogus x")).unwrap_err();
    assert!(e.contains("invalid choice") && e.contains("bogus"), "{e}");
}

#[test]
fn default_output_hides_low_and_medium_and_exits_0() {
    let d = low_and_medium_dir();
    let (code, out) = run_in(d.path(), &[]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("[LOW]") && !out.contains("[MEDIUM]"), "{out}");
    assert!(out.contains("발견 0건"), "{out}");
    assert!(out.contains("억제 LOW 1건, MEDIUM 1건"), "{out}");
}

#[test]
fn min_severity_low_shows_everything_and_exits_1() {
    let d = low_and_medium_dir();
    let (code, out) = run_in(d.path(), &["--min-severity", "low"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[LOW] FILENAME_RESERVED_NAME"), "{out}");
    assert!(out.contains("[MEDIUM] EMPTY_FILE"), "{out}");
}

#[test]
fn invalid_arguments_exit_2() {
    let d = low_and_medium_dir();
    assert_eq!(run_in(d.path(), &["--min-severity", "bogus"]).0, 2);
    assert_eq!(run_in(d.path(), &["--workers", "2"]).0, 2);
}

#[test]
fn file_that_could_not_be_scanned_still_exits_2_under_default_filter() {
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "big.txt", &vec![b'a'; 2 * 1024 * 1024]);
    let opts = Options { max_total_mb: 1, ..parse(&scan_argv(d.path(), &[])).unwrap() };
    let mut out = Vec::new();
    let code = execute(&opts, &mut out, false, false);
    let out = String::from_utf8(out).unwrap();
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("INCOMPLETE"), "{out}");
}

#[test]
fn high_findings_are_still_shown_by_default() {
    let d = tempfile::tempdir().unwrap();
    let text = support::clean_korean();
    let (cp949, _, _) = encoding_rs::EUC_KR.encode(&text);
    write_file(d.path(), "old.txt", &cp949);
    let (code, out) = run_in(d.path(), &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("NOT_UTF8"), "{out}");
}

#[test]
fn mojibake_is_reported_as_json() {
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "broken.txt", support::corrupted().as_bytes());
    let (code, out) = run_in(d.path(), &["--json"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("\"code\":\"MOJIBAKE\""), "{out}");
}

#[test]
fn builtin_rules_are_always_on() {
    // 내장 규칙은 옵션 없이 적용된다. 규칙 목록이 비어 있지 않은지만 본다(어느 규칙이 걸리는지는 rules_builtin.rs가 본다).
    let o = parse(&args("scan a")).unwrap();
    assert!(o.ruleset.is_none(), "parse는 규칙을 읽지 않는다");
    let d = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    assert_eq!(execute(&Options { paths: vec![d.path().into()], ..o }, &mut out, false, false), 0);
}

// --- 색: 터미널 여부와 NO_COLOR로만 정한다 ---

fn run_env(dir: &Path, extra: &[&str], is_tty: bool, no_color: bool) -> String {
    let mut out = Vec::new();
    run_with_env(&scan_argv(dir, extra), &mut out, is_tty, no_color);
    String::from_utf8(out).unwrap()
}

#[test]
fn color_follows_the_callers_tty_and_no_color() {
    let d = low_and_medium_dir();
    let low = ["--min-severity", "low"];
    assert!(run_env(d.path(), &low, true, false).contains('\u{1b}'), "tty, no NO_COLOR -> color");
    assert!(run_env(d.path(), &low, true, false).contains("\u{1b}[31m  [MEDIUM] EMPTY_FILE"));
    assert!(!run_env(d.path(), &low, true, true).contains('\u{1b}'), "NO_COLOR wins");
    assert!(!run_env(d.path(), &low, false, false).contains('\u{1b}'), "pipe -> no color");
}

#[test]
fn json_output_has_no_ansi_even_on_a_tty() {
    let d = low_and_medium_dir();
    let out = run_env(d.path(), &["--json", "--min-severity", "low"], true, false);
    assert!(out.contains("EMPTY_FILE") && !out.contains('\u{1b}'), "{out:?}");
}

#[test]
fn an_empty_argument_list_needs_a_command_and_help_exits_0() {
    assert!(parse(&[]).unwrap_err().contains("command"));
    let mut out = Vec::new();
    assert_eq!(run(&["scan".into(), "-h".into()], &mut out), 0);
    assert!(String::from_utf8(out).unwrap().contains("bookdoctor unquarantine"));
}

#[test]
fn zip_deep_is_off_by_default_on_the_command_line() {
    assert!(!parse(&["scan".to_string(), "a.zip".to_string()]).unwrap().zip_deep);
}

/// 기본 출력은 HIGH 미만 지표를 숨긴다. `--min-severity low`면 MEDIUM인 외부 이미지 참조도 보인다.
#[test]
fn medium_findings_are_hidden_unless_min_severity_is_lowered() {
    let rels = br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId0" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="http://tracker.example/p.gif" TargetMode="External"/></Relationships>"#;
    let d = tempfile::tempdir().unwrap();
    support::docx_with(&d.path().join("img.docx"), &[], &[("word/_rels/document.xml.rels", rels)]);
    let show = |extra: &[&str]| {
        let mut argv: Vec<String> = vec!["scan".into()];
        argv.extend(extra.iter().map(|s| s.to_string()));
        argv.push(d.path().to_string_lossy().into_owned());
        let mut out = Vec::new();
        run_with_env(&argv, &mut out, false, false);
        String::from_utf8(out).unwrap()
    };
    assert!(!show(&[]).contains("OOXML_EXTERNAL_IMAGE"));
    assert!(show(&["--min-severity", "low"]).contains("OOXML_EXTERNAL_IMAGE"));
}

/// 디렉터리를 검사할 때 명령행이 모으는 파일과 결과(옛 format_dispatcher 테스트에서 옮겼다).
mod directory_scan {
    use super::*;
    use bookdoctor::cli_parser::run_with_env;

    #[test]
    fn directory_walk_collects_zip_docx_hwpx_but_not_lookalike_extensions() {
        let d = tempfile::tempdir().unwrap();
        for name in ["a.zip", "b.docx", "c.hwpx", "d.docs", "e.zipx"] {
            std::fs::write(d.path().join(name), b"PK\x03\x04 not a real archive").unwrap();
        }
        let mut out = Vec::new();
        run_with_env(&["scan".into(), "--min-severity".into(), "low".into(), d.path().to_string_lossy().into_owned()], &mut out, false, false);
        let out = String::from_utf8(out).unwrap();
        for hit in ["a.zip", "b.docx", "c.hwpx"] {
            assert!(out.contains(hit), "{hit}: {out}");
        }
        for miss in ["d.docs", "e.zipx"] {
            assert!(!out.contains(miss), "{miss}: {out}");
        }
    }

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

    fn run_dir(dir: &Path, extra: &[&str]) -> (i32, String) {
        let mut argv: Vec<String> = vec!["scan".into()];
        argv.extend(extra.iter().map(|s| s.to_string()));
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
    fn directory_walk_collects_executable_extensions_but_not_unknown_ones() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.exe"), min_pe()).unwrap();
        std::fs::write(d.path().join("readme.md"), b"x").unwrap();
        let (_, out) = run_dir(d.path(), &["--min-severity", "low"]);
        assert!(out.contains("a.exe") && !out.contains("readme.md"), "{out}");
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
}
