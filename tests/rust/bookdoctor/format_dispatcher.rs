//! `bookdoctor::format_dispatcher` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

mod support;

// `tests/rust/bookdoctor/common_checks.rs`에서 옮긴 테스트.
mod common_checks {
    use super::{support};

    use std::fs::{self, File};
    use std::io::Write;
    use std::sync::mpsc;
    use std::time::Duration;

    use bookdoctor::cli_parser::Options;

    use bookdoctor::format_dispatcher::scan_file;

    use bookdoctor::{Category, Format, Severity};

    fn count(r: &bookdoctor::FileReport, code: &str) -> usize {
        r.findings.iter().filter(|f| f.code == code).count()
    }

    fn find<'a>(r: &'a bookdoctor::FileReport, code: &str) -> &'a bookdoctor::Finding {
        r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", r.findings))
    }

    #[test]
    fn sparse_below_one_mib_is_not_flagged() {
        let f = support::file_with("a.txt", b"");
        File::options().write(true).open(f.path()).unwrap().set_len(1024 * 1024 - 1).unwrap();
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert_eq!(count(&r, "SPARSE_FILE"), 0);
    }

    #[test]
    fn dense_large_file_is_not_sparse() {
        let f = support::file_with("a.txt", &vec![b'a'; 2 * 1024 * 1024]);
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert_eq!(count(&r, "SPARSE_FILE"), 0);
    }

    fn tail_zeroed(len: usize) -> Vec<u8> {
        let mut v = vec![1u8; len];
        let from = len.saturating_sub(4096);
        v[from..].fill(0);
        v
    }

    #[test]
    fn zero_tail_needs_4096_bytes() {
        let f = support::file_with("a.txt", &vec![0u8; 4095]);
        assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "ZERO_TAIL"), 0);
        let f = support::file_with("a.txt", &vec![0u8; 4096]);
        assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "ZERO_TAIL"), 1);
    }

    #[test]
    fn nonzero_last_byte_is_not_zero_tail() {
        let mut v = tail_zeroed(8192);
        *v.last_mut().unwrap() = 1;
        let f = support::file_with("a.txt", &v);
        assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "ZERO_TAIL"), 0);
    }

    #[test]
    fn flags_format_mismatch_for_pdf_that_is_text() {
        let f = support::file_with("a.pdf", "그냥 텍스트\n".as_bytes());
        assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "FORMAT_MISMATCH"), 1);
    }

    #[test]
    fn flags_format_mismatch_for_epub_that_is_not_zip() {
        let f = support::file_with("a.epub", b"%PDF-1.4\n%%EOF\n");
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert_eq!(count(&r, "FORMAT_MISMATCH"), 1);
        assert_eq!(r.format, Some(Format::Pdf));
    }

    #[test]
    fn flags_format_mismatch_for_txt_that_is_a_zip() {
        let f = support::file_with("a.txt", b"PK\x03\x04rest");
        assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "FORMAT_MISMATCH"), 1);
    }

    #[test]
    fn matching_content_has_no_format_mismatch() {
        let f = support::epub_named("a.epub");
        assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "FORMAT_MISMATCH"), 0);
        let f = support::file_with("a.pdf", b"%PDF-1.4\n");
        assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "FORMAT_MISMATCH"), 0);
    }

    #[test]
    fn flags_filename_control_char_in_non_utf8_name() {
        use std::os::unix::ffi::OsStrExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(std::ffi::OsStr::from_bytes(b"\xff\x07.txt"));
        fs::write(&p, "본문\n").unwrap();
        assert_eq!(count(&scan_file(&p, &Options { no_pdf_worker: true, ..Options::default() }), "FILENAME_CONTROL_CHAR"), 1);
    }

    #[test]
    fn flags_filename_too_long() {
        // 파일시스템이 255바이트 초과 이름을 만들지 못하므로 없는 경로로 검사한다. 이름 검사는 파일을 열기 전에 한다.
        let d = tempfile::tempdir().unwrap();
        let long = d.path().join(format!("{}.txt", "a".repeat(252)));
        assert_eq!(long.file_name().unwrap().len(), 256);
        let r = scan_file(&long, &Options { no_pdf_worker: true, ..Options::default() });
        let x = find(&r, "FILENAME_TOO_LONG");
        assert_eq!((x.category, x.severity), (Category::Defect, Severity::Low));
        let ok = d.path().join(format!("{}.txt", "a".repeat(251)));
        assert_eq!(count(&scan_file(&ok, &Options { no_pdf_worker: true, ..Options::default() }), "FILENAME_TOO_LONG"), 0);
    }

    #[test]
    fn clean_text_file_has_no_common_findings() {
        let f = support::file_with("a.txt", "소림사 장로가 강호에서 수련했다.\n".as_bytes());
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert!(!r.incomplete);
        assert_eq!(r.format, Some(Format::Txt));
    }

    #[test]
    fn flags_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        unsafe extern "C" {
            #[link_name = "geteuid"]
            fn libc_geteuid() -> u32;
        }
        if unsafe { libc_geteuid() } == 0 {
            eprintln!("skipped: running as root");
            return;
        }
        let f = support::file_with("a.txt", b"hello");
        fs::set_permissions(f.path(), fs::Permissions::from_mode(0o000)).unwrap();
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert!(r.incomplete);
        assert!(r.reasons[0].starts_with("unreadable:"), "{:?}", r.reasons);
        assert!(r.findings.is_empty());
    }

    #[test]
    fn skipped_over_limit() {
        let f = support::file_with("a.txt", b"");
        let mut w = File::options().write(true).open(f.path()).unwrap();
        w.write_all(&vec![b'a'; 2 * 1024 * 1024]).unwrap();
        let opts = Options { max_total_mb: 1, no_pdf_worker: true, ..Options::default() };
        let r = scan_file(f.path(), &opts);
        assert!(r.incomplete);
        assert!(r.reasons[0].starts_with("skipped:"), "{:?}", r.reasons);
        assert!(r.reasons[0].contains("exceeds --max-total-mb"));
        assert_eq!(r.format, None);
    }

    #[test]
    fn directory_is_not_a_regular_file() {
        let d = tempfile::tempdir().unwrap();
        let r = scan_file(d.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert!(r.incomplete);
        assert_eq!(r.reasons[0], "unreadable: not a regular file");
    }

    #[test]
    fn fifo_returns_promptly_as_incomplete() {
        let d = tempfile::tempdir().unwrap();
        let fifo = d.path().join("pipe.txt");
        match std::process::Command::new("mkfifo").arg(&fifo).status() {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("skipped: mkfifo unavailable");
                return;
            }
        }
        let (tx, rx) = mpsc::channel();
        let p = fifo.clone();
        std::thread::spawn(move || {
            let _ = tx.send(scan_file(&p, &Options { no_pdf_worker: true, ..Options::default() }));
        });
        let r = rx.recv_timeout(Duration::from_secs(5)).expect("FIFO 검사가 멈췄다");
        assert!(r.incomplete);
        assert_eq!(r.reasons[0], "unreadable: not a regular file");
    }

    #[test]
    fn text_starting_with_mz_has_no_findings() {
        let f = support::file_with("a.txt", b"MZ Software notes\n");
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    #[test]
    fn valid_pe_in_txt_is_disguised_executable() {
        let f = support::file_with("a.txt", &support::minimal_pe());
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert_eq!(find(&r, "DISGUISED_EXECUTABLE").severity, Severity::Critical);
    }

    #[test]
    fn file_of_exactly_max_total_mb_is_not_skipped() {
        let f = support::file_with("a.txt", &vec![b'a'; 1024 * 1024]);
        let opts = Options { max_total_mb: 1, no_pdf_worker: true, ..Options::default() };
        let r = scan_file(f.path(), &opts);
        assert!(!r.incomplete, "{:?}", r.reasons);
        assert_eq!(r.format, Some(Format::Txt));
    }

    /// MZ + e_lfanew=`lfanew`인 머리 뒤를 0으로 채워 `total` 바이트로 만든다. `sig`가 있으면 `lfanew`에 쓴다.
    fn far_pe(lfanew: u32, total: usize, sig: bool) -> Vec<u8> {
        let mut v = vec![0u8; total];
        v[..2].copy_from_slice(b"MZ");
        v[0x3C..0x40].copy_from_slice(&lfanew.to_le_bytes());
        if sig {
            v[lfanew as usize..lfanew as usize + 4].copy_from_slice(b"PE\0\0");
        }
        v
    }

    #[test]
    fn far_pe_header_beyond_head_is_disguised_executable() {
        let f = support::file_with("a.epub", &far_pe(70000, 70004, true));
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert_eq!(find(&r, "DISGUISED_EXECUTABLE").severity, Severity::Critical);
        assert_eq!(count(&r, "FORMAT_MISMATCH"), 1);
    }

    #[test]
    fn far_mz_without_pe_signature_is_not_disguised() {
        let f = support::file_with("a.epub", &far_pe(70000, 70100, false));
        assert_eq!(count(&scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() }), "DISGUISED_EXECUTABLE"), 0);
    }

    #[test]
    fn mz_with_lfanew_past_eof_is_not_disguised() {
        let f = support::file_with("a.epub", &far_pe(70000, 69999, false));
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert_eq!(count(&r, "DISGUISED_EXECUTABLE"), 0);
        assert!(!r.incomplete || r.reasons.iter().all(|x| !x.starts_with("unreadable")));
    }

    #[test]
    fn flags_bidi_and_zero_width_characters_in_the_file_name() {
        for name in ["evil\u{202e}gpj.txt", "a\u{200b}b.txt", "a\u{feff}b.txt", "a\u{2066}b.txt"] {
            let f = support::file_with(name, "본문\n".as_bytes());
            let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
            let x = find(&r, "FILENAME_CONTROL_CHAR");
            assert_eq!((x.category, x.severity), (Category::Defect, Severity::Medium), "{name:?}");
            assert_eq!(count(&r, "FILENAME_CONTROL_CHAR"), 1, "{name:?}");
        }
        let ok = support::file_with("정상 이름.txt", "본문\n".as_bytes());
        assert_eq!(count(&scan_file(ok.path(), &Options { no_pdf_worker: true, ..Options::default() }), "FILENAME_CONTROL_CHAR"), 0);
    }
}

