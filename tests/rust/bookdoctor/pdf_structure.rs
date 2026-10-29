//! bookdoctor PDF 구조·렌더링 검사(계획 Task 12): 코드마다 양성·음성 쌍을 둔다. Poppler는 `FakeRunner`로 대신한다.

mod common;

use std::fs;
use std::path::Path;

use bookdoctor::cli::Options;
use bookdoctor::pdf::{self, structure};
use bookdoctor::scan::scan_file;
use common::{FakeRunner, file_with};
use lopdf::{Dictionary, Document, Object};
use scan_common::{Category, FileReport, Severity};

/// 객체 본문 목록으로 PDF를 만든다. 1번 객체가 Catalog다.
fn build(bodies: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).bytes());
    out
}

/// 쪽 하나짜리 PDF. `page_extra`는 쪽 사전에, `pages_extra`는 Pages 사전에 들어간다. 4번 객체부터 `more`다.
fn pdf_with(page_extra: &str, pages_extra: &str, more: &[String]) -> Vec<u8> {
    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [3 0 R] /Count 1 {pages_extra} >>"),
        format!("<< /Type /Page /Parent 2 0 R {page_extra} >>"),
    ];
    bodies.extend_from_slice(more);
    build(&bodies)
}

fn plain() -> Vec<u8> {
    pdf_with("/MediaBox [0 0 612 792]", "", &[])
}

fn pdf_with_mediabox(b: [f32; 4]) -> Vec<u8> {
    pdf_with(&format!("/MediaBox [{} {} {} {}]", b[0], b[1], b[2], b[3]), "", &[])
}

/// 4번 객체가 FontDescriptor인 PDF.
fn pdf_with_descriptor(descriptor_body: &str) -> Vec<u8> {
    pdf_with("/MediaBox [0 0 612 792]", "", &[format!("<< /Type /FontDescriptor {descriptor_body} >>")])
}

fn opts() -> Options {
    Options { no_poppler: true, ..Options::default() }
}

fn run(bytes: &[u8], opts: &Options, runner: &FakeRunner) -> FileReport {
    let f = file_with("t.pdf", bytes);
    let mut r = FileReport::new(f.path(), None);
    pdf::scan_pdf_file_with(f.path(), opts, &mut r, runner);
    r
}

fn run_no_poppler(bytes: &[u8]) -> FileReport {
    run(bytes, &opts(), &FakeRunner::pages(1))
}

fn finding<'a>(r: &'a FileReport, code: &str) -> Option<&'a scan_common::Finding> {
    r.findings.iter().find(|f| f.code == code)
}

fn assert_flag(r: &FileReport, code: &str, category: Category, severity: Severity) {
    let f = finding(r, code).unwrap_or_else(|| panic!("{code} 없음: {r:?}"));
    assert_eq!((f.category, f.severity), (category, severity), "{code}");
}

fn assert_absent(r: &FileReport, code: &str) {
    assert!(finding(r, code).is_none(), "{code}가 있으면 안 된다: {r:?}");
}

// ---- TINY_PAGE ----

#[test]
fn flags_tiny_page_for_a_page_smaller_than_one_point() {
    let f = file_with("t.pdf", &pdf_with_mediabox([0.0, 0.0, 0.09, 0.09]));
    let mut r = FileReport::new(f.path(), None);
    pdf::scan_pdf_file_with(f.path(), &Options::default(), &mut r, &FakeRunner::pages(1));
    assert_flag(&r, "TINY_PAGE", Category::Render, Severity::High);
}

#[test]
fn tiny_page_needs_only_one_small_side() {
    assert_flag(&run_no_poppler(&pdf_with_mediabox([0.0, 0.0, 612.0, 0.5])), "TINY_PAGE", Category::Render, Severity::High);
    assert_flag(&run_no_poppler(&pdf_with_mediabox([0.0, 0.0, 0.5, 792.0])), "TINY_PAGE", Category::Render, Severity::High);
}

#[test]
fn one_point_and_normal_pages_are_not_tiny() {
    assert_absent(&run_no_poppler(&pdf_with_mediabox([0.0, 0.0, 1.0, 1.0])), "TINY_PAGE");
    assert_absent(&run_no_poppler(&plain()), "TINY_PAGE");
}

#[test]
fn tiny_page_resolves_inherited_mediabox_from_pages_parent() {
    let r = run_no_poppler(&pdf_with("", "/MediaBox [0 0 0.09 0.09]", &[]));
    assert_flag(&r, "TINY_PAGE", Category::Render, Severity::High);
    // 쪽이 자기 MediaBox를 가지면 부모 값보다 우선한다.
    let r = run_no_poppler(&pdf_with("/MediaBox [0 0 612 792]", "/MediaBox [0 0 0.09 0.09]", &[]));
    assert_absent(&r, "TINY_PAGE");
}

