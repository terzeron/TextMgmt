//! PDF 쪽 편집 테스트의 환경(`Env`, `env`). 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use super::es_fake::FakeEsManager;
use super::faulty_fs::FaultyFs;
use super::pdf::make_pdf;
use super::shared_fixtures::split;
use backend::book_manager::PdfEditResult;
use backend::book_manager::{BookManager, BookManagerDeps};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
pub struct Env {
    pub bm: BookManager,
    pub es: Arc<FakeEsManager>,
    pub fs: Arc<FaultyFs>,
    pub pdf: PathBuf,
    pub root: tempfile::TempDir,
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
impl Env {
    pub async fn edit(&self, pages: &[i64], order: Option<&[i64]>) -> (Option<PdfEditResult>, Option<String>) {
        split(self.bm.delete_pdf_pages(1, pages, None, None, None, order).await)
    }
    pub fn backup(&self) -> PathBuf {
        self.root.path().join("0_trash/book.pdf")
    }
    pub fn assert_no_parts(&self) {
        for dir in [self.pdf.parent().unwrap().to_path_buf(), self.root.path().join("0_trash")] {
            if let Ok(entries) = fs::read_dir(dir) {
                assert!(entries.flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
            }
        }
    }
}

pub fn env() -> Env {
    super::setup_env();
    let root = tempfile::tempdir().unwrap();
    let pdf = root.path().join("cat/book.pdf");
    make_pdf(&pdf, 5);
    let es = FakeEsManager::with_doc(
        json!({"category":"cat", "title":"Title", "author":"Author", "file_path":"cat/book.pdf", "file_type":"pdf", "file_size": 0, "page_count":5, "updated_time":"2024-01-01T00:00:00.000000"}),
    );
    let fs = FaultyFs::new();
    let bm = BookManager::with_deps(BookManagerDeps::new(root.path().to_path_buf(), es.clone()).with_fs(fs.clone()));
    Env { bm, es, fs, pdf, root }
}
