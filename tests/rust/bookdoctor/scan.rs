//! 디스패치와 파일 단위 검사 테스트.

mod common;

use std::path::Path;

use bookdoctor::cli::run;
use bookdoctor::scan::dispatch_format;
use scan_common::Format;
use scan_common::sniff::Sniffed;

#[test]
fn dispatch_follows_content_for_pdf_that_is_a_zip() {
    assert_eq!(dispatch_format(Path::new("a.pdf"), Sniffed::Zip), Some(Format::Epub));
}

#[test]
fn dispatch_follows_extension_otherwise() {
    assert_eq!(dispatch_format(Path::new("a.txt"), Sniffed::Unknown), Some(Format::Txt));
    assert_eq!(dispatch_format(Path::new("a.epub"), Sniffed::Unknown), Some(Format::Epub));
    assert_eq!(dispatch_format(Path::new("a.dat"), Sniffed::Pdf), None);
}

#[test]
fn missing_path_is_incomplete_exit_2() {
    let d = tempfile::tempdir().unwrap();
    let missing = d.path().join("none.epub");
    let mut out = Vec::new();
    assert_eq!(run(&[missing.to_string_lossy().into_owned()], &mut out), 2);
    assert!(String::from_utf8(out).unwrap().contains("unreadable"));
}

#[cfg(unix)]
#[test]
fn unreadable_subdir_is_reported_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    // root는 권한 검사를 우회하므로 이 시나리오를 만들 수 없다.
    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let sub = d.path().join("locked");
    std::fs::create_dir(&sub).unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
    let mut out = Vec::new();
    let code = run(&["-r".into(), d.path().to_string_lossy().into_owned()], &mut out);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(code, 2);
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}