#[test]
fn tiny_page_is_detected_on_any_page_not_just_the_first() {
    let bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 612 792] >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 0.09 0.09] >>".to_owned(),
    ];
    assert_flag(&run_no_poppler(&build(&bodies)), "TINY_PAGE", Category::Render, Severity::High);
}

#[test]
fn page_without_any_mediabox_is_not_flagged_tiny() {
    assert_absent(&run_no_poppler(&pdf_with("", "", &[])), "TINY_PAGE");
}

// ---- NO_PDF_HEADER / NO_EOF_MARKER / XREF_BROKEN ----

#[test]
fn no_pdf_header_is_flagged_and_pdf_extension_adds_format_mismatch() {
    let f = file_with("x.pdf", b"hello, this is plain text\n%%EOF\n");
    let r = scan_file(f.path(), &opts());
    assert_flag(&r, "NO_PDF_HEADER", Category::Defect, Severity::High);
    assert_flag(&r, "FORMAT_MISMATCH", Category::Defect, Severity::Medium);
}

#[test]
fn headerless_file_stays_incomplete_because_security_scan_did_not_run() {
    let r = run_no_poppler(b"hello");
    assert!(r.incomplete && r.reasons.iter().any(|m| m == "no %PDF- header"), "{r:?}");
}

#[test]
fn header_present_has_no_header_finding() {
    assert_absent(&run_no_poppler(&plain()), "NO_PDF_HEADER");
}

#[test]
fn header_beyond_first_kib_does_not_count() {
    let mut data = vec![b'\n'; 2000];
    data.extend(plain());
    assert_flag(&run_no_poppler(&data), "NO_PDF_HEADER", Category::Defect, Severity::High);
}

#[test]
fn missing_eof_marker_is_flagged_and_present_one_is_not() {
    let full = plain();
    assert_absent(&run_no_poppler(&full), "NO_EOF_MARKER");
    let cut = &full[..full.len() - 6];
    assert_flag(&run_no_poppler(cut), "NO_EOF_MARKER", Category::Defect, Severity::Medium);
}

#[test]
fn parse_error_yields_xref_broken_and_incomplete() {
    let r = run_no_poppler(b"%PDF-1.7\nthis is not a pdf at all\n");
    assert_flag(&r, "XREF_BROKEN", Category::Defect, Severity::High);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.starts_with("parse error")), "{r:?}");
    assert_absent(&run_no_poppler(&plain()), "XREF_BROKEN");
}

#[test]
fn truncated_pdf_reports_structure_defects_not_abort() {
    let full = plain();
    let d = tempfile::tempdir().unwrap();
    for cut in [full.len() / 4, full.len() / 2, full.len() - 40] {
        let p = d.path().join(format!("cut{cut}.pdf"));
        fs::write(&p, &full[..cut]).unwrap();
        let r = scan_file(&p, &opts());
        assert!(finding(&r, "NO_EOF_MARKER").is_some() || finding(&r, "XREF_BROKEN").is_some(), "cut={cut}: {r:?}");
        assert!(!r.reasons.iter().any(|m| m.starts_with("panic")), "cut={cut}: {:?}", r.reasons);
    }
    // 앞부분만 남으면 구문 분석도 실패하므로 incomplete다.
    let p = d.path().join("quarter.pdf");
    fs::write(&p, &full[..full.len() / 4]).unwrap();
    assert!(scan_file(&p, &opts()).incomplete);
}

// ---- NO_ROOT / ZERO_PAGES / ENCRYPTED (문서를 직접 구성한다) ----

fn inspect_doc(doc: &Document) -> FileReport {
    let mut r = FileReport::new(Path::new("x.pdf"), None);
    structure::inspect_with(b"%PDF-1.7\n%%EOF\n", Some(doc), bookdoctor::pdf::security::LoadInfo::default(), None, Path::new("x.pdf"), &opts(), &mut r, &FakeRunner::pages(1));
    r
}

fn doc_with_pages(kids: Vec<Object>) -> Document {
    let mut doc = Document::with_version("1.7");
    let mut cat = Dictionary::new();
    cat.set("Type", "Catalog");
    cat.set("Pages", Object::Reference((2, 0)));
    doc.objects.insert((1, 0), Object::Dictionary(cat));
    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Count", kids.len() as i64);
    pages.set("Kids", Object::Array(kids));
    doc.objects.insert((2, 0), Object::Dictionary(pages));
    doc.trailer.set("Root", Object::Reference((1, 0)));
    doc.max_id = 2;
    doc
}

