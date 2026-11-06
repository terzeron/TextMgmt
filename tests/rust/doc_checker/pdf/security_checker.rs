//! `doc_checker::pdf::security_checker` 단위 테스트.

#[path = "../../shared/doc_support/mod.rs"]
mod support;
use crate::support::shared_fixtures::pdf;
use doc_checker::CheckOptions;
use doc_checker::format_dispatcher::scan_file;
use doc_checker::pdf::security_checker;
use doc_checker::{Category, FileReport, Severity};
use lopdf::{Dictionary, Document, Object, StringFormat};
use std::fs;

const PAGES: &str = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";

const PAGE: &str = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>";

fn catalog(extra: &str) -> String {
    format!("<< /Type /Catalog /Pages 2 0 R {extra} >>")
}

/// 카탈로그에 `extra`를 달고 `more`를 4번 객체부터 붙인 PDF를 검사한다.
fn scan_pdf(extra: &str, more: &[&str]) -> (i32, String) {
    let cat = catalog(extra);
    let mut objs = vec![cat.as_str(), PAGES, PAGE];
    objs.extend_from_slice(more);
    scan_bytes(&pdf(&objs))
}

fn scan_bytes(bytes: &[u8]) -> (i32, String) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.pdf");
    fs::write(&p, bytes).unwrap();
    go(&p, &CheckOptions { no_pdf_worker: true, ..CheckOptions::default() })
}

fn pdf_with_large_object_stream(expanded_bytes: usize) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", Object::Reference((2, 0)));
    catalog.set("JS", Object::String(vec![b'A'; expanded_bytes], StringFormat::Literal));
    doc.objects.insert((1, 0), Object::Dictionary(catalog));
    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", Object::Array(vec![Object::Reference((3, 0))]));
    pages.set("Count", 1);
    doc.objects.insert((2, 0), Object::Dictionary(pages));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", Object::Reference((2, 0)));
    doc.objects.insert((3, 0), Object::Dictionary(page));

    doc.trailer.set("Root", Object::Reference((1, 0)));
    doc.max_id = 3;
    let mut bytes = Vec::new();
    doc.save_modern(&mut bytes).unwrap();
    bytes
}

/// 옛 `print_text`와 같은 모양으로 폈다. 지표 줄은 `[등급] code (location): detail`이다.
/// 공통 검사(`support::inspect`)가 더하는 Defect 지표는 이 이식 범위가 아니므로 Security만 편다.
fn render(r: &FileReport) -> String {
    let security: Vec<_> = r.findings.iter().filter(|f| f.category == Category::Security).collect();
    let status = if !security.is_empty() {
        "INDICATORS"
    } else if r.incomplete {
        "INCOMPLETE"
    } else {
        "clean"
    };
    let mut out = format!("{}: {status}\n", r.path);
    for n in &r.reasons {
        out.push_str(&format!("  note: {n}\n"));
    }
    for f in security {
        out.push_str(&format!("  [{}] {} ({})", f.severity.label(), f.code, f.location));
        if !f.detail.is_empty() {
            out.push_str(&format!(": {}", f.detail));
        }
        out.push('\n');
    }
    out
}

/// 옛 종료 코드: 지표 있으면 1, 불완전하면 2, 아니면 0.
fn verdict(r: &FileReport) -> (i32, String) {
    let code = if r.findings.iter().any(|f| f.category == Category::Security) {
        1
    } else if r.incomplete {
        2
    } else {
        0
    };
    (code, render(r))
}

fn go(path: &std::path::Path, opts: &CheckOptions) -> (i32, String) {
    verdict(&scan_file(path, opts))
}

#[test]
fn clean_pdf_exits_zero() {
    let (code, out) = scan_pdf("", &[]);
    assert_eq!(code, 0);
    assert!(out.contains(": clean"));
}

