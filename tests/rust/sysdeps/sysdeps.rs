//! sysdeps 테스트: 시계, 파일시스템, 외부 명령, HTTP 응답 값 객체.
//! 설계: docs/rust-test-seams.md 3.1

use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sysdeps::{Clock, CommandError, CommandRunner, FsOps, HttpResponse, RealFs, SystemClock, SystemRunner};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

mod clock {
    use super::*;

    #[test]
    fn now_is_unix_time_in_seconds() {
        let now = SystemClock.now();
        // 2020-01-01 이후이고 소수 초를 가진다.
        assert!(now > 1_577_836_800.0, "now={now}");
    }

    #[test]
    fn monotonic_never_decreases() {
        let a = SystemClock.monotonic();
        let b = SystemClock.monotonic();
        assert!(b >= a);
    }

    #[test]
    fn sleep_waits_at_least_the_requested_time() {
        let start = Instant::now();
        SystemClock.sleep(0.05);
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn sleep_ignores_negative_and_non_finite_values() {
        let start = Instant::now();
        SystemClock.sleep(-1.0);
        SystemClock.sleep(f64::NAN);
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}

mod poppler {
    use super::*;

    struct FakeRunner {
        outputs: std::sync::Mutex<Vec<sysdeps::CommandOutput>>,
        calls: std::sync::Mutex<Vec<Vec<String>>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, argv: &[String], timeout: Option<Duration>) -> Result<sysdeps::CommandOutput, CommandError> {
            assert_eq!(timeout, Some(Duration::from_secs(30)));
            self.calls.lock().unwrap().push(argv.to_vec());
            Ok(self.outputs.lock().unwrap().remove(0))
        }

        fn which(&self, program: &str) -> Option<std::path::PathBuf> {
            Some(std::path::PathBuf::from(format!("/usr/bin/{program}")))
        }
    }

    fn runner(outputs: &[(&str, &str, i32)]) -> FakeRunner {
        FakeRunner {
            outputs: std::sync::Mutex::new(
                outputs.iter().map(|(stdout, stderr, returncode)| sysdeps::CommandOutput { stdout: stdout.as_bytes().to_vec(), stderr: stderr.as_bytes().to_vec(), returncode: *returncode }).collect(),
            ),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

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
    fn page_count_uses_pdfinfo_and_rejects_malformed_output() {
        let fake = runner(&[("Title: sample\nPages:          12\n", "", 0)]);
        assert_eq!(sysdeps::poppler::page_count(&fake, Path::new("a book.pdf")).unwrap(), 12);
        assert_eq!(fake.calls.lock().unwrap()[0], ["pdfinfo", "a book.pdf"]);

        let malformed = runner(&[("Pages: nope\n", "", 0)]);
        assert!(sysdeps::poppler::page_count(&malformed, Path::new("a.pdf")).is_err());
    }

    #[test]
    fn text_and_render_commands_keep_paths_as_arguments_and_report_tool_errors() {
        let fake = runner(&[("본문\n", "", 0), ("P6\n1 1\n255\n", "", 0)]);
        assert_eq!(sysdeps::poppler::extract_text(&fake, Path::new("a book.pdf"), 2, 5).unwrap(), "본문\n");
        assert_eq!(fake.calls.lock().unwrap()[0], ["pdftotext", "-q", "-enc", "UTF-8", "-f", "2", "-l", "5", "a book.pdf", "-"]);
        assert_eq!(sysdeps::poppler::render_page(&fake, Path::new("a book.pdf"), 3, 900).unwrap(), b"P6\n1 1\n255\n");
        assert_eq!(fake.calls.lock().unwrap()[1], ["pdftoppm", "-q", "-f", "3", "-l", "3", "-scale-to", "900", "-singlefile", "a book.pdf"]);

        let failed = runner(&[("", "Syntax Error: broken file", 1)]);
        assert!(sysdeps::poppler::extract_text(&failed, Path::new("broken.pdf"), 1, 1).unwrap_err().contains("broken file"));
    }

    #[test]
    fn reports_missing_tools_invalid_ranges_and_invalid_output() {
        let missing = FailingRunner { missing: true };
        assert!(sysdeps::poppler::page_count(&missing, Path::new("a.pdf")).unwrap_err().contains("install poppler-utils"));

        let failed = FailingRunner { missing: false };
        assert!(sysdeps::poppler::page_count(&failed, Path::new("a.pdf")).unwrap_err().contains("runner failure"));

        let no_count = runner(&[("Title: no page count\n", "", 0)]);
        assert!(sysdeps::poppler::page_count(&no_count, Path::new("a.pdf")).unwrap_err().contains("did not report"));

        let valid = runner(&[]);
        assert!(sysdeps::poppler::extract_text(&valid, Path::new("a.pdf"), 0, 1).unwrap_err().contains("invalid Poppler page range"));
        assert!(sysdeps::poppler::extract_text(&valid, Path::new("a.pdf"), 2, 1).unwrap_err().contains("invalid Poppler page range"));
        assert!(sysdeps::poppler::extract_text(&InvalidUtf8Runner, Path::new("a.pdf"), 1, 1).unwrap_err().contains("invalid UTF-8"));
        assert!(sysdeps::poppler::render_page(&valid, Path::new("a.pdf"), 0, 900).unwrap_err().contains("invalid Poppler render dimensions"));
        assert!(sysdeps::poppler::render_page(&valid, Path::new("a.pdf"), 1, 0).unwrap_err().contains("invalid Poppler render dimensions"));
    }
}

mod fs {
    use super::*;

    struct EmptyFs;

    impl FsOps for EmptyFs {}

    #[test]
    fn default_fs_operations_return_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        let opened = std::fs::File::open(&file).unwrap();
        let fs = EmptyFs;

        let errors = [
            fs.read_dir(dir.path()).unwrap_err(),
            fs.metadata(&file).unwrap_err(),
            fs.is_file(&file).unwrap_err(),
            fs.rename(&file, &dir.path().join("other")).unwrap_err(),
            fs.remove_file(&file).unwrap_err(),
            fs.remove_dir(dir.path()).unwrap_err(),
            fs.remove_dir_all(dir.path()).unwrap_err(),
            fs.canonicalize(&file).unwrap_err(),
            fs.same_file(&file, &file).unwrap_err(),
            fs.open(&file).unwrap_err(),
            fs.write(&file, b"x").unwrap_err(),
            fs.disk_free(dir.path()).unwrap_err(),
            fs.sync_file(&opened).unwrap_err(),
            fs.lock_exclusive(&opened, false).unwrap_err(),
        ];

        assert!(errors.iter().all(|err| err.kind() == ErrorKind::Unsupported));
    }

    #[test]
    fn read_dir_lists_entries_as_full_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let mut got = RealFs.read_dir(dir.path()).unwrap();
        got.sort();
        assert_eq!(got, vec![dir.path().join("a.txt"), dir.path().join("sub")]);
    }

    #[test]
    fn read_dir_of_a_missing_dir_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = RealFs.read_dir(&dir.path().join("missing")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn metadata_reports_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"12345").unwrap();
        assert_eq!(RealFs.metadata(&p).unwrap().len(), 5);
    }

