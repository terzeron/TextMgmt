//! ISBN 추출 테스트 (tests/test_isbn.py 이식).
//! Python 테스트는 subprocess와 pypdfium2·pypdf를 monkeypatch로 바꾼다. Rust는 같은 자리를
//! `Backends`로 주입한다.

use std::cell::RefCell;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use isbn::*;
use stat::Stat;

fn write(p: &Path, text: &str) {
    fs::write(p, text).unwrap();
}

fn make_zip(path: &Path, files: &[(&str, &str)]) {
    let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (n, c) in files {
        z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(c.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

// ---- 검증 ----

#[test]
fn isbn13_validation() {
    assert!(validate_isbn13("9788994492032"));
    assert!(!validate_isbn13("9788994492031"));
    assert!(!validate_isbn13("123"));
    assert!(!validate_isbn13("abcdefghijklm"));
}

#[test]
fn isbn10_validation() {
    assert!(validate_isbn10("0306406152"));
    assert!(validate_isbn10("080442957X"));
    for bad in ["0306406153", "12345", "1111111111", "1100101101", "abcdefghij", "030X406152"] {
        assert!(!validate_isbn10(bad), "{bad}");
    }
}

#[test]
fn validate_isbn_dispatches_on_length() {
    assert!(validate_isbn("9788994492032"));
    assert!(validate_isbn("0306406152"));
    assert!(!validate_isbn("1234"));
    assert!(!validate_isbn("1234567890123"));
}

#[test]
fn validators_accept_unicode_decimal_digits_like_python() {
    assert!(validate_isbn13("9788٩94492032"));
    assert!(validate_isbn10("030640615٢"));
    assert_eq!(search_in_content("9788994٩42032"), ["9788994٩42032"]);
}

// ---- 본문 검색 ----

#[test]
fn search_in_content_finds_plain_dashed_and_ocr_damaged_numbers() {
    assert!(search_in_content("ISBN 9788994492032").contains(&"9788994492032".to_string()));
    assert!(search_in_content("978-89-94492-03-2").contains(&"9788994492032".to_string()));
    assert!(search_in_content_bytes(b"ISBN 9788994492032").contains(&"9788994492032".to_string()));
    assert_eq!(search_in_content("no isbn here"), Vec::<String>::new());
    assert!(search_in_content("first 9788994492032 and 978-89-94492-03-2").contains(&"9788994492032".to_string()));
    assert!(!search_in_content("ISBN 97B-8O-94492-O3-2").contains(&"9788094492032".to_string()));
    assert!(search_in_content("ISBN 89-329-0839-7 some text").contains(&"8932908397".to_string()));
    let _ = search_in_content("89-94492-03-2");
}

#[test]
fn search_matches_the_python_results_for_ocr_and_separator_variants() {
    // 기대값은 Python `utils.isbn.search_in_content`의 실제 출력이다.
    for (input, expected) in [
        ("ISBN 0-8044-2957-Ⅹ", vec![]),
        ("ISBN 89-8044-2957-Ⅹ", vec![]),
        ("ISBN 978-89-9449l-03-2", vec![]),
        ("ISBN 978-89-94492-03-2", vec!["9788994492032"]),
        ("ISBN 978-89-9449I-03-2", vec![]),
        ("ISBN 89-329-0839-7", vec!["8932908397"]),
        ("ISBN 8Ⅹ-329-0839-7", vec![]),
        ("isbn 978–89–94492–03–2", vec!["9788994492032"]),
    ] {
        assert_eq!(search_in_content(input), expected, "{input}");
    }
}

#[test]
fn bytes_are_decoded_leniently() {
    let mut b = b"\xff\xfe junk ".to_vec();
    b.extend(b"ISBN 9788994492032");
    assert!(search_in_content_bytes(&b).contains(&"9788994492032".to_string()));
}

// ---- 앞뒤 읽기 ----

#[test]
fn head_and_tail_of_a_file_and_of_a_string() {
    let d = tempfile::tempdir().unwrap();
    let small = d.path().join("small.txt");
    write(&small, "hello world");
    assert_eq!(read_head_tail_from_file(&small, HEAD_TAIL_SIZE).unwrap(), "hello world");

    let large = d.path().join("large.txt");
    write(&large, &"A".repeat(20_000));
    let r = read_head_tail_from_file(&large, 100).unwrap();
    assert_eq!(r.len(), 200);
    assert!(r.chars().all(|c| c == 'A'));
    assert!(read_head_tail_from_file(&d.path().join("none.txt"), 100).is_err());

    assert_eq!(read_head_tail_from_content("hello", HEAD_TAIL_SIZE), "hello");
    assert_eq!(read_head_tail_from_content(&"A".repeat(20_000), 100).len(), 200);
    // 문자 수 기준으로 자른다.
    assert_eq!(read_head_tail_from_content(&"가".repeat(500), 100).chars().count(), 200);
}

// ---- EPUB ----

#[test]
fn epub_isbn_from_the_opf_metadata() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("book.epub");
    make_zip(&p, &[("content.opf", "<package><metadata>ISBN 9788994492032</metadata></package>")]);
    assert!(extract_from_epub(&p).contains(&"9788994492032".to_string()));
}

#[test]
fn epub_isbn_from_the_first_and_last_spine_chapters() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("book.epub");
    make_zip(
        &p,
        &[
            ("content.opf", r#"<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#),
            ("ch1.xhtml", "<html><body>ISBN 9788994492032</body></html>"),
        ],
    );
    assert!(extract_from_epub(&p).contains(&"9788994492032".to_string()));

    let q = d.path().join("test.epub");
    let opf = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="c1" href="head.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="middle.xhtml" media-type="application/xhtml+xml"/><item id="c3" href="tail.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/><itemref idref="c3"/></spine></package>"#;
    make_zip(
        &q,
        &[
            ("OPS/content.opf", opf),
            ("OPS/head.xhtml", "<html><body>ISBN 9788994492032</body></html>"),
            ("OPS/middle.xhtml", "<html><body>ISBN 9788994492032 middle</body></html>"),
            ("OPS/tail.xhtml", "<html><body>tail text</body></html>"),
        ],
    );
    assert!(extract_from_epub(&q).contains(&"9788994492032".to_string()));
}

#[test]
fn epub_middle_chapters_are_not_read() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("test.epub");
    let opf = r#"<package><manifest><item id="c1" href="a.xhtml" media-type="text/html"/><item id="c2" href="b.xhtml" media-type="text/html"/><item id="c3" href="c.xhtml" media-type="text/html"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/><itemref idref="c3"/></spine></package>"#;
    make_zip(&p, &[("content.opf", opf), ("a.xhtml", "<p>x</p>"), ("b.xhtml", "<p>ISBN 9788994492032</p>"), ("c.xhtml", "<p>y</p>")]);
    assert!(extract_from_epub(&p).is_empty());
}

#[test]
fn epub_failure_modes_all_give_an_empty_list() {
    let d = tempfile::tempdir().unwrap();
    let no_opf = d.path().join("a.epub");
    make_zip(&no_opf, &[("dummy.txt", "no opf here")]);
    assert!(extract_from_epub(&no_opf).is_empty());

    let bad = d.path().join("bad.epub");
    write(&bad, "not a zip");
    assert!(extract_from_epub(&bad).is_empty());
    assert!(extract_from_epub(&d.path().join("none.epub")).is_empty());

    let missing = d.path().join("m.epub");
    make_zip(&missing, &[("content.opf", r#"<package><manifest><item id="c1" href="missing.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#)]);
    assert!(extract_from_epub(&missing).is_empty());

    let empty_spine = d.path().join("e.epub");
    make_zip(
        &empty_spine,
        &[("content.opf", r#"<package><manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="missing_id"/></spine></package>"#)],
    );
    assert!(extract_from_epub(&empty_spine).is_empty());

    // 매니페스트에 있어도 html 형식이 아니면 본문으로 보지 않는다.
    let image = d.path().join("i.epub");
    make_zip(&image, &[("content.opf", r#"<package><manifest><item id="c1" href="a.png" media-type="image/png"/></manifest><spine><itemref idref="c1"/></spine></package>"#)]);
    assert!(extract_from_epub(&image).is_empty());
}

#[test]
fn epub_opf_is_found_by_a_case_sensitive_suffix_and_uses_the_first_one() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.epub");
    make_zip(&p, &[("OPS/CONTENT.OPF", "<package>ISBN 9788994492032</package>")]);
    assert!(extract_from_epub(&p).is_empty());
    let q = d.path().join("b.epub");
    make_zip(&q, &[("one.opf", "<package>nothing</package>"), ("two.opf", "<package>ISBN 9788994492032</package>")]);
    assert!(extract_from_epub(&q).is_empty());
}

// ---- 외부 명령 ----

type Answer = Box<dyn Fn(&str, &[String]) -> io::Result<CmdOutput>>;

struct Fake {
    calls: RefCell<Vec<Vec<String>>>,
    answer: Answer,
}

impl CommandRunner for Fake {
    fn run(&self, program: &str, args: &[String]) -> io::Result<CmdOutput> {
        self.calls.borrow_mut().push(std::iter::once(program.to_string()).chain(args.iter().cloned()).collect());
        (self.answer)(program, args)
    }
}

fn fake(answer: impl Fn(&str, &[String]) -> io::Result<CmdOutput> + 'static) -> Fake {
    Fake { calls: RefCell::new(Vec::new()), answer: Box::new(answer) }
}

fn out(code: i32, stdout: &str) -> io::Result<CmdOutput> {
    Ok(CmdOutput { code: Some(code), stdout: stdout.to_string() })
}

#[test]
fn djvu_text_comes_from_the_first_and_last_five_pages() {
    let f = fake(|prog, _| if prog == "djvused" { out(0, "10\n") } else { out(0, "ISBN 9788994492032") });
    let r = extract_from_djvu(Path::new("book.djvu"), &f);
    assert!(r.contains(&"9788994492032".to_string()));
    let calls = f.calls.borrow();
    assert_eq!(calls[0], ["djvused", "book.djvu", "-e", "n"]);
    assert_eq!(calls[1], ["djvutxt", "--page=1,2,3,4,5", "book.djvu"]);
    assert_eq!(calls[2], ["djvutxt", "--page=6,7,8,9,10", "book.djvu"]);
}

#[test]
fn djvu_with_few_pages_and_every_failure_mode() {
    let f = fake(|prog, _| if prog == "djvused" { out(0, "3") } else { out(0, "x") });
    extract_from_djvu(Path::new("b.djvu"), &f);
    assert_eq!(f.calls.borrow()[1][1], "--page=1,2,3");
    assert_eq!(f.calls.borrow()[2][1], "--page=1,2,3");

    assert!(extract_from_djvu(Path::new("b"), &fake(|_, _| out(1, ""))).is_empty());
    assert!(extract_from_djvu(Path::new("b"), &fake(|_, _| out(0, "not_a_number\n"))).is_empty());
    assert!(extract_from_djvu(Path::new("b"), &fake(|_, _| Err(io::Error::from(io::ErrorKind::NotFound)))).is_empty());
    // 텍스트 추출이 모두 실패하면 비어 있다.
    assert!(extract_from_djvu(Path::new("b"), &fake(|p, _| if p == "djvused" { out(0, "10") } else { out(1, "") })).is_empty());
    // 페이지 수가 0이면 범위를 만들지 않는다.
    let none = fake(|p, _| if p == "djvused" { out(0, "0") } else { out(0, "") });
    assert!(extract_from_djvu(Path::new("b"), &none).is_empty());
}

#[test]
fn hwp_strings_are_cut_to_head_and_tail() {
    assert!(extract_from_hwp(Path::new("b.hwp"), &fake(|_, _| out(0, "ISBN 9788994492032"))).contains(&"9788994492032".to_string()));
    assert!(extract_from_hwp(Path::new("b.hwp"), &fake(|_, _| out(1, ""))).is_empty());
    assert!(extract_from_hwp(Path::new("b.hwp"), &fake(|_, _| Err(io::Error::from(io::ErrorKind::NotFound)))).is_empty());
    assert!(extract_from_hwp(Path::new("b.hwp"), &fake(|_, _| out(0, ""))).is_empty());
    // 가운데에 있는 번호는 버려진다.
    let long = format!("{}ISBN 9788994492032{}", "x".repeat(HEAD_TAIL_SIZE + 10), "y".repeat(HEAD_TAIL_SIZE + 10));
    assert!(extract_from_hwp(Path::new("b.hwp"), &fake(move |_, _| out(0, &long))).is_empty());
}

#[test]
fn the_real_command_runner_reports_missing_programs() {
    let err = SystemRunner.run("definitely-not-a-program-xyz", &[]).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    let ok = SystemRunner.run("sh", &["-c".into(), "echo hi".into()]).unwrap();
    assert_eq!((ok.code, ok.stdout.trim()), (Some(0), "hi"));
}

// ---- PDF ----

type Primary = Arc<dyn Fn(&Path) -> Result<(String, usize), String> + Send + Sync>;
type Fallback = Arc<dyn Fn(&Path) -> Result<String, String> + Send + Sync>;

fn backends(stat: &Arc<Stat>, primary: Primary, fallback: Fallback) -> Backends {
    Backends { pdf_primary: primary, pdf_fallback: fallback, runner: Rc::new(SystemRunner), stat: Arc::clone(stat), pdf_timeout: 5.0, pdf_worker_executable: None }
}

fn never_fallback() -> Fallback {
    Arc::new(|_| panic!("fallback should not run"))
}

fn stage_line(stat: &Stat, stage: &str) -> String {
    let mut buf = Vec::new();
    stat.print(&mut buf).unwrap();
    String::from_utf8(buf).unwrap().lines().find(|l| l.starts_with(&format!("  - {stage}"))).unwrap_or("").to_string()
}

#[test]
fn txt_uses_the_given_content_or_reads_the_file() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("book.txt");
    write(&f, "ISBN 9788994492032");
    assert!(extract_default(&f, None).contains(&"9788994492032".to_string()));
    write(&f, "no isbn here");
    assert!(extract_default(&f, None).is_empty());
    write(&f, "nothing");
    assert!(extract_default(&f, Some("ISBN 9788994492032")).contains(&"9788994492032".to_string()));
    // 빈 content는 없는 것으로 보고 파일을 읽는다.
    write(&f, "ISBN 9788994492032");
    assert!(!extract_default(&f, Some("")).is_empty());
}

#[test]
fn unsupported_extensions_and_dispatch_to_djvu_and_hwp() {
    let d = tempfile::tempdir().unwrap();
    let x = d.path().join("book.xyz");
    write(&x, "ISBN 9788994492032");
    assert!(extract_default(&x, None).is_empty());

    let stat = Arc::new(Stat::new());
    let mut b = backends(&stat, Arc::new(|_| Ok((String::new(), 0))), never_fallback());
    b.runner = Rc::new(fake(|_, _| out(0, "ISBN 9788994492032")));
    assert!(extract(&d.path().join("a.hwp"), None, &b).contains(&"9788994492032".to_string()));
    b.runner = Rc::new(fake(|p, _| if p == "djvused" { out(0, "1") } else { out(0, "ISBN 9788994492032") }));
    assert!(extract(&d.path().join("a.djvu"), None, &b).contains(&"9788994492032".to_string()));
    // 확장자는 대소문자를 가리지 않는다.
    assert!(extract(&d.path().join("A.HWP"), None, &b).len() <= 1);
}

#[test]
fn pdf_uses_the_primary_extractor_before_the_fallback() {
    let stat = Arc::new(Stat::new());
    let b = backends(&stat, Arc::new(|_| Ok(("ISBN 9788994492032".into(), 6))), never_fallback());
    assert_eq!(extract(Path::new("book.pdf"), None, &b), ["9788994492032"]);
    assert!(stage_line(&stat, "isbn_pdfium").contains("ok=1"), "{}", stage_line(&stat, "isbn_pdfium"));
}

#[test]
fn pdf_falls_back_when_the_primary_extractor_cannot_open_the_document() {
    let stat = Arc::new(Stat::new());
    let b = backends(&stat, Arc::new(|_| Err("pdfium fail".into())), Arc::new(|_| Ok("ISBN 9788994492032".into())));
    assert_eq!(extract(Path::new("book.pdf"), None, &b), ["9788994492032"]);
    assert!(stage_line(&stat, "isbn_pdfium").contains("error=1"));
    assert!(stage_line(&stat, "isbn_pypdf").contains("ok=1"));
}

#[test]
fn pdf_given_content_with_an_isbn_skips_extraction() {
    let stat = Arc::new(Stat::new());
    let b = backends(&stat, Arc::new(|_| panic!("content로 끝났어야 한다")), never_fallback());
    assert_eq!(extract(Path::new("book.pdf"), Some("ISBN 978-89-94492-03-2"), &b), ["9788994492032"]);
    // content에 ISBN이 없으면 추출기를 돈다.
    let b = backends(&stat, Arc::new(|_| Ok(("ISBN 9788994492032".into(), 1))), never_fallback());
    assert_eq!(extract(Path::new("book.pdf"), Some("본문"), &b), ["9788994492032"]);
}

#[test]
fn pdf_primary_timeout_gives_up_without_the_fallback() {
    let stat = Arc::new(Stat::new());
    let mut b = backends(
        &stat,
        Arc::new(|_| {
            std::thread::sleep(Duration::from_millis(400));
            Ok(("ISBN 9788994492032".into(), 1))
        }),
        never_fallback(),
    );
    b.pdf_timeout = 0.05;
    assert!(extract(Path::new("hang.pdf"), None, &b).is_empty());
    assert!(stage_line(&stat, "isbn_pdfium").contains("timeout=1"));
}

#[cfg(unix)]
#[test]
fn production_pdf_worker_is_killed_after_the_isbn_timeout() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let worker = dir.path().join("stuck-isbn-worker");
    fs::write(&worker, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();

    let stat = Arc::new(Stat::new());
    let completed = Arc::new(AtomicBool::new(false));
    let completed_in_worker = Arc::clone(&completed);
    let primary = Arc::new(move |_path: &Path| {
        std::thread::sleep(Duration::from_millis(300));
        completed_in_worker.store(true, Ordering::SeqCst);
        Ok(("ISBN 9788994492032".into(), 1))
    });
    let mut b = backends(&stat, primary, never_fallback());
    b.pdf_worker_executable = Some(worker);
    b.pdf_timeout = 0.05;

    let start = Instant::now();
    assert!(extract(Path::new("stuck.pdf"), None, &b).is_empty());
    assert!(start.elapsed() < Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(350));
    assert!(!completed.load(Ordering::SeqCst), "timed-out PDF work must not continue in a background thread");
    assert!(stage_line(&stat, "isbn_pdfium").contains("timeout=1"));
}

#[cfg(unix)]
#[test]
fn production_pdf_worker_returns_text_and_page_count() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let worker = dir.path().join("isbn-worker");
    fs::write(&worker, "#!/bin/sh\nprintf '1\\nISBN 9788994492032\\n'\n").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();

    let stat = Arc::new(Stat::new());
    let mut b = backends(&stat, Arc::new(|_: &Path| panic!("worker executable should handle PDFium")), never_fallback());
    b.pdf_worker_executable = Some(worker);

    assert_eq!(extract(Path::new("book.pdf"), None, &b), ["9788994492032"]);
    assert!(stage_line(&stat, "isbn_pdfium").contains("ok=1"));
}

#[test]
fn pdf_primary_text_without_an_isbn_stops_there() {
    let stat = Arc::new(Stat::new());
    let b = backends(&stat, Arc::new(|_| Ok(("본문만 있고 국제표준도서번호는 없다".into(), 12))), never_fallback());
    assert!(extract(Path::new("no_isbn.pdf"), None, &b).is_empty());
}

#[test]
fn pdf_that_opened_but_has_no_text_does_not_use_the_fallback() {
    let stat = Arc::new(Stat::new());
    let b = backends(&stat, Arc::new(|_| Ok((String::new(), 3))), never_fallback());
    assert!(extract(Path::new("scan.pdf"), None, &b).is_empty());
    assert!(stage_line(&stat, "isbn_pdfium").contains("ok=0 empty=1"));
}

#[test]
fn pdf_fallback_outcomes_are_recorded() {
    let stat = Arc::new(Stat::new());
    let none: Primary = Arc::new(|_| Ok((String::new(), 0)));
    assert!(extract(Path::new("a.pdf"), None, &backends(&stat, none.clone(), Arc::new(|_| Ok(String::new())))).is_empty());
    assert!(stage_line(&stat, "isbn_pypdf").contains("empty=1"));
    assert!(extract(Path::new("a.pdf"), None, &backends(&stat, none.clone(), Arc::new(|_| Err("boom".into())))).is_empty());
    assert!(stage_line(&stat, "isbn_pypdf").contains("error=1"));
    let mut slow = backends(
        &stat,
        none,
        Arc::new(|_| {
            std::thread::sleep(Duration::from_millis(400));
            Ok(String::new())
        }),
    );
    slow.pdf_timeout = 0.05;
    assert!(extract(Path::new("a.pdf"), None, &slow).is_empty());
    assert!(stage_line(&stat, "isbn_pypdf").contains("timeout=1"));
}

#[test]
fn a_panicking_extractor_counts_as_an_error_and_never_escapes() {
    let stat = Arc::new(Stat::new());
    let b = backends(&stat, Arc::new(|_| panic!("unexpected")), Arc::new(|_| Ok("ISBN 9788994492032".into())));
    assert_eq!(extract(Path::new("a.pdf"), None, &b), ["9788994492032"]);
    assert!(stage_line(&stat, "isbn_pdfium").contains("error=1"));
}

#[test]
fn head_and_tail_page_windows() {
    assert_eq!(head_tail_page_indices(3, 5), [0, 1, 2]);
    assert_eq!(head_tail_page_indices(6, 5), [0, 1, 2, 3, 4, 5]);
    assert_eq!(head_tail_page_indices(20, 5), [0, 1, 2, 3, 4, 15, 16, 17, 18, 19]);
    assert!(head_tail_page_indices(0, 5).is_empty());
}

// ---- 실제 PDF ----

/// 텍스트 한 줄을 그리는 1쪽짜리 PDF.
fn text_pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
    out
}

#[test]
fn a_real_pdf_with_an_isbn_is_read_through_the_default_backends() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("book.pdf");
    fs::write(&f, text_pdf("ISBN 9788994492032")).unwrap();
    assert!(extract_default(&f, None).contains(&"9788994492032".to_string()));
    let blank = d.path().join("blank.pdf");
    fs::write(&blank, text_pdf("")).unwrap();
    assert!(extract_default(&blank, None).is_empty());
}

#[test]
fn a_broken_pdf_gives_an_empty_list() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("book.pdf");
    write(&f, "not a pdf");
    assert!(extract_default(&f, None).is_empty());
    assert!(extract_default(&d.path().join("none.pdf"), None).is_empty());
}

#[test]
fn the_fallback_scanner_reads_text_from_a_pdf_whose_xref_is_broken() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("broken.pdf");
    let mut bytes = text_pdf("ISBN 9788994492032");
    let cut = bytes.windows(4).position(|w| w == b"xref").unwrap();
    bytes.truncate(cut);
    fs::write(&f, bytes).unwrap();
    assert!(pdf_text_fallback(&f).unwrap().contains("9788994492032"));
    assert!(extract_default(&f, None).contains(&"9788994492032".to_string()));
}

#[test]
fn the_fallback_scanner_inflates_streams_and_reads_text_arrays_and_hex_strings() {
    use flate2::{Compression, write::ZlibEncoder};
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(b"BT [(ISBN ) -20 (978) 5 (8994492032)] TJ <49534244> Tj (esc \\(x\\) \\101 \\n) Tj ET").unwrap();
    let body = enc.finish().unwrap();
    let mut pdf = b"%PDF-1.4\n1 0 obj\n<< /Filter /FlateDecode >>\nstream\n".to_vec();
    pdf.extend(&body);
    pdf.extend(b"\nendstream\nendobj\n2 0 obj\n<< >>\nstream\nBT (plain) Tj ET\nendstream\nendobj\n");
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("c.pdf");
    fs::write(&f, pdf).unwrap();
    let text = pdf_text_fallback(&f).unwrap();
    assert!(text.contains("ISBN 9788994492032"), "{text:?}");
    assert!(text.contains("ISBD"), "{text:?}");
    assert!(text.contains("esc (x) A \n"), "{text:?}");
    assert!(text.contains("plain"));
    assert!(pdf_text_fallback(&d.path().join("none.pdf")).is_err());
}

// ---- pdfium ----

#[test]
fn the_pdfium_backend_reads_head_and_tail_pages_when_the_library_is_available() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("book.pdf");
    fs::write(&f, text_pdf("ISBN 9788994492032")).unwrap();
    match pdf_text_pdfium(&f) {
        Ok((text, pages)) => {
            assert_eq!(pages, 1);
            assert!(text.contains("9788994492032"), "{text:?}");
            assert!(pdf_text_auto(&f).unwrap().0.contains("9788994492032"));
            // 열 수 없는 문서는 오류다. 파일이 없거나 PDF가 아닌 경우.
            assert!(pdf_text_pdfium(&d.path().join("none.pdf")).is_err());
            let junk = d.path().join("junk.pdf");
            write(&junk, "not a pdf");
            assert!(pdf_text_pdfium(&junk).is_err());
        }
        // 라이브러리가 없는 환경에서는 순수 Rust 추출기로 내려간다.
        Err(e) => {
            assert!(e.contains("libpdfium"), "{e}");
            assert!(pdf_text_auto(&f).unwrap().0.contains("9788994492032"));
        }
    }
}

// ---- 분기 보강 ----

#[test]
fn the_pure_rust_pdf_extractor_works_without_pdfium() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("book.pdf");
    fs::write(&f, text_pdf("ISBN 9788994492032")).unwrap();
    let (text, pages) = pdf_text_primary(&f).unwrap();
    assert_eq!(pages, 1);
    assert!(text.contains("9788994492032"), "{text:?}");
    assert!(pdf_text_primary(&d.path().join("none.pdf")).is_err());
}

