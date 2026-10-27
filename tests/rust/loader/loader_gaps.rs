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
use doc_reader::pdf::{Stage, StageError, StageRunner};
use es_client::EsError;
use es_client::indexing::IndexStore;
use loader::cli::{ChildRunner, World, run};
use serde_json::Value;

#[cfg(unix)]
fn executable_script(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sh");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    (dir, path)
}

#[cfg(unix)]
#[test]
fn child_runner_parses_successful_json_and_reports_its_executable() {
    let (_dir, executable) = executable_script("printf '%s' '{\"text\":\"parsed\",\"pages\":3}'");
    let runner = ChildRunner::new(executable.clone());

    assert_eq!(runner.run(Stage::Pdfium, Path::new("sample.pdf")).unwrap(), ("parsed".into(), 3));
    assert_eq!(runner.worker_executable(), Some(executable.as_path()));
}

#[cfg(unix)]
#[test]
fn child_runner_reports_nonzero_exit_and_invalid_json() {
    let (_dir, failed) = executable_script("echo 'worker failed' >&2\nexit 7");
    let error = ChildRunner::new(failed).run(Stage::Repair, Path::new("sample.pdf")).unwrap_err();
    assert!(matches!(error, StageError::Failed(message) if message == "worker failed"));

    let (_dir, malformed) = executable_script("printf 'not-json'");
    let error = ChildRunner::new(malformed).run(Stage::Plumber, Path::new("sample.pdf")).unwrap_err();
    assert!(matches!(error, StageError::Failed(_)));
}

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
    fail_existing_paths: bool,
    fail_created_meta: bool,
    fail_insert: bool,
    fail_refresh: bool,
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
        if self.state.lock().unwrap().fail_existing_paths { Err(EsError::Transport("path lookup failed".to_owned())) } else { Ok(HashMap::new()) }
    }
    fn get_existing_created_time_metadata(&self, _doc_ids: &[i64]) -> Result<HashMap<i64, BTreeMap<String, String>>, EsError> {
        let state = self.state.lock().unwrap();
        if state.fail_created_meta { Err(EsError::Transport("metadata lookup failed".to_owned())) } else { Ok(state.created_meta.clone()) }
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
        let mut state = self.state.lock().unwrap();
        if state.fail_insert {
            Err(EsError::Transport("insert failed".to_owned()))
        } else {
            state.inserted.push(docs.to_vec());
            Ok(())
        }
    }
    fn refresh(&self) -> Result<(), EsError> {
        if self.state.lock().unwrap().fail_refresh { Err(EsError::Transport("refresh failed".to_owned())) } else { Ok(()) }
    }
}

#[test]
fn test_existing_path_lookup_failure_aborts_loader_run() {
    let dir = tempfile::tempdir().unwrap();
    let env = setup_loader_env(dir.path());
    fs::write(dir.path().join("book.txt"), "[author] title\ntext").unwrap();
    let store = FakeStore { state: Arc::new(Mutex::new(State { fail_existing_paths: true, ..State::default() })) };
    let args: Vec<String> = ["book", dir.path().to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    let get = |k: &str| env.get(k).cloned();
    let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { Ok(Box::new(store.clone())) };
    let world = World { env: &get, connect: &connect, runner: &InProcess };
    let mut out = Vec::new();

    assert_eq!(run("loader", &args, &world, &mut out), 255);
    assert!(store.state.lock().unwrap().inserted.is_empty());
}

#[test]
fn test_insert_failure_returns_error_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let env = setup_loader_env(dir.path());
    let file = dir.path().join("[author] title.txt");
    fs::write(&file, "text").unwrap();
    let store = FakeStore { state: Arc::new(Mutex::new(State { fail_insert: true, ..State::default() })) };
    let args: Vec<String> = ["--reload", "book", file.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    let get = |k: &str| env.get(k).cloned();
    let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { Ok(Box::new(store.clone())) };
    let world = World { env: &get, connect: &connect, runner: &InProcess };
    let mut out = Vec::new();

    assert_eq!(run("loader", &args, &world, &mut out), 255);
}

#[test]
fn test_created_time_metadata_lookup_failure_returns_error_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let env = setup_loader_env(dir.path());
    let file = dir.path().join("[author] title.txt");
    fs::write(&file, "text").unwrap();
    let store = FakeStore { state: Arc::new(Mutex::new(State { fail_created_meta: true, ..State::default() })) };
    let args: Vec<String> = ["--reload", "book", file.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    let get = |k: &str| env.get(k).cloned();
    let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { Ok(Box::new(store.clone())) };
    let world = World { env: &get, connect: &connect, runner: &InProcess };
    let mut out = Vec::new();

    assert_eq!(run("loader", &args, &world, &mut out), 255);
    assert!(store.state.lock().unwrap().inserted.is_empty());
}

#[test]
fn test_final_refresh_failure_returns_error_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let env = setup_loader_env(dir.path());
    let file = dir.path().join("[author] title.txt");
    fs::write(&file, "text").unwrap();
    let store = FakeStore { state: Arc::new(Mutex::new(State { fail_refresh: true, ..State::default() })) };
    let args: Vec<String> = ["--reload", "book", file.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    let get = |k: &str| env.get(k).cloned();
    let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { Ok(Box::new(store.clone())) };
    let world = World { env: &get, connect: &connect, runner: &InProcess };
    let mut out = Vec::new();

    assert_eq!(run("loader", &args, &world, &mut out), 255);
}

#[test]
fn test_orphan_scan_refresh_failure_returns_error_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let env = setup_loader_env(dir.path());
    fs::write(dir.path().join("[author] title.txt"), "text").unwrap();
    let store = FakeStore { state: Arc::new(Mutex::new(State { fail_refresh: true, ..State::default() })) };
    let args: Vec<String> = ["--recursive", "--reload", "book", dir.path().to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    let get = |k: &str| env.get(k).cloned();
    let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { Ok(Box::new(store.clone())) };
    let world = World { env: &get, connect: &connect, runner: &InProcess };
    let mut out = Vec::new();

    assert_eq!(run("loader", &args, &world, &mut out), 255);
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
