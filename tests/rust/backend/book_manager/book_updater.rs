//! `backend::book_manager::book_updater` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;

#[path = "../common/book_fixtures.rs"]
mod fx;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use backend::book_manager::{AddBookError, BookAdder, BookManager, BookManagerDeps, FileReader, ReadFileError};
use backend::main::{BookManagerApi};
use common::es_fake::{Docs, FakeEsManager};
use common::faulty_fs::{fail_on_nth_call, FaultyFs, other_err};
use serde_json::{json, Value};
use tempfile::TempDir;

/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

// ---------------------------------------------------------------------------
// 공용 헬퍼 (전반부 정의를 복사)
// ---------------------------------------------------------------------------

/// pytest `tmp_path`에 대응한다. 반환값이 drop되면 디렉토리가 지워진다.
fn tmp_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// `asyncio.run(coro)`에 대응한다. 동기 테스트 안에서 future를 끝까지 돌린다.
fn asyncio_runner<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut)
}

/// 기본 구성의 `BookManagerDeps`. `with_*`를 더 붙인 뒤 `BookManager::with_deps`로 만든다.
fn deps(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManagerDeps {
    common::setup_env();
    BookManagerDeps::new(tmp_path.to_path_buf(), es.clone())
}

// ---- seam 어댑터 (공용 모듈에 아직 없는 것만 이 파일에 둔다) ----

/// 구 `utils.parser_timeout.ParserTimeout`. `FileReader`는 오류를 돌려주지 못해(빈 묶음이 곧 실패다) 이 값은 어댑터가 빈 묶음으로 바꾼다.
#[derive(Debug)]
#[allow(dead_code)]
struct ParserTimeout {
    message: String,
}

/// 구 `Loader.read_file`이 던지는 오류.
#[derive(Debug)]
#[allow(dead_code)]
enum LoaderReadError {
    Timeout(ParserTimeout),
    Other(String),
}

type ReadFileFn = dyn Fn(&Path) -> Result<Value, LoaderReadError> + Send + Sync;

// TODO(common): 클로저를 `FileReader`로 감싸는 어댑터. 문서 3.7의 `fx::fetcher`처럼 `common::fx`에 두면 된다.
struct FnFileReader(Box<ReadFileFn>);

impl FileReader for FnFileReader {
    /// 객체 `{"<book_id>": doc}`를 문서 묶음으로 바꾼다. 오류와 객체가 아닌 값은 빈 묶음이다.
    fn read_file(&self, path: &Path, _path_prefix: Option<&Path>) -> Docs {
        match (self.0)(path) {
            Ok(Value::Object(map)) => map.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect(),
            _ => Docs::new(),
        }
    }

    fn try_read_file(&self, path: &Path, path_prefix: Option<&Path>, _skip_text: bool) -> Result<Docs, ReadFileError> {
        match (self.0)(path) {
            Err(LoaderReadError::Timeout(timeout)) => Err(ReadFileError::Timeout(timeout.message)),
            Err(LoaderReadError::Other(message)) => Err(ReadFileError::Other(message)),
            Ok(_) => Ok(self.read_file(path, path_prefix)),
        }
    }
}

/// 구 `loader_read_file_override(closure)`.
fn file_reader(f: impl Fn(&Path) -> Result<Value, LoaderReadError> + Send + Sync + 'static) -> Arc<dyn FileReader> {
    Arc::new(FnFileReader(Box::new(f)))
}

/// `Result<(), _>`를 Python 원본의 `("Ok" | "Error", 메시지)`로 펼친다.
fn as_status(result: Result<(), String>) -> (String, Option<String>) {
    match result {
        Ok(()) => ("Ok".to_string(), None),
        Err(error) => ("Error".to_string(), Some(error)),
    }
}

fn make_doc(rel_path: &str, file_type: &str) -> Value {
    json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
}

fn make_manager(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManager {
    BookManager::with_deps(deps(tmp_path, es))
}

type AddBookFn = dyn Fn() -> Result<i64, AddBookError> + Send + Sync;

// TODO(common): 클로저를 `BookAdder`로 감싸는 어댑터.
struct FnBookAdder(Box<AddBookFn>);

impl BookAdder for FnBookAdder {
    fn add_book(&self) -> Result<i64, AddBookError> {
        (self.0)()
    }
}

/// 구 `manager.override_add_book(closure)`.
fn book_adder(f: impl Fn() -> Result<i64, AddBookError> + Send + Sync + 'static) -> Arc<dyn BookAdder> {
    Arc::new(FnBookAdder(Box::new(f)))
}

// ---- coverage: index_single_file unsupported format ----

#[test]
fn test_index_single_file_unsupported() {
    // Line 1011: Loader.read_file returns falsy
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    // Loader::read_file이 빈 dict를 돌려주게 한다.
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_file_reader(file_reader(|_path| Ok(json!({})))));
    let f = tmp_path.join("test.xyz");
    fs::write(&f, "data").unwrap();

    let (result, err) = split(asyncio_runner(manager.index_single_file("test.xyz")));
    assert!(result.is_none());
    assert!(err.expect("err").contains("지원하지 않는"));
}

// ---- coverage: update_book IOError and force overwrite ----

#[test]
fn test_update_book_ioerror_on_rename() {
    // Lines 829-830: IOError during file rename
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    // 파일 이동(rename)이 항상 실패하게 한다.
    let faulty = FaultyFs::new();
    faulty.set_rename(|_from, _to| Some(other_err("disk error")));
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_fs(faulty));
    fs::create_dir(tmp_path.join("A")).unwrap();
    let src = tmp_path.join("A").join("src.txt");
    fs::write(&src, "content").unwrap();
    es.set_doc(Some(make_doc("A/src.txt", ".txt")));

    let (result, err) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "A", "dst", "U", &tmp_path.join("A").join("dst.txt"), "txt", false)));
    assert_eq!(result, "Error");
    assert!(err.expect("err").contains("can't move"));
}

