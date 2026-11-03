use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained")
}

#[test]
fn verbose_classify_emits_debug_diagnostics() {
    let dir = TempDir::new().unwrap();
    let epub = dir.path().join("broken.epub");
    fs::write(&epub, b"not a zip archive").unwrap();
    let run = |verbose: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_trainer"));
        if verbose {
            command.arg("-v");
        }
        command.args(["classify", "--no-es", "--model"]).arg(fixture()).arg(&epub).output().unwrap()
    };
    let quiet = run(false);
    assert!(quiet.status.success(), "{}", String::from_utf8_lossy(&quiet.stderr));
    assert!(!String::from_utf8_lossy(&quiet.stderr).contains("DEBUG"));
    let output = run(true);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("DEBUG") && stderr.contains("읽지 못했다"), "{stderr}");
}

#[test]
fn classify_reports_missing_invalid_and_incomplete_es_environment_without_connecting() {
    use std::process::{Command, Output};

    fn run_with_es(command_name: &str, url: Option<&str>, index: Option<&str>, configure_optional_auth: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_trainer"));
        if command_name == "classify" {
            command.args(["classify", "/no/such/file", "--model"]).arg(fixture());
        } else {
            let output = TempDir::new().unwrap();
            command.args(["collect", "--out"]).arg(output.path().join("corpus.jsonl"));
        }
        for key in ["TM_ES_URL", "TM_ES_BOOK_INDEX", "TM_ES_USER", "TM_ES_PASSWORD", "TM_ES_CA_CERT"] {
            command.env_remove(key);
        }
        if let Some(url) = url {
            command.env("TM_ES_URL", url);
        }
        if let Some(index) = index {
            command.env("TM_ES_BOOK_INDEX", index);
        }
        if configure_optional_auth {
            command.env("TM_ES_USER", "test-user").env("TM_ES_PASSWORD", "test-password").env("TM_ES_CA_CERT", "/etc/ssl/certs/ca-certificates.crt");
        }
        command.output().unwrap()
    }

    let missing_url = run_with_es("classify", None, None, false);
    assert_eq!(missing_url.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing_url.stderr).contains("TM_ES_URL이 없다"));

    let invalid_url = run_with_es("classify", Some("ftp://localhost:9200"), Some("books"), false);
    assert_eq!(invalid_url.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid_url.stderr).contains("http:// 또는 https://"));

    let missing_index = run_with_es("classify", Some("http://localhost:9200"), None, false);
    assert_eq!(missing_index.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing_index.stderr).contains("TM_ES_BOOK_INDEX가 없다"));

    let configured = run_with_es("classify", Some("https://localhost:9200"), Some("books"), true);
    assert!(configured.status.success(), "{}", String::from_utf8_lossy(&configured.stderr));
    assert!(String::from_utf8_lossy(&configured.stdout).contains("/no/such/file: 파일이 없다"));

    for (url, index, error) in [(None, None, "TM_ES_URL이 없다"), (Some("ftp://localhost"), Some("books"), "http:// 또는 https://"), (Some("http://localhost"), None, "TM_ES_BOOK_INDEX가 없다")]
    {
        let output = run_with_es("collect", url, index, false);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains(error));
    }
}

#[test]
fn executable_exit_codes_and_help_match_the_library() {
    let binary = env!("CARGO_BIN_EXE_trainer");
    let output = std::process::Command::new(binary).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("사용법"));
    let output = std::process::Command::new(binary).arg("unknown").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}
