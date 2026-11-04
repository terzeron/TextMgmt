//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]
#![allow(clippy::type_complexity)]

use doc_reader::pdf::{InProcess, StageRunner};
use es_client::EsError;
use es_client::indexing::IndexStore;
use file_record::env_config::Config;
use loader::cli_parser::{RunHooks, World, run, run_with_hooks};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Clone)]
pub struct FakeStore {
    pub state: Arc<Mutex<State>>,
    pub ping_ok: bool,
    pub name: String,
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
impl FakeStore {
    pub fn new() -> Self {
        FakeStore { state: Arc::new(Mutex::new(State::default())), ping_ok: true, name: "test_books".to_owned() }
    }

    pub fn update(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.state.lock().unwrap());
    }

    pub fn set_existing_paths(&self, f: impl Fn(&[i64]) -> HashMap<i64, String> + Send + 'static) {
        self.update(|s| s.existing_paths = Some(Box::new(f)));
    }

    /// insert 호출 전체를 합친 `{inode: 문서}`. 같은 inode는 나중 것이 이긴다.
    pub fn inserted_docs(&self) -> HashMap<i64, Value> {
        self.state.lock().unwrap().inserted.iter().flatten().cloned().collect()
    }

    pub fn batch_sizes(&self) -> Vec<usize> {
        self.state.lock().unwrap().inserted.iter().map(Vec::len).collect()
    }
}

impl IndexStore for FakeStore {
    fn index_name(&self) -> &str {
        &self.name
    }
    fn ping(&self) -> Result<(), EsError> {
        if self.ping_ok { Ok(()) } else { Err(EsError::Transport("ping 실패".to_owned())) }
    }
    fn index_exists(&self) -> Result<bool, EsError> {
        let s = self.state.lock().unwrap();
        match &s.exists_error {
            Some(msg) => Err(EsError::Transport(msg.clone())),
            None => Ok(s.index_exists),
        }
    }
    fn create_index(&self) -> Result<(), EsError> {
        Ok(())
    }
    fn delete_index(&self) -> Result<(), EsError> {
        self.state.lock().unwrap().deleted_index = true;
        Ok(())
    }
    fn get_existing_paths(&self, doc_ids: &[i64]) -> Result<HashMap<i64, String>, EsError> {
        let s = self.state.lock().unwrap();
        Ok(s.existing_paths.as_ref().map(|f| f(doc_ids)).unwrap_or_default())
    }
    fn get_existing_created_time_metadata(&self, _doc_ids: &[i64]) -> Result<HashMap<i64, BTreeMap<String, String>>, EsError> {
        Ok(self.state.lock().unwrap().created_meta.clone())
    }
    fn delete_by_file_paths(&self, _file_paths: &[String], _exclude_ids: &[i64]) -> Result<i64, EsError> {
        Ok(self.state.lock().unwrap().delete_by_paths_result)
    }
    fn get_doc_ids_by_path_prefix(&self, _path_prefix: &str) -> Result<HashSet<i64>, EsError> {
        // 실제 적재된 inode + 디스크에 없는 orphan
        let s = self.state.lock().unwrap();
        if s.prefix_lookup_error {
            return Err(EsError::Transport("scroll failed".to_owned()));
        }
        Ok(s.inserted.iter().flatten().map(|(id, _)| *id).chain(s.orphan_ids.iter().copied()).collect())
    }
    fn delete_by_ids(&self, ids: &[i64]) -> Result<i64, EsError> {
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        self.state.lock().unwrap().deleted_ids = Some(sorted);
        Ok(ids.len() as i64)
    }
    fn insert(&self, docs: &[(i64, Value)]) -> Result<(), EsError> {
        self.state.lock().unwrap().inserted.push(docs.to_vec());
        Ok(())
    }
    fn refresh(&self) -> Result<(), EsError> {
        Ok(())
    }
}

// ---- ES 가짜 저장소 (MagicMock/ESManager 대체) ----

