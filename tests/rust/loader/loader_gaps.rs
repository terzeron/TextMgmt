//! 원본: tests/test_loader_gaps.py (전체)
//!
//! loader 재적재 경로의 드문 분기를 확인한다. ES는 `IndexStore`를 구현한 가짜 저장소로 바꾼다.
//! 실패 코드는 Rust crate의 실제 값을 쓴다. 가정한 API는 없다(실제 crate의 공개 API만 쓴다).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use doc_reader::pdf::InProcess;
use es_client::EsError;
use es_client::indexing::IndexStore;
use loader::cli::{World, run};
use serde_json::Value;

/// conftest.py/`_setup_loader_env` 대응: 책 prefix는 root, 만화 prefix는 root/comics.
fn setup_loader_env(root: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("TM_BOOK_DIR".to_owned(), root.to_string_lossy().into_owned());
    env.insert("TM_COMICS_DIR".to_owned(), root.join("comics").to_string_lossy().into_owned());
    env.insert("TM_ES_BOOK_INDEX".to_owned(), "test_books".to_owned());
    env.insert("TM_ES_COMICS_INDEX".to_owned(), "test_comics".to_owned());
    env
}

#[derive(Default)]
struct State {
    /// ES가 돌려주는 메타데이터. 이번 배치에 없는 inode가 섞여 있어도 그대로 돌려준다.
    created_meta: HashMap<i64, BTreeMap<String, String>>,
    inserted: Vec<Vec<(i64, Value)>>,
}

#[derive(Clone)]
struct FakeStore {
    state: Arc<Mutex<State>>,
}

impl IndexStore for FakeStore {
    fn index_name(&self) -> &str {
        "test_books"
    }
    fn ping(&self) -> Result<(), EsError> {
        Ok(())
    }
    fn index_exists(&self) -> Result<bool, EsError> {
        Ok(true)
    }
    fn create_index(&self) -> Result<(), EsError> {
        Ok(())
    }
    fn delete_index(&self) -> Result<(), EsError> {
        Ok(())
    }
    fn get_existing_paths(&self, _doc_ids: &[i64]) -> Result<HashMap<i64, String>, EsError> {
        Ok(HashMap::new())
    }
    fn get_existing_created_time_metadata(&self, _doc_ids: &[i64]) -> Result<HashMap<i64, BTreeMap<String, String>>, EsError> {
        Ok(self.state.lock().unwrap().created_meta.clone())
    }
    fn delete_by_file_paths(&self, _file_paths: &[String], _exclude_ids: &[i64]) -> i64 {
        0
    }
    fn get_doc_ids_by_path_prefix(&self, _path_prefix: &str) -> HashSet<i64> {
        HashSet::new()
    }
    fn delete_by_ids(&self, ids: &[i64]) -> i64 {
        ids.len() as i64
    }
    fn insert(&self, docs: &[(i64, Value)]) -> Result<(), EsError> {
        self.state.lock().unwrap().inserted.push(docs.to_vec());
        Ok(())
    }
    fn refresh(&self) -> Result<(), EsError> {
        Ok(())
    }
}

#[test]
fn test_reload_ignores_stored_created_time_of_files_outside_the_batch() {
    // ES가 이번 배치에 없는 inode의 메타데이터를 같이 돌려줘도 무시한다.
    let dir = tempfile::tempdir().unwrap();
    let env = setup_loader_env(dir.path());
    let file_path = dir.path().join("[author] book.txt");
    fs::write(&file_path, "content").unwrap();
    let inode = fs::metadata(&file_path).unwrap().ino() as i64;

    let stranger = inode + 1_000_000;
    let meta = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> { pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
    let store = FakeStore { state: Arc::new(Mutex::new(State::default())) };
    store.state.lock().unwrap().created_meta =
        HashMap::from([(stranger, meta(&[("created_time", "1999-01-01T00:00:00"), ("created_time_source", "statx_btime")])), (inode, meta(&[("created_time", "2020-01-01T00:00:00")]))]);

    let args: Vec<String> = ["--reload", "book", file_path.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    let get = |k: &str| env.get(k).cloned();
    let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { Ok(Box::new(store.clone())) };
    let world = World { env: &get, connect: &connect, runner: &InProcess };
    let mut out = Vec::new();

    assert_eq!(run("loader", &args, &world, &mut out), 0);
    let inserted_data: HashMap<i64, Value> = store.state.lock().unwrap().inserted.iter().flatten().cloned().collect();
    assert_eq!(inserted_data.keys().copied().collect::<HashSet<i64>>(), HashSet::from([inode]));
    assert_eq!(inserted_data[&inode]["created_time"], "2020-01-01T00:00:00");
}
