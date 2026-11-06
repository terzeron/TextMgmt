//! `tm-index-worker`(`libs/file_indexer/src/bin/index_worker.rs`) 통합 테스트. 실행 파일을 하위 프로세스로 돌린다.
//! backend·loader 실행 파일이 워커를 겸하던 때의 부명령 테스트를 옮겼다.

#[path = "../../shared/pdf_bytes.rs"]
mod pdf_bytes;

use std::fs;
use std::process::Command;

use pdf_bytes::minimal_pdf;
use serde_json::Value;

fn worker() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tm-index-worker"))
}

#[test]
fn worker_without_arguments_prints_usage() {
    let output = worker().output().unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("사용법:"));
}

#[test]
fn worker_rejects_an_unknown_command() {
    let output = worker().arg("serve").output().unwrap();

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn worker_runs_the_isbn_stream_scan_stage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-pdf.pdf");
    fs::write(&path, b"not a PDF").unwrap();

    let output = worker().args(["__pdf-stage", "isbn-stream-scan"]).arg(path).output().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!((result["text"].as_str(), result["pages"].as_u64()), (Some(""), Some(0)));
}

#[test]
fn worker_reports_pdf_stage_usage() {
    let output = worker().arg("__pdf-stage").output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: __pdf-stage"));
}

#[test]
fn worker_runs_poppler_pdf_stage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two-pages.pdf");
    fs::write(&path, minimal_pdf(2)).unwrap();

    let output = worker().args(["__pdf-stage", "text-poppler"]).arg(&path).output().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["pages"], 2);
}

#[test]
fn worker_reports_a_failing_pdf_stage_with_exit_1() {
    let dir = tempfile::tempdir().unwrap();

    let output = worker().args(["__pdf-stage", "text-poppler"]).arg(dir.path().join("missing.pdf")).output().unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty());
}

#[test]
fn worker_reports_read_file_usage() {
    let output = worker().args(["__read-file", "book-root"]).output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: __read-file"));
}

#[test]
fn worker_reads_a_text_file_into_an_index_record() {
    let dir = tempfile::tempdir().unwrap();
    let (books, comics) = (dir.path().join("book"), dir.path().join("comics"));
    fs::create_dir_all(books.join("3_판타지")).unwrap();
    fs::create_dir_all(&comics).unwrap();
    let path = books.join("3_판타지/a.txt");
    fs::write(&path, "소림사 장로가 강호에서 내공을 운기조식했다.\n".repeat(30)).unwrap();

    let output = worker().arg("__read-file").arg(&books).arg(&comics).arg(&path).args(["0", "", "0"]).output().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let item = result["item"].as_array().unwrap_or_else(|| panic!("{result}"));
    assert!(item[0].as_i64().is_some_and(|id| id > 0), "문서 id는 파일의 inode다: {result}");
    assert!(result["messages"].is_array());
}
