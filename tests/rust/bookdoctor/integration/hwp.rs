#[path = "../support/mod.rs"]
mod support;

use std::fs;

use crate::support::shared_fixtures::hwp_cfb;
use crate::support::shared_fixtures::scan;
use bookdoctor::Format;
use bookdoctor::cli_parser::run;

#[test]
fn hwp_is_collected_and_lookalikes_are_not() {
    let d = tempfile::tempdir().unwrap();
    // 깨끗한 파일은 출력에 나오지 않으므로 이름 불일치가 나오는 내용을 쓴다.
    for name in ["a.hwp", "b.HWP", "c.hwpp"] {
        fs::write(d.path().join(name), "그냥 텍스트\n".repeat(50)).unwrap();
    }
    let mut out = Vec::new();
    run(&["scan".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("a.hwp") && out.contains("b.HWP"), "{out}");
    assert!(!out.contains("c.hwpp"), "{out}");
    assert_eq!(scan("x.hwp", &hwp_cfb(&[], 0).bytes).format, Some(Format::Hwp));
}

// --- 압축 해제 ---
