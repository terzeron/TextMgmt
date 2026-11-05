//! `backend::book_manager::book_updater` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;

#[path = "../common/book_fixtures.rs"]
mod fx;
use crate::common::shared_fixtures::{LoaderReadError, book_adder, file_reader};
use crate::common::shared_fixtures::{as_status, block_on, deps, make_doc, make_manager, split, tmp_dir};
use backend::api_router::BookManagerApi;
use backend::book_manager::{AddBookError, BookManager};
use common::es_fake::FakeEsManager;
use common::faulty_fs::{FaultyFs, fail_on_nth_call, other_err};
use serde_json::json;
use std::fs;

// ---- coverage: index_single_file unsupported format ----

#[test]
fn test_index_single_file_unsupported() {
    // Loader.read_file returns falsy
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    // Loader::read_file이 빈 dict를 돌려주게 한다.
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_file_reader(file_reader(|_path| Ok(json!({})))));
    let f = tmp_path.join("test.xyz");
    fs::write(&f, "data").unwrap();

    let (result, err) = split(block_on(manager.index_single_file("test.xyz")));
    assert!(result.is_none());
    assert!(err.expect("err").contains("지원하지 않는"));
}

// ---- coverage: update_book IOError and force overwrite ----

#[test]
fn test_update_book_ioerror_on_rename() {
    // IOError during file rename
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

    let (result, err) = as_status(block_on(BookManagerApi::update_book(&manager, 1, "A", "dst", "U", &tmp_path.join("A").join("dst.txt"), "txt", false)));
    assert_eq!(result, "Error");
    assert!(err.expect("err").contains("can't move"));
}

#[test]
fn test_update_book_force_overwrite() {
    // force overwrite when destination exists
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

    let (result, _err) = as_status(block_on(BookManagerApi::update_book(&manager, 1, "A", "dst", "U", &dst, "txt", true)));
    assert_eq!(result, "Ok");
    assert!(!src.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "source");
}

// ── update_book: 책이 없을 때 Error 반환 ─────────────────────────

#[test]
fn test_update_book_no_such_book_returns_error() {
    // ES 에서 책을 찾지 못하면 ('Error', '…no such a book') 을 반환한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let new_path = tmp_path.join("A").join("new.txt");

    let (status, msg) = as_status(block_on(BookManagerApi::update_book(&manager, 999, "A", "Title", "Author", &new_path, "txt", false)));
    assert_eq!(status, "Error");
    assert!(msg.unwrap_or_default().contains("no such a book"));
}

#[test]
fn test_update_book_reindex_and_rollback_branches() {
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
    let (st1, msg1) = as_status(block_on(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st1, "Error");
    assert!(msg1.expect("msg1").contains("파일 롤백도 실패"));

    // 2. 1188-1190: rollback success, restore old doc raises exception
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager =
        BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(empty_reader()).with_book_adder(book_adder(|| Err(AddBookError::Exception("restore old doc exception".to_string())))));

    let (st2, msg2) = as_status(block_on(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st2, "Error");
    assert!(msg2.expect("msg2").contains("기존 ES 문서 복구 실패"));

    // 3. 1192-1193: restore old doc returns error
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(empty_reader()).with_book_adder(book_adder(|| Err(AddBookError::Rejected("restore error message".to_string())))));

    let (st3, msg3) = as_status(block_on(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st3, "Error");
    assert!(msg3.expect("msg3").contains("restore error message"));

    // 4. 1204-1206: exception during reindex, rollback succeeds, restore old doc raises exception
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager =
        BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Err(AddBookError::Exception("restore old doc exception".to_string())))));

    let (st4, msg4) = as_status(block_on(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st4, "Error");
    assert!(msg4.expect("msg4").contains("기존 ES 문서 복구 실패"));

    // 5. 1208-1210: exception during reindex, rollback succeeds, restore old doc returns error
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Err(AddBookError::Rejected("restore error message".to_string())))));

    let (st5, msg5) = as_status(block_on(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st5, "Error");
    assert!(msg5.expect("msg5").contains("restore error message"));

    // 6. 1211: exception during reindex, rollback succeeds, restore old doc succeeds!
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Ok(1))));
    let (st6, msg6) = as_status(block_on(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st6, "Error");
    assert!(msg6.expect("msg6").contains("기존 ES 문서 복구 완료"));
}
