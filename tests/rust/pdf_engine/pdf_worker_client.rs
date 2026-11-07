//! `pdf_engine::pdf_worker_client` 단위 테스트.

mod common;

// ---- 어댑터: worker 출력과 종료 코드를 오류로 바꾼다 ----

struct Fake {
    output: Result<CommandOutput, ()>,
    argv: Mutex<Vec<String>>,
}

impl Fake {
    fn exits(code: i32, stdout: &[u8], stderr: &str) -> Self {
        Fake { output: Ok(CommandOutput { returncode: code, stdout: stdout.to_vec(), stderr: stderr.as_bytes().to_vec() }), argv: Mutex::new(Vec::new()) }
    }
}

impl CommandRunner for Fake {
    fn run(&self, argv: &[String], _: Option<Duration>) -> Result<CommandOutput, CommandError> {
        *self.argv.lock().unwrap() = argv.to_vec();
        match &self.output {
            Ok(o) => Ok(CommandOutput { returncode: o.returncode, stdout: o.stdout.clone(), stderr: o.stderr.clone() }),
            Err(()) => Err(CommandError::Timeout),
        }
    }
    fn which(&self, _: &str) -> Option<PathBuf> {
        None
    }
}
use crate::common::shared_fixtures::runner;
use pdf_engine::pdf_worker_client::{extract_pages, extract_text, first_page_jpeg, page_count, render_page};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use sysdeps::{CommandError, CommandOutput, CommandRunner};

#[test]
fn adapter_maps_worker_exits_to_errors() {
    let f = Path::new("x.pdf");
    assert_eq!(extract_pages(&Fake::exits(127, b"", ""), f, 1, 1).unwrap_err(), "PDF worker unavailable; build tm-pdf-worker");
    assert_eq!(extract_pages(&Fake::exits(1, b"", " "), f, 1, 1).unwrap_err(), "PDF worker terminated without a diagnostic");
    assert!(first_page_jpeg(&Fake::exits(1, b"", "malloc of 99 bytes failed"), f).unwrap_err().starts_with("PDF worker resource limit:"));
    assert_eq!(first_page_jpeg(&Fake::exits(1, b"", "broken"), f).unwrap_err(), "PDF worker failed: broken");
    let timeout = Fake { output: Err(()), argv: Mutex::new(Vec::new()) };
    assert!(page_count(&timeout, f).unwrap_err().starts_with("PDF worker failed:"));
    assert_eq!(first_page_jpeg(&Fake::exits(0, b"JPEG", ""), f).unwrap(), b"JPEG");
    assert_eq!(extract_pages(&Fake::exits(0, b"PDF", ""), f, 2, 3).unwrap(), b"PDF");
    assert_eq!(extract_pages(&Fake::exits(0, b"", ""), f, 3, 2).unwrap_err(), "invalid PDF page range");
    assert_eq!(render_page(&Fake::exits(0, b"", ""), f, 0, 10).unwrap_err(), "invalid PDF render dimensions");
    assert_eq!(extract_text(&Fake::exits(0, b"", ""), f, 0, 1).unwrap_err(), "invalid PDF page range");
    assert_eq!(page_count(&Fake::exits(0, b"Pages: x\n", ""), f).unwrap_err(), "PDF worker returned an invalid page count");
    assert_eq!(page_count(&Fake::exits(0, b"", ""), f).unwrap_err(), "PDF worker did not report a page count");
    assert!(extract_text(&Fake::exits(0, b"\xff", ""), f, 1, 1).unwrap_err().contains("invalid UTF-8"));
}

