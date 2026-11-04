//! `backend::epub_cover::xml_tree` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use backend::book_metadata::BookLookupError;
use backend::book_manager::{BookLookup, BookManager, BookManagerDeps, BookRef};
use backend::direct_cover_url::{FetchError, ImageFetcher};
use common::epub::{EpubSpec, make_epub};
use common::es_fake::FakeEsManager;
use common::image::red_jpeg;
use common::zipx::Members;
use serde_json::Value;
use crate::common::shared_fixtures::{spine_idrefs, xml_text};

const URL: &str = "https://img.example.com/cover.jpg";

// ── 대역 (env fixture) ─────────────────────────────────────

type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync>;

/// `monkeypatch.setattr("backend.cover_management.fetch_url_image", ...)`의 대응. 테스트 중간에 바꿀 수 있다.
/// `BookManagerDeps::with_image_fetcher`로 주입한다. 오류 타입은 `FetchError`(구 `EpubCoverError`).
struct FetchSlot(Mutex<Fetch>);

impl FetchSlot {    }

impl ImageFetcher for FetchSlot {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        (self.0.lock().unwrap())(url)
    }
}

/// `AsyncMock(return_value=(book, None))` get_book.
struct FakeBooks(Mutex<(Option<BookRef>, Option<String>)>);

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
impl FakeBooks {
    fn set_file_type(&self, file_type: &str) {
        self.0.lock().unwrap().0.as_mut().unwrap().file_type = file_type.to_string();
    }

    fn set_missing(&self, message: &str) {
        *self.0.lock().unwrap() = (None, Some(message.to_string()));
    }
}

#[async_trait::async_trait]
impl BookLookup for FakeBooks {
    async fn get_book(&self, book_id: i64) -> Result<BookRef, BookLookupError> {
        match self.0.lock().unwrap().clone() {
            (Some(book), _) => Ok(book),
            (None, Some(error)) => Err(BookLookupError::Failed(error)),
            (None, None) => Err(BookLookupError::NotFound(book_id)),
        }
    }
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
/// (manager 대역, EPUB 경로, 책). get_book은 이 EPUB를 가리키는 책을 돌려준다.
struct Env {
    tmp: tempfile::TempDir,
    path: PathBuf,
    manager: BookManager,
    es: Arc<FakeEsManager>,
    books: Arc<FakeBooks>,
    fetcher: Arc<FetchSlot>,
}

fn env() -> Env {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(FetchSlot(Mutex::new(Box::new(|_| Ok(red_jpeg())))));
    let path = make_epub(&tmp.path().join("분류").join("book.epub"), &EpubSpec::cover("3.0", false));
    let es = FakeEsManager::new();
    let books = Arc::new(FakeBooks(Mutex::new((Some(BookRef { book_id: 1, file_path: path.clone(), file_type: "epub".into() }), None))));
    let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_trash_dirname("0_trash").with_book_lookup(books.clone()).with_image_fetcher(fetcher.clone()));
    Env { tmp, path, manager, es, books, fetcher }
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
impl Env {
    fn dir(&self) -> &Path {
        self.tmp.path()
    }

    async fn call(&self, url: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.set_epub_cover(1, Some(url), None).await)
    }

    async fn upload(&self, image_data: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.set_epub_cover(1, None, Some(image_data)).await)
    }

    async fn remove(&self) -> (Option<Value>, Option<String>) {
        split(self.manager.remove_epub_cover(1).await)
    }

    async fn illustrate(&self, before: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.insert_epub_illustration(1, before, Some(URL), None).await)
    }

    fn size(&self) -> u64 {
        fs::metadata(&self.path).unwrap().len()
    }
}

/// `"x" in error`: error가 None이면 Python은 TypeError로 실패한다.
/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
fn split(result: Result<Value, String>) -> (Option<Value>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

/// 교체로 빠지는 기존 표지는 새 표지 바로 뒤, 본문 앞의 도판으로 남는다.
#[tokio::test]
async fn test_replaced_cover_stays_as_an_illustration_after_the_new_cover() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    let old_cover = Members::read(&env.path).get("OPS/cover.jpg").to_vec();

    let (_, error) = env.call(URL).await;

    assert!(error.is_none());
    let after = Members::read(&env.path);
    assert!(!after.contains("OPS/cover.jpg"));
    let plates: Vec<String> = after.names().into_iter().filter(|name| name.starts_with("OPS/tm-plate-") && name.ends_with(".jpg")).collect();
    assert!(plates.len() == 1 && after.get(&plates[0]) == old_cover);
    let ids = spine_idrefs(&env.path);
    assert!(ids[0].starts_with("tm-cover-page-") && ids[1].starts_with("tm-plate-page-") && ids[2] == "chapter");
    let page = xml_text(after.get(&format!("OPS/{}.xhtml", ids[1])));
    assert!(page.contains(plates[0].strip_prefix("OPS/").unwrap()));
    // 도판은 표지로 오인되지 않는다. 표지는 새 이미지다.
    let mut archive = zip::ZipArchive::new(File::open(&env.path).unwrap()).unwrap();
    assert!(BookManager::find_epub_cover_member(&mut archive).unwrap().contains("tm-cover-"));
}
