//! `bookdoctor::scan` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

mod support;

// `tests/rust/bookdoctor/common_checks.rs`에서 옮긴 테스트.
mod common_checks {
    use super::{support};

    use std::fs::{self, File};
    use std::io::Write;
    use std::sync::mpsc;
    use std::time::Duration;

    use bookdoctor::cli::Options;

    use bookdoctor::scan::scan_file;

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

    use bookdoctor::cli::run_with_env;
    use bookdoctor::scan::dispatch_format;
    use bookdoctor::Format;
    use bookdoctor::sniff::Sniffed;

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

    use bookdoctor::cli::Options;
    use bookdoctor::scan::scan_file;
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