// `tests/rust/bookdoctor/scan.rs`에서 옮긴 테스트.
mod scan {
    use super::{support};

    use std::path::Path;

    use bookdoctor::cli_parser::run_with_env;
    use bookdoctor::format_dispatcher::dispatch_format;
    use bookdoctor::Format;
    use bookdoctor::magic_sniffer::Sniffed;

    #[test]
    fn dispatch_follows_content_for_pdf_that_is_a_zip() {
        assert_eq!(dispatch_format(Path::new("a.pdf"), Sniffed::Zip), Some(Format::Epub));
    }

    #[test]
    fn dispatch_follows_extension_otherwise() {
        assert_eq!(dispatch_format(Path::new("a.txt"), Sniffed::Unknown), Some(Format::Txt));
        assert_eq!(dispatch_format(Path::new("a.epub"), Sniffed::Unknown), Some(Format::Epub));
        assert_eq!(dispatch_format(Path::new("a.dat"), Sniffed::Pdf), None);
    }

    // --- 위험 점수와 조합 승격 ---

    use bookdoctor::cli_parser::Options;
    use bookdoctor::format_dispatcher::scan_file;
    use bookdoctor::{Category, Severity};

    fn epub_with_script(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        support::epub_with_chapter(&p, &support::xhtml(&format!("<p>본문</p><script>{script}</script>")));
        (d, p)
    }

