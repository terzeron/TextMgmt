#[path = "../support/mod.rs"]
mod support;

use bookdoctor::cli::{run, run_with_env};

#[test]
fn member_name_with_escape_never_reaches_the_terminal_raw() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("esc.zip");
    support::zip_of(&p, &[("a\u{1b}[2J.exe", b"x")]);
    let mut out = Vec::new();
    run_with_env(&["scan".into(), "--min-severity".into(), "low".into(), p.to_string_lossy().into_owned()], &mut out, false, false);
    assert!(!String::from_utf8(out).unwrap().contains('\u{1b}'));
    let mut json = Vec::new();
    run(&["scan".into(), "--json".into(), "--min-severity".into(), "low".into(), p.to_string_lossy().into_owned()], &mut json);
    let json = String::from_utf8(json).unwrap();
    assert!(!json.contains("\\u001b") && !json.contains('\u{1b}'), "{json}");
}