#[test]
fn javascript_and_open_action_are_flagged_even_with_escaped_names() {
    let (code, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /Java#53cript /JS (app.alert\\(1\\)\u{1}) >>"]);
    assert_eq!(code, 1);
    assert!(out.contains("[HIGH] JavaScript action"), "{out}");
    assert!(out.contains("app.alert(1)."), "제어문자는 점으로 바뀐다: {out}");
    assert!(out.contains("[MEDIUM] OpenAction"));
}

#[test]
fn launch_actions_and_platform_parameters_are_flagged() {
    let (code, out) = scan_pdf("/AA << /WC 4 0 R >>", &["<< /S /Launch /F (calc.exe) /Win << /F (cmd.exe) /P (/c calc) >> >>"]);
    assert_eq!(code, 1);
    for needle in ["Launch action", "executable-looking file name", "platform launch parameters", "cmd.exe /c calc", "additional-actions trigger"] {
        assert!(out.contains(needle), "{needle}: {out}");
    }
}

#[test]
fn external_and_media_actions_are_medium_and_uri_is_low() {
    let (_, out) = scan_pdf(
        "/OpenAction [4 0 R 5 0 R 6 0 R 7 0 R]",
        &["<< /S /SubmitForm /F (http://x.example/post) >>", "<< /S /GoToR /F (other.pdf) >>", "<< /S /URI /URI (http://x.example/) >>", "<< /S /Sound >>"],
    );
    assert!(out.contains("[MEDIUM] external/media action") && out.contains("/S /SubmitForm") && out.contains("/S /GoToR") && out.contains("/S /Sound"), "{out}");
    assert!(out.contains("[LOW] URI action") && out.contains("http://x.example/"), "{out}");
}

#[test]
fn rich_media_execute_is_high() {
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /RichMediaExecute >>"]);
    assert!(out.contains("[HIGH] RichMediaExecute action"), "{out}");
}

#[test]
fn embedded_files_attachments_and_other_active_keys_are_flagged() {
    let (_, out) = scan_pdf(
        "/Names << /EmbeddedFiles << /Names [(a.exe) 4 0 R] >> /JavaScript << /Names [(j) 5 0 R] >> >> /XFA 6 0 R",
        &[
            "<< /Type /Filespec /F (payload.exe) /EF << /F 6 0 R >> >>",
            "<< /S /JavaScript /JS (x) >>",
            "<< /Type /Annot /Subtype /FileAttachment >>",
            "<< /Subtype /Screen /3DD 3 0 R /RichMediaContent 3 0 R >>",
        ],
    );
    for needle in ["embedded file", "JavaScript name tree", "XFA form", "file attachment annotation", "screen annotation", "3D content", "RichMedia content", "executable-looking file name"] {
        assert!(out.contains(needle), "{needle}: {out}");
    }
}

#[test]
fn script_outside_an_action_dictionary_and_stream_sources_are_reported() {
    let stream = "<< /Length 12 >>\nstream\napp.alert(2)\nendstream";
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 5 0 R >>", stream, "<< /FT /Tx /JS 7 0 R >>", "(hello from a string object)"]);
    assert!(out.contains("(stream): app.alert(2)"), "{out}");
    assert!(out.contains("[HIGH] JavaScript code"), "{out}");
}

#[test]
fn references_to_other_objects_and_missing_objects_are_shown_by_number() {
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 5 0 R >>", "42"]);
    assert!(out.contains("5 0 R"), "{out}");
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 99 0 R >>"]);
    assert!(out.contains("99 0 R") || out.contains("JavaScript action"), "{out}");
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 7 >>"]);
    assert!(out.contains("JavaScript action"), "{out}");
}

#[test]
fn risky_file_names_are_found_through_trailing_dots_and_uppercase() {
    for name in ["run.EXE", "run.exe.", "run.exe  ", "x.Js", "x.vbs"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /Type /Filespec /UF ({name}) /EF << >> >>")]);
        assert!(out.contains("executable-looking file name"), "{name}: {out}");
    }
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /Type /Filespec /F (notes.txt) /EF << >> >>"]);
    assert!(!out.contains("executable-looking"), "{out}");
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /Type /Filespec /F (README) /EF << >> >>"]);
    assert!(!out.contains("executable-looking"), "{out}");
}

#[test]
fn deeply_nested_structures_stop_at_the_depth_limit() {
    let nested = format!("{}1{}", "[".repeat(150), "]".repeat(150));
    let (_, out) = scan_pdf("", &[&format!("<< /X {nested} >>")]);
    assert!(out.contains("nesting limit exceeded") || out.contains("INCOMPLETE") || out.contains("clean"), "{out}");
}

#[test]
fn long_previews_are_truncated_and_names_with_control_characters_are_sanitized() {
    let long = "A".repeat(500);
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /JavaScript /JS ({long}) >>")]);
    assert!(out.contains(&format!("{}…", "A".repeat(100))), "{out}");
    assert!(!out.contains(&"A".repeat(101)));
}

// ---- 파일과 디렉토리 ----

#[test]
fn unreadable_and_non_pdf_inputs_are_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let not_pdf = d.path().join("a.pdf");
    fs::write(&not_pdf, b"hello").unwrap();
    let (code, out) = go(&not_pdf, &CheckOptions { no_pdf_worker: true, ..CheckOptions::default() });
    assert_eq!(code, 2);
    assert!(out.contains("INCOMPLETE") && out.contains("no %PDF- header"), "{out}");

    let (code, out) = go(&d.path().join("missing.pdf"), &CheckOptions { no_pdf_worker: true, ..CheckOptions::default() });
    assert_eq!(code, 2);
    assert!(out.contains("unreadable"), "{out}");

    let garbage = d.path().join("g.pdf");
    fs::write(&garbage, b"%PDF-1.7\nthis is not a pdf at all\n").unwrap();
    let (code, out) = go(&garbage, &CheckOptions { no_pdf_worker: true, ..CheckOptions::default() });
    assert_eq!(code, 2);
    assert!(out.contains("parse error") || out.contains("panicked"), "{out}");
}

#[test]
fn files_over_max_total_mb_are_skipped_as_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.pdf");
    // 크기 제한은 파싱 전에 검사하므로 뒤에 붙인 채움 데이터도 상관없다.
    let mut big = pdf(&[&catalog(""), PAGES, PAGE]);
    big.extend(vec![b'%'; 2 * 1024 * 1024]);
    fs::write(&p, big).unwrap();
    let (code, out) = go(&p, &CheckOptions { max_total_mb: 1, no_pdf_worker: true, ..CheckOptions::default() });
    assert_eq!(code, 2);
    assert!(out.contains("exceeds --max-total-mb"), "{out}");
}

#[test]
fn oversized_compressed_object_stream_is_rejected_as_incomplete() {
    let bytes = pdf_with_large_object_stream(20 * 1024 * 1024);
    assert!(bytes.len() < 1024 * 1024, "fixture must be a highly compressed PDF");
    let (code, out) = scan_bytes(&bytes);
    assert_eq!(code, 2, "oversized object stream must not be scanned as complete: {out}");
    assert!(out.contains("INCOMPLETE") && out.contains("object stream"), "{out}");
}

// ---- 인자 ----

// ---- CRITICAL ----

#[test]
fn launch_of_executables_and_shells_is_critical_but_launch_of_a_document_is_high() {
    for target in ["(calc.exe)", "(C:\\\\Windows\\\\System32\\\\cmd.exe)", "(powershell -enc AAAA)", "<< /Type /Filespec /F (run.bat) >>"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {target} >>")]);
        assert!(out.contains("[CRITICAL] Launch action"), "{target}: {out}");
    }
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /Launch /F (readme.txt) >>"]);
    assert!(out.contains("[HIGH] Launch action") && !out.contains("CRITICAL"), "{out}");
}

#[test]
fn platform_launch_parameters_naming_a_shell_are_critical() {
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /Launch /Win << /F (cmd.exe) /P (/c calc) >> >>"]);
    assert!(out.contains("[CRITICAL] platform launch parameters"), "{out}");
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /Launch /F (readme.txt) /Win << /F (notes.txt) >> >>"]);
    assert!(out.contains("[HIGH] platform launch parameters") && !out.contains("CRITICAL"), "{out}");
}

#[test]
fn known_exploit_javascript_is_critical_and_ordinary_javascript_stays_high() {
    for js in [
        "this.exportDataObject({ cName: 'a.exe', nLaunch: 2 });",
        "app.launchURL('file:///C:/x.exe');",
        "eval(unescape('%61%6c'));",
        "eval ( String.fromCharCode(97,98) )",
        "var s = unescape('%u9090%u9090%u9090%u9090%uc0de');",
    ] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /JavaScript /JS ({js}) >>")]);
        assert!(out.contains("[CRITICAL] exploit JavaScript"), "{js}: {out}");
    }
    // 같은 JS가 JS 키로만 있어도(action 사전이 아니어도) 같다.
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /FT /Tx /JS (eval\\(unescape\\('x'\\)\\)) >>"]);
    assert!(out.contains("[CRITICAL] exploit JavaScript"), "{out}");
    // 괄호가 균형이면 PDF 리터럴 문자열에서 이스케이프하지 않아도 된다.
    for js in ["app.alert('hi');", "event.value = util.printd('yyyy', new Date());", "AFNumber_Format(2, 0, 0, 0, '', true);"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /JavaScript /JS ({js}) >>")]);
        assert!(out.contains("[HIGH] JavaScript action") && !out.contains("CRITICAL"), "{js}: {out}");
    }
}

