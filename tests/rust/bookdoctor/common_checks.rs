//! 포맷과 무관한 공통 파일 검사 테스트.

mod common;

use std::fs::{self, File};
use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

use bookdoctor::cli::Options;
use bookdoctor::common::sniff_head;
use bookdoctor::scan::scan_file;
use scan_common::sniff::Sniffed;
use scan_common::{Category, Format, Severity};

fn count(r: &scan_common::FileReport, code: &str) -> usize {
    r.findings.iter().filter(|f| f.code == code).count()
}

fn find<'a>(r: &'a scan_common::FileReport, code: &str) -> &'a scan_common::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", r.findings))
}

#[test]
fn flags_empty_file() {
    let f = common::file_with("a.txt", b"");
    let r = scan_file(f.path(), &Options::default());
    let x = find(&r, "EMPTY_FILE");
    assert_eq!((x.category, x.severity), (Category::Defect, Severity::Medium));
    assert!(!r.incomplete);
    assert_eq!(r.format, None);
}

#[test]
fn flags_sparse_file() {
    let f = common::file_with("a.txt", b"");
    File::options().write(true).open(f.path()).unwrap().set_len(2 * 1024 * 1024).unwrap();
    let r = scan_file(f.path(), &Options::default());
    let x = find(&r, "SPARSE_FILE");
    assert_eq!((x.category, x.severity), (Category::Defect, Severity::High));
}

#[test]
fn sparse_below_one_mib_is_not_flagged() {
    let f = common::file_with("a.txt", b"");
    File::options().write(true).open(f.path()).unwrap().set_len(1024 * 1024 - 1).unwrap();
    let r = scan_file(f.path(), &Options::default());
    assert_eq!(count(&r, "SPARSE_FILE"), 0);
}

#[test]
fn dense_large_file_is_not_sparse() {
    let f = common::file_with("a.txt", &vec![b'a'; 2 * 1024 * 1024]);
    let r = scan_file(f.path(), &Options::default());
    assert_eq!(count(&r, "SPARSE_FILE"), 0);
}

fn tail_zeroed(len: usize) -> Vec<u8> {
    let mut v = vec![1u8; len];
    let from = len.saturating_sub(4096);
    v[from..].fill(0);
    v
}

#[test]
fn flags_zero_tail() {
    let f = common::file_with("a.txt", &tail_zeroed(8192));
    let r = scan_file(f.path(), &Options::default());
    let x = find(&r, "ZERO_TAIL");
    assert_eq!((x.category, x.severity), (Category::Defect, Severity::High));
}

#[test]
fn zero_tail_needs_4096_bytes() {
    let f = common::file_with("a.txt", &vec![0u8; 4095]);
    assert_eq!(count(&scan_file(f.path(), &Options::default()), "ZERO_TAIL"), 0);
    let f = common::file_with("a.txt", &vec![0u8; 4096]);
    assert_eq!(count(&scan_file(f.path(), &Options::default()), "ZERO_TAIL"), 1);
}

#[test]
fn nonzero_last_byte_is_not_zero_tail() {
    let mut v = tail_zeroed(8192);
    *v.last_mut().unwrap() = 1;
    let f = common::file_with("a.txt", &v);
    assert_eq!(count(&scan_file(f.path(), &Options::default()), "ZERO_TAIL"), 0);
}

#[test]
fn flags_format_mismatch_for_pdf_that_is_really_an_epub() {
    let f = common::epub_named("book.pdf");
    let r = scan_file(f.path(), &Options::default());
    assert_eq!(r.format, Some(Format::Epub));
    assert_eq!(count(&r, "FORMAT_MISMATCH"), 1);
    let x = find(&r, "FORMAT_MISMATCH");
    assert_eq!((x.category, x.severity), (Category::Defect, Severity::Medium));
}

#[test]
fn flags_format_mismatch_for_pdf_that_is_text() {
    let f = common::file_with("a.pdf", "그냥 텍스트\n".as_bytes());
    assert_eq!(count(&scan_file(f.path(), &Options::default()), "FORMAT_MISMATCH"), 1);
}

#[test]
fn flags_format_mismatch_for_epub_that_is_not_zip() {
    let f = common::file_with("a.epub", b"%PDF-1.4\n%%EOF\n");
    let r = scan_file(f.path(), &Options::default());
    assert_eq!(count(&r, "FORMAT_MISMATCH"), 1);
    assert_eq!(r.format, Some(Format::Pdf));
}

#[test]
fn flags_format_mismatch_for_txt_that_is_a_zip() {
    let f = common::file_with("a.txt", b"PK\x03\x04rest");
    assert_eq!(count(&scan_file(f.path(), &Options::default()), "FORMAT_MISMATCH"), 1);
}

#[test]
fn matching_content_has_no_format_mismatch() {
    let f = common::epub_named("a.epub");
    assert_eq!(count(&scan_file(f.path(), &Options::default()), "FORMAT_MISMATCH"), 0);
    let f = common::file_with("a.pdf", b"%PDF-1.4\n");
    assert_eq!(count(&scan_file(f.path(), &Options::default()), "FORMAT_MISMATCH"), 0);
}

#[test]
fn epub_embedding_a_pdf_early_is_still_a_zip() {
    let mut bytes = b"PK\x03\x04".to_vec();
    bytes.extend_from_slice(b"....%PDF-1.4....");
    assert_eq!(sniff_head(&bytes), Sniffed::Zip);
    let f = common::file_with("a.epub", &bytes);
    assert_eq!(count(&scan_file(f.path(), &Options::default()), "FORMAT_MISMATCH"), 0);
}

