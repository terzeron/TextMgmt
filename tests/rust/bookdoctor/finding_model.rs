//! `bookdoctor::finding_model` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

mod support;

// `tests/rust/bookdoctor/common_checks.rs`에서 옮긴 테스트.
mod common_checks {
    use super::{support};

    use std::fs::File;

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
    fn flags_empty_file() {
        let f = support::file_with("a.txt", b"");
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        let x = find(&r, "EMPTY_FILE");
        assert_eq!((x.category, x.severity), (Category::Defect, Severity::Medium));
        assert!(!r.incomplete);
        assert_eq!(r.format, None);
    }

    #[test]
    fn flags_sparse_file() {
        let f = support::file_with("a.txt", b"");
        File::options().write(true).open(f.path()).unwrap().set_len(2 * 1024 * 1024).unwrap();
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        let x = find(&r, "SPARSE_FILE");
        assert_eq!((x.category, x.severity), (Category::Defect, Severity::High));
    }

    fn tail_zeroed(len: usize) -> Vec<u8> {
        let mut v = vec![1u8; len];
        let from = len.saturating_sub(4096);
        v[from..].fill(0);
        v
    }

    #[test]
    fn flags_zero_tail() {
        let f = support::file_with("a.txt", &tail_zeroed(8192));
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        let x = find(&r, "ZERO_TAIL");
        assert_eq!((x.category, x.severity), (Category::Defect, Severity::High));
    }

    #[test]
    fn flags_format_mismatch_for_pdf_that_is_really_an_epub() {
        let f = support::epub_named("book.pdf");
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        assert_eq!(r.format, Some(Format::Epub));
        assert_eq!(count(&r, "FORMAT_MISMATCH"), 1);
        let x = find(&r, "FORMAT_MISMATCH");
        assert_eq!((x.category, x.severity), (Category::Defect, Severity::Medium));
    }

    #[test]
    fn flags_disguised_executable() {
        for (name, head) in [("a.epub", &b"\x7fELF\x02\x01"[..]), ("a.pdf", &support::minimal_pe()[..]), ("a.txt", b"\xCF\xFA\xED\xFE")] {
            let f = support::file_with(name, head);
            let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
            let x = find(&r, "DISGUISED_EXECUTABLE");
            assert_eq!((x.category, x.severity), (Category::Security, Severity::Critical), "{name}");
            assert_eq!(count(&r, "FORMAT_MISMATCH"), 1, "{name}");
        }
    }

    #[test]
    fn flags_filename_control_char() {
        let f = support::file_with("a\u{1}b.txt", "본문\n".as_bytes());
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        let x = find(&r, "FILENAME_CONTROL_CHAR");
        assert_eq!((x.category, x.severity), (Category::Defect, Severity::Medium));
    }

    #[test]
    fn flags_filename_reserved_name() {
        let f = support::file_with("CON.txt", "본문\n".as_bytes());
        let r = scan_file(f.path(), &Options { no_pdf_worker: true, ..Options::default() });
        let x = find(&r, "FILENAME_RESERVED_NAME");
        assert_eq!((x.category, x.severity), (Category::Defect, Severity::Low));
    }
}

// `tests/rust/bookdoctor/epub_perf.rs`에서 옮긴 테스트.
mod epub_perf {
    use super::{support};

    use std::path::Path;

    use bookdoctor::epub::security_checker::Limits;
    use bookdoctor::{FileReport, Format};