#[test]
fn utf16_encoded_exploit_javascript_is_still_found() {
    // UTF-16BE(BOM) 16진 문자열로 인코딩만 바꿔도 탐지를 피하면 안 된다.
    let hex = |src: &str, be: bool| -> String {
        let bom = if be { "FEFF" } else { "FFFE" };
        let body: String = src.encode_utf16().map(|u| if be { format!("{u:04X}") } else { format!("{:02X}{:02X}", u & 0xff, u >> 8) }).collect();
        format!("<{bom}{body}>")
    };
    for be in [true, false] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /JavaScript /JS {} >>", hex("eval(unescape('x'))", be))]);
        assert!(out.contains("[CRITICAL] exploit JavaScript"), "be={be}: {out}");
    }
}

#[test]
fn a_dot_com_domain_is_not_an_executable_but_a_url_to_an_executable_is() {
    // `.com`은 DOS 실행 파일 확장자이기도 하다. 도메인 이름의 .com을 실행 파일로 읽으면 안 된다.
    for target in ["(https://example.com)", "(http://www.example.com/)", "(https://example.com/a?x=1.exe)", "(mailto:a@example.com)"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {target} >>")]);
        assert!(out.contains("[HIGH] Launch action") && !out.contains("CRITICAL") && !out.contains("executable-looking"), "{target}: {out}");
    }
    for target in ["(https://x.example/dl/evil.exe)", "(http://x.example/a.bat#frag)", "(FILE://host/share/run.msi)"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {target} >>")]);
        assert!(out.contains("[CRITICAL] Launch action"), "{target}: {out}");
    }
    // 로컬 `x.com`은 여전히 실행 파일이다.
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /Launch /F (setup.com) >>"]);
    assert!(out.contains("[CRITICAL] Launch action"), "{out}");
}

// ---- 로딩 중 줄이기(메모리) ----

#[test]
fn huge_numeric_arrays_do_not_change_what_is_reported() {
    // 숫자만 든 큰 배열은 로딩 중에 비우지만, 같은 문서의 지표는 그대로여야 한다.
    let numbers = "0 ".repeat(200_000);
    let (code, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F (calc.exe) /Widths [{numbers}] >>"), &format!("[{numbers}]")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[CRITICAL] Launch action"), "{out}");
}

#[test]
fn a_dictionary_hidden_in_a_long_array_is_still_scanned() {
    // 길이가 임계(32)를 넘는 배열이라도 숫자·이름·참조 말고 다른 원소가 있으면 비우지 않는다.
    let numbers = "7 ".repeat(40);
    let (code, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("[{numbers} << /S /Launch /F (calc.exe) >> {numbers}]")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[CRITICAL] Launch action"), "{out}");
    // 문자열이 섞여 있어도 마찬가지다.
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("[{numbers} (note) << /S /JavaScript /JS (app.alert(1)) >>]")]);
    assert!(out.contains("[HIGH] JavaScript action"), "{out}");
}

#[test]
fn javascript_in_a_stream_over_the_keep_limit_is_reported_as_not_fully_inspected() {
    // 6만 4천 바이트가 넘는 스트림은 로딩 중에 잘린다. 그 스트림이 JavaScript면 끝까지 못 본 것이므로 알려야 한다.
    let body = "x".repeat(300_000);
    let stream = format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len());
    let (code, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 5 0 R >>", &stream]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[HIGH] JavaScript stream not fully inspected"), "{out}");
    // 작은 스트림에는 붙지 않는다.
    let small = "<< /Length 12 >>\nstream\napp.alert(2)\nendstream";
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 5 0 R >>", small]);
    assert!(!out.contains("not fully inspected"), "{out}");
}

/// Flate로 압축한 JavaScript 스트림 하나를 OpenAction으로 연 PDF.
fn pdf_with_flate_javascript(source: &str) -> (Vec<u8>, usize) {
    let mut doc = Document::with_version("1.7");
    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", Object::Reference((2, 0)));
    catalog.set("OpenAction", Object::Reference((4, 0)));
    doc.objects.insert((1, 0), Object::Dictionary(catalog));
    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", Object::Array(vec![Object::Reference((3, 0))]));
    pages.set("Count", 1);
    doc.objects.insert((2, 0), Object::Dictionary(pages));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", Object::Reference((2, 0)));
    doc.objects.insert((3, 0), Object::Dictionary(page));
    let mut action = Dictionary::new();
    action.set("S", Object::Name(b"JavaScript".to_vec()));
    action.set("JS", Object::Reference((5, 0)));
    doc.objects.insert((4, 0), Object::Dictionary(action));
    let mut stream = lopdf::Stream::new(Dictionary::new(), source.as_bytes().to_vec());
    stream.compress().unwrap();
    let compressed = stream.content.len();
    doc.objects.insert((5, 0), Object::Stream(stream));
    doc.trailer.set("Root", Object::Reference((1, 0)));
    doc.max_id = 5;
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    (bytes, compressed)
}

#[test]
fn exploit_javascript_that_only_decompresses_past_64kib_is_still_found() {
    // 압축하면 작지만 풀면 미리보기 한도(64KB)를 넘는 JS. 미리보기 한도에서 잘라 압축 원본으로 되돌리면 탐지를 피한다.
    let js = format!("{}eval(unescape('x'));", "var a = 1;\n".repeat(20_000));
    assert!(js.len() > 64 * 1024);
    let (bytes, compressed) = pdf_with_flate_javascript(&js);
    assert!(compressed < 64 * 1024, "압축본이 {compressed}바이트라 보존 한도를 넘는다. 이 테스트는 한도 아래여야 한다");
    let (_, out) = scan_bytes(&bytes);
    assert!(out.contains("[CRITICAL] exploit JavaScript"), "{out}");
    assert!(!out.contains("not fully inspected"), "{out}");
}