#[test]
fn flags_missing_root() {
    let mut doc = doc_with_pages(vec![]);
    doc.trailer.remove(b"Root");
    let r = inspect_doc(&doc);
    assert_flag(&r, "NO_ROOT", Category::Defect, Severity::High);
    assert_absent(&r, "ZERO_PAGES");
}

#[test]
fn root_present_has_no_root_finding() {
    assert_absent(&inspect_doc(&doc_with_pages(vec![])), "NO_ROOT");
}

#[test]
fn flags_zero_pages_only_when_there_are_none() {
    assert_flag(&inspect_doc(&doc_with_pages(vec![])), "ZERO_PAGES", Category::Defect, Severity::High);
    let mut doc = doc_with_pages(vec![Object::Reference((3, 0))]);
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", Object::Reference((2, 0)));
    doc.objects.insert((3, 0), Object::Dictionary(page));
    assert_absent(&inspect_doc(&doc), "ZERO_PAGES");
}

#[test]
fn flags_encrypted_trailer() {
    let mut doc = doc_with_pages(vec![]);
    assert_absent(&inspect_doc(&doc), "ENCRYPTED");
    doc.trailer.set("Encrypt", Object::Reference((9, 0)));
    assert_flag(&inspect_doc(&doc), "ENCRYPTED", Category::Render, Severity::High);
}

// ---- POPPLER_OPEN_FAILED ----

#[test]
fn poppler_failure_is_flagged_and_success_is_not() {
    let o = Options::default();
    let r = run(&plain(), &o, &FakeRunner::fails("Syntax Error: Couldn't read xref table"));
    assert_flag(&r, "POPPLER_OPEN_FAILED", Category::Render, Severity::High);
    assert_absent(&run(&plain(), &o, &FakeRunner::pages(3)), "POPPLER_OPEN_FAILED");
}

#[test]
fn poppler_not_installed_is_incomplete_not_a_finding() {
    let r = run(&plain(), &Options::default(), &FakeRunner::not_found());
    assert_absent(&r, "POPPLER_OPEN_FAILED");
    assert!(r.incomplete && r.reasons.iter().any(|m| m == "poppler not installed"), "{r:?}");
}

#[test]
fn no_poppler_option_never_calls_the_runner() {
    let runner = FakeRunner::fails("x");
    let o = Options { no_poppler: true, pdf_text: true, ..Options::default() };
    let r = run(&plain(), &o, &runner);
    assert_eq!(runner.call_count(), 0);
    assert_absent(&r, "POPPLER_OPEN_FAILED");
}

#[test]
fn poppler_page_count_runs_once_and_text_is_probed_only_on_request() {
    let runner = FakeRunner::pages(3);
    run(&plain(), &Options::default(), &runner);
    assert_eq!(runner.programs(), ["pdfinfo"]);
    let runner = FakeRunner::pages(3);
    run(&plain(), &Options { pdf_text: true, ..Options::default() }, &runner);
    assert_eq!(runner.programs(), ["pdfinfo", "pdftotext"]);
}

#[test]
fn poppler_is_still_asked_when_lopdf_cannot_parse_but_never_without_header() {
    let o = Options::default();
    let runner = FakeRunner::fails("Syntax Error");
    let r = run(b"%PDF-1.7\nnot a pdf at all\n", &o, &runner);
    assert_eq!(runner.call_count(), 1);
    assert_flag(&r, "XREF_BROKEN", Category::Defect, Severity::High);
    assert_flag(&r, "POPPLER_OPEN_FAILED", Category::Render, Severity::High);
    let runner = FakeRunner::fails("Syntax Error");
    run(b"no header here", &o, &runner);
    assert_eq!(runner.call_count(), 0);
}

#[test]
fn poppler_timeout_is_incomplete_not_a_finding() {
    let r = run(&plain(), &Options::default(), &FakeRunner::timeout());
    assert_absent(&r, "POPPLER_OPEN_FAILED");
    assert!(r.incomplete && r.reasons.iter().any(|m| m == "poppler timed out"), "{r:?}");
}

#[test]
fn poppler_spawn_error_is_incomplete_not_a_finding() {
    let r = run(&plain(), &Options::default(), &FakeRunner::spawn_error("permission denied\x1b[31m"));
    assert_absent(&r, "POPPLER_OPEN_FAILED");
    let reason = r.reasons.iter().find(|m| m.starts_with("poppler failed to run: ")).unwrap_or_else(|| panic!("{r:?}"));
    assert!(reason.contains("permission denied") && !reason.contains('\x1b'), "{reason:?}");
    assert!(r.incomplete);
}

