//! `Arc<dyn EsManagerApi>`로 `BookManager`를 만드는 준비물. 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use super::shared_fixtures::make_deps;
use backend::book_manager::BookManager;
use backend::es_manager::EsManagerApi;
use std::path::Path;
use std::sync::Arc;

/// Python `make_manager(tmp_path, es)`.
pub fn make_manager(tmp_path: &Path, es: Arc<dyn EsManagerApi>) -> BookManager {
    BookManager::with_deps(make_deps(tmp_path, es))
}