#[test]
fn javascript_too_big_to_keep_is_reported_instead_of_silently_passing() {
    // 압축 안 한 110KB JS는 보존 한도(64KB)에서 잘린다. 끝의 exploit을 못 보더라도 "다 못 봤다"는 사실은 남겨야 한다.
    let js = format!("{}eval(unescape('x'));", "var a = 1;\n".repeat(10_000));
    let stream = format!("<< /Length {} >>\nstream\n{js}\nendstream", js.len());
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 5 0 R >>", &stream]);
    assert!(out.contains("[HIGH] JavaScript stream not fully inspected"), "{out}");
    assert!(out.contains("[HIGH] JavaScript action"), "{out}");
}

/// 압축된 객체 스트림 하나가 로딩 중 보존 한도(64KB)를 넘는 PDF. 안에 Launch 동작이 하나 들어 있다.
fn pdf_with_object_stream_over_the_keep_limit() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", Object::Reference((2, 0)));
    doc.objects.insert((1, 0), Object::Dictionary(catalog));
    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", Object::Array(vec![Object::Reference((3, 0))]));
    pages.set("Count", 1);
    doc.objects.insert((2, 0), Object::Dictionary(pages));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", Object::Reference((2, 0)));
    doc.objects.insert((3, 0), Object::Dictionary(page));
    let mut launch = Dictionary::new();
    launch.set("S", Object::Name(b"Launch".to_vec()));
    launch.set("F", Object::String(b"calc.exe".to_vec(), StringFormat::Literal));
    doc.objects.insert((4, 0), Object::Dictionary(launch));
    // 압축이 안 되는 값으로 채운다(결정적인 의사난수). 압축된 객체 스트림이 64KB를 넘어야 한다.
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for id in 5..4005u32 {
        let mut filler = Dictionary::new();
        let noise: Vec<u8> = (0..160)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x & 0xff) as u8
            })
            .collect();
        filler.set("Noise", Object::String(noise, StringFormat::Hexadecimal));
        doc.objects.insert((id, 0), Object::Dictionary(filler));
    }
    doc.trailer.set("Root", Object::Reference((1, 0)));
    doc.max_id = 4004;
    let options = lopdf::SaveOptions::builder().use_object_streams(true).use_xref_streams(true).max_objects_per_stream(10_000).build();
    let mut bytes = Vec::new();
    doc.save_with_options(&mut bytes, options).unwrap();
    bytes
}

#[test]
fn an_object_stream_over_the_keep_limit_is_not_truncated_and_its_objects_are_still_scanned() {
    let bytes = pdf_with_object_stream_over_the_keep_limit();
    // 전제: 객체 스트림이 정말 보존 한도(64KB)를 넘는다. 아니면 이 테스트는 아무것도 증명하지 못한다.
    let doc = Document::load_mem(&bytes).unwrap();
    let biggest = doc.objects.values().filter_map(|o| o.as_stream().ok()).filter(|s| s.dict.has_type(b"ObjStm")).map(|s| s.content.len()).max().unwrap_or(0);
    assert!(biggest > 64 * 1024, "객체 스트림이 {biggest}바이트라 한도를 넘지 못했다. 채우는 양을 늘려야 한다");
    let (code, out) = scan_bytes(&bytes);
    assert_eq!(code, 1, "{out}");
    assert!(!out.contains("INCOMPLETE") && !out.contains("could not be decoded"), "{out}");
    assert!(out.contains("[CRITICAL] Launch action"), "{out}");
}

#[test]
fn nested_numeric_arrays_are_dropped_but_arrays_holding_a_dictionary_or_string_are_not() {
    // `[[x y] [x y] ...]`처럼 중첩된 숫자 배열은 한 덩어리로 비워도 된다. 지표는 그대로여야 한다.
    let pairs = "[1 2] ".repeat(50_000);
    let (code, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F (calc.exe) /Path [{pairs}] >>")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[CRITICAL] Launch action"), "{out}");
    // 안쪽 배열 하나에 사전이 있으면 바깥 배열 전체를 지키고 그 사전을 끝까지 따라간다.
    let numbers = "3 ".repeat(40);
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("[[{numbers}] [{numbers} << /S /JavaScript /JS (app.alert(1)) >>]]")]);
    assert!(out.contains("[HIGH] JavaScript action"), "{out}");
}

#[test]
fn every_name_key_of_a_file_specification_is_checked_not_only_the_first() {
    // /F가 무해해도 /UF(또는 /DOS·/Mac·/Unix)에 실행 파일이 있으면 위험이다. 뷰어는 /UF를 먼저 쓰기도 한다.
    for spec in ["<< /F (readme.txt) /UF (calc.exe) >>", "<< /F (readme.txt) /DOS (setup.bat) >>", "<< /UF (notes.txt) /Unix (run.sh) >>", "<< /F (a.txt) /Mac (x.app.exe) >>"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {spec} >>")]);
        assert!(out.contains("[CRITICAL] Launch action"), "{spec}: {out}");
    }
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /Launch /F << /F (a.txt) /UF (b.txt) >> >>"]);
    assert!(out.contains("[HIGH] Launch action") && !out.contains("CRITICAL"), "{out}");
    // 이름만 있는 파일 지정 사전도 같은 키 전부를 본다.
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /Type /Filespec /F (a.txt) /UF (calc.exe) /EF << >> >>"]);
    assert!(out.contains("executable-looking file name"), "{out}");
}

#[test]
fn only_a_leading_url_with_a_host_is_read_by_its_path() {
    // 맨 앞의 http·https·ftp 주소만 경로로 읽는다. 호스트가 비었거나 중간에 `://`가 있는 이름은 이름 그대로 본다.
    for target in ["(https:///dl/evil.exe)", "(mailto:evil.exe@example.com)", "(x.exe)", "(HTTPS://x.example/a.PS1)"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {target} >>")]);
        assert!(out.contains("[CRITICAL] Launch action"), "{target}: {out}");
    }
    for target in ["(ftp://example.com)", "(mailto:a@example.com)", "(https://example.com/)", "(https://example.com/page.html)"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {target} >>")]);
        assert!(out.contains("[HIGH] Launch action") && !out.contains("CRITICAL"), "{target}: {out}");
    }
}