    #[test]
    fn scan_file_adds_combined_suspicious_for_obfuscation_plus_network() {
        let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
        let r = scan_file(&p, &Options::default());
        let f = r.findings.iter().find(|f| f.code == "COMBINED_SUSPICIOUS").unwrap_or_else(|| panic!("{:?}", r.findings));
        assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
        assert_eq!(f.detail, "OBFUSCATION + NETWORK_API");
    }

    #[test]
    fn scan_file_does_not_escalate_a_single_medium_signal() {
        let (_d, p) = epub_with_script("var x = atob('aGk=');");
        assert!(!scan_file(&p, &Options::default()).has("COMBINED_SUSPICIOUS"));
    }

    #[test]
    fn scan_file_fills_risk_score_and_a_clean_file_stays_below_high() {
        let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
        assert!(scan_file(&p, &Options::default()).risk_score >= 70);
        let d = tempfile::tempdir().unwrap();
        let clean = d.path().join("ok.epub");
        support::make_epub(&clean);
        // make_epub의 기본형은 LOW 지표(NAV_MISSING)만 낸다. LOW는 10점이다.
        let r = scan_file(&clean, &Options::default());
        assert!(r.findings.iter().all(|f| f.severity == Severity::Low), "{:?}", r.findings);
        assert_eq!(r.risk_score, if r.findings.is_empty() { 0 } else { 10 });
    }