#[test]
fn nonzero_pdfinfo_exit_is_still_a_finding_and_not_incomplete() {
    let r = run(&plain(), &Options::default(), &FakeRunner::fails("Syntax Error: bad"));
    assert_flag(&r, "POPPLER_OPEN_FAILED", Category::Render, Severity::High);
    assert!(!r.incomplete, "{r:?}");
}

fn encrypted_doc() -> Document {
    let mut doc = doc_with_pages(vec![]);
    doc.trailer.set("Encrypt", Object::Reference((9, 0)));
    doc
}

fn inspect_doc_with(doc: &Document, runner: &FakeRunner) -> FileReport {
    let mut r = FileReport::new(Path::new("x.pdf"), None);
    let o = Options::default();
    structure::inspect_with(b"%PDF-1.7\n%%EOF\n", Some(doc), bookdoctor::pdf::security::LoadInfo::default(), None, Path::new("x.pdf"), &o, &mut r, runner);
    r
}

#[test]
fn encrypted_document_does_not_also_report_zero_pages() {
    let r = inspect_doc(&encrypted_doc());
    assert_flag(&r, "ENCRYPTED", Category::Render, Severity::High);
    assert_absent(&r, "ZERO_PAGES");
}

#[test]
fn encrypted_file_suppresses_poppler_failure_that_mentions_a_password() {
    // 실제 pdfinfo(qpdf 256비트 암호화 파일): "Command Line Error: Incorrect password"
    let r = inspect_doc_with(&encrypted_doc(), &FakeRunner::fails("Command Line Error: Incorrect password"));
    assert_flag(&r, "ENCRYPTED", Category::Render, Severity::High);
    assert_absent(&r, "POPPLER_OPEN_FAILED");
}

#[test]
fn encrypted_file_keeps_poppler_failure_with_unrelated_message() {
    let r = inspect_doc_with(&encrypted_doc(), &FakeRunner::fails("Syntax Error: Couldn't read xref table"));
    assert_flag(&r, "POPPLER_OPEN_FAILED", Category::Render, Severity::High);
}

#[test]
fn unencrypted_file_keeps_poppler_failure_even_if_message_mentions_a_password() {
    let r = inspect_doc_with(&doc_with_pages(vec![]), &FakeRunner::fails("Incorrect password"));
    assert_flag(&r, "POPPLER_OPEN_FAILED", Category::Render, Severity::High);
}

// ---- FONT_NOT_EMBEDDED ----

#[test]
fn flags_font_descriptor_without_font_file() {
    let r = run_no_poppler(&pdf_with_descriptor("/FontName /MyriadPro-Regular"));
    assert_flag(&r, "FONT_NOT_EMBEDDED", Category::Defect, Severity::Low);
    assert!(finding(&r, "FONT_NOT_EMBEDDED").unwrap().detail.contains("MyriadPro-Regular"));
}

#[test]
fn embedded_fonts_are_not_flagged_for_any_font_file_key() {
    for key in ["FontFile", "FontFile2", "FontFile3"] {
        let r = run_no_poppler(&pdf_with_descriptor(&format!("/FontName /MyriadPro-Regular /{key} 9 0 R")));
        assert_absent(&r, "FONT_NOT_EMBEDDED");
    }
}

#[test]
fn base_14_fonts_are_not_flagged_even_with_subset_prefix() {
    for name in ["Helvetica", "Times-Roman", "Courier-BoldOblique", "Symbol", "ZapfDingbats", "ABCDEF+Helvetica-Bold"] {
        assert_absent(&run_no_poppler(&pdf_with_descriptor(&format!("/FontName /{name}"))), "FONT_NOT_EMBEDDED");
    }
}

#[test]
fn fonts_without_descriptor_and_type3_are_not_flagged() {
    let more = vec!["<< /Type /Font /Subtype /Type1 /BaseFont /Foo >>".to_owned(), "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] >>".to_owned()];
    assert_absent(&run_no_poppler(&pdf_with("/MediaBox [0 0 612 792]", "", &more)), "FONT_NOT_EMBEDDED");
}