#[test]
fn javascript_that_cannot_be_decompressed_within_the_limit_is_reported_not_passed_silently() {
    // 압축본은 작지만 풀면 4MB 한도를 넘는 JS. 한도를 넘으면 압축 원본으로 되돌아가 조용히 지나가던 구멍을 막는다.
    let (bytes, compressed) = pdf_with_flate_javascript(&"a".repeat(5 * 1024 * 1024));
    assert!(compressed < 64 * 1024, "압축본이 {compressed}바이트라 보존 한도에는 걸리지 않아야 한다");
    let (code, out) = scan_bytes(&bytes);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[HIGH] JavaScript stream not fully inspected"), "{out}");
}

/// UTF-16 문자열을 16진 문자열로 쓴다. `bom`이 `Some(true)`면 BE, `Some(false)`면 LE(BOM 포함), `None`이면 BOM 없는 LE.
fn utf16_hex(text: &str, bom: Option<bool>) -> String {
    let be = bom.unwrap_or(false) && bom == Some(true);
    let body: String = text.encode_utf16().map(|u| if be { format!("{u:04X}") } else { format!("{:02X}{:02X}", u & 0xff, u >> 8) }).collect();
    let prefix = match bom {
        Some(true) => "FEFF",
        Some(false) => "FFFE",
        None => "",
    };
    format!("<{prefix}{body}>")
}

#[test]
fn utf16_encoded_file_names_do_not_hide_an_executable() {
    // 파일 이름은 UTF-16(BOM)이거나 BOM 없는 UTF-16일 수 있다. 바이트를 그대로 UTF-8로 읽으면 확장자 검사를 빠져나간다.
    for bom in [Some(true), Some(false), None] {
        let name = utf16_hex("calc.exe", bom);
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {name} >>")]);
        assert!(out.contains("[CRITICAL] Launch action"), "bom={bom:?} {name}: {out}");
        // 파일 지정 사전의 이름 키도 같다. 읽을 수 있는 이름으로 지표에 남는다.
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /Type /Filespec /UF {name} /EF << >> >>")]);
        assert!(out.contains("executable-looking file name") && out.contains("calc.exe"), "bom={bom:?}: {out}");
    }
    // 무해한 UTF-16 이름은 올리지 않는다.
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {} >>", utf16_hex("readme.txt", Some(true)))]);
    assert!(out.contains("[HIGH] Launch action") && !out.contains("CRITICAL"), "{out}");
}

#[test]
fn exploit_javascript_in_utf16_without_a_bom_is_still_found() {
    // BOM 없는 UTF-16LE JS는 글자 사이에 NUL이 끼어 `eval(unescape(`가 안 보인다. NUL을 걷어 내고 본다.
    let js = utf16_hex("eval(unescape('x'));", None);
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /JavaScript /JS {js} >>")]);
    assert!(out.contains("[CRITICAL] exploit JavaScript"), "{out}");
}

/// 바이트를 그대로 16진 문자열로 쓴다.
fn hex_bytes(bytes: &[u8]) -> String {
    format!("<{}>", bytes.iter().map(|b| format!("{b:02X}")).collect::<String>())
}

#[test]
fn a_name_is_dangerous_if_any_reading_of_it_is_dangerous() {
    // 첫 NUL에서 자르는 소비자는 `calc.exe`로 읽고, NUL을 지우는 쪽은 `calc.exe.txt`로 읽는다. 하나라도 위험하면 위험이다.
    for name in [&b"calc.exe\0.txt"[..], b"readme.txt\0.exe", b"setup.bat\0", b"\0run.sh"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {} >>", hex_bytes(name))]);
        assert!(out.contains("[CRITICAL] Launch action"), "{:?}: {out}", String::from_utf8_lossy(name));
    }
    // `FF FE`는 PDF 명세의 텍스트 문자열 BOM이 아니다. 뒤에 ASCII가 오면 LE로 풀 때는 쓰레기지만 그대로 읽으면 `ÿþcalc.exe`다.
    let mut le_bom_then_ascii = vec![0xFF, 0xFE];
    le_bom_then_ascii.extend_from_slice(b"calc.exe");
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {} >>", hex_bytes(&le_bom_then_ascii))]);
    assert!(out.contains("[CRITICAL] Launch action"), "{out}");
    // 어느 해석으로도 무해하면 올리지 않는다.
    for name in [&b"notes\0.txt"[..], b"readme.txt\0", b"\xFF\xFEreadme.txt"] {
        let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /Launch /F {} >>", hex_bytes(name))]);
        assert!(out.contains("[HIGH] Launch action") && !out.contains("CRITICAL"), "{:?}: {out}", String::from_utf8_lossy(name));
    }
}

#[test]
fn exploit_javascript_is_found_whichever_way_the_bytes_are_read() {
    // `FF FE`로 시작하는 ASCII JS를 LE로만 풀면 쓰레기가 되어 `eval(unescape(`를 놓친다.
    let mut js = vec![0xFF, 0xFE];
    js.extend_from_slice(b"eval(unescape('x'));");
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /JavaScript /JS {} >>", hex_bytes(&js))]);
    assert!(out.contains("[CRITICAL] exploit JavaScript"), "{out}");
    // BOM이 올바른 UTF-16LE JS도 계속 찾는다.
    let mut utf16 = vec![0xFF, 0xFE];
    for u in "eval(unescape('x'));".encode_utf16() {
        utf16.extend_from_slice(&u.to_le_bytes());
    }
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &[&format!("<< /S /JavaScript /JS {} >>", hex_bytes(&utf16))]);
    assert!(out.contains("[CRITICAL] exploit JavaScript"), "{out}");
}

// ---- 새로 추가 ----

#[test]
fn truncated_pdf_is_incomplete_not_abort() {
    // 중간에서 잘린 PDF는 panic 없이 incomplete(또는 지표)로 끝나야 한다.
    let full = pdf(&[&catalog("/OpenAction 4 0 R"), PAGES, PAGE, "<< /S /JavaScript /JS (app.alert\\(1\\)) >>"]);
    let d = tempfile::tempdir().unwrap();
    for cut in [full.len() / 4, full.len() / 2, full.len() - 40] {
        let p = d.path().join(format!("cut{cut}.pdf"));
        fs::write(&p, &full[..cut]).unwrap();
        let r = scan_file(&p, &CheckOptions { no_pdf_worker: true, ..CheckOptions::default() });
        assert!(r.incomplete || !r.findings.is_empty(), "cut={cut}: {r:?}");
        assert!(!r.reasons.iter().any(|m| m.starts_with("panic")), "cut={cut}: {:?}", r.reasons);
    }
}