    #[test]
    fn risk_score_keeps_counting_findings_hidden_by_the_output_filter() {
        let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
        let mut r = scan_file(&p, &Options::default());
        let before = r.risk_score;
        r.suppress_below(Severity::Critical);
        assert!(r.findings.is_empty());
        assert_eq!(r.risk_score, before);
    }

    // --- zip · docx · hwpx 디스패치 ---

    #[test]
    fn dispatch_routes_zip_docx_and_hwpx_by_extension() {
        assert_eq!(dispatch_format(Path::new("a.zip"), Sniffed::Zip), Some(Format::Zip));
        assert_eq!(dispatch_format(Path::new("a.zip"), Sniffed::Unknown), Some(Format::Zip));
        for name in ["a.docx", "a.DOCM", "a.dotx", "a.dotm"] {
            assert_eq!(dispatch_format(Path::new(name), Sniffed::Zip), Some(Format::Docx), "{name}");
            assert_eq!(dispatch_format(Path::new(name), Sniffed::Unknown), Some(Format::Docx), "{name}");
        }
        assert_eq!(dispatch_format(Path::new("a.hwpx"), Sniffed::Zip), Some(Format::Hwpx));
    }

    #[test]
    fn dispatch_follows_pdf_content_for_zip_family_extensions_and_keeps_epub() {
        for name in ["a.zip", "a.docx", "a.hwpx"] {
            assert_eq!(dispatch_format(Path::new(name), Sniffed::Pdf), Some(Format::Pdf), "{name}");
        }
        assert_eq!(dispatch_format(Path::new("a.epub"), Sniffed::Zip), Some(Format::Epub));
        assert_eq!(dispatch_format(Path::new("a.docs"), Sniffed::Zip), None);
    }

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
}

mod exe_scanner {
    // 실행 파일 이중 확인 테스트: 확장자와 매직바이트가 모두 맞아야 "실행 파일 확정"이다.

    use std::path::Path;

    use bookdoctor::cli_parser::{Options, run};
    use bookdoctor::format_dispatcher::{EXEC_EXTENSIONS, is_exec_extension};
    use bookdoctor::format_dispatcher::{dispatch_format, scan_file};
    use bookdoctor::magic_sniffer::Sniffed;
    use bookdoctor::{Category, FileReport, Format, Severity};

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

