#[path = "../support/mod.rs"]
mod support;

use std::fs;

use crate::support::shared_fixtures::{has_vba_under, scan_bytes, vba_docx, zip_bytes};
use bookdoctor::cli_parser::run;
use bookdoctor::option_model::Options;

#[test]
fn no_recurse_turns_the_nested_scan_off() {
    let outer = zip_bytes(&[("inner.docx", &vba_docx())]);
    let opts = Options { recurse: false, ..Options::default() };
    let r = scan_bytes("a.zip", &outer, &opts.check_options());
    assert!(!has_vba_under(&r, "inner.docx!"), "{:?}", r.findings);
    assert!(Options::default().recurse);
    // 명령행 옵션도 받는다.
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.zip"), &outer).unwrap();
    let mut out = Vec::new();
    run(&["--no-recurse".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().join("a.zip").to_string_lossy().into_owned()], &mut out);
    assert!(!String::from_utf8(out).unwrap().contains("inner.docx!"));
}
