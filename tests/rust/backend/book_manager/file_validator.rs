//! `backend::book_manager::file_validator` 단위 테스트.

#[path = "../common/mod.rs"]
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
    // Lines 395, 397: PDF with Producer and CreationDate metadata
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
    assert!(result["publication"].get("producer").is_some());
    assert!(result["publication"].get("creation_date").is_some());
}