    fn binary(len: usize, seed: u8) -> Vec<u8> {
        // UTF-8로 유효하지 않은 바이트가 많이 든 데이터(이미지·폰트처럼).
        (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) | 0x80).collect()
    }

    // --- 렌더링 단계의 중복 압축 해제 제거 ---

    use std::collections::HashMap;

    use bookdoctor::epub::render_checker::{inspect_with, inspect_with_verified};

    use bookdoctor::epub::security_checker::scan_verified;

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
        support::epub_with_files(path, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/a.jpg", &jpg), ("OEBPS/b.png", &png), ("OEBPS/c.ttf", &jpg)]);
    }

    #[test]
    fn skipping_verified_members_does_not_change_render_results() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("media.epub");
        epub_with_media(&p);
        let (new, old, verified) = render_both_ways(&p, &Limits::default());
        assert_same(&new, &old);
        assert!(verified.len() >= 3, "{verified:?}");
    }

    #[test]
    fn corrupt_member_is_still_reported_by_the_render_pass() {
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
        support::epub_with_files(&p, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/a.jpg", &jpg)]);
        // 압축되지 않는 데이터라 본문 한가운데를 깨뜨려도 스트림은 풀리고 CRC만 틀린다.
        let mut b = std::fs::read(&p).unwrap();
        let at = support::data_offset(&b, "OEBPS/a.jpg");
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
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big.epub");
        epub_with_media(&p);
        for max_total in [200_000u64, 450_000, 600_000, 2_000_000] {
            let limits = Limits { max_total, ..Limits::default() };
            let (new, old, _) = render_both_ways(&p, &limits);
            assert_same(&new, &old);
        }
    }
}

// `tests/rust/bookdoctor/entropy.rs`에서 옮긴 테스트.
mod entropy {
    //! ZIP 계열 멤버의 엔트로피 검사 테스트.

    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    use bookdoctor::cli_parser::Options;
    use bookdoctor::format_dispatcher::scan_file;
    use bookdoctor::{FileReport, Severity};

    fn random_bytes(n: usize, mut seed: u32) -> Vec<u8> {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect()
    }

