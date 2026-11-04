//! PDF `--sanitize`: 위험한 동작과 스크립트를 지우고 다시 쓴다. 쪽 수와 쪽 내용 보존, 보안 재검사, 휴지통 보관과 제자리 덮어쓰기.

#[path = "../support/mod.rs"]
mod support;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bookdoctor::cli_parser::{Options};
use bookdoctor::pdf::pdf_sanitizer::{rebuild, verify};
use bookdoctor::format_dispatcher::scan_file;
use lopdf::Document;
use bookdoctor::{Category, Severity};
use crate::support::shared_fixtures::{pdf, run_in};

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
    let report = scan_file(&p, &Options { no_pdf_worker: true, ..Options::default() });
    report.findings.iter().filter(|f| f.category == Category::Security && f.severity >= Severity::High).map(|f| f.code.to_string()).collect()
}

fn page_texts(bytes: &[u8]) -> Vec<Vec<u8>> {
    let doc = Document::load_mem(bytes).unwrap();
    doc.get_pages().values().map(|id| doc.get_page_content(*id)).collect()
}

fn sanitized(bytes: &[u8]) -> Vec<u8> {
    let plan = rebuild(bytes).unwrap();
    verify(bytes, &plan.bytes).unwrap();
    plan.bytes
}

// ---- 위험한 동작 제거 ----

#[test]
fn a_javascript_open_action_is_removed_and_the_page_stays() {
    let src = build("/OpenAction 5 0 R", "", &["<< /S /JavaScript /JS (app.alert(1)) >>"]);
    assert!(!security_high(&src).is_empty());
    let out = sanitized(&src);
    assert_eq!(security_high(&out), Vec::<String>::new());
    assert_eq!(page_texts(&out), page_texts(&src));
    assert_eq!(page_texts(&out).len(), 1);
}

#[test]
fn a_launch_action_on_an_annotation_is_removed() {
    let annot = "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A 6 0 R >>";
    let src = build("", "/Annots [5 0 R]", &[annot, "<< /S /Launch /F (cmd.exe) /Win << /F (cmd.exe) /P (/c calc) >> >>"]);
    assert!(!security_high(&src).is_empty());
    let out = sanitized(&src);
    assert_eq!(security_high(&out), Vec::<String>::new());
    let doc = Document::load_mem(&out).unwrap();
    assert!(doc.objects.values().all(|o| !format!("{o:?}").contains("Launch")), "Launch 동작이 남았다");
    // 링크 주석 자체는 남는다.
    assert!(doc.objects.values().any(|o| format!("{o:?}").contains("Link")));
}

#[test]
fn an_inline_additional_action_is_removed() {
    let src = build("/AA << /WC << /S /JavaScript /JS (this.print()) >> >>", "", &[]);
    let out = sanitized(&src);
    assert_eq!(security_high(&out), Vec::<String>::new());
    assert_eq!(page_texts(&out), page_texts(&src));
}

#[test]
fn a_javascript_name_tree_is_removed_with_its_actions() {
    let src = build("/Names << /JavaScript 5 0 R >>", "", &["<< /Names [(init) 6 0 R] >>", "<< /S /JavaScript /JS (var a = 1;) >>"]);
    assert!(!security_high(&src).is_empty());
    let out = sanitized(&src);
    assert_eq!(security_high(&out), Vec::<String>::new());
    let doc = Document::load_mem(&out).unwrap();
    assert!(doc.objects.values().all(|o| !format!("{o:?}").contains("var a")), "스크립트 본문이 남았다");
}

#[test]
fn an_exploit_script_in_a_stream_is_removed() {
    let js = "var s = unescape('%u9090'); util.printf('%45000f', 1);";
    let stream = format!("<< /Length {} >>\nstream\n{js}\nendstream", js.len());
    let src = build("/OpenAction 5 0 R", "", &["<< /S /JavaScript /JS 6 0 R >>", stream.as_str()]);
    assert!(security_high(&src).iter().any(|c| c.contains("exploit")), "{:?}", security_high(&src));
    let out = sanitized(&src);
    assert_eq!(security_high(&out), Vec::<String>::new());
    assert!(!String::from_utf8_lossy(&out).contains("util.printf"));
}

#[test]
fn a_clean_pdf_has_nothing_to_remove() {
    let src = build("", "", &[]);
    let plan = rebuild(&src).unwrap();
    assert!(plan.removed.is_empty());
}

#[test]
fn an_encrypted_pdf_is_refused() {
    let src = build("", "", &[]);
    let text = String::from_utf8_lossy(&src).replace("/Root 1 0 R", "/Root 1 0 R /Encrypt 99 0 R");
    assert!(rebuild(text.as_bytes()).is_err());
}

#[test]
fn verify_rejects_a_rewrite_that_changes_the_pages() {
    let src = build("/OpenAction 5 0 R", "", &["<< /S /JavaScript /JS (x) >>"]);
    let other = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>",
        "<< /Length 21 >>\nstream\nBT (Other text) Tj ET\nendstream",
    ]);
    assert!(verify(&src, &other).unwrap_err().contains("쪽"));
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
    fs::write(&p, build("/OpenAction 5 0 R /Names << /EmbeddedFiles << /Names [(a) 6 0 R] >> >>", "", &["<< /S /JavaScript /JS (x) >>", "<< /Type /Filespec /F (evil.exe) /EF << /F 7 0 R >> >>", "<< /Length 2 >>\nstream\nMZ\nendstream"])).unwrap();
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