#[derive(Default)]
pub struct State {
    pub index_exists: bool,
    pub exists_error: Option<String>,
    pub deleted_index: bool,
    pub existing_paths: Option<Box<dyn Fn(&[i64]) -> HashMap<i64, String> + Send>>,
    pub created_meta: HashMap<i64, BTreeMap<String, String>>,
    pub delete_by_paths_result: i64,
    pub inserted: Vec<Vec<(i64, Value)>>,
    pub orphan_ids: HashSet<i64>,
    pub prefix_lookup_error: bool,
    pub deleted_ids: Option<Vec<i64>>,
}

// ---- TestFastPdfPageCount -------------------------------------------------------------------

pub type XrefStream = (Vec<u8>, [usize; 3], Vec<i64>, Option<i64>);

/// pypdf로 만든 72x72 빈 한 쪽짜리 PDF 대응.
pub fn blank_pdf() -> Vec<u8> {
    let objs = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>"];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// `Loader.path_prefix = book` 과 같다. 만화 prefix는 conftest 기본값이다.
pub fn config_with_book(book: &Path) -> Config {
    Config::new(book, tests_root().join("comics"))
}

/// conftest.py의 환경변수. 이 파일의 테스트는 프로세스 환경을 건드리지 않고 이 함수를 `Config::from_env`에 넘긴다.
pub fn conftest_env(name: &str) -> Option<String> {
    match name {
        "TM_BOOK_DIR" => Some(tests_root().join("books").to_string_lossy().into_owned()),
        "TM_COMICS_DIR" => Some(tests_root().join("comics").to_string_lossy().into_owned()),
        _ => None,
    }
}

/// conftest.py의 환경 변수 중 loader가 읽는 것. 색인 이름은 `_setup_loader_env`가 정한 값이다.
pub fn env_for(book: &Path, comics: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("TM_BOOK_DIR".to_owned(), path_str(book));
    env.insert("TM_COMICS_DIR".to_owned(), path_str(comics));
    env.insert("TM_ES_BOOK_INDEX".to_owned(), "test_books".to_owned());
    env.insert("TM_ES_COMICS_INDEX".to_owned(), "test_comics".to_owned());
    env
}

// ---- 헬퍼 ----

pub fn hwp_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/books/_hwp")
}

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `main()` 실행. (종료 코드, 표준 출력)
pub fn run_main(args: &[&str], env: &HashMap<String, String>, store: &FakeStore) -> (i32, String) {
    run_main_with(args, env, store, &InProcess)
}

pub fn run_main_with(args: &[&str], env: &HashMap<String, String>, store: &FakeStore, runner: &(dyn StageRunner + Sync)) -> (i32, String) {
    let args = to_args(args);
    let get = |k: &str| env.get(k).cloned();
    let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { if store.ping_ok { Ok(Box::new(store.clone())) } else { Err("ES ping 실패".to_owned()) } };
    let world = World { env: &get, connect: &connect, runner };
    let mut out = Vec::new();
    let code = run("loader", &args, &world, &mut out);
    (code, String::from_utf8(out).unwrap())
}

pub fn run_main_with_hooks(args: &[&str], env: &HashMap<String, String>, store: &FakeStore, hooks: &RunHooks) -> (i32, String) {
    let args = to_args(args);
    let get = |k: &str| env.get(k).cloned();
    let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { Ok(Box::new(store.clone())) };
    let world = World { env: &get, connect: &connect, runner: &InProcess };
    let mut out = Vec::new();
    let code = run_with_hooks("loader", &args, &world, &mut out, hooks);
    (code, String::from_utf8(out).unwrap())
}

/// `_setup_loader_env`: 책 prefix는 root, 만화 prefix는 root/comics.
pub fn setup_loader_env(root: &Path) -> HashMap<String, String> {
    env_for(root, &root.join("comics"))
}

// ---- 공통 헬퍼 ------------------------------------------------------------------------------

/// tests/conftest.py가 정하는 TM_BOOK_DIR / TM_COMICS_DIR 값.
pub fn tests_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

// ---- 공통 헬퍼 ----

pub fn tmp() -> TempDir {
    tempfile::tempdir().unwrap()
}

pub fn to_args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

pub fn write(path: &Path, data: impl AsRef<[u8]>) {
    fs::write(path, data).unwrap();
}

pub fn write_blank_pdf(path: &Path) {
    fs::write(path, blank_pdf()).unwrap();
}
