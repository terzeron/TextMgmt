//! HIGH로 남던 공격을 CRITICAL로 올리는 규칙 테스트: 번들 실행 파일, 이벤트 핸들러·javascript: URL 본문, 난독화 로더 우회,
//! 클릭 없는 UNC·원격 file 로드, meta refresh의 exploit 스킴, PDF exploit JavaScript 변형.

mod common;

use std::fs;

use bookdoctor::cli::Options;
use bookdoctor::epub::security::{Limits, scan as scan_into};
use bookdoctor::scan::scan_file;
use common::*;
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