    fn finding<'a>(r: &'a FileReport, code: &str) -> &'a bookdoctor::Finding {
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

    // --- 최종 리뷰 반영 ---

    #[test]
    fn executable_with_a_pdf_marker_in_the_head_is_still_confirmed() {
        let mut b = pe_bytes(0x200, 0x300);
        b[0x80..0x85].copy_from_slice(b"%PDF-");
        assert!(scan_named("evil.exe", &b).has("EXECUTABLE_FILE"));
    }

    #[test]
    fn far_pe_with_a_pdf_marker_in_the_head_is_still_confirmed() {
        let mut b = pe_bytes(70_000, 70_100);
        b[0x80..0x85].copy_from_slice(b"%PDF-");
        assert!(scan_named("evil.dll", &b).has("EXECUTABLE_FILE"));
    }

    #[test]
    fn txt_with_pe_and_a_pdf_marker_still_reports_disguised_executable() {
        let mut b = pe_bytes(0x200, 0x300);
        b[0x80..0x85].copy_from_slice(b"%PDF-");
        assert!(scan_named("evil.txt", &b).has("DISGUISED_EXECUTABLE"));
    }

    #[test]
    fn data_like_executable_extension_does_not_get_zero_tail_or_sparse_findings() {
        let mut b: Vec<u8> = (0..8192u32).map(|i| (i % 251) as u8 + 1).collect();
        b.extend(vec![0u8; 8192]);
        let r = scan_named("firmware.bin", &b);
        assert!(!r.has("ZERO_TAIL") && !r.has("SPARSE_FILE"), "{:?}", r.findings);
        assert!(r.has("EXECUTABLE_EXTENSION_NO_MAGIC"));
    }

    #[test]
    fn executable_over_the_size_limit_is_still_confirmed_and_not_incomplete() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big.exe");
        let mut b = pe_bytes(0x40, 0x100);
        b.resize(2 * 1024 * 1024, 0xAA);
        std::fs::write(&p, &b).unwrap();
        let r = scan_file(&p, &Options { max_total_mb: 1, ..Options::default() });
        assert!(r.has("EXECUTABLE_FILE"), "{:?} {:?}", r.findings, r.reasons);
        assert!(!r.incomplete);
    }

    #[test]
    fn non_executable_files_keep_the_size_limit_and_zero_tail_checks() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big.txt");
        std::fs::write(&p, vec![b'a'; 2 * 1024 * 1024]).unwrap();
        assert!(scan_file(&p, &Options { max_total_mb: 1, ..Options::default() }).incomplete);
    }
}

mod html_scanner {
    // 독립 HTML 테스트. 규칙은 EPUB의 HTML 규칙을 재사용한다.

    use std::fs;
    use std::path::PathBuf;

    use bookdoctor::cli_parser::Options;
    use bookdoctor::format_dispatcher::scan_file;
    use bookdoctor::{FileReport, Format};

    fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(name);
        fs::write(&p, bytes).unwrap();
        (d, p)
    }

    fn scan(name: &str, bytes: &[u8]) -> FileReport {
        let (_d, p) = write(name, bytes);
        scan_file(&p, &Options::default())
    }

    fn utf16le_bom(s: &str) -> Vec<u8> {
        let mut out = vec![0xFF, 0xFE];
        out.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        out
    }

    #[test]
    fn plain_html_is_clean() {
        let r = scan("a.html", b"<!DOCTYPE html><html><head><title>t</title></head><body><p>hello</p></body></html>");
        assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    }

    #[test]
    fn saved_pages_that_start_with_a_comment_or_meta_are_html_not_a_mismatch() {
        for head in ["<!-- saved from url=(0030)http://example.com/ -->\n<html><body>hi</body></html>", "<meta charset=\"utf-8\"><p>fragment only</p>", "\u{feff}  <p>bom and spaces</p>", "plain text with no tags at all"] {
            let r = scan("a.html", head.as_bytes());
            assert_eq!(r.format, Some(Format::Html), "{head}");
            assert!(!r.has("FORMAT_MISMATCH"), "{head}: {:?}", r.findings);
        }
    }

    #[test]
    fn script_event_handler_and_remote_script_use_the_epub_rules() {
        let page = r#"<html><body onload="x()"><script src="http://evil.example/a.js"></script><script>var a = 1;</script></body></html>"#;
        let r = scan("a.html", page.as_bytes());
        for code in ["EVENT_HANDLER_ATTR", "REMOTE_SCRIPT", "SCRIPT_TAG"] {
            assert!(r.has(code), "{code}: {:?}", r.findings);
        }
        // 위치는 파일 이름이 아니라 검사 대상 이름이다.
        assert!(r.findings.iter().all(|f| !f.location.is_empty()));
    }

    #[test]
    fn binary_content_named_html_is_a_mismatch() {
        let pdf = scan("a.html", b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n");
        assert!(pdf.has("FORMAT_MISMATCH"), "{:?}", pdf.findings);
        assert_eq!(pdf.format, Some(Format::Pdf));
        let mut pe = vec![0u8; 0x100];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        assert!(scan("b.htm", &pe).has("DISGUISED_EXECUTABLE"));
        let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        ole.extend_from_slice(&[0u8; 600]);
        let r = scan("c.html", &ole);
        assert!(r.has("FORMAT_MISMATCH") && r.incomplete, "unanalyzed content must not be clean: {:?} {:?}", r.findings, r.reasons);
        let r = scan("d.html", b"{\\rtf1{\\*\\objclass X}}");
        assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
    }

    #[test]
    fn large_html_is_incomplete_after_the_limit() {
        let mut page = b"<html><script src=\"http://evil.example/a.js\"></script>".to_vec();
        page.extend(std::iter::repeat_n(b' ', 17 * 1024 * 1024));
        let r = scan("big.html", &page);
        assert!(r.has("REMOTE_SCRIPT"), "{:?}", r.findings);
        assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("limit")), "{:?}", r.reasons);
    }

    #[test]
    fn utf16_html_is_decoded() {
        let r = scan("a.html", &utf16le_bom("<html><script src=\"http://evil.example/a.js\"></script></html>"));
        assert!(r.has("REMOTE_SCRIPT"), "{:?}", r.findings);
    }

    // --- 인코딩 우회 (EPUB의 `check_member`와 같은 처리) ---

    const REMOTE: &str = r#"<html><script src="http://evil.example/a.js"></script></html>"#;

    #[test]
    fn utf32_html_is_incomplete_because_it_cannot_be_inspected() {
        let mut bytes = vec![0xFF, 0xFE, 0, 0];
        bytes.extend(REMOTE.chars().flat_map(|c| (c as u32).to_le_bytes()));
        let r = scan("a.html", &bytes);
        assert!(r.incomplete && r.reasons.iter().any(|m| m.to_lowercase().contains("utf-32")), "{:?} {:?}", r.findings, r.reasons);
    }

    #[test]
    fn nul_bytes_in_utf8_html_are_not_reported_clean() {
        let mut bytes = REMOTE.as_bytes().to_vec();
        bytes.splice(6..6, [0u8]);
        let r = scan("a.html", &bytes);
        assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("NUL")), "{:?} {:?}", r.findings, r.reasons);
    }

    #[test]
    fn utf16_bom_followed_by_ascii_markup_is_still_scanned() {
        // `FF FE` 뒤에 ASCII 마크업이 오면 UTF-16 변환 결과는 깨진 글자다. 바이트를 그대로 읽은 텍스트도 함께 검사한다.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(REMOTE.as_bytes());
        assert!(scan("a.html", &bytes).has("REMOTE_SCRIPT"));
    }

    #[test]
    fn a_utf8_bom_does_not_hide_the_markup() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(REMOTE.as_bytes());
        assert!(scan("a.html", &bytes).has("REMOTE_SCRIPT"));
    }

    #[test]
    fn legacy_encoded_pages_are_still_scanned() {
        // EUC-KR 한글(0xB0 0xA1)이 든 페이지도 ASCII 마크업은 그대로 검사한다.
        let mut bytes = b"<html><body>".to_vec();
        bytes.extend_from_slice(&[0xB0, 0xA1, 0xB3, 0xAA]);
        bytes.extend_from_slice(br#"<script src="http://evil.example/a.js"></script></body></html>"#);
        assert!(scan("a.html", &bytes).has("REMOTE_SCRIPT"));
    }

    #[test]
    fn a_missing_or_unreadable_html_path_is_incomplete() {
        use bookdoctor::format_dispatcher::scan_html_file;
        use bookdoctor::FileReport;
        use bookdoctor::magic_sniffer::Sniffed;
        let d = tempfile::tempdir().unwrap();
        let mut r = FileReport::new(&d.path().join("none.html"), None);
        scan_html_file(&d.path().join("none.html"), Sniffed::Html, &mut r);
        assert!(r.reasons.iter().any(|m| m == "HTML file could not be opened"), "{:?}", r.reasons);
        // 디렉터리는 열리지만 읽지 못한다.
        let mut r = FileReport::new(d.path(), None);
        scan_html_file(d.path(), Sniffed::Html, &mut r);
        assert!(r.reasons.iter().any(|m| m == "HTML file could not be read"), "{:?}", r.reasons);
    }
}
