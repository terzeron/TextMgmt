//! PDF `--sanitize`: 위험한 동작과 스크립트를 지우고 다시 쓴다. 쪽 수와 쪽 내용 보존, 보안 재검사, 휴지통 보관과 제자리 덮어쓰기.

#[path = "../shared/doc_support/mod.rs"]
mod support;

use std::fs;

use crate::support::shared_fixtures::pdf;
use doc_checker::CheckOptions;
use doc_checker::format_dispatcher::scan_file;
use doc_checker::{Category, Severity};
use doc_repairer::pdf_sanitizer::{rebuild, verify};
use lopdf::Document;

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

#[test]
fn an_action_type_given_by_reference_is_followed_only_to_a_name() {
    // 5: /S가 이름 객체를 가리키는 JavaScript 동작. 8·9: /S가 이름이 아닌 값이라 동작으로 보지 않는다.
    let src = build("/OpenAction 5 0 R /First 8 0 R /Second 9 0 R", "", &["<< /S 6 0 R /Next 7 0 R >>", "/JavaScript", "42", "<< /S 7 0 R >>", "<< /S 42 >>"]);
    let plan = rebuild(&src).unwrap();
    assert!(plan.removed.iter().any(|(_, what)| what.contains("JavaScript 동작")), "{:?}", plan.removed);
    let out = Document::load_mem(&plan.bytes).unwrap();
    let catalog = out.catalog().unwrap();
    assert!(!catalog.has(b"OpenAction"));
    assert!(catalog.has(b"First") && catalog.has(b"Second"));
}

#[test]
fn nesting_past_the_depth_limit_is_left_alone() {
    let deep = format!("/Deep {}/JS (x){}", "[".repeat(70), "]".repeat(70));
    let src = build(&deep, "", &[]);
    let plan = rebuild(&src).unwrap();
    // 상한보다 깊은 곳의 스크립트 키는 보지 않는다.
    assert!(plan.removed.is_empty(), "{:?}", plan.removed);
}

#[test]
fn an_oversized_file_is_refused_before_parsing() {
    let huge = vec![0u8; 128 * 1024 * 1024 + 1];
    assert!(rebuild(&huge).err().is_some_and(|e| e.contains("너무 커서")));
}

#[test]
fn verify_rejects_a_changed_page_count_and_an_unreadable_original() {
    let src = build("", "", &[]);
    let two_pages = pdf(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>", &page(""), &content_stream(), &page("")]);
    assert!(verify(&src, &two_pages).unwrap_err().contains("쪽 수"));
    assert!(verify(b"not a pdf", &src).unwrap_err().contains("읽을 수 없다"));
}