    fn zip_with(name: &str, members: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(name);
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        for (n, data) in members {
            // 무작위 바이트가 압축되지 않아도 같은 결과가 되도록 저장 방식으로 쓴다.
            z.start_file(*n, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        (d, p)
    }

    fn scan(name: &str, members: &[(&str, &[u8])]) -> FileReport {
        let (_d, p) = zip_with(name, members);
        scan_file(&p, &Options::default())
    }

    #[test]
    fn random_bytes_named_xml_in_a_zip_are_flagged_medium() {
        let r = scan("a.zip", &[("word/document.xml", &random_bytes(8192, 1))]);
        let f = r.findings.iter().find(|f| f.code == "HIGH_ENTROPY_TEXT_MEMBER").unwrap_or_else(|| panic!("{:?}", r.findings));
        assert_eq!(f.severity, Severity::Medium);
        assert!(f.location.contains("document.xml"), "{}", f.location);
        // DOCX와 HWPX도 같다.
        assert!(scan("b.docx", &[("[Content_Types].xml", b"<Types/>"), ("word/styles.xml", &random_bytes(4096, 2))]).has("HIGH_ENTROPY_TEXT_MEMBER"));
        assert!(scan("c.hwpx", &[("Contents/section0.xml", &random_bytes(4096, 3))]).has("HIGH_ENTROPY_TEXT_MEMBER"));
    }
}

// `tests/rust/bookdoctor/finding.rs`에서 옮긴 테스트.
mod finding {
    //! finding 테스트: 심각도, 지표 기록과 Report 연동, JSON 모양, 위험 점수와 조합 승격.

    use std::path::Path;

    use bookdoctor::finding_model::{COMBINATIONS, Suppressed, escalate, risk_score};
    use bookdoctor::{Category, FileReport, Finding, Format, Report, Severity};

    #[test]
    fn push_records_a_finding_and_report_trait_sees_it() {
        let mut r = FileReport::new(Path::new("a.txt"), Some(Format::Txt));
        r.push(Category::Defect, Severity::High, "NOT_UTF8", "", "encoding=cp949");
        assert!(r.has("NOT_UTF8") && !r.has("X"));
        assert_eq!(r.severities().collect::<Vec<_>>(), vec![Severity::High]);
        assert!(!r.is_incomplete());
    }

    #[test]
    fn mark_incomplete_keeps_the_reason() {
        let mut r = FileReport::new(Path::new("a.pdf"), Some(Format::Pdf));
        r.mark_incomplete("skipped");
        assert_eq!(r.reasons, vec!["skipped".to_string()]);
        assert!(r.is_incomplete());
    }

    #[test]
    fn path_keeps_the_original_name_so_quarantine_finds_the_real_file() {
        let r = FileReport::new(Path::new("a\u{1b}b\u{202e}.txt"), None);
        assert_eq!(r.path, "a\u{1b}b\u{202e}.txt");
    }

    #[test]
    fn json_uses_lowercase_category_and_na_viewer() {
        let mut r = FileReport::new(Path::new("a.txt"), Some(Format::Txt));
        r.push(Category::Defect, Severity::High, "NOT_UTF8", "", "d");
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["format"], "txt");
        assert_eq!(v["findings"][0]["category"], "defect");
        assert_eq!(v["findings"][0]["severity"], "HIGH");
        assert_eq!(v["findings"][0]["viewer_blocked"], "n/a");
    }

    fn mixed_report() -> FileReport {
        let mut r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
        r.push(Category::Defect, Severity::Low, "L1", "", "");
        r.push(Category::Defect, Severity::Low, "L2", "", "");
        r.push(Category::Render, Severity::Medium, "M1", "", "");
        r.push(Category::Security, Severity::High, "H1", "", "");
        r
    }

    #[test]
    fn suppress_below_removes_lower_and_counts_by_severity() {
        let mut r = mixed_report();
        r.suppress_below(Severity::High);
        assert_eq!(r.findings.len(), 1);
        assert!(r.has("H1"));
        assert_eq!(r.suppressed, Suppressed { low: 2, medium: 1, high: 0 });
        assert_eq!(r.suppressed.total(), 3);
        assert!(!r.suppressed.is_empty());
    }

    #[test]
    fn suppress_below_low_keeps_everything() {
        let mut r = mixed_report();
        r.suppress_below(Severity::Low);
        assert_eq!(r.findings.len(), 4);
        assert!(r.suppressed.is_empty());
    }

    #[test]
    fn suppress_below_critical_also_counts_high() {
        let mut r = mixed_report();
        r.suppress_below(Severity::Critical);
        assert!(r.findings.is_empty());
        assert_eq!(r.suppressed, Suppressed { low: 2, medium: 1, high: 1 });
    }

    #[test]
    fn suppress_below_does_not_touch_incomplete_state() {
        let mut r = mixed_report();
        r.mark_incomplete("skipped");
        r.suppress_below(Severity::Critical);
        assert!(r.is_incomplete());
        assert_eq!(r.reasons, vec!["skipped".to_string()]);
    }

    #[test]
    fn suppressed_is_omitted_from_json_when_empty_and_present_otherwise() {
        let mut r = mixed_report();
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("suppressed").is_none(), "{v}");
        r.suppress_below(Severity::High);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["suppressed"]["low"], 2);
        assert_eq!(v["suppressed"]["medium"], 1);
        assert_eq!(v["suppressed"]["high"], 0);
    }

    #[test]
    fn encrypted_reason_markers_are_specific() {
        assert!(FileReport::is_encrypted_reason("encrypted: string values may be unreadable"));
        assert!(FileReport::is_encrypted_reason("a.bin: ZIP-encrypted entry cannot be reliably inspected"));
        // 이름에 encrypted가 들어 있을 뿐인 다른 사유는 암호화로 보지 않는다.
        assert!(!FileReport::is_encrypted_reason("encrypted.xml: OPF is not well-formed XML"));
        assert!(!FileReport::is_encrypted_reason("skipped: 5 bytes exceeds --max-total-mb"));
    }

    #[test]
    fn encryption_codes_include_the_pdf_encrypted_code() {
        assert!(bookdoctor::finding_model::ENCRYPTION_CODES.contains(&"ENCRYPTED"));
    }

    fn findings(severities: &[Severity]) -> Vec<Finding> {
        let mut r = FileReport::new(Path::new("a"), None);
        for (i, s) in severities.iter().enumerate() {
            r.push(Category::Security, *s, &format!("C{i}"), "", "");
        }
        r.findings
    }

    #[test]
    fn risk_score_empty_is_zero() {
        assert_eq!(risk_score(&[]), 0);
    }

    #[test]
    fn risk_score_uses_the_max_weight() {
        assert_eq!(risk_score(&findings(&[Severity::Low])), 10);
        assert_eq!(risk_score(&findings(&[Severity::Medium])), 30);
        assert_eq!(risk_score(&findings(&[Severity::High])), 70);
        assert_eq!(risk_score(&findings(&[Severity::Critical])), 100);
        assert_eq!(risk_score(&findings(&[Severity::Low, Severity::High, Severity::Medium])), 70 + 5);
    }

    #[test]
    fn risk_score_adds_five_per_extra_medium_or_higher() {
        assert_eq!(risk_score(&findings(&[Severity::High, Severity::Medium, Severity::Medium])), 80);
        assert_eq!(risk_score(&findings(&[Severity::Medium, Severity::Medium])), 35);
    }

    #[test]
    fn risk_score_caps_at_100() {
        assert_eq!(risk_score(&findings(&[Severity::High; 10])), 100);
        assert_eq!(risk_score(&findings(&[Severity::Critical; 3])), 100);
    }

    #[test]
    fn risk_score_ignores_low_for_the_bonus() {
        assert_eq!(risk_score(&findings(&[Severity::High, Severity::Low, Severity::Low])), 70);
    }

    fn report_with(codes: &[&str]) -> FileReport {
        let mut r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
        for c in codes {
            r.push(Category::Security, Severity::Medium, c, "x.js", "d");
        }
        r
    }

    #[test]
    fn escalate_adds_combined_when_a_pair_is_present() {
        let mut r = report_with(&["OBFUSCATION", "NETWORK_API"]);
        escalate(&mut r);
        let f = r.findings.iter().find(|f| f.code == "COMBINED_SUSPICIOUS").expect("combined");
        assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
        assert_eq!(f.detail, "OBFUSCATION + NETWORK_API");
    }

    #[test]
    fn escalate_covers_every_partner_in_the_table() {
        for (left, partners) in COMBINATIONS {
            for partner in *partners {
                let mut r = report_with(&[left, partner]);
                escalate(&mut r);
                assert!(r.has("COMBINED_SUSPICIOUS"), "{left} + {partner}");
            }
        }
    }

    #[test]
    fn escalate_does_nothing_when_only_one_side_is_present() {
        for codes in [&["OBFUSCATION"][..], &["NETWORK_API"][..], &["NETWORK_API", "NAVIGATION"][..], &[][..]] {
            let mut r = report_with(codes);
            escalate(&mut r);
            assert!(!r.has("COMBINED_SUSPICIOUS"), "{codes:?}");
        }
    }

    #[test]
    fn escalate_is_idempotent_and_adds_one_finding_for_many_partners() {
        let mut r = report_with(&["OBFUSCATION", "NETWORK_API", "NAVIGATION"]);
        escalate(&mut r);
        escalate(&mut r);
        assert_eq!(r.findings.iter().filter(|f| f.code == "COMBINED_SUSPICIOUS").count(), 1);
    }

    #[test]
    fn json_always_has_risk_score() {
        let r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["risk_score"], 0);
    }

    #[test]
    fn severity_orders_from_low_to_critical_and_has_upper_case_labels() {
        assert!(Severity::Low < Severity::Medium && Severity::Medium < Severity::High && Severity::High < Severity::Critical);
        let labels: Vec<_> = [Severity::Low, Severity::Medium, Severity::High, Severity::Critical].iter().map(|s| s.label()).collect();
        assert_eq!(labels, ["LOW", "MEDIUM", "HIGH", "CRITICAL"]);
    }

    #[test]
    fn severity_serializes_as_its_upper_case_label() {
        assert_eq!(serde_json::to_string(&Severity::Critical).unwrap(), "\"CRITICAL\"");
        assert_eq!(serde_json::to_string(&Severity::Low).unwrap(), "\"LOW\"");
    }

    #[test]
    fn severity_parse_accepts_four_lowercase_names_and_rejects_others() {
        assert_eq!(Severity::parse("low"), Some(Severity::Low));
        assert_eq!(Severity::parse("medium"), Some(Severity::Medium));
        assert_eq!(Severity::parse("high"), Some(Severity::High));
        assert_eq!(Severity::parse("critical"), Some(Severity::Critical));
        for bad in ["", "HIGH", "High", "info", "highest", " high"] {
            assert_eq!(Severity::parse(bad), None, "{bad:?}");
        }
    }
}