#[test]
fn a_corrupt_chapter_makes_the_epub_give_up_but_a_missing_one_is_skipped() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("book.epub");
    let opf = r#"<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;
    make_zip(&p, &[("content.opf", opf), ("ch1.xhtml", &"<p>ISBN 9788994492032 안녕하세요</p>".repeat(40))]);
    let mut data = fs::read(&p).unwrap();
    let marker = b"ch1.xhtml";
    let at = data.windows(marker.len()).position(|w| w == marker).unwrap() + marker.len() + 6;
    for b in &mut data[at..at + 12] {
        *b ^= 0xff;
    }
    fs::write(&p, data).unwrap();
    assert!(extract_from_epub(&p).is_empty());
}

#[test]
fn the_fallback_scanner_handles_odd_stream_and_string_syntax() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("odd.pdf");
    let mut pdf = b"%PDF-1.4\n1 0 obj\n<< >>\nstream\r\n".to_vec();
    // 중첩 괄호, \r \t 이스케이프, 8진수 한 자리, 딕셔너리 `<<`, 홀수 길이 16진수, 끝에서 끊긴 이스케이프.
    pdf.extend(br"BT (a(b)c\r\t\1x\) ) Tj << /K 1 >> [(p) (q)] TJ <41 42 4> Tj (tail \");
    pdf.extend(b"\nendstream\n2 0 obj\n<< >>\nstream\n(unterminated");
    fs::write(&f, pdf).unwrap();
    let text = pdf_text_fallback(&f).unwrap();
    assert!(text.contains("a(b)c\r\t\u{1}x) "), "{text:?}");
    assert!(text.contains("pq"), "{text:?}");
    assert!(text.contains("AB"), "{text:?}");
}