#[test]
fn test_update_book_force_overwrite() {
    // Line 824: force overwrite when destination exists
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir(tmp_path.join("A")).unwrap();
    let src = tmp_path.join("A").join("src.txt");
    let dst = tmp_path.join("A").join("dst.txt");
    fs::write(&src, "source").unwrap();
    fs::write(&dst, "existing").unwrap();
    es.set_doc(Some(make_doc("A/src.txt", ".txt")));

    let (result, _err) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "A", "dst", "U", &dst, "txt", true)));
    assert_eq!(result, "Ok");
    assert!(!src.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "source");
}

// ── update_book: 책이 없을 때 Error 반환 (line 978) ─────────────────────────

#[test]
fn test_update_book_no_such_book_returns_error() {
    // ES 에서 책을 찾지 못하면 ('Error', '…no such a book') 을 반환한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let new_path = tmp_path.join("A").join("new.txt");

    let (status, msg) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 999, "A", "Title", "Author", &new_path, "txt", false)));
    assert_eq!(status, "Error");
    assert!(msg.unwrap_or_default().contains("no such a book"));
}

#[test]
fn test_update_book_reindex_and_rollback_branches() {
    // Lines 1169, 1182-1184, 1188-1190, 1192-1193, 1204-1206, 1208-1211
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let src_file = tmp_path.join("A").join("book.txt");
    fs::create_dir_all(src_file.parent().unwrap()).unwrap();
    fs::write(&src_file, "content").unwrap();

    let old_doc = make_doc("A/book.txt", ".txt");

    let update_es = FakeEsManager::new();
    update_es.set_doc(Some(old_doc));
    // 단계마다 seam이 달라 `with_*`로 매니저를 새로 만든다(구 전역 가드와 `override_add_book`의 생명주기).
    let empty_reader = || file_reader(|_path| Ok(json!({})));
    let crash_reader = || file_reader(|_path| Err(LoaderReadError::Other("read_file crashed".to_string())));

    // 1. 1169 & 1182-1184: Loader.read_file returns empty -> reindex_error, rename raises OSError on rollback (second call)
    let faulty_rename = FaultyFs::new();
    faulty_rename.set_rename(fail_on_nth_call(2, "rename rollback failed"));
    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(empty_reader()).with_fs(faulty_rename));

    let dst = tmp_path.join("B").join("book.txt");
    let (st1, msg1) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st1, "Error");
    assert!(msg1.expect("msg1").contains("파일 롤백도 실패"));

    // 2. 1188-1190: rollback success, restore old doc raises exception
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager =
        BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(empty_reader()).with_book_adder(book_adder(|| Err(AddBookError::Exception("restore old doc exception".to_string())))));

    let (st2, msg2) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st2, "Error");
    assert!(msg2.expect("msg2").contains("기존 ES 문서 복구 실패"));

    // 3. 1192-1193: restore old doc returns error
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(empty_reader()).with_book_adder(book_adder(|| Err(AddBookError::Rejected("restore error message".to_string())))));

    let (st3, msg3) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st3, "Error");
    assert!(msg3.expect("msg3").contains("restore error message"));

    // 4. 1204-1206: exception during reindex, rollback succeeds, restore old doc raises exception
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager =
        BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Err(AddBookError::Exception("restore old doc exception".to_string())))));

    let (st4, msg4) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st4, "Error");
    assert!(msg4.expect("msg4").contains("기존 ES 문서 복구 실패"));

    // 5. 1208-1210: exception during reindex, rollback succeeds, restore old doc returns error
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Err(AddBookError::Rejected("restore error message".to_string())))));

    let (st5, msg5) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st5, "Error");
    assert!(msg5.expect("msg5").contains("restore error message"));

    // 6. 1211: exception during reindex, rollback succeeds, restore old doc succeeds!
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Ok(1))));
    let (st6, msg6) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st6, "Error");
    assert!(msg6.expect("msg6").contains("기존 ES 문서 복구 완료"));
}
