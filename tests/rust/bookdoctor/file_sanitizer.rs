//! `bookdoctor::file_sanitizer` 테스트: `--sanitize`가 파일을 모으고, 재구성·검증·재검사를 거쳐 휴지통에 원본을 두고 적용하는 흐름.
//! 재구성 기법 자체는 `tests/rust/doc_repairer/epub_sanitizer.rs`, `pdf_sanitizer.rs`가 검증한다.

mod support;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::support::shared_fixtures::{member, names, pdf, run_in};
use doc_checker::format_dispatcher::scan_file;
use doc_checker::{Category, CheckOptions, Severity};

mod epub {
    use super::*;
    use crate::support::*;

    fn opf_with(items: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>{items}</manifest><spine><itemref idref="c1"/></spine></package>"#
        )
    }

    #[test]
    fn output_never_carries_terminal_escapes_from_member_names() {
        let d = tempfile::tempdir().unwrap();
        let opf = opf_with(r#"<item id="e" href="Misc/a%1B%5B31mred.exe" media-type="application/x-dosexec"/>"#);
        let p = d.path().join("a.epub");
        epub_with_files(&p, &opf, &xhtml("<script>x</script><p>t</p>"), &[("OEBPS/Misc/a\u{1b}[31mred.exe", b"MZ\x90\x00more")]);
        let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out:?}");
        assert!(!out.contains('\u{1b}'), "{out:?}");
    }

    #[test]
    fn a_file_with_security_findings_left_after_rebuild_is_not_applied() {
        // manifest가 압축 밖을 가리키는 항목은 허용 목록으로 고치지 못한다. 남은 보안 지표가 있으면 적용하지 않는다.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.epub");
        epub_with_files(&p, &opf_with(r#"<item id="esc" href="../../outside.xhtml" media-type="application/xhtml+xml"/>"#), &xhtml("<p>본문</p><script>alert(1)</script>"), &[]);
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("SKIP"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
    }

    fn infected(dir: &Path, name: &str) -> std::path::PathBuf {
        let opf = opf_with(r#"<item id="exe" href="Misc/setup.exe" media-type="application/x-dosexec"/>"#);
        let p = dir.join(name);
        epub_with_files(&p, &opf, &xhtml("<p>본문</p><script>alert(1)</script>"), &[("OEBPS/Misc/setup.exe", b"MZ\x90\x00more")]);
        p
    }

    #[test]
    fn apply_moves_the_original_to_trash_and_overwrites_in_place() {
        let d = tempfile::tempdir().unwrap();
        let trash = d.path().join("0_trash");
        let p = infected(d.path(), "a.epub");
        let original = fs::read(&p).unwrap();
        let (ino, mtime) = {
            let m = fs::metadata(&p).unwrap();
            (m.ino(), m.modified().unwrap())
        };
        let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        let m = fs::metadata(&p).unwrap();
        assert_eq!((m.ino(), m.modified().unwrap()), (ino, mtime), "inode와 mtime을 유지한다");
        assert_eq!(fs::read(trash.join("a.epub")).unwrap(), original);
        let now = fs::read(&p).unwrap();
        assert!(!names(&now).iter().any(|n| n.ends_with("setup.exe")));
        assert!(!member(&now, "OEBPS/ch1.xhtml").contains("script"));
        // 다시 돌려도 더 바꿀 것이 없고 휴지통에 새 사본이 늘지 않는다.
        let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 1);
    }

    #[test]
    fn trash_name_collisions_get_a_suffix_and_clean_files_are_left_alone() {
        let d = tempfile::tempdir().unwrap();
        let trash = d.path().join("0_trash");
        fs::create_dir(&trash).unwrap();
        fs::write(trash.join("a.epub"), b"older").unwrap();
        let p = infected(d.path(), "a.epub");
        let clean = d.path().join("clean.epub");
        epub_with_chapter(&clean, &xhtml("<p>본문</p>"));
        let clean_before = fs::read(&clean).unwrap();
        let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap(), clean.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert_eq!(fs::read(trash.join("a.epub")).unwrap(), b"older");
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 2);
        assert_eq!(fs::read(&clean).unwrap(), clean_before);
    }
}

mod pdf_file {
    use super::*;

    const CONTENT: &str = "BT /F1 12 Tf 10 20 Td (Hello world) Tj ET";

    fn content_stream() -> String {
        format!("<< /Length {} >>\nstream\n{CONTENT}\nendstream", CONTENT.len())
    }

    fn page(extra: &str) -> String {
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R {extra} >>")
    }

    /// 카탈로그 `catalog_extra`, 쪽 `page_extra`, 5번 객체부터 `more`를 단 PDF.
    fn build(catalog_extra: &str, page_extra: &str, more: &[&str]) -> Vec<u8> {
        let cat = format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>");
        let pg = page(page_extra);
        let cs = content_stream();
        let mut objs = vec![cat.as_str(), "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", pg.as_str(), cs.as_str()];
        objs.extend_from_slice(more);
        pdf(&objs)
    }

    fn security_high(bytes: &[u8]) -> Vec<String> {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.pdf");
        fs::write(&p, bytes).unwrap();
        let report = scan_file(&p, &CheckOptions { no_pdf_worker: true, ..CheckOptions::default() });
        report.findings.iter().filter(|f| f.category == Category::Security && f.severity >= Severity::High).map(|f| f.code.to_string()).collect()
    }

    fn infected(dir: &Path, name: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        fs::write(&p, build("/OpenAction 5 0 R", "", &["<< /S /JavaScript /JS (app.alert(1)) >>"])).unwrap();
        p
    }

    #[test]
    fn apply_moves_the_original_to_trash_and_overwrites_in_place() {
        let d = tempfile::tempdir().unwrap();
        let trash = d.path().join("0_trash");
        let p = infected(d.path(), "a.pdf");
        let original = fs::read(&p).unwrap();
        let (ino, mtime) = {
            let m = fs::metadata(&p).unwrap();
            (m.ino(), m.modified().unwrap())
        };
        let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        let m = fs::metadata(&p).unwrap();
        assert_eq!((m.ino(), m.modified().unwrap()), (ino, mtime), "inode와 mtime을 유지한다");
        assert_eq!(fs::read(trash.join("a.pdf")).unwrap(), original);
        assert_eq!(security_high(&fs::read(&p).unwrap()), Vec::<String>::new());
        // 다시 돌려도 바꿀 것이 없고 휴지통에 새 사본이 늘지 않는다.
        let (code, _) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
        assert_eq!(code, 0);
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 1);
    }

    #[test]
    fn a_pdf_with_remaining_high_findings_is_left_untouched() {
        let d = tempfile::tempdir().unwrap();
        // 실행 파일처럼 보이는 첨부 파일 이름은 이번 정리가 지우는 대상이 아니다. 재검사에서 남으면 적용하지 않는다.
        let p = d.path().join("b.pdf");
        fs::write(
            &p,
            build(
                "/OpenAction 5 0 R /Names << /EmbeddedFiles << /Names [(a) 6 0 R] >> >>",
                "",
                &["<< /S /JavaScript /JS (x) >>", "<< /Type /Filespec /F (evil.exe) /EF << /F 7 0 R >> >>", "<< /Length 2 >>\nstream\nMZ\nendstream"],
            ),
        )
        .unwrap();
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("SKIP"), "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
    }

    #[test]
    fn a_clean_pdf_is_not_touched() {
        let d = tempfile::tempdir().unwrap();
        let trash = d.path().join("0_trash");
        let p = d.path().join("ok.pdf");
        fs::write(&p, build("", "", &[])).unwrap();
        let before = fs::read(&p).unwrap();
        let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}");
        assert_eq!(fs::read(&p).unwrap(), before);
        assert!(!trash.exists() || fs::read_dir(&trash).unwrap().count() == 0);
    }
}