// `tests/rust/bookdoctor/critical_promotion.rs`에서 옮긴 테스트.
mod critical_promotion {
    use super::{support};

    use std::fs;

    use bookdoctor::cli_parser::Options;
    use bookdoctor::epub::security_checker::{Limits, scan as scan_into};
    use bookdoctor::format_dispatcher::scan_file;
    use support::*;
    use bookdoctor::{FileReport, Format, Severity};

    fn scan_epub(chapter: &str, extra: &[(&str, &[u8])]) -> FileReport {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_files(&p, &clean_opf(), &xhtml(chapter), extra);
        let mut r = FileReport::new(&p, Some(Format::Epub));
        scan_into(&p, &Limits::default(), &mut r);
        r
    }

    fn sev(r: &FileReport, code: &str) -> Option<Severity> {
        r.findings.iter().filter(|f| f.code == code).map(|f| f.severity).max()
    }

    fn js(body: &str) -> FileReport {
        scan_epub(&format!("<script>{body}</script>"), &[])
    }

    // ---- 번들 실행 파일은 무조건 CRITICAL ----

    #[test]
    fn bundled_executables_are_always_critical() {
        for name in ["OEBPS/Misc/setup.exe", "OEBPS/a.dll", "OEBPS/run.sh", "OEBPS/x.lnk", "OEBPS/a.bat", "OEBPS/a.jar"] {
            let r = scan_epub("<p>t</p>", &[(name, b"hello")]);
            assert_eq!(sev(&r, "EMBEDDED_EXECUTABLE"), Some(Severity::Critical), "{name}");
        }
        let r = scan_epub("<p>t</p>", &[("OEBPS/Misc/a.exe", b"MZ\x90\x00more")]);
        assert_eq!(sev(&r, "PE_EXECUTABLE_CONTENT"), Some(Severity::Critical));
        let r = scan_epub("<p>t</p>", &[("OEBPS/a.so", b"\x7fELF\x02\x01")]);
        assert_eq!(sev(&r, "ELF_BINARY_CONTENT"), Some(Severity::Critical));
        let r = scan_epub("<p>t</p>", &[("OEBPS/a.dylib", b"\xcf\xfa\xed\xfe\x07")]);
        assert_eq!(sev(&r, "MACHO_BINARY_CONTENT"), Some(Severity::Critical));
        // 실행 확장자가 아니고 내용도 실행 코드가 아니면 지표가 없다.
        assert!(scan_epub("<p>t</p>", &[("OEBPS/a.png", b"\x89PNG\r\n\x1a\nrest")]).findings.is_empty());
    }