#[test]
fn pipeline_reports_javascript_action_with_legacy_code_and_object_location() {
    let cat = catalog("/OpenAction 4 0 R");
    let bytes = pdf(&[cat.as_str(), PAGES, PAGE, "<< /S /JavaScript /JS (app.alert(1)) >>"]);
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.pdf");
    fs::write(&p, &bytes).unwrap();
    let r = scan_file(&p, &CheckOptions { no_pdf_worker: true, ..CheckOptions::default() });
    let f = r.findings.iter().find(|f| f.code == "JavaScript action").unwrap_or_else(|| panic!("{r:?}"));
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert!(f.location.starts_with("obj 4 0:"), "{}", f.location);
}

#[test]
fn scan_on_bytes_without_header_marks_incomplete_only() {
    let mut r = FileReport::new(std::path::Path::new("x.pdf"), None);
    security_checker::scan(b"hello", &mut r);
    assert!(r.incomplete && r.findings.is_empty() && r.reasons == ["no %PDF- header"], "{r:?}");
}

mod action_and_stream_abuse {
    use doc_checker::pdf::security_checker;
    use doc_checker::{Category, FileReport, Severity};
    use std::path::Path;

    /// 객체 본문 목록으로 PDF를 만든다. 1번 Catalog, 2번 Pages, 3번 Page, 4번부터 `more`다. 본문은 바이트라 스트림을 담을 수 있다.
    fn pdf(catalog_extra: &str, more: &[Vec<u8>]) -> Vec<u8> {
        let mut bodies: Vec<Vec<u8>> = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>").into_bytes(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_vec(),
        ];
        bodies.extend(more.iter().cloned());
        build(&bodies)
    }

    fn build(bodies: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).bytes());
            out.extend(body);
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).bytes());
        out
    }

    fn stream(dict_extra: &str, content: &[u8]) -> Vec<u8> {
        let mut v = format!("<< /Length {} {dict_extra} >>\nstream\n", content.len()).into_bytes();
        v.extend(content);
        v.extend(b"\nendstream");
        v
    }

    fn run(bytes: &[u8]) -> FileReport {
        let mut r = FileReport::new(Path::new("t.pdf"), None);
        security_checker::scan(bytes, &mut r);
        r
    }

    fn hits<'a>(r: &'a FileReport, code: &str) -> Vec<&'a doc_checker::Finding> {
        r.findings.iter().filter(|f| f.code == code && f.category == Category::Security).collect()
    }

    fn s(x: &str) -> Vec<u8> {
        x.as_bytes().to_vec()
    }

    /// 카탈로그 OpenAction으로 동작 사전을 4번 객체에 달아 검사한다.
    fn action(body: &str, more: &[Vec<u8>]) -> FileReport {
        let mut objs = vec![s(body)];
        objs.extend_from_slice(more);
        run(&pdf("/OpenAction 4 0 R", &objs))
    }

    // ---------------------------------------------------------------- 동작 코드

    #[test]
    fn flags_gotor_action_with_target() {
        let r = action("<< /S /GoToR /F (other.pdf) /D [0 /Fit] >>", &[]);
        let h = hits(&r, "GoToR action");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Medium);
        assert!(h[0].detail.contains("other.pdf"), "{}", h[0].detail);
        assert!(h[0].location.starts_with("obj 4 0:"), "{}", h[0].location);
    }

    #[test]
    fn flags_gotoe_action() {
        let r = action("<< /S /GoToE /T << /R /C /N (a) >> >>", &[]);
        let h = hits(&r, "GoToE action");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Medium);
    }

    #[test]
    fn flags_submitform_action_with_target() {
        let r = action("<< /S /SubmitForm /F (http://x.example/post) >>", &[]);
        let h = hits(&r, "SubmitForm action");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Medium);
        assert!(h[0].detail.contains("x.example"), "{}", h[0].detail);
    }

    #[test]
    fn flags_importdata_action() {
        let r = action("<< /S /ImportData /F (data.fdf) >>", &[]);
        let h = hits(&r, "ImportData action");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Medium);
    }

    #[test]
    fn new_action_codes_do_not_fire_for_goto_and_legacy_code_is_kept() {
        let r = action("<< /S /GoTo /D [0 /Fit] >>", &[]);
        for code in ["GoToR action", "GoToE action", "SubmitForm action", "ImportData action", "unresolvable action type"] {
            assert!(hits(&r, code).is_empty(), "{code}");
        }
        // 옛 묶음 지표는 새 코드와 함께 그대로 나온다.
        let r = action("<< /S /GoToR /F (o.pdf) >>", &[]);
        assert_eq!(hits(&r, "external/media action").len(), 1);
    }

    // ---------------------------------------------------------------- 은닉 이름

    #[test]
    fn flags_hex_obfuscated_action_name() {
        // /J#53은 /JS, /Java#53cript는 /JavaScript, /L#61unch는 /Launch다.
        for (token, plain) in [("/J#53", "JS"), ("/Java#53cript", "JavaScript"), ("/L#61unch", "Launch")] {
            let r = run(&pdf(&format!("{token} (x)"), &[]));
            let h = hits(&r, "hex-obfuscated action name");
            assert_eq!(h.len(), 1, "{token}");
            assert_eq!(h[0].severity, Severity::Medium);
            assert!(h[0].detail.contains(plain), "{}", h[0].detail);
        }
    }

    #[test]
    fn hex_name_check_ignores_plain_names_and_unrelated_escapes() {
        for extra in ["/JS (x)", "/Launch (x)", "/Foo#20Bar 1", "/J#5 1", "/Other#53 1"] {
            let r = run(&pdf(extra, &[]));
            assert!(hits(&r, "hex-obfuscated action name").is_empty(), "{extra}");
        }
    }

    // ---------------------------------------------------------------- 필터 체인

    #[test]
    fn flags_long_filter_chain() {
        let names = "/ASCIIHexDecode ".repeat(5);
        let r = run(&pdf("", &[stream(&format!("/Filter [{names}]"), b"x")]));
        let h = hits(&r, "long filter chain");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Medium);
        assert!(h[0].location.starts_with("obj 4 0:"));
    }

    #[test]
    fn short_filter_chain_is_not_flagged() {
        let names = "/ASCIIHexDecode ".repeat(4);
        let r = run(&pdf("", &[stream(&format!("/Filter [{names}]"), b"x")]));
        assert!(hits(&r, "long filter chain").is_empty());
        let r = run(&pdf("", &[stream("/Filter /FlateDecode", b"x")]));
        assert!(hits(&r, "long filter chain").is_empty());
    }

    #[test]
    fn filter_chain_truncated_at_load_is_still_flagged() {
        // 로딩 훅이 16개로 자르고 표지를 단다. 자른 체인은 "16개 이상"으로 본다.
        let names = "/ASCIIHexDecode ".repeat(300);
        let r = run(&pdf("", &[stream(&format!("/Filter [{names}]"), b"x")]));
        let h = hits(&r, "long filter chain");
        assert_eq!(h.len(), 1);
        assert!(h[0].detail.contains("16 or more"), "{}", h[0].detail);
    }

    // ---------------------------------------------------------------- 증분 저장

    #[test]
    fn flags_many_incremental_updates() {
        let mut bytes = pdf("", &[]);
        for i in 0..9 {
            bytes.extend(format!("startxref\n{i}\n%%EOF\n").bytes());
        }
        // 원본의 startxref 1개 + 9개 = 10개
        let r = run(&bytes);
        let h = hits(&r, "many incremental updates");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Low);
    }

    #[test]
    fn nine_startxref_markers_are_not_flagged() {
        let mut bytes = pdf("", &[]);
        for i in 0..8 {
            bytes.extend(format!("startxref\n{i}\n%%EOF\n").bytes());
        }
        assert!(hits(&run(&bytes), "many incremental updates").is_empty());
    }

    // ---------------------------------------------------------------- 내장 실행 파일

    fn elf() -> Vec<u8> {
        let mut v = b"\x7fELF\x02\x01\x01".to_vec();
        v.resize(64, 0);
        v
    }

    fn pe_at(offset: usize) -> Vec<u8> {
        let mut v = vec![0u8; offset + 64];
        v[0] = b'M';
        v[1] = b'Z';
        v[0x3C..0x40].copy_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
        v[offset..offset + 4].copy_from_slice(b"PE\0\0");
        v
    }

    fn embedded(content: &[u8]) -> FileReport {
        run(&pdf("", &[stream("/Type /EmbeddedFile", content)]))
    }

    #[test]
    fn flags_embedded_elf() {
        let r = embedded(&elf());
        let h = hits(&r, "embedded executable content");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Critical);
    }

    #[test]
    fn flags_embedded_macho_and_pe() {
        let macho = [0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0];
        assert_eq!(hits(&embedded(&macho), "embedded executable content").len(), 1);
        assert_eq!(hits(&embedded(&pe_at(0x80)), "embedded executable content").len(), 1);
    }

    #[test]
    fn flags_pe_whose_header_is_past_1kib() {
        // 11-C: 계획의 1024바이트 창으로는 놓치는 PE. 헤더는 오프셋 4000에 있다.
        let r = embedded(&pe_at(4000));
        assert_eq!(hits(&r, "embedded executable content").len(), 1);
    }

    #[test]
    fn embedded_text_starting_with_mz_is_not_flagged() {
        assert!(hits(&embedded(b"MZ Software notes, plain text"), "embedded executable content").is_empty());
        assert!(hits(&embedded(b"hello"), "embedded executable content").is_empty());
    }

    #[test]
    fn stream_referenced_through_ef_is_checked_and_non_embedded_stream_is_not() {
        let objs = [s("<< /Type /Filespec /F (a.bin) /EF << /F 5 0 R >> >>"), stream("", &elf())];
        let r = run(&pdf("", &objs));
        let h = hits(&r, "embedded executable content");
        assert_eq!(h.len(), 1);
        assert!(h[0].location.starts_with("obj 5 0:"), "{}", h[0].location);
        // 내장 파일이 아닌 스트림(이미지 등)은 ELF 바이트여도 이 코드를 내지 않는다.
        let r = run(&pdf("", &[stream("", &elf())]));
        assert!(hits(&r, "embedded executable content").is_empty());
    }

    // ---------------------------------------------------------------- 11-A 간접 /S

    #[test]
    fn indirect_action_type_is_judged_like_inline() {
        let r = action("<< /S 5 0 R /JS (app.alert(1)) >>", &[s("/JavaScript")]);
        let h = hits(&r, "JavaScript action");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::High);
        let inline = action("<< /S /JavaScript /JS (app.alert(1)) >>", &[]);
        assert_eq!(hits(&inline, "JavaScript action")[0].severity, h[0].severity);

        let r = action("<< /S 5 0 R /F (o.pdf) >>", &[s("/GoToR")]);
        assert_eq!(hits(&r, "GoToR action").len(), 1);
        let r = action("<< /S 5 0 R /F (a.txt) >>", &[s("/Launch")]);
        assert_eq!(hits(&r, "Launch action").len(), 1);
    }

    #[test]
    fn indirect_goto_action_type_is_not_flagged() {
        let r = action("<< /S 5 0 R /D [0 /Fit] >>", &[s("/GoTo")]);
        let codes: Vec<_> = r.findings.iter().map(|f| f.code.as_str()).collect();
        assert!(r.findings.iter().all(|f| f.code == "OpenAction (runs on open)"), "{codes:?}");
    }

    #[test]
    fn dangling_action_type_is_reported_low() {
        let r = action("<< /S 99 0 R >>", &[]);
        let h = hits(&r, "unresolvable action type");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Low);
        assert!(h[0].detail.contains("99 0 R"), "{}", h[0].detail);
        assert!(h[0].location.starts_with("obj 4 0:"));
    }

    #[test]
    fn action_type_of_wrong_type_is_reported_low() {
        // 인라인 비-Name은 사전이 `/Type /Action`이라고 밝힐 때만 알린다(I1).
        let r = action("<< /Type /Action /S 7 >>", &[]);
        assert_eq!(hits(&r, "unresolvable action type").len(), 1);
        let r = action("<< /S 5 0 R >>", &[s("42")]);
        assert_eq!(hits(&r, "unresolvable action type").len(), 1);
    }

    #[test]
    fn cyclic_action_type_reference_terminates() {
        let r = action("<< /S 5 0 R >>", &[s("6 0 R"), s("5 0 R")]);
        let h = hits(&r, "unresolvable action type");
        assert_eq!(h.len(), 1, "{:?}", r.findings.iter().map(|f| (&f.code, &f.location)).collect::<Vec<_>>());
        assert!(h[0].detail.contains("does not resolve"));
    }

    // ---------------------------------------------------------------- 11-B 간접 filespec

    #[test]
    fn indirect_filespec_dictionary_under_launch_is_critical() {
        let r = action("<< /S /Launch /F 5 0 R >>", &[s("<< /Type /Filespec /F (cmd.exe) >>")]);
        let h = hits(&r, "Launch action");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Critical);
        let inline = action("<< /S /Launch /F << /Type /Filespec /F (cmd.exe) >> >>", &[]);
        assert_eq!(hits(&inline, "Launch action")[0].severity, Severity::Critical);
    }

    #[test]
    fn indirect_filespec_string_under_launch_is_critical() {
        let r = action("<< /S /Launch /F 5 0 R >>", &[s("(calc.exe)")]);
        assert_eq!(hits(&r, "Launch action")[0].severity, Severity::Critical);
    }

    #[test]
    fn benign_indirect_filespec_under_launch_stays_high() {
        let r = action("<< /S /Launch /F 5 0 R >>", &[s("<< /Type /Filespec /F (notes.txt) >>")]);
        assert_eq!(hits(&r, "Launch action")[0].severity, Severity::High);
    }

    #[test]
    fn indirect_win_dictionary_is_judged_like_inline() {
        let r = action("<< /S /Launch /F (a.txt) /Win 5 0 R >>", &[s("<< /F (cmd.exe) /P (/c calc) >>")]);
        let h = hits(&r, "platform launch parameters");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].severity, Severity::Critical);
        assert!(h[0].detail.contains("cmd.exe"), "{}", h[0].detail);
    }

    #[test]
    fn indirect_file_name_string_is_flagged_as_executable_looking() {
        let r = run(&pdf("", &[s("<< /Type /Filespec /F 5 0 R /EF << >> >>"), s("(evil.exe)")]));
        assert_eq!(hits(&r, "executable-looking file name").len(), 1);
        let r = run(&pdf("", &[s("<< /Type /Filespec /F 5 0 R /EF << >> >>"), s("(notes.txt)")]));
        assert!(hits(&r, "executable-looking file name").is_empty());
    }

    // ---------------------------------------------------------------- 리뷰 보완 (I1, M2~M5)

    #[test]
    fn linearized_hint_stream_integer_s_is_not_flagged() {
        // I1: 선형화 PDF의 힌트 스트림 사전은 `/S <정수>`를 가진다. 동작이 아니므로 지표가 없어야 한다.
        let r = run(&pdf("", &[stream("/Filter /FlateDecode /S 36", b"x")]));
        assert!(hits(&r, "unresolvable action type").is_empty());
        // 동작이 아닌 사전의 인라인 비-Name도 마찬가지다.
        let r = action("<< /S 7 /D [0 /Fit] >>", &[]);
        assert!(hits(&r, "unresolvable action type").is_empty());
    }

    /// 객체 5부터 `refs`개의 참조가 이어지고 마지막이 `/Launch`인 사슬. `/S 5 0 R`가 따라가는 참조 횟수는 `refs + 1`이다.
    fn chain(refs: usize) -> Vec<Vec<u8>> {
        let mut objs: Vec<Vec<u8>> = (0..refs).map(|i| s(&format!("{} 0 R", 6 + i))).collect();
        objs.push(s("/Launch"));
        objs
    }

    #[test]
    fn reference_chain_of_eight_hops_resolves_and_nine_does_not() {
        // M4: 8번 따라가면 Name에 닿고(참조 7개 + Name), 9번이면 한도에 걸려 풀리지 않는다.
        let r = action("<< /S 5 0 R /F (a.txt) >>", &chain(7));
        assert_eq!(hits(&r, "Launch action").len(), 1);
        assert!(hits(&r, "unresolvable action type").is_empty());
        let r = action("<< /S 5 0 R /F (a.txt) >>", &chain(8));
        assert!(hits(&r, "Launch action").is_empty());
        assert_eq!(hits(&r, "unresolvable action type").len(), 1);
    }

    #[test]
    fn raw_byte_findings_carry_offset_and_startxref_locations() {
        let mut bytes = pdf("/J#53 (x)", &[]);
        let offset = bytes.windows(5).position(|w| w == b"/J#53").unwrap();
        for i in 0..9 {
            bytes.extend(format!("startxref\n{i}\n%%EOF\n").bytes());
        }
        let r = run(&bytes);
        assert_eq!(hits(&r, "hex-obfuscated action name")[0].location, format!("obj 0 0:raw@{offset}"));
        assert_eq!(hits(&r, "many incremental updates")[0].location, "obj 0 0:startxref");
    }

    /// 저장(stored) 블록만 쓰는 zlib 스트림. 압축되지 않아 입력과 같은 크기다.
    fn zlib_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let mut chunks = data.chunks(65535).peekable();
        while let Some(c) = chunks.next() {
            out.push(u8::from(chunks.peek().is_none()));
            let len = u16::try_from(c.len()).unwrap();
            out.extend(len.to_le_bytes());
            out.extend((!len).to_le_bytes());
            out.extend(c);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + u32::from(x)) % 65521;
            b = (b + a) % 65521;
        }
        out.extend(((b << 16) | a).to_be_bytes());
        out
    }

    #[test]
    fn flate_embedded_pe_larger_than_64kib_is_still_found() {
        // M2: 로딩 훅이 스트림을 64KiB로 자르지만, Flate는 잘린 앞부분이 해제되어 PE를 판별할 수 있다.
        let mut payload = pe_at(0x80);
        let mut x = 12345u32;
        payload.extend((0..100_000).map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (x >> 16) as u8
        }));
        let z = zlib_stored(&payload);
        assert!(z.len() > 64 * 1024);
        let r = run(&pdf("", &[stream("/Type /EmbeddedFile /Filter /FlateDecode", &z)]));
        assert_eq!(hits(&r, "embedded executable content").len(), 1);
    }

    #[test]
    fn long_filter_finding_does_not_claim_truncation_when_only_decode_parms_were_cut() {
        // M3: DecodeParms만 16개를 넘고 /Filter는 짧다. 필터 체인이 길다는 지표는 없어야 한다.
        let parms = "null ".repeat(30);
        let r = run(&pdf("", &[stream(&format!("/Filter /FlateDecode /DecodeParms [{parms}]"), b"x")]));
        assert!(hits(&r, "long filter chain").is_empty());
    }
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
    scan_file(&p, &CheckOptions { no_pdf_worker: true, ..CheckOptions::default() })
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
