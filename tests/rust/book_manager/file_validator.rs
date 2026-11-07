//! `book_manager::file_validator` 단위 테스트.

#[path = "common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{blank_pdf_bytes, block_on, make_doc, make_manager, tmp_dir};
use common::es_fake::FakeEsManager;
use std::fs;

// ---------------------------------------------------------------------------
// 테스트 본문 (원본 순서)
// ---------------------------------------------------------------------------
// ---- coverage: validate_pdf ----

#[test]
fn test_validate_pdf_producer_and_creation_date() {
    // PDF with Producer and CreationDate metadata
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let pdf = tmp_path.join("A").join("test.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();

    // 최소한의 유효한 PDF를 만든다.
    fs::write(&pdf, blank_pdf_bytes(1, &[("Producer", "TestProducer"), ("CreationDate", "D:20240101"), ("Title", "Test"), ("Author", "Author")])).unwrap();

    es.set_doc(Some(make_doc("A/test.pdf", "pdf")));

    let result = block_on(manager.validate_pdf(1)).expect("result is not None");
    assert_eq!(result.publication.producer.as_deref(), Some("TestProducer"));
    assert!(result.publication.creation_date.is_some());
}

#[test]
fn test_validate_epub_reports_an_epubveri_run_failure() {
    use crate::common::shared_fixtures::deps;
    use common::runner::FakeRunner;
    let tmp = tmp_dir();
    fs::create_dir_all(tmp.path().join("A")).unwrap();
    fs::write(tmp.path().join("A").join("book.epub"), b"x").unwrap();
    let es = FakeEsManager::new();
    es.set_doc(Some(make_doc("A/book.epub", "epub")));
    let manager = book_manager::BookManager::with_deps(deps(tmp.path(), &es).with_runner(FakeRunner::returning(Err(sysdeps::CommandError::Failed("boom".into())))));
    let Err(error) = block_on(manager.validate_epub(1)) else { panic!("실행 실패는 오류다") };
    assert_eq!(error, "epubveri failed: boom");
}