#[test]
fn adapter_prefers_a_worker_on_the_path() {
    let d = tempfile::tempdir().unwrap();
    let worker = d.path().join("tm-pdf-worker");
    std::fs::write(&worker, b"").unwrap();
    let old = std::env::var_os("PATH");
    // SAFETY: 이 테스트 바이너리에서 PATH를 읽고 바꾸는 테스트는 이것뿐이다.
    unsafe { std::env::set_var("PATH", d.path()) };
    let fake = Fake::exits(0, b"Pages: 1\n", "");
    let pages = page_count(&fake, Path::new("x.pdf"));
    unsafe {
        match old {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
    }
    assert_eq!(pages.unwrap(), 1);
    assert_eq!(fake.argv.lock().unwrap()[4], worker.to_string_lossy());
}

#[test]
fn page_count_uses_pdfinfo_and_rejects_malformed_output() {
    let fake = runner(&[("Title: sample\nPages:          12\n", "", 0)]);
    assert_eq!(pdf_engine::pdf_worker_client::page_count(&fake, Path::new("a book.pdf")).unwrap(), 12);
    assert_eq!(&fake.calls.lock().unwrap()[0][5..], ["info", "a book.pdf"]);
    assert_eq!(&fake.calls.lock().unwrap()[0][..4], ["prlimit", "--as=536870912", "--cpu=25", "--"]);

    let malformed = runner(&[("Pages: nope\n", "", 0)]);
    assert!(pdf_engine::pdf_worker_client::page_count(&malformed, Path::new("a.pdf")).is_err());
}

/// 경로 인자 유지, 도구 오류·잘못된 범위·잘못된 출력 보고.
mod adapter_errors {
    use crate::common::shared_fixtures::runner;
    use std::path::Path;
    use std::time::Duration;
    use sysdeps::{CommandError, CommandRunner};

    struct FailingRunner {
        missing: bool,
    }

    impl CommandRunner for FailingRunner {
        fn run(&self, _argv: &[String], _timeout: Option<Duration>) -> Result<sysdeps::CommandOutput, CommandError> {
            if self.missing { Err(CommandError::NotFound) } else { Err(CommandError::Failed("runner failure".into())) }
        }

        fn which(&self, _program: &str) -> Option<std::path::PathBuf> {
            None
        }
    }

    struct InvalidUtf8Runner;

    impl CommandRunner for InvalidUtf8Runner {
        fn run(&self, _argv: &[String], _timeout: Option<Duration>) -> Result<sysdeps::CommandOutput, CommandError> {
            Ok(sysdeps::CommandOutput { returncode: 0, stdout: vec![0xff], stderr: Vec::new() })
        }

        fn which(&self, _program: &str) -> Option<std::path::PathBuf> {
            None
        }
    }

    #[test]
    fn text_and_render_commands_keep_paths_as_arguments_and_report_tool_errors() {
        let fake = runner(&[("본문\n", "", 0), ("P6\n1 1\n255\n", "", 0)]);
        assert_eq!(pdf_engine::pdf_worker_client::extract_text(&fake, Path::new("a book.pdf"), 2, 5).unwrap(), "본문\n");
        assert_eq!(&fake.calls.lock().unwrap()[0][5..], ["text", "a book.pdf", "2", "5"]);
        assert_eq!(pdf_engine::pdf_worker_client::render_page(&fake, Path::new("a book.pdf"), 3, 900).unwrap(), b"P6\n1 1\n255\n");
        assert_eq!(&fake.calls.lock().unwrap()[1][5..], ["render", "a book.pdf", "3", "900"]);

        let failed = runner(&[("", "Syntax Error: broken file", 1)]);
        assert!(pdf_engine::pdf_worker_client::extract_text(&failed, Path::new("broken.pdf"), 1, 1).unwrap_err().contains("broken file"));
    }

    #[test]
    fn reports_missing_tools_invalid_ranges_and_invalid_output() {
        let missing = FailingRunner { missing: true };
        assert!(pdf_engine::pdf_worker_client::page_count(&missing, Path::new("a.pdf")).unwrap_err().contains("build tm-pdf-worker"));

        let failed = FailingRunner { missing: false };
        assert!(pdf_engine::pdf_worker_client::page_count(&failed, Path::new("a.pdf")).unwrap_err().contains("runner failure"));

        let no_count = runner(&[("Title: no page count\n", "", 0)]);
        assert!(pdf_engine::pdf_worker_client::page_count(&no_count, Path::new("a.pdf")).unwrap_err().contains("did not report"));

        let valid = runner(&[]);
        assert!(pdf_engine::pdf_worker_client::extract_text(&valid, Path::new("a.pdf"), 0, 1).unwrap_err().contains("invalid PDF page range"));
        assert!(pdf_engine::pdf_worker_client::extract_text(&valid, Path::new("a.pdf"), 2, 1).unwrap_err().contains("invalid PDF page range"));
        assert!(pdf_engine::pdf_worker_client::extract_text(&InvalidUtf8Runner, Path::new("a.pdf"), 1, 1).unwrap_err().contains("invalid UTF-8"));
        assert!(pdf_engine::pdf_worker_client::render_page(&valid, Path::new("a.pdf"), 0, 900).unwrap_err().contains("invalid PDF render dimensions"));
        assert!(pdf_engine::pdf_worker_client::render_page(&valid, Path::new("a.pdf"), 1, 0).unwrap_err().contains("invalid PDF render dimensions"));
    }
}
