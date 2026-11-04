#[path = "../support/mod.rs"]
mod support;

use std::fs;

use crate::support::shared_fixtures::doc_cfb;
use crate::support::shared_fixtures::write;
use bookdoctor::Format;
use bookdoctor::cli_parser::{Options, run};
use bookdoctor::format_dispatcher::scan_file;

#[test]
fn doc_and_dot_are_collected_and_lookalikes_are_not() {
    let d = tempfile::tempdir().unwrap();
    let bytes = doc_cfb(&[], 0, b"x").bytes;
    for name in ["a.doc", "b.DOT", "c.docs"] {
        fs::write(d.path().join(name), &bytes).unwrap();
    }
    let mut out = Vec::new();
    run(&["-r".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(!out.contains("c.docs"), "{out}");
    let (_d2, p) = write("b.dot", &bytes);
    assert_eq!(scan_file(&p, &Options::default()).format, Some(Format::Doc));
}
