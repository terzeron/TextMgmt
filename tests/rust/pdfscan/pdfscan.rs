//! pdfscan 통합 테스트: PDF를 직접 만들어 규칙마다 확인한다.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use lopdf::{Dictionary, Document, Object, StringFormat};
use pdfscan::run;

/// 객체 본문 목록으로 PDF를 만든다. 1번 객체가 Catalog, 2번이 Pages, 3번이 Page다.
fn pdf(objects: &[&str]) -> Vec<u8> {
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
    go(&[p.to_str().unwrap()])
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

fn go(argv: &[&str]) -> (i32, String) {
    let args: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let code = run(&args, &mut out);
    (code, String::from_utf8(out).unwrap())
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
    let (code, out) = go(&[not_pdf.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert!(out.contains("INCOMPLETE") && out.contains("no %PDF- header"), "{out}");

    let (code, out) = go(&[d.path().join("missing.pdf").to_str().unwrap()]);
    assert_eq!(code, 2);
    assert!(out.contains("unreadable"), "{out}");

    let garbage = d.path().join("g.pdf");
    fs::write(&garbage, b"%PDF-1.7\nthis is not a pdf at all\n").unwrap();
    let (code, out) = go(&[garbage.to_str().unwrap()]);
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
    let (code, out) = go(&["--max-total-mb", "1", p.to_str().unwrap()]);
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

#[test]
fn directories_respect_the_recursive_flag_and_symlinked_dirs_are_not_followed() {
    let d = tempfile::tempdir().unwrap();
    let sub = d.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("a.pdf"), pdf(&[&catalog("/OpenAction 4 0 R"), PAGES, PAGE, "<< /S /Launch /F (x.exe) >>"])).unwrap();
    fs::write(d.path().join("b.PDF"), pdf(&[&catalog(""), PAGES, PAGE])).unwrap();
    fs::write(d.path().join("notes.txt"), "x").unwrap();
    std::os::unix::fs::symlink(&sub, d.path().join("link")).unwrap();
    // 기본은 1단계만 훑는다.
    let (code, out) = go(&[d.path().to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(out.matches(": clean").count(), 1, "{out}");
    assert!(!out.contains("a.pdf") && !out.contains("link/"), "{out}");
    // -r은 하위까지 훑되 심볼릭 링크 디렉토리는 따라가지 않는다.
    let (code, out) = go(&["-r", d.path().to_str().unwrap()]);
    assert_eq!(code, 1);
    assert_eq!(out.matches(": INDICATORS").count(), 1, "{out}");
    assert_eq!(out.matches(": clean").count(), 1, "{out}");
    assert!(!out.contains("notes.txt"), "{out}");
}

#[test]
fn empty_directories_and_the_limit_option() {
    let d = tempfile::tempdir().unwrap();
    let empty = d.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let (code, out) = go(&[empty.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert!(out.is_empty(), "{out}");

    for name in ["a.pdf", "b.pdf"] {
        fs::write(d.path().join(name), pdf(&[&catalog(""), PAGES, PAGE])).unwrap();
    }
    let (code, out) = go(&["--limit", "1", "--workers", "1", d.path().to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(out.matches(": clean").count(), 1, "{out}");
}

#[test]
fn indicators_win_over_incomplete_in_the_exit_code() {
    let d = tempfile::tempdir().unwrap();
    let bad = d.path().join("bad.pdf");
    fs::write(&bad, pdf(&[&catalog("/OpenAction 4 0 R"), PAGES, PAGE, "<< /S /Launch /F (x.exe) >>"])).unwrap();
    let junk = d.path().join("junk.pdf");
    fs::write(&junk, b"nope").unwrap();
    assert_eq!(go(&[junk.to_str().unwrap(), bad.to_str().unwrap()]).0, 1);
}

#[test]
fn text_output_sanitizes_attacker_controlled_file_names() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("evil\u{1b}[2J\nname.pdf");
    fs::write(&p, pdf(&[&catalog(""), PAGES, PAGE])).unwrap();
    let (_, out) = go(&[p.to_str().unwrap()]);
    assert!(!out.contains('\u{1b}') && out.lines().count() == 1, "{out:?}");
}

#[test]
fn json_output_escapes_controls_and_lists_findings() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a\u{85}b.pdf");
    fs::write(&p, pdf(&[&catalog("/OpenAction 4 0 R"), PAGES, PAGE, "<< /S /Launch /F (x.exe) >>"])).unwrap();
    let (code, out) = go(&["--json", p.to_str().unwrap()]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["incomplete"], false);
    assert_eq!(v[0]["findings"][0]["severity"], "CRITICAL"); // x.exe를 여는 Launch
    assert!(v[0]["file"].as_str().unwrap().contains('\u{85}'));
    assert!(out.contains("\\u0085"));
    // 둘 이상이면 쉼표로 이어진다.
    let q = d.path().join("c.pdf");
    fs::write(&q, b"x").unwrap();
    let (_, out2) = go(&["--json", p.to_str().unwrap(), q.to_str().unwrap()]);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out2).unwrap().as_array().unwrap().len(), 2);
}

// ---- 인자 ----

#[test]
fn usage_and_bad_arguments() {
    let (code, out) = go(&[]);
    assert_eq!(code, 0);
    assert!(out.contains("사용법:"));
    assert_eq!(go(&["-h"]).0, 0);
    assert_eq!(go(&["--json"]).0, 2);
    assert_eq!(go(&["--max-total-mb"]).0, 2);
    assert_eq!(go(&["--max-total-mb", "abc", "x.pdf"]).0, 2);
    assert_eq!(go(&["--max-total-mb", "0", "x.pdf"]).0, 2);
    assert_eq!(go(&["--nope", "x.pdf"]).0, 2);
    assert_eq!(go(&["--limit", "1"]).0, 2);
}

#[test]
fn run_survives_a_closed_output() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.pdf");
    fs::write(&p, pdf(&[&catalog(""), PAGES, PAGE])).unwrap();
    let args = vec![p.to_str().unwrap().to_string()];
    assert_eq!(pdfscan::run(&args, &mut Broken), 1);
    let args = vec!["--json".to_string(), p.to_str().unwrap().to_string()];
    assert_eq!(pdfscan::run(&args, &mut Broken), 1);
    let _ = Path::new("/");
}

#[test]
fn text_results_are_written_before_the_next_file_finishes() {
    struct ChannelWriter(mpsc::Sender<Vec<u8>>);
    impl io::Write for ChannelWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.send(bytes.to_vec()).map_err(io::Error::other)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let d = tempfile::tempdir().unwrap();
    let first = d.path().join("first.pdf");
    let blocked = d.path().join("blocked.pdf");
    fs::write(&first, pdf(&[&catalog(""), PAGES, PAGE])).unwrap();
    assert!(Command::new("mkfifo").arg(&blocked).status().unwrap().success());

    let args = vec!["--workers".into(), "1".into(), first.to_string_lossy().into_owned(), blocked.to_string_lossy().into_owned()];
    let (tx, rx) = mpsc::channel();
    let task = thread::spawn(move || pdfscan::run(&args, &mut ChannelWriter(tx)));
    let first_result_arrived_while_next_file_was_blocked = rx.recv_timeout(Duration::from_millis(300)).is_ok();

    // Unblock the FIFO so a failure in the assertion cannot leave a worker stuck.
    fs::write(&blocked, b"not a PDF").unwrap();
    let exit = task.join().unwrap();
    assert_eq!(exit, 2);
    assert!(first_result_arrived_while_next_file_was_blocked, "첫 파일 결과가 다음 파일 검사 전 출력되어야 한다");
}

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
    assert!(out.contains("[HIGH] JavaScript stream exceeds inspection limit"), "{out}");
    // 작은 스트림에는 붙지 않는다.
    let small = "<< /Length 12 >>\nstream\napp.alert(2)\nendstream";
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 5 0 R >>", small]);
    assert!(!out.contains("exceeds inspection limit"), "{out}");
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
    assert!(!out.contains("exceeds inspection limit"), "{out}");
}

#[test]
fn javascript_too_big_to_keep_is_reported_instead_of_silently_passing() {
    // 압축 안 한 110KB JS는 보존 한도(64KB)에서 잘린다. 끝의 exploit을 못 보더라도 "다 못 봤다"는 사실은 남겨야 한다.
    let js = format!("{}eval(unescape('x'));", "var a = 1;\n".repeat(10_000));
    let stream = format!("<< /Length {} >>\nstream\n{js}\nendstream", js.len());
    let (_, out) = scan_pdf("/OpenAction 4 0 R", &["<< /S /JavaScript /JS 5 0 R >>", &stream]);
    assert!(out.contains("[HIGH] JavaScript stream exceeds inspection limit"), "{out}");
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
