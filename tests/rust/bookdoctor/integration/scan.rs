#[path = "../support/mod.rs"]
mod support;

use bookdoctor::cli_parser::run;

#[test]
fn missing_path_is_incomplete_exit_2() {
    let d = tempfile::tempdir().unwrap();
    let missing = d.path().join("none.epub");
    let mut out = Vec::new();
    assert_eq!(run(&["scan".into(), missing.to_string_lossy().into_owned()], &mut out), 2);
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
    let code = run(&["scan".into(), d.path().to_string_lossy().into_owned()], &mut out);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(code, 2);
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}
