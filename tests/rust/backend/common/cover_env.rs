//! EPUB 표지 설정 테스트의 환경(`Env`, `env`, `split`). 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use backend::book_manager::{BookManager, BookManagerDeps, BookRef};
use super::epub::{EpubSpec, make_epub};
use super::es_fake::FakeEsManager;
use super::image::red_jpeg;
use serde_json::Value;
use super::shared_fixtures::{FakeBooks, FetchSlot, URL};

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
/// (manager 대역, EPUB 경로, 책). get_book은 이 EPUB를 가리키는 책을 돌려준다.
pub struct Env {
    pub tmp: tempfile::TempDir,
    pub path: PathBuf,
    pub manager: BookManager,
    pub es: Arc<FakeEsManager>,
    pub books: Arc<FakeBooks>,
    pub fetcher: Arc<FetchSlot>,
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
impl Env {
    pub fn dir(&self) -> &Path {
        self.tmp.path()
    }

    pub async fn call(&self, url: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.set_epub_cover(1, Some(url), None).await)
    }

    pub async fn upload(&self, image_data: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.set_epub_cover(1, None, Some(image_data)).await)
    }

    pub async fn remove(&self) -> (Option<Value>, Option<String>) {
        split(self.manager.remove_epub_cover(1).await)
    }

    pub async fn illustrate(&self, before: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.insert_epub_illustration(1, before, Some(URL), None).await)
    }

    pub fn size(&self) -> u64 {
        fs::metadata(&self.path).unwrap().len()
    }
}

pub fn env() -> Env {
    super::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(FetchSlot(Mutex::new(Box::new(|_| Ok(red_jpeg())))));
    let path = make_epub(&tmp.path().join("분류").join("book.epub"), &EpubSpec::cover("3.0", false));
    let es = FakeEsManager::new();
    let books = Arc::new(FakeBooks(Mutex::new((Some(BookRef { book_id: 1, file_path: path.clone(), file_type: "epub".into() }), None))));
    let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_trash_dirname("0_trash").with_book_lookup(books.clone()).with_image_fetcher(fetcher.clone()));
    Env { tmp, path, manager, es, books, fetcher }
}

/// `"x" in error`: error가 None이면 Python은 TypeError로 실패한다.
/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
pub fn split(result: Result<Value, String>) -> (Option<Value>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}
