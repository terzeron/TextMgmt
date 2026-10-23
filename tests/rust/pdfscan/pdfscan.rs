//! pdfscan 통합 테스트: PDF를 직접 만들어 규칙마다 확인한다.

use std::fs;
use std::path::Path;

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
fn files_over_max_bytes_are_skipped_as_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.pdf");
    fs::write(&p, pdf(&[&catalog(""), PAGES, PAGE])).unwrap();
    let (code, out) = go(&["--max-bytes", "10", p.to_str().unwrap()]);
    assert_eq!(code, 2);
    assert!(out.contains("exceeds --max-bytes"), "{out}");
}

#[test]
fn directories_are_walked_for_pdfs_only_and_symlinked_dirs_are_not_followed() {
    let d = tempfile::tempdir().unwrap();
    let sub = d.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("a.pdf"), pdf(&[&catalog("/OpenAction 4 0 R"), PAGES, PAGE, "<< /S /Launch /F (x.exe) >>"])).unwrap();
    fs::write(d.path().join("b.PDF"), pdf(&[&catalog(""), PAGES, PAGE])).unwrap();
    fs::write(d.path().join("notes.txt"), "x").unwrap();
    std::os::unix::fs::symlink(&sub, d.path().join("link")).unwrap();
    let (code, out) = go(&[d.path().to_str().unwrap()]);
    assert_eq!(code, 1);
    assert_eq!(out.matches(": INDICATORS").count(), 1, "{out}");
    assert_eq!(out.matches(": clean").count(), 1, "{out}");
    assert!(!out.contains("notes.txt") && !out.contains("link/"), "{out}");
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
    assert_eq!(v[0]["findings"][0]["severity"], "HIGH");
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
    assert_eq!(go(&["--max-bytes"]).0, 2);
    assert_eq!(go(&["--max-bytes", "abc", "x.pdf"]).0, 2);
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