#[test]
fn flags_disguised_executable() {
    for (name, head) in [("a.epub", &b"\x7fELF\x02\x01"[..]), ("a.pdf", &common::minimal_pe()[..]), ("a.txt", b"\xCF\xFA\xED\xFE")] {
        let f = common::file_with(name, head);
        let r = scan_file(f.path(), &Options::default());
        let x = find(&r, "DISGUISED_EXECUTABLE");
        assert_eq!((x.category, x.severity), (Category::Security, Severity::Critical), "{name}");
        assert_eq!(count(&r, "FORMAT_MISMATCH"), 1, "{name}");
    }
}

#[test]
fn flags_filename_control_char() {
    let f = common::file_with("a\u{1}b.txt", "본문\n".as_bytes());
    let r = scan_file(f.path(), &Options::default());
    let x = find(&r, "FILENAME_CONTROL_CHAR");
    assert_eq!((x.category, x.severity), (Category::Defect, Severity::Medium));
}

#[test]
fn flags_filename_control_char_in_non_utf8_name() {
    use std::os::unix::ffi::OsStrExt;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(std::ffi::OsStr::from_bytes(b"\xff\x07.txt"));
    fs::write(&p, "본문\n").unwrap();
    assert_eq!(count(&scan_file(&p, &Options::default()), "FILENAME_CONTROL_CHAR"), 1);
}

#[test]
fn flags_filename_too_long() {
    // 파일시스템이 255바이트 초과 이름을 만들지 못하므로 없는 경로로 검사한다. 이름 검사는 파일을 열기 전에 한다.
    let d = tempfile::tempdir().unwrap();
    let long = d.path().join(format!("{}.txt", "a".repeat(252)));
    assert_eq!(long.file_name().unwrap().len(), 256);
    let r = scan_file(&long, &Options::default());
    let x = find(&r, "FILENAME_TOO_LONG");
    assert_eq!((x.category, x.severity), (Category::Defect, Severity::Low));
    let ok = d.path().join(format!("{}.txt", "a".repeat(251)));
    assert_eq!(count(&scan_file(&ok, &Options::default()), "FILENAME_TOO_LONG"), 0);
}

#[test]
fn flags_filename_reserved_name() {
    let f = common::file_with("CON.txt", "본문\n".as_bytes());
    let r = scan_file(f.path(), &Options::default());
    let x = find(&r, "FILENAME_RESERVED_NAME");
    assert_eq!((x.category, x.severity), (Category::Defect, Severity::Low));
}

#[test]
fn clean_text_file_has_no_common_findings() {
    let f = common::file_with("a.txt", "소림사 장로가 강호에서 수련했다.\n".as_bytes());
    let r = scan_file(f.path(), &Options::default());
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    assert!(!r.incomplete);
    assert_eq!(r.format, Some(Format::Txt));
}

#[test]
fn flags_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    unsafe extern "C" {
        #[link_name = "geteuid"]
        fn libc_geteuid() -> u32;
    }
    if unsafe { libc_geteuid() } == 0 {
        eprintln!("skipped: running as root");
        return;
    }
    let f = common::file_with("a.txt", b"hello");
    fs::set_permissions(f.path(), fs::Permissions::from_mode(0o000)).unwrap();
    let r = scan_file(f.path(), &Options::default());
    assert!(r.incomplete);
    assert!(r.reasons[0].starts_with("unreadable:"), "{:?}", r.reasons);
    assert!(r.findings.is_empty());
}

#[test]
fn skipped_over_limit() {
    let f = common::file_with("a.txt", b"");
    let mut w = File::options().write(true).open(f.path()).unwrap();
    w.write_all(&vec![b'a'; 2 * 1024 * 1024]).unwrap();
    let opts = Options { max_total_mb: 1, ..Options::default() };
    let r = scan_file(f.path(), &opts);
    assert!(r.incomplete);
    assert!(r.reasons[0].starts_with("skipped:"), "{:?}", r.reasons);
    assert!(r.reasons[0].contains("exceeds --max-total-mb"));
    assert_eq!(r.format, None);
}

#[test]
fn directory_is_not_a_regular_file() {
    let d = tempfile::tempdir().unwrap();
    let r = scan_file(d.path(), &Options::default());
    assert!(r.incomplete);
    assert_eq!(r.reasons[0], "unreadable: not a regular file");
}

#[test]
fn fifo_returns_promptly_as_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let fifo = d.path().join("pipe.txt");
    match std::process::Command::new("mkfifo").arg(&fifo).status() {
        Ok(s) if s.success() => {}
        _ => {
            eprintln!("skipped: mkfifo unavailable");
            return;
        }
    }
    let (tx, rx) = mpsc::channel();
    let p = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(scan_file(&p, &Options::default()));
    });
    let r = rx.recv_timeout(Duration::from_secs(5)).expect("FIFO 검사가 멈췄다");
    assert!(r.incomplete);
    assert_eq!(r.reasons[0], "unreadable: not a regular file");
}

#[test]
fn text_starting_with_mz_has_no_findings() {
    let f = common::file_with("a.txt", b"MZ Software notes\n");
    let r = scan_file(f.path(), &Options::default());
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn valid_pe_in_txt_is_disguised_executable() {
    let f = common::file_with("a.txt", &common::minimal_pe());
    let r = scan_file(f.path(), &Options::default());
    assert_eq!(find(&r, "DISGUISED_EXECUTABLE").severity, Severity::Critical);
}

#[test]
fn file_of_exactly_max_total_mb_is_not_skipped() {
    let f = common::file_with("a.txt", &vec![b'a'; 1024 * 1024]);
    let opts = Options { max_total_mb: 1, ..Options::default() };
    let r = scan_file(f.path(), &opts);
    assert!(!r.incomplete, "{:?}", r.reasons);
    assert_eq!(r.format, Some(Format::Txt));
}
