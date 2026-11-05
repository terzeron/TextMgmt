//! PDF 쪽 읽기·미리보기 테스트의 환경(`Env`, `build`, `make_doc`). 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use super::es_fake::FakeEsManager;
use super::shared_fixtures::create_test_pdf;
use book_manager::{BookManager, BookManagerDeps};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
pub struct Env {
    pub bm: Arc<BookManager>,
    pub es: Arc<FakeEsManager>,
    pub temp_dir: PathBuf,
    pub _tmp: TempDir,
}

pub trait ResponseExt {
    fn header(&self, name: &str) -> Option<&str>;
}

/// `configure`는 임시 디렉터리와 기본 Deps를 받는다. 캐시 한도처럼 만들어 둔 파일 크기가 필요한 설정에 쓴다.
pub fn build(configure: impl FnOnce(&Path, BookManagerDeps) -> BookManagerDeps) -> Env {
    super::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let temp_dir = tmp.path().to_path_buf();
    let pdf = temp_dir.join("test_category").join("test_book.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();
    create_test_pdf(5, &pdf);
    let default_doc = make_doc("test_category/test_book.pdf", fs::metadata(&pdf).unwrap().len(), 5, "pdf");
    let es = FakeEsManager::with_doc(default_doc.clone());
    let deps = configure(&temp_dir, BookManagerDeps::new(temp_dir.clone(), es.clone()));
    Env { bm: Arc::new(BookManager::with_deps(deps)), es, temp_dir, _tmp: tmp }
}

pub fn env() -> Env {
    build(|_, d| d)
}

/// ES 문서 dict 생성 헬퍼.
pub fn make_doc(relative_path: &str, file_size: u64, page_count: i64, file_type: &str) -> Value {
    json!({
        "category": "test_category", "title": "Test Book", "author": "Author", "file_path": relative_path, "file_type": file_type,
        "file_size": file_size, "line_count": 0, "page_count": page_count, "isbn": "", "summary": "test", "updated_time": "2024-01-01T00:00:00.000000"
    })
}
