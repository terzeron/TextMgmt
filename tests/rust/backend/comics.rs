//! 원본: tests/test_comics.py
//!
//! 가정한 API (아직 구현 전이다. Python 이름 그대로이고 비공개 `_x`는 `pub fn x`로 본다):
//! - `backend::comics::Comics::new(book_id: i64, info: &serde_json::Value, score: f64) -> Comics` (Python의 `score=0.0` 기본값은 호출부에서 0.0으로 채운다)
//! - `Comics`의 공개 필드: `line_count`, `page_count`, `isbn`, `summary`, `score: f64`. `dict(&self) -> Value`, `json(&self) -> String`, `impl Display`
//! - `backend::comics::path_prefix_from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String>` (import 시점 `SystemExit`에 대응)
//! - `backend::comics_manager::ComicsManager::from_env(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>) -> Result<ComicsManager, RuntimeError>`
//!   (`RuntimeError: Display`, 메시지에 `TM_COMICS_DIR`. 설계 3.12: 구 `from_env_with_es`와 `IndexManager`를 `from_env(env, es_factory)`와 `EsManagerApi`로 통일했다)
//!   - `backend::es_manager::EsManagerApi::create_index(&self)`: 기본 구현은 `Err(Unsupported)`다. fake는 `common::es_fake::FakeEsManager`(호출 횟수 `create_index_calls`)
//!   - `ComicsManager::path_prefix() -> &Path`, `es_index_name() -> &str`, `mismatch_cache() -> Option<&Value>`, `mismatch_cache_time() -> f64`,
//!     `set_mismatch_cache(Value)`, `clear_mismatch_cache()`

mod common;

use std::sync::{Arc, Mutex};

use backend::comics::{Comics, path_prefix_from_env};
use backend::comics_manager::ComicsManager;
use backend::es_manager::EsManagerApi;
use common::es_fake::FakeEsManager;
use serde_json::{Value, json};

fn info() -> Value {
    json!({"category": "cat", "title": "title", "author": "author", "file_path": "cat/file.epub", "file_type": "epub", "file_size": 10, "updated_time": "2021-01-01T00:00:00.000000"})
}

mod test_comics {
    use super::*;

    #[test]
    fn test_init_and_dict() {
        common::setup_env();
        let c = Comics::new(1, &info(), 1.5);
        let data = c.dict();
        assert_eq!(data["book_id"], 1);
        assert_eq!(data["category"], "cat");
        assert_eq!(data["file_path"], "cat/file.epub");
        assert_eq!(data["score"], 1.5);
    }

    #[test]
    fn test_json_and_str() {
        common::setup_env();
        let c = Comics::new(2, &info(), 0.0);
        assert!(c.json().contains("category"), "{}", c.json());
        assert!(c.to_string().contains("title"), "{c}");
    }

    #[test]
    fn test_init_uses_optional_defaults_and_score() {
        common::setup_env();
        let info = json!({
            "category": "cat2",
            "title": "title2",
            "author": "author2",
            "file_path": "cat2/file.pdf",
            "file_type": "pdf",
            "file_size": 20,
            "updated_time": "2022-02-02T00:00:00.000000",
        });
        let c = Comics::new(3, &info, 2.5);
        assert_eq!(c.line_count, 0);
        assert_eq!(c.page_count, 0);
        assert_eq!(c.isbn, "");
        assert_eq!(c.summary, "");
        assert_eq!(c.score, 2.5);
        assert_eq!(c.dict()["file_path"], "cat2/file.pdf");
    }

    #[test]
    #[should_panic]
    fn test_dict_rejects_paths_outside_comics_root() {
        common::setup_env();
        let mut value = info();
        value["file_path"] = json!("/outside/book.epub");
        Comics::new(4, &value, 0.0).dict();
    }
}

#[test]
fn test_comics_manager_missing_env() {
    // comics_manager.py line 21: TM_COMICS_DIR가 없으면 RuntimeError. 환경 변수를 지우지 않고 조회 함수를 주입한다.
    let err = ComicsManager::from_env(&|_| None, &|_| FakeEsManager::new() as Arc<dyn EsManagerApi>).err().expect("TM_COMICS_DIR 없이도 성공했다");
    assert!(err.to_string().contains("TM_COMICS_DIR"), "{err}");
}

#[test]
fn test_comics_manager_init() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_str().unwrap().to_string();
    let es = FakeEsManager::new();
    let index_names = Arc::new(Mutex::new(Vec::<String>::new()));

    let get = move |key: &str| match key {
        "TM_COMICS_DIR" => Some(dir.clone()),
        "TM_ES_COMICS_INDEX" => Some("test_comics".to_string()),
        _ => None,
    };
    let (es_in, names_in) = (es.clone(), index_names.clone());
    // Python `DummyESManager`: 인덱스 이름을 기억하고 `create_index` 호출을 기록한다.
    let mut manager = ComicsManager::from_env(&get, &move |index_name: &str| {
        names_in.lock().unwrap().push(index_name.to_string());
        es_in.clone() as Arc<dyn EsManagerApi>
    })
    .unwrap();

    assert_eq!(manager.path_prefix(), tmp.path());
    assert_eq!(manager.es_index_name(), "test_comics");
    assert_eq!(*index_names.lock().unwrap(), ["test_comics"]);
    assert_eq!(es.with(|s| s.create_index_calls), 1);
    assert!(manager.mismatch_cache().is_none());
    assert_eq!(manager.mismatch_cache_time(), 0.0);
    // 부모 __init__을 부르지 않으므로 불일치 캐시 lock도 직접 만들어야 한다.
    // 빠지면 만화 카테고리 관리의 /comics/category-mismatches가 AttributeError로 실패한다.
    manager.set_mismatch_cache(json!({"stale": true}));
    manager.clear_mismatch_cache();
    assert!(manager.mismatch_cache().is_none());
}

// ---- merged from test_comics_env_guard.py ----

#[test]
fn test_comics_requires_comics_dir() {
    // TM_COMICS_DIR가 없으면 import 시점에 SystemExit가 난다. 환경 변수를 지우지 않고 조회 함수를 주입해 같은 조건을 만든다.
    let missing = path_prefix_from_env(&|_| None);
    assert!(missing.is_err(), "TM_COMICS_DIR 없이도 성공했다: {missing:?}");
}