#[test]
fn common_windows_aliases_of_base_14_fonts_are_not_flagged() {
    for name in ["Arial", "Arial,Bold", "Arial-BoldMT", "ArialMT", "TimesNewRoman", "TimesNewRomanPSMT", "TimesNewRomanPS-BoldMT", "CourierNew", "CourierNewPSMT", "CourierNew,Italic", "ABCDEF+Arial-ItalicMT"] {
        assert_absent(&run_no_poppler(&pdf_with_descriptor(&format!("/FontName /{name}"))), "FONT_NOT_EMBEDDED");
    }
}

#[test]
fn custom_or_look_alike_font_names_are_still_flagged() {
    for name in ["ArialNarrow", "ArialUnicodeMS", "TimesTen-Roman", "CourierPrimeCode", "Calibri"] {
        assert_flag(&run_no_poppler(&pdf_with_descriptor(&format!("/FontName /{name}"))), "FONT_NOT_EMBEDDED", Category::Defect, Severity::Low);
    }
}

// ---- NO_TEXT_LAYER ----

#[test]
fn flags_missing_text_layer_only_with_pdf_text() {
    let o = Options { pdf_text: true, ..Options::default() };
    assert_flag(&run(&plain(), &o, &FakeRunner::text(" \n\t\x0c")), "NO_TEXT_LAYER", Category::Defect, Severity::Low);
    assert_absent(&run(&plain(), &o, &FakeRunner::text("본문")), "NO_TEXT_LAYER");
    let runner = FakeRunner::text("");
    assert_absent(&run(&plain(), &Options::default(), &runner), "NO_TEXT_LAYER");
    assert_eq!(runner.programs(), ["pdfinfo"]);
}

#[test]
fn text_probe_is_skipped_for_zero_pages() {
    let o = Options { pdf_text: true, ..Options::default() };
    let runner = FakeRunner::pages(0);
    run(&plain(), &o, &runner);
    assert_eq!(runner.programs(), ["pdfinfo"]);
}

// ---- 파이프라인·읽기 ----

#[test]
fn pipeline_flags_tiny_page_through_scan_file() {
    let f = file_with("manga.pdf", &pdf_with_mediabox([0.0, 0.0, 0.09, 0.09]));
    let r = scan_file(f.path(), &opts());
    assert_flag(&r, "TINY_PAGE", Category::Render, Severity::High);
}

#[test]
fn unreadable_file_is_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    let f = file_with("locked.pdf", &plain());
    fs::set_permissions(f.path(), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(f.path()).is_ok() {
        eprintln!("skip: root는 권한 없는 파일도 읽는다");
        return;
    }
    let mut r = FileReport::new(f.path(), None);
    pdf::scan_pdf_file(f.path(), &opts(), &mut r);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.starts_with("unreadable")), "{r:?}");
}

#[test]
fn read_is_bounded_by_max_total_mb() {
    let mut big = plain();
    big.extend(vec![b'%'; 2 * 1024 * 1024]);
    let f = file_with("big.pdf", &big);
    let mut r = FileReport::new(f.path(), None);
    pdf::scan_pdf_file(f.path(), &Options { max_total_mb: 1, no_poppler: true, ..Options::default() }, &mut r);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.starts_with("skipped")), "{r:?}");
}

#[test]
fn real_poppler_smoke_when_installed() {
    let installed = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("pdfinfo").is_file()));
    if !installed {
        eprintln!("skip: pdfinfo가 없다");
        return;
    }
    let f = file_with("ok.pdf", &plain());
    let r = scan_file(f.path(), &Options::default());
    assert_absent(&r, "POPPLER_OPEN_FAILED");
}

// ---- 긴 /Kids 배열 (실제 load 경로, 객체 16개 이상이라 lopdf가 rayon으로 읽는다) ----

fn pdf_bytes_with_kids(kids: u32) -> Vec<u8> {
    let mut doc = doc_with_pages((0..kids).map(|n| Object::Reference((10 + n, 0))).collect());
    for n in 0..kids {
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", Object::Reference((2, 0)));
        doc.objects.insert((10 + n, 0), Object::Dictionary(page));
    }
    doc.max_id = 10 + kids;
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn oversized_kids_array_is_incomplete_not_zero_pages_through_the_real_load() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("big.pdf");
    fs::write(&p, pdf_bytes_with_kids(25_000)).unwrap();
    let r = scan_file(&p, &opts());
    assert_absent(&r, "ZERO_PAGES");
    assert!(r.incomplete, "{r:?}");
}

#[test]
fn normal_kids_array_through_the_real_load_is_complete() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("ok.pdf");
    fs::write(&p, pdf_bytes_with_kids(100)).unwrap();
    let r = scan_file(&p, &opts());
    assert_absent(&r, "ZERO_PAGES");
    assert!(!r.incomplete, "{r:?}");
}
