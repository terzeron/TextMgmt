//! `tm-pdf-worker` 실행 파일 통합 테스트: 인자를 받아 `run_worker` 결과를 stdout과 종료 코드로 돌려준다.

use std::process::Command;

use lopdf::{Document, Object, dictionary};

fn one_page_pdf(dir: &std::path::Path) -> std::path::PathBuf {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let page = doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()]});
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1}));
    let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
    doc.trailer.set("Root", catalog);
    let path = dir.join("a.pdf");
    doc.save(&path).unwrap();
    path
}

fn worker() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tm-pdf-worker"))
}

#[test]
fn a_valid_request_writes_the_result_to_stdout_and_exits_0() {
    let d = tempfile::tempdir().unwrap();
    let out = worker().arg("info").arg(one_page_pdf(d.path())).output().unwrap();
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "Pages: 1\n");
}

#[test]
fn a_request_without_arguments_prints_the_usage_to_stderr_and_exits_1() {
    let out = worker().output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stderr).unwrap().starts_with("usage:"));
    assert!(out.stdout.is_empty());
}