    #[test]
    fn is_file_is_false_for_dirs_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"x").unwrap();
        assert!(RealFs.is_file(&p).unwrap());
        assert!(!RealFs.is_file(dir.path()).unwrap());
        // Python Path.is_file()은 없는 경로에서 예외 없이 False다.
        assert!(!RealFs.is_file(&dir.path().join("missing")).unwrap());
    }

    #[test]
    fn is_file_propagates_invalid_path_errors() {
        let err = RealFs.is_file(Path::new("bad\0path")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rename_moves_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"x").unwrap();
        RealFs.rename(&a, &b).unwrap();
        assert!(!a.exists() && b.exists());
    }

    #[test]
    fn remove_file_and_remove_dir_all() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        std::fs::write(&f, b"x").unwrap();
        RealFs.remove_file(&f).unwrap();
        assert!(!f.exists());
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(sub.join("inner")).unwrap();
        std::fs::write(sub.join("inner/x"), b"x").unwrap();
        RealFs.remove_dir_all(&sub).unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn canonicalize_resolves_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::write(&real, b"x").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(RealFs.canonicalize(&link).unwrap(), real.canonicalize().unwrap());
    }

    #[test]
    fn same_file_follows_links_and_rejects_different_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&a, &link).unwrap();
        assert!(RealFs.same_file(&a, &link).unwrap());
        assert!(!RealFs.same_file(&a, &b).unwrap());
    }

    #[test]
    fn open_and_write_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        RealFs.write(&p, b"hello").unwrap();
        let mut f = RealFs.open(&p).unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut f, &mut s).unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn disk_free_is_positive_for_an_existing_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(RealFs.disk_free(dir.path()).unwrap() > 0);
    }

    #[test]
    fn disk_free_rejects_paths_containing_nul() {
        let err = RealFs.disk_free(Path::new("bad\0path")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn disk_free_reports_missing_directories() {
        let err = RealFs.disk_free(Path::new("/definitely/missing/directory")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn sync_file_succeeds_on_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        RealFs.write(&p, b"x").unwrap();
        let f = RealFs.open(&p).unwrap();
        RealFs.sync_file(&f).unwrap();
    }

    #[test]
    fn nonblocking_exclusive_lock_reports_would_block_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("lock");
        RealFs.write(&p, b"").unwrap();
        let first = RealFs.open(&p).unwrap();
        let second = RealFs.open(&p).unwrap();
        RealFs.lock_exclusive(&first, true).unwrap();
        let err = RealFs.lock_exclusive(&second, true).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
        drop(first);
        RealFs.lock_exclusive(&second, true).unwrap();
    }
}

mod runner {
    use super::*;

    #[test]
    fn run_captures_stdout_and_returncode() {
        let out = SystemRunner.run(&argv(&["sh", "-c", "printf hi; exit 3"]), None).unwrap();
        assert_eq!(out.returncode, 3);
        assert_eq!(out.stdout, b"hi");
    }

    #[test]
    fn run_captures_stderr() {
        let out = SystemRunner.run(&argv(&["sh", "-c", "printf oops >&2"]), None).unwrap();
        assert_eq!(out.stderr, b"oops");
    }

    #[test]
    fn run_kills_a_command_that_exceeds_the_timeout() {
        let start = Instant::now();
        let err = SystemRunner.run(&argv(&["sleep", "5"]), Some(Duration::from_millis(100))).unwrap_err();
        assert!(matches!(err, CommandError::Timeout), "{err:?}");
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn run_reports_a_missing_program_as_not_found() {
        let err = SystemRunner.run(&argv(&["definitely-not-a-real-program-xyz"]), None).unwrap_err();
        assert!(matches!(err, CommandError::NotFound), "{err:?}");
    }

    #[test]
    fn run_rejects_an_empty_argv() {
        let err = SystemRunner.run(&[], None).unwrap_err();
        assert!(matches!(err, CommandError::Failed(_)), "{err:?}");
    }

    #[test]
    fn which_finds_programs_on_path() {
        let found = SystemRunner.which("sh").unwrap();
        assert!(found.is_absolute() && Path::new(&found).exists());
        assert!(SystemRunner.which("definitely-not-a-real-program-xyz").is_none());
    }

    #[test]
    fn which_accepts_an_explicit_executable_path() {
        let found = SystemRunner.which("/bin/sh").unwrap();
        assert_eq!(found, Path::new("/bin/sh"));
        assert!(SystemRunner.which("/definitely/missing/program").is_none());
    }

    #[test]
    fn errors_have_python_compatible_display_text() {
        assert_eq!(CommandError::Timeout.to_string(), "명령이 시간 상한을 넘었다");
        assert_eq!(CommandError::NotFound.to_string(), "명령을 찾을 수 없다");
        assert_eq!(CommandError::Failed("broken".into()).to_string(), "명령 실행 실패: broken");
        assert_eq!(sysdeps::HttpError::Timeout.to_string(), "HTTP 요청 시간 초과");
        assert_eq!(sysdeps::HttpError::Connection("closed".into()).to_string(), "HTTP 연결 실패: closed");
        assert_eq!(sysdeps::HttpError::Other("bad status".into()).to_string(), "HTTP 오류: bad status");
    }
}

mod http {
    use super::*;

    #[test]
    fn from_text_exposes_status_text_and_json() {
        let r = HttpResponse::from_text(200, "{\"a\": 1}");
        assert_eq!(r.status_code, 200);
        assert_eq!(r.json()["a"], 1);
        assert_eq!(HttpResponse::from_text(200, "한글").text(), "한글");
    }

    #[test]
    fn debug_output_contains_response_metadata() {
        let r = HttpResponse::from_text(204, "");
        let debug = format!("{r:?}");
        assert!(debug.contains("status_code: 204"));
        assert!(debug.contains("headers: []"));
    }

    #[test]
    fn from_chunks_concatenates_chunks_and_marks_the_body_closed() {
        let closed = Arc::new(AtomicBool::new(false));
        let r = HttpResponse::from_chunks(200, vec![], vec![b"ab".to_vec(), b"cd".to_vec()], closed.clone());
        assert!(!closed.load(Ordering::SeqCst));
        assert_eq!(r.text(), "abcd");
        assert!(closed.load(Ordering::SeqCst), "본문을 다 읽으면 닫아야 한다");
    }

    #[test]
    fn dropping_an_unread_response_closes_the_body() {
        let closed = Arc::new(AtomicBool::new(false));
        let r = HttpResponse::from_chunks(200, vec![], vec![b"ab".to_vec()], closed.clone());
        drop(r);
        assert!(closed.load(Ordering::SeqCst));
    }

    #[test]
    fn next_chunk_streams_until_exhausted() {
        let closed = Arc::new(AtomicBool::new(false));
        let mut r = HttpResponse::from_chunks(200, vec![], vec![b"ab".to_vec(), b"cd".to_vec()], closed);
        assert_eq!(r.next_chunk().unwrap().unwrap(), b"ab");
        assert_eq!(r.next_chunk().unwrap().unwrap(), b"cd");
        assert!(r.next_chunk().is_none());
    }

    #[test]
    fn redirect_statuses_expose_the_location_header_case_insensitively() {
        for status in [301u16, 302, 303, 307, 308] {
            let r = HttpResponse::from_chunks(status, vec![("Location".into(), "https://x.test/a".into())], vec![], Arc::new(AtomicBool::new(false)));
            assert!(r.is_redirect(), "status={status}");
            assert_eq!(r.location(), Some("https://x.test/a"));
        }
        let ok = HttpResponse::from_text(200, "");
        assert!(!ok.is_redirect());
        assert_eq!(ok.location(), None);
        assert_eq!(ok.header("Content-Type"), None);
    }
}
