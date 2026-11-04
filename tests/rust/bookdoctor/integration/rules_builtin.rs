//! 내장 규칙 테스트. 규칙마다 양성과 음성 두 가지를 확인한다.

use std::fs;

use bookdoctor::cli_parser::run;

/// EICAR 표준 시험 문자열(악성 코드가 아니다). 저장소 파일이 백신에 걸리지 않도록 두 조각으로 둔다.
fn eicar() -> Vec<u8> {
    [&b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$"[..], &b"EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*"[..]].concat()
}

#[test]
fn scan_applies_the_builtin_rules_without_any_option() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("eicar.txt");
    fs::write(&f, eicar()).unwrap();
    let mut out = Vec::new();
    run(&["scan".into(), "--json".into(), f.to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    let v: serde_json::Value = serde_json::from_str(out.lines().next().unwrap_or_else(|| panic!("no output"))).unwrap();
    let hit = v["findings"].as_array().unwrap().iter().find(|f| f["code"] == "RULE_MATCH").unwrap_or_else(|| panic!("{out}"));
    assert_eq!((hit["severity"].as_str(), hit["location"].as_str()), (Some("CRITICAL"), Some("EICAR_TEST_FILE")));
}
