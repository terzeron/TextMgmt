//! 표지 테스트의 문서·PDF 준비물(`make_doc`, `use_doc`, `create_test_pdf`). 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use std::path::Path;
use super::es_fake::FakeEsManager;
use super::pdf::write_blank_pdf;
use serde_json::{json, Value};

pub fn create_test_pdf(output_path: &Path) {
    write_blank_pdf(output_path, 612, 792);
}

pub fn make_doc(relative_path: &str, file_type: &str) -> Value {
    json!({"category": "test_category", "title": "Test Book", "author": "Author", "file_path": relative_path, "file_type": file_type, "file_size": 1, "line_count": 0, "page_count": 1, "isbn": "", "summary": "test", "updated_time": "2024-01-01T00:00:00.000000"})
}

pub fn use_doc(es: &FakeEsManager, relative_path: &str, file_type: &str) {
    es.set_doc(Some(make_doc(relative_path, file_type)));
}