    // ---- 이벤트 핸들러·javascript: URL 본문 ----

    #[test]
    fn event_handler_bodies_are_scanned_like_scripts() {
        let r = scan_epub(r#"<div onclick="eval(atob('YWxlcnQoMSk='))">x</div>"#, &[]);
        assert_eq!(sev(&r, "OBFUSCATED_LOADER"), Some(Severity::Critical), "{:?}", r.findings);
        let r = scan_epub(r#"<div onclick="go()">x</div>"#, &[]);
        assert_eq!(sev(&r, "EVENT_HANDLER_ATTR"), Some(Severity::High));
        assert_eq!(sev(&r, "OBFUSCATED_LOADER"), None);
    }

    #[test]
    fn javascript_url_bodies_are_scanned_like_scripts() {
        for href in ["javascript:eval(atob('YWxlcnQoMSk='))", "JaVa\tScRiPt:eval(atob('x'))", "javascript:document.write(atob('PHNjcmlwdD4='))"] {
            let r = scan_epub(&format!(r#"<a href="{href}">x</a>"#), &[]);
            assert_eq!(sev(&r, "OBFUSCATED_LOADER"), Some(Severity::Critical), "{href}: {:?}", r.findings);
        }
        let r = scan_epub(r#"<a href="javascript:void(0)">x</a>"#, &[]);
        assert_eq!(sev(&r, "JS_URL"), Some(Severity::High));
        assert_eq!(sev(&r, "OBFUSCATED_LOADER"), None);
    }

    // ---- 난독화 로더 우회 ----

    #[test]
    fn decoded_payload_loaders_are_critical_in_every_shape() {
        for body in [
            "eval(atob('YWxlcnQoMSk='))",
            "setTimeout(atob('YWxlcnQoMSk='),1)",
            "setInterval(unescape('%61%6c'),1)",
            "document.writeln(atob('PHNjcmlwdD4='))",
            "document.write(String.fromCharCode(60,115))",
            "var e=eval; var p=atob('YWxlcnQoMSk='); e(p);",
            "var f=window.Function; f(String.fromCharCode(97,108))()",
            "window['ev'+'al'](atob('YWxlcnQoMSk='))",
            "self[\"ev\"+\"al\"](decodeURIComponent('%61'))",
            "(0,eval)(atob('YWxlcnQoMSk='))",
        ] {
            assert_eq!(sev(&js(body), "OBFUSCATED_LOADER"), Some(Severity::Critical), "{body}");
        }
    }

    #[test]
    fn analytics_style_document_write_unescape_and_distant_decode_calls_are_not_critical() {
        // Google Analytics 방식 로더: High로만 남는다.
        let r = js("document.write(unescape('%3Cscript src=\"' + document.location.protocol + '//x.example/a.js\"%3E%3C/script%3E'))");
        assert_eq!(sev(&r, "OBFUSCATED_LOADER"), Some(Severity::High), "{:?}", r.findings);
        // jQuery의 globalEval과 멀리 떨어진 decodeURIComponent는 한 로더로 묶지 않는다.
        let filler = " ".repeat(2000);
        let r = js(&format!("function g(d){{ window[ \"eval\" ].call( window, d ); }}{filler} var q = decodeURIComponent(location.search);"));
        assert_eq!(sev(&r, "OBFUSCATED_LOADER"), None, "{:?}", r.findings);
        assert_eq!(sev(&r, "DYNAMIC_CODE_EXEC"), Some(Severity::High));
    }

    #[test]
    fn aliased_exec_without_decoding_is_high_and_ordinary_code_is_not_critical() {
        let r = js("window['ev'+'al']('1+1')");
        assert_eq!(sev(&r, "DYNAMIC_CODE_EXEC"), Some(Severity::High), "{:?}", r.findings);
        assert_eq!(sev(&r, "OBFUSCATED_LOADER"), None);
        for body in [
            "document.write('hello')",
            "var s = unescape('%41');",
            "var x = atob('YQ==');",
            "function evaluate(a){return a}; var retrieval = 2;",
            "var o = {eval: 1}; o.evaluate = 2;",
            "var s = 'a' + 'b'; var t = s + atob('YQ==');",
        ] {
            assert_ne!(sev(&js(body), "OBFUSCATED_LOADER"), Some(Severity::Critical), "{body}");
        }
    }

    // ---- 클릭 없는 UNC·원격 file 로드, meta refresh ----

    #[test]
    fn zero_click_network_resource_loads_are_critical() {
        for tag in [
            r#"<img src="\\evil\share\x.png">"#,
            r#"<img src="smb://evil/share/x.png">"#,
            r#"<img src="file://evil/share/x.png">"#,
            r#"<link rel="stylesheet" href="\\evil\share\a.css">"#,
            r#"<iframe src="smb://evil/s/a.html"></iframe>"#,
        ] {
            let r = scan_epub(tag, &[]);
            let worst = r.findings.iter().filter(|f| matches!(f.code.as_str(), "UNC_PATH" | "DANGEROUS_URI_SCHEME")).map(|f| f.severity).max();
            assert_eq!(worst, Some(Severity::Critical), "{tag}: {:?}", r.findings);
        }
    }

    #[test]
    fn clicked_links_and_local_file_loads_stay_high() {
        for tag in [
            r#"<a href="\\evil\share\x.png">x</a>"#,
            r#"<a href="smb://evil/share/x">x</a>"#,
            r#"<a href="file://evil/share/x">x</a>"#,
            r#"<img src="file:///C:/Users/a/x.png">"#,
            r#"<img src="file://localhost/C:/x.png">"#,
        ] {
            let r = scan_epub(tag, &[]);
            let worst = r.findings.iter().filter(|f| matches!(f.code.as_str(), "UNC_PATH" | "DANGEROUS_URI_SCHEME")).map(|f| f.severity).max();
            assert_eq!(worst, Some(Severity::High), "{tag}: {:?}", r.findings);
        }
    }

    #[test]
    fn meta_refresh_to_an_exploit_scheme_is_critical() {
        let r = scan_epub(r#"<meta http-equiv="refresh" content="0;url=ms-msdt:/id PCWDiagnostic">"#, &[]);
        assert_eq!(sev(&r, "META_REFRESH"), Some(Severity::Critical));
        let r = scan_epub(r#"<meta http-equiv="refresh" content="0;url=search-ms:query=x">"#, &[]);
        assert_eq!(sev(&r, "META_REFRESH"), Some(Severity::Critical));
        // 일반 원격 이동과 그 밖의 위험 스킴은 High로 남는다.
        let r = scan_epub(r#"<meta http-equiv="refresh" content="0;url=https://evil.example/">"#, &[]);
        assert_eq!(sev(&r, "META_REFRESH"), Some(Severity::High));
        let r = scan_epub(r#"<meta http-equiv="refresh" content="0;url=file:///etc/passwd">"#, &[]);
        assert_eq!(sev(&r, "META_REFRESH"), Some(Severity::High));
    }

    // ---- PDF ----

    fn pdf_bytes(objects: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        out
    }

    fn scan_pdf_js(js: &str) -> FileReport {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R /OpenAction 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>",
            &format!("<< /S /JavaScript /JS ({js}) >>"),
        ];
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.pdf");
        fs::write(&p, pdf_bytes(&objs)).unwrap();
        scan_file(&p, &Options { no_pdf_worker: true, ..Options::default() })
    }

    fn pdf_worst(r: &FileReport) -> Option<Severity> {
        r.findings.iter().filter(|f| f.code == "exploit JavaScript" || f.code == "JavaScript action").map(|f| f.severity).max()
    }

    #[test]
    fn pdf_exploit_javascript_variants_are_critical() {
        for js in [
            "this[\"export\"+\"DataObject\"]({cName:'a.exe', nLaunch:2});",
            "this['exportData'+'Object']({ cName: 'a.exe', nLaunch: 2 });",
            "util.printf('%45000f', 1);",
            "util.printf(\"%8000d\", 1);",
            "Collab.getIcon('x');",
            "Collab.collectEmailInfo({msg: 'A'});",
            "media.newPlayer(null);",
            "var e=eval; e(unescape('%61%6c'));",
            "var f=this['ev'+'al']; f(String.fromCharCode(97));",
        ] {
            let r = scan_pdf_js(js);
            assert_eq!(pdf_worst(&r), Some(Severity::Critical), "{js}: {:?}", r.findings);
        }
    }

    #[test]
    fn pdf_ordinary_javascript_stays_high() {
        for js in ["app.alert('hi');", "event.value = util.printd('yyyy', new Date());", "var s = util.printf('%d items', 3);", "var e = 1; this.getField('a').value = e;"] {
            let r = scan_pdf_js(js);
            assert_eq!(pdf_worst(&r), Some(Severity::High), "{js}: {:?}", r.findings);
        }
    }

    // ---- 번들 실행 파일의 SHA-256 ----

    #[test]
    fn bundled_executable_findings_carry_the_sha256_of_the_member() {
        // sha256("hello")
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let r = scan_epub("<p>t</p>", &[("OEBPS/Misc/setup.exe", b"hello")]);
        let f = r.findings.iter().find(|f| f.code == "EMBEDDED_EXECUTABLE").unwrap();
        assert!(f.detail.contains(&format!("sha256={hello}")), "{}", f.detail);
        let mut pe = b"MZ\x90\x00".to_vec();
        pe.extend(b"hello");
        let r = scan_epub("<p>t</p>", &[("OEBPS/Misc/a.exe", &pe)]);
        let f = r.findings.iter().find(|f| f.code == "PE_EXECUTABLE_CONTENT").unwrap();
        assert!(f.detail.contains("sha256=") && f.detail.len() > 64, "{}", f.detail);
        // 실행 파일이 아닌 멤버에는 붙지 않는다.
        let r = scan_epub("<p>t</p>", &[("OEBPS/a.png", b"\x89PNG\r\n\x1a\nrest")]);
        assert!(r.findings.iter().all(|f| !f.detail.contains("sha256=")));
    }
}
