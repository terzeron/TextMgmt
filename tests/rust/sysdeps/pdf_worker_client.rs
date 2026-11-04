//! `sysdeps::pdf_worker_client` 단위 테스트.

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
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use sysdeps::pdf_worker_client::{extract_pages, extract_text, first_page_jpeg, page_count, render_page};
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
    assert_eq!(sysdeps::poppler::page_count(&fake, Path::new("a book.pdf")).unwrap(), 12);
    assert_eq!(&fake.calls.lock().unwrap()[0][5..], ["info", "a book.pdf"]);
    assert_eq!(&fake.calls.lock().unwrap()[0][..4], ["prlimit", "--as=536870912", "--cpu=25", "--"]);

    let malformed = runner(&[("Pages: nope\n", "", 0)]);
    assert!(sysdeps::poppler::page_count(&malformed, Path::new("a.pdf")).is_err());
}
