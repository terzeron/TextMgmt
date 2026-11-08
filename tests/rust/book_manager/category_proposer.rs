//! `book_manager::category_proposer` 단위 테스트.

#[path = "common/mod.rs"]
mod common;

#[path = "../shared/book_fixtures.rs"]
mod fx;
use crate::common::category_apply::{ApplyOpts, apply_opts};
use crate::common::shared_fixtures::manager;
use crate::common::shared_fixtures::{allowed_set, block_on, deps, make_doc, make_manager, make_source_file, outcome, propose, split, tmp_dir};
use crate::common::shared_fixtures::{book_adder, file_reader, setup};
use book_manager::bookstore_evidence::{BookstoreLookup, LookupOutcome, LookupStatus, StoreResult, StoreRow};
use book_manager::bookstore_query_builder::BookQuery;
use book_manager::{AddBookError, BookManager, KeywordMatch, MoveAction, MoveOpts, MoveStatus, Proposal, ProposeOptions};
use classifier::{Classify, Prediction};
use common::env::EnvGuard;
use common::es_fake::FakeEsManager;
use common::faulty_fs::{FaultyFs, fail_on_nth_call, other_err};
use common::log_capture::LogCapture;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ---- 제안 생성 ----

/// 카테고리 -> 키워드 매핑.
/// 키/키워드가 문자열이 아닌 경우(예: 정수)를 만들 수 있도록 값은 `Value`다.
type Mappings = Vec<(Value, Vec<Value>)>;

fn mappings(pairs: &[(&str, &[&str])]) -> Mappings {
    pairs.iter().map(|(cat, kws)| (json!(cat), kws.iter().map(|k| json!(k)).collect())).collect()
}

/// 제안 후보의 카테고리만 뽑는다.
fn candidate_categories(proposal: &Proposal) -> Vec<String> {
    proposal.candidates.iter().map(|c| c.category.clone()).collect()
}

/// `propose_category_changes`의 인자 묶음(테스트 로컬).
struct ProposeOpts {
    use_bookstore: bool,
    use_content_meta: bool,
    on_progress: Option<Box<dyn Fn(Value) + Send + Sync>>,
}

/// 매핑은 입력 순서를 지킨 목록이다. 문자열이 아닌 카테고리는 JSON 객체 키가 될 수 없어 구현이 어차피 건너뛰므로 여기서 뺀다.
async fn propose_opts(manager: &BookManager, category: &str, mappings: &Mappings, opts: ProposeOpts) -> (Value, Option<String>) {
    let object: serde_json::Map<String, Value> = mappings.iter().filter_map(|(k, v)| Some((k.as_str()?.to_string(), Value::Array(v.clone())))).collect();
    let options = ProposeOptions {
        content_type: Some("book"),
        delay: Some(0.0),
        use_bookstore: Some(opts.use_bookstore),
        use_content_meta: Some(opts.use_content_meta),
        on_progress: opts.on_progress.as_deref(),
    };
    outcome(manager.propose_category_changes(category, Some(&Value::Object(object)), options).await)
}

fn manager_propose(manager: &BookManager, category: &str, mappings: &Mappings) -> (Value, Option<String>) {
    block_on(propose_opts(manager, category, mappings, ProposeOpts { use_bookstore: false, use_content_meta: false, on_progress: None }))
}

#[test]
fn test_classification_haystack_relative_to_error() {
    // CoverSourceError in classification_haystack
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    let outside_file = Path::new("/tmp/outside/nested/book.epub");
    let haystack = manager.classification_haystack(outside_file);
    assert!(haystack.contains("nested"));
    assert!(haystack.contains("book"));
}

#[test]
fn test_move_classified_file_keeps_both_when_content_differs() {
    // 이름만 같고 내용이 다르면 둘 다 남기고 새 파일에 괄호 번호를 붙인다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let src = tmp_path.join("A").join("book.txt");
    fs::create_dir_all(src.parent().unwrap()).unwrap();
    fs::write(&src, "새 파일 내용").unwrap();
    let existing = tmp_path.join("B").join("book.txt");
    fs::create_dir_all(existing.parent().unwrap()).unwrap();
    fs::write(&existing, "기존 파일 내용은 다르다").unwrap();

    let (result, err) = split(block_on(manager.move_classified_file(&src, "B", "book", MoveOpts { clean_existing: true, ..Default::default() })));

    assert!(err.is_none());
    assert_eq!(result.unwrap().to, "B/book (1).txt");
    assert_eq!(fs::read_to_string(&existing).unwrap(), "기존 파일 내용은 다르다");
    assert_eq!(fs::read_to_string(tmp_path.join("B").join("book (1).txt")).unwrap(), "새 파일 내용");
    assert!(!src.exists());
}

#[test]
fn test_move_classified_file_numbers_up_when_bracket_name_taken() {
    // '(1)'도 이미 있으면 '(2)'로 넘어간다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let src = tmp_path.join("A").join("book.txt");
    fs::create_dir_all(src.parent().unwrap()).unwrap();
    fs::write(&src, "세 번째").unwrap();
    fs::create_dir(tmp_path.join("B")).unwrap();
    fs::write(tmp_path.join("B").join("book.txt"), "첫 번째").unwrap();
    fs::write(tmp_path.join("B").join("book (1).txt"), "두 번째").unwrap();

    let (result, err) = split(block_on(manager.move_classified_file(&src, "B", "book", MoveOpts::default())));

    assert!(err.is_none());
    assert_eq!(result.unwrap().to, "B/book (2).txt");
    assert_eq!(fs::read_to_string(tmp_path.join("B").join("book (2).txt")).unwrap(), "세 번째");
}

#[test]
fn test_move_classified_file_same_size_different_content_keeps_both() {
    // 크기가 같아도 내용이 다르면 중복이 아니다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let src = tmp_path.join("A").join("book.txt");
    fs::create_dir_all(src.parent().unwrap()).unwrap();
    fs::write(&src, "AAAA").unwrap();
    fs::create_dir(tmp_path.join("B")).unwrap();
    fs::write(tmp_path.join("B").join("book.txt"), "BBBB").unwrap();

    let (result, err) = split(block_on(manager.move_classified_file(&src, "B", "book", MoveOpts { clean_existing: true, ..Default::default() })));

    assert!(err.is_none());
    assert_eq!(result.unwrap().to, "B/book (1).txt");
    assert_eq!(fs::read_to_string(tmp_path.join("B").join("book.txt")).unwrap(), "BBBB");
}

#[test]
fn test_move_classified_file_edge_cases() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());

    // 1. 1379-1380: old_rel_path CoverSourceError
    let outside_file = Path::new("/tmp/outside/file.txt");
    let (res, err) = split(block_on(manager.move_classified_file(outside_file, "B", "book", MoveOpts::default())));
    assert!(res.is_none());
    assert_eq!(err.as_deref(), Some("잘못된 파일 경로입니다"));

    // 2. 1385-1387: target_path resolve error
    let f1 = tmp_path.join("A").join("f1.txt");
    fs::create_dir_all(f1.parent().unwrap()).unwrap();
    fs::write(&f1, "f1").unwrap();
    // 단계마다 seam이 달라 `with_fs`로 매니저를 새로 만든다(구 전역 가드의 생명주기).
    let faulty_resolve = FaultyFs::new();
    faulty_resolve.set_canonicalize(|p| p.to_string_lossy().contains("bad_target").then(|| other_err("target path resolve error")));
    let manager_resolve = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_fs(faulty_resolve));
    let (res2, err2) = split(block_on(manager_resolve.move_classified_file(&f1, "bad_target", "book", MoveOpts::default())));
    assert!(res2.is_none());
    assert_eq!(err2.as_deref(), Some("잘못된 대상 경로입니다"));

    // 3. 1393-1394 & 1398: target exists, samefile OSError, clean_existing and dry_run
    let target_f = tmp_path.join("B").join("f1.txt");
    fs::create_dir_all(target_f.parent().unwrap()).unwrap();
    fs::write(&target_f, "f1").unwrap(); // 내용까지 같아야 중복 정리 대상이다

    let faulty_dup = FaultyFs::new();
    faulty_dup.set_same_file(|_a, _b| Some(other_err("samefile failed")));
    let manager_dup = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_fs(faulty_dup.clone()));
    let (res3, err3) = split(block_on(manager_dup.move_classified_file(&f1, "B", "book", MoveOpts { dry_run: true, clean_existing: true, ..Default::default() })));
    let res3 = res3.expect("res3");
    assert_eq!(res3.action, Some(MoveAction::DuplicateClean));
    assert_eq!(res3.status, MoveStatus::DryRun);
    assert!(err3.is_none());

    // 4. 1420-1421: clean_existing exception
    // samefile 실패는 그대로 두고 unlink(remove_file)만 더 실패시킨다.
    faulty_dup.set_remove_file(|_p| Some(other_err("delete duplicate failed")));
    let (res4, err4) = split(block_on(manager_dup.move_classified_file(&f1, "B", "book", MoveOpts { dry_run: false, clean_existing: true, ..Default::default() })));
    assert!(res4.is_none());
    assert!(err4.expect("err4").contains("중복 파일 정리 실패"));

    // 5. 1440-1441 & 1450: shutil.move OSError, search_by_id exception
    fs::remove_file(&target_f).unwrap();

    let err_es = FakeEsManager::new();
    err_es.fail("search_by_id", "es lookup error");
    // `shutil.move`는 `FsOps::rename`으로 합쳤다(파일 머리말 참고).
    let faulty_move = FaultyFs::new();
    faulty_move.set_rename(|_src, _dst| Some(other_err("cross-device link failed")));
    let manager_err = BookManager::with_deps(deps(tmp_path, &err_es).with_fs(faulty_move));
    let (res5, err5) = split(block_on(manager_err.move_classified_file(&f1, "B", "book", MoveOpts::default())));
    assert!(res5.is_none());
    assert!(err5.expect("err5").contains("파일 이동 실패"));

    // 6. 1473, 1490-1491, 1494-1499, 1503: add_book failed, file rollback failed, restore old doc failed
    let f2 = tmp_path.join("A").join("f2.txt");
    fs::write(&f2, "f2").unwrap();

    let add_err_es = FakeEsManager::new();
    add_err_es.set_doc(Some(json!({"file_path": "A/f2.txt", "title": "f2"})));
    // 앞으로 가는 이동은 성공시키고(첫 호출) 롤백 이동(두 번째 호출)만 실패시킨다.
    let faulty_rollback = FaultyFs::new();
    faulty_rollback.set_rename(fail_on_nth_call(2, "rollback move error"));
    let manager_add_err = BookManager::with_deps(deps(tmp_path, &add_err_es).with_book_adder(book_adder(|| Err(AddBookError::Rejected("ES add error".to_string())))).with_fs(faulty_rollback));

    let (res6, err6) = split(block_on(manager_add_err.move_classified_file(&f2, "B", "book", MoveOpts::default())));
    assert!(res6.is_none());
    let err6 = err6.expect("err6");
    assert!(err6.contains("ES add error"));
    assert!(err6.contains("파일 롤백 실패"));
}

#[test]
fn test_move_classified_file_is_a_noop_when_already_in_target_category() {
    // 지금 있는 카테고리를 목적지로 고르면 아무것도 하지 않고 성공으로 끝낸다.
    //
    // 관리자가 표의 드롭다운에서 현재 카테고리를 그대로 고르면 원본과 대상 경로가 같아진다.
    // 이 분기가 없으면 os.rename 이 같은 경로에 대해 조용히 성공한 뒤 ES 문서를 지웠다가
    // 같은 내용으로 다시 넣는다. 파일도 문서도 그대로여야 하고, ES 를 건드리지 않아야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let category_dir = tmp_path.join("2_science");
    fs::create_dir_all(&category_dir).unwrap();
    let book = category_dir.join("제자리 도서.txt");
    fs::write(&book, "내용").unwrap();

    let (result, err) = split(block_on(manager.move_classified_file(&book, "2_science", "book", MoveOpts::default())));

    assert!(err.is_none());
    let result = result.expect("result");
    assert_eq!(result.status, MoveStatus::Moved);
    assert_eq!(result.action, Some(MoveAction::Noop));
    assert_eq!(result.from, result.to);
    assert_eq!(result.to, "2_science/제자리 도서.txt");
    assert!(book.exists());
    let deleted_paths: Vec<String> = es.deleted_file_paths().into_iter().flat_map(|(paths, _exclude)| paths).collect();
    assert!(deleted_paths.is_empty(), "제자리인데 ES 문서를 지웠다");
}

#[test]
fn test_move_classified_file_does_not_orphan_a_hardlink() {
    // 대상에 같은 실체(하드링크)가 이미 있으면 옮기지 않고 거부한다.
    //
    // 하드링크는 경로가 달라도 samefile 이 참이다. 이동으로 처리하면 os.rename 이 원본
    // 링크를 지우지 않은 채 성공해(실측), 디스크에는 파일이 남았는데 ES 문서만 사라진
    // 고아가 만들어진다. 그 고아는 '이상 항목'으로만 드러나고 왜 생겼는지 알 수 없다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let source_dir = tmp_path.join("0_inbox");
    fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("링크 도서.txt");
    fs::write(&source, "내용").unwrap();
    let target_dir = tmp_path.join("2_science");
    fs::create_dir(&target_dir).unwrap();
    fs::hard_link(&source, target_dir.join(source.file_name().unwrap())).unwrap();

    let (result, err) = split(block_on(manager.move_classified_file(&source, "2_science", "book", MoveOpts { clean_existing: false, ..Default::default() })));

    assert!(result.is_none());
    assert!(err.expect("err").contains("대상 경로에 파일이 이미 존재합니다"));
    assert!(source.exists(), "원본 링크가 남았는데 옮긴 것으로 처리했다");
}

#[test]
fn test_move_classified_file_rejects_existing_duplicate_without_clean() {
    // 내용까지 같은 파일이 이미 있고 clean_existing이 꺼져 있으면 옮기지 않고 실패로 남긴다.
    //
    // apply_category_changes가 clean_existing=False로 이 메서드를 부를 때 거치는 경로다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let source_dir = tmp_path.join("0_inbox");
    fs::create_dir_all(&source_dir).unwrap();
    let conflict = source_dir.join("중복 도서.txt");
    fs::write(&conflict, "같은 내용").unwrap();
    let target_dir = tmp_path.join("2_science");
    fs::create_dir(&target_dir).unwrap();
    fs::write(target_dir.join(conflict.file_name().unwrap()), "같은 내용").unwrap();

    let (result, err) = split(block_on(manager.move_classified_file(&conflict, "2_science", "book", MoveOpts { clean_existing: false, ..Default::default() })));

    assert!(result.is_none());
    assert!(err.expect("err").contains("대상 경로에 파일이 이미 존재합니다"));
    assert!(conflict.exists());
}

#[test]
fn test_move_classified_file_preserves_existing_es_metadata() {
    // 관리자가 ES에서 고친 제목/저자/요약은 재색인 때도 유지된다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let source_dir = tmp_path.join("0_inbox");
    fs::create_dir_all(&source_dir).unwrap();
    let source_file = source_dir.join("수정 과학.txt");
    fs::write(&source_file, "hello").unwrap();
    let mut old_doc = make_doc("0_inbox/수정 과학.txt", "txt");
    old_doc["category"] = json!("0_inbox");
    old_doc["title"] = json!("관리자가 수정한 제목");
    old_doc["author"] = json!("관리자가 수정한 저자");
    old_doc["summary"] = json!("관리자가 유지하려는 요약");
    let es = FakeEsManager::with_doc(old_doc);
    // Loader::read_file 교체 훅. 호출되면 테스트를 실패시킨다.
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_file_reader(file_reader(|_path| panic!("existing ES metadata should be reused"))));

    let (result, err) = split(block_on(manager.move_classified_file(&source_file, "2_science", "book", MoveOpts::default())));

    assert!(err.is_none());
    assert_eq!(result.expect("result").status, MoveStatus::Moved);
    let indexed_doc = es.inserted()[0].values().next().unwrap().clone();
    assert_eq!(indexed_doc["category"], "2_science");
    assert_eq!(indexed_doc["file_path"], "2_science/수정 과학.txt");
    assert_eq!(indexed_doc["title"], "관리자가 수정한 제목");
    assert_eq!(indexed_doc["author"], "관리자가 수정한 저자");
    assert_eq!(indexed_doc["summary"], "관리자가 유지하려는 요약");
}

#[test]
fn test_move_classified_file_logs_a_failed_lookup_of_the_existing_es_document_and_still_moves() {
    let tmp = tmp_dir();
    let source_dir = tmp.path().join("0_inbox");
    fs::create_dir_all(&source_dir).unwrap();
    let source_file = source_dir.join("조회 실패.txt");
    fs::write(&source_file, "hello").unwrap();
    let es = FakeEsManager::new();
    es.fail("search_by_id", "es lookup down");
    let manager = BookManager::with_deps(deps(tmp.path(), &es).with_file_reader(file_reader(|_path| Ok(json!({"1": make_doc("2_science/조회 실패.txt", "txt")})))));
    let logs = common::log_capture::LogCapture::start(log::Level::Warn);

    let (result, err) = split(block_on(manager.move_classified_file(&source_file, "2_science", "book", MoveOpts::default())));

    assert!(err.is_none(), "{err:?}");
    assert_eq!(result.expect("result").status, MoveStatus::Moved);
    let logged = logs.text();
    assert!(logged.contains("기존 ES 문서 조회 실패 (0_inbox/조회 실패.txt)") && logged.contains("es lookup down"), "{logged}");
}

#[test]
fn test_match_category_by_keywords_skips_invalid_mapping_entries() {
    // _classify_file_to_top_category가 사라진 뒤에도 이 검증 분기들은 _match_category_by_keywords에
    // 그대로 남아 있다: 문자열이 아닌 카테고리/키워드, 안전하지 않은 카테고리 이름, 빈/중복 키워드.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let test_file = tmp_path.join("A").join("sample.txt");
    fs::create_dir_all(test_file.parent().unwrap()).unwrap();
    fs::write(&test_file, "content").unwrap();

    let mappings: Mappings = vec![
        (json!(123), vec![json!("kw")]),                                                                    // 카테고리가 문자열이 아니면 건너뛴다
        (json!("safe_target"), vec![json!(999), json!("  "), json!("dup"), json!("dup"), json!("sample")]), // 키워드가 문자열이 아니거나 비었거나 중복이면 건너뛴다
        (json!("../unsafe"), vec![json!("kw")]),                                                            // 안전하지 않은 카테고리 이름은 건너뛴다
    ];
    let KeywordMatch { category, keywords: matched, tie_reason: reason, ranked: _ranked } = manager.match_category_by_keywords(&test_file, "A", &mappings);
    assert_eq!(category.as_deref(), Some("safe_target"));
    assert!(matched.iter().any(|k| k == "sample"));
    assert!(reason.is_none());
}

#[test]
fn test_match_category_by_keywords_rejects_control_char_category() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let file_path = tmp_path.join("test.txt");
    fs::write(&file_path, "hello").unwrap();

    let KeywordMatch { category, keywords: matched, tie_reason: reason, ranked: _ranked } = manager.match_category_by_keywords(&file_path, "0_inbox", &mappings(&[("unsafe\x00cat", &["test"])]));

    assert!(category.is_none());
    assert!(matched.is_empty());
    assert!(reason.is_none());
}

#[test]
fn test_match_category_by_keywords_reports_tie() {
    // 동점이면 목적지를 고르지 않고 사유를 돌려준다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = tmp_path.join("A").join("SF음악.epub");
    fs::create_dir(target.parent().unwrap()).unwrap();
    fs::write(&target, "x").unwrap();
    let mappings = mappings(&[("3_SF", &["SF"]), ("5_음악", &["음악"])]);

    let KeywordMatch { category, keywords, tie_reason, ranked } = manager.match_category_by_keywords(&target, "A", &mappings);

    assert!(category.is_none());
    assert!(tie_reason.is_some());
    assert!(!keywords.is_empty());
    // 동점 상위 2개가 candidates 조립의 재료다(Task A) — 여기서도 같이 확인한다.
    assert_eq!(ranked.len(), 2);
    let cats: BTreeSet<&str> = ranked.iter().map(|(cat, _kw)| cat.as_str()).collect();
    assert_eq!(cats, BTreeSet::from(["3_SF", "5_음악"]));
}

#[test]
fn test_match_category_by_keywords_picks_single_best() {
    // 단독 최고점이면 그 카테고리를 고른다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = tmp_path.join("A").join("과학소설 모음.epub");
    fs::create_dir(target.parent().unwrap()).unwrap();
    fs::write(&target, "x").unwrap();
    let mappings = mappings(&[("3_SF", &["과학소설"]), ("5_음악", &["음악"])]);

    let KeywordMatch { category, keywords, tie_reason, ranked } = manager.match_category_by_keywords(&target, "A", &mappings);

    assert_eq!(category.as_deref(), Some("3_SF"));
    assert_eq!(keywords, vec!["과학소설".to_string()]);
    assert!(tie_reason.is_none());
    // 순위 1위는 채택된 카테고리와 같아야 한다(Task A candidates 조립의 전제).
    assert_eq!(ranked[0], ("3_SF".to_string(), vec!["과학소설".to_string()]));
}

#[test]
fn test_match_category_by_keywords_ignores_case() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let directory = tmp.path().join("A");
    fs::create_dir_all(&directory).unwrap();
    let target = directory.join("Straße .TXT");
    // 소문자 비교라 `ß`는 `ss`가 아니다. 대문자 키워드 `STRAßE`가 맞는다.
    fs::write(&target, "x").unwrap();

    let KeywordMatch { category, keywords: matched, tie_reason, ranked: _ } = manager.match_category_by_keywords(&target, "A", &mappings(&[("3_SF", &["STRAßE .TXT"])]));

    assert_eq!(category.as_deref(), Some("3_SF"));
    assert_eq!(matched, ["STRAßE .TXT"]);
    assert!(tie_reason.is_none());
}

#[test]
fn test_move_classified_file_target_traversal_and_restore_err() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir_all(tmp_path.join("cat")).unwrap();
    let f = tmp_path.join("cat").join("a.txt");
    fs::write(&f, "data").unwrap();

    // 1. target_path not relative to root
    let (_res1, err1) = split(block_on(manager.move_classified_file(&f, "../outside", "txt", MoveOpts::default())));
    assert_eq!(err1.as_deref(), Some("잘못된 대상 경로입니다"));

    // 2. ES reindex fails, rollback add_book raises exception
    let doc = make_doc("cat/a.txt", ".txt");
    es.set_doc(Some(doc));
    es.fail("insert", "insert boom");

    let manager = BookManager::with_deps(deps(tmp_path, &es).with_book_adder(book_adder(|| Err(AddBookError::Exception("restore add_book boom".to_string())))));
    fs::create_dir_all(tmp_path.join("target")).unwrap();
    let (_res2, err2) = split(block_on(manager.move_classified_file(&f, "target", "txt", MoveOpts::default())));
    let err2 = err2.expect("err2 is not None");
    assert!(err2.contains("기존 ES 문서 복구 실패"));
}

#[test]
fn test_propose_category_candidates_keyword_top_with_runner_up() {
    // 키워드 단독 최고점이면 1위와 2위를 후보로 낸다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "과학소설 클래식 모음.epub");
    let mappings = mappings(&[("3_SF", &["과학소설"]), ("5_음악", &["클래식"])]);

    let proposal = manager.propose_category_for_file(&target, "A", &mappings, None);

    assert_eq!(proposal.target_category.as_deref(), Some("3_SF"));
    assert_eq!(candidate_categories(&proposal), ["3_SF", "5_음악"]);
    assert_eq!(Some(proposal.candidates[0].category.as_str()), proposal.target_category.as_deref());
}

#[test]
fn test_propose_category_item_always_carries_matched_keywords() {
    // 제안 항목은 어느 경로로 나오든 matched_keywords 키를 달고 나온다.
    //
    // 화면이 '왜 이 카테고리인가'를 설명할 때 쓰는 근거다. 이 키가 조용히 빠져도
    // 다른 테스트는 전부 통과하므로(실제로 한 번 그렇게 빠뜨렸다) 여기서 직접 잠근다.
    // 키워드가 맞은 경로는 맞은 키워드가 담기고, 아무것도 못 맞힌 경로는 빈 배열이다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let matched = make_source_file(tmp_path, "A", "과학소설 모음.epub");
    let unmatched = make_source_file(tmp_path, "A", "제목 없음.epub");

    let hit = manager.propose_category_for_file(&matched, "A", &mappings(&[("3_SF", &["과학소설"])]), None);
    let miss = manager.propose_category_for_file(&unmatched, "A", &mappings(&[("3_SF", &["과학소설"])]), None);

    assert_eq!(hit.matched_keywords, ["과학소설"]);
    assert!(miss.matched_keywords.is_empty());
}

#[test]
fn test_propose_category_candidates_keyword_tie() {
    // 키워드 수가 같으면 점수도 같다. 동점은 카테고리 이름 오름차순으로 1등을 정하고 둘 다 후보로 낸다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "SF음악.epub");
    let mappings = mappings(&[("3_SF", &["SF"]), ("5_음악", &["음악"])]);

    let proposal = manager.propose_category_for_file(&target, "A", &mappings, None);

    assert_eq!(proposal.target_category.as_deref(), Some("3_SF"));
    assert_eq!(candidate_categories(&proposal), ["3_SF", "5_음악"]);
    assert_eq!(proposal.candidates[0].score, proposal.candidates[1].score);
}

#[test]
fn test_propose_category_candidates_capped_at_two_even_with_more_contenders() {
    // 경합이 3개 이상이어도 후보는 최대 2개만 보여준다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "과학 음악 역사.epub");
    let mappings = mappings(&[("3_SF", &["과학"]), ("5_음악", &["음악"]), ("7_역사", &["역사"])]);

    let proposal = manager.propose_category_for_file(&target, "A", &mappings, None);

    assert!(proposal.candidates.len() <= 2);
}

// ---- 근거별 가중치 합산 ----

/// 순위를 고정해 돌려주는 모델 대역.
struct FixedModel(Vec<(&'static str, f64)>);

impl Classify for FixedModel {
    fn classify_document(&self, _doc: &Value) -> Result<Prediction, String> {
        Ok(Prediction { ranked: self.0.iter().map(|(c, p)| (c.to_string(), *p)).collect() })
    }
}

/// 파일의 inode로 ES 문서를 넣는다. 문서 키는 inode다.
fn index_doc(es: &FakeEsManager, file: &Path, doc: Value) {
    es.put_doc(book_manager::category_proposer::book_id_for_path(file).unwrap(), doc);
}

fn make_dirs(root: &Path, names: &[&str]) {
    for name in names {
        fs::create_dir_all(root.join(name)).unwrap();
    }
}

fn candidate_sources(proposal: &Proposal) -> Vec<String> {
    proposal.candidates.iter().map(|c| c.source.clone()).collect()
}

#[test]
fn proposal_top_two_follow_combined_score() {
    // 있는 근거 가중치 합 = ISBN 0.45 + 키워드 0.15 + 모델 0.10 = 0.70.
    // B = (0.45·1.0 + 0.10·0.1)/0.70, C = 0.15/0.70, D = 0.10·0.9/0.70.
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    make_dirs(tmp.path(), &["B", "C", "D"]);
    let book = make_source_file(tmp.path(), "A", "검마법.epub");
    index_doc(&es, &book, json!({"title": "검과 마법", "isbn": "111", "file_path": "A/검마법.epub"}));
    es.put_doc(999_999_001, json!({"isbn": "111", "file_path": "B/사본.epub"}));
    let model = FixedModel(vec![("D", 0.9), ("B", 0.1)]);

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[("C", &["검마법"])]), Some(&model));

    assert_eq!(proposal.target_category.as_deref(), Some("B"));
    assert_eq!(candidate_categories(&proposal), ["B", "C"]);
    assert_eq!(candidate_sources(&proposal), ["isbn", "keyword"]);
    assert!((proposal.score.unwrap() - 0.46 / 0.70).abs() < 1e-9, "{:?}", proposal.score);
    assert_eq!(proposal.candidates[0].score, proposal.score.unwrap());
    assert_eq!(proposal.model_category.as_deref(), Some("D"));
    assert!(proposal.classified);
}

#[test]
fn isbn_match_in_current_category_keeps_book_in_place() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let book = make_source_file(tmp.path(), "A", "책.epub");
    index_doc(&es, &book, json!({"title": "책", "isbn": "222", "file_path": "A/책.epub"}));
    es.put_doc(999_999_002, json!({"isbn": "222", "file_path": "A/다른 판.epub"}));

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[]), None);

    assert_eq!(proposal.target_category.as_deref(), Some("A"));
}

#[test]
fn own_inode_is_excluded_from_isbn_matches() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let book = make_source_file(tmp.path(), "A", "책.epub");
    index_doc(&es, &book, json!({"title": "책", "isbn": "333", "file_path": "A/책.epub"}));

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[]), None);

    assert!(proposal.target_category.is_none());
    assert!(proposal.candidates.is_empty());
    assert!(proposal.classified);
}

#[test]
fn isbn_matches_at_the_limit_are_ignored_even_after_dropping_own_inode() {
    // 상한은 자기 문서를 빼기 전의 건수로 본다. 빼고 세면 50건이 49건이 되어 상한을 빠져나간다.
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let book = make_source_file(tmp.path(), "A", "책.epub");
    index_doc(&es, &book, json!({"title": "책", "isbn": "444", "file_path": "A/책.epub"}));
    for i in 0..(search_manager::ISBN_MATCH_LIMIT as i64 - 1) {
        es.put_doc(999_000_000 + i, json!({"isbn": "444", "file_path": format!("B/{i}.epub")}));
    }

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[]), None);

    assert!(proposal.target_category.is_none(), "{proposal:?}");
}

#[test]
fn unindexed_file_uses_keyword_only() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    make_dirs(tmp.path(), &["C", "D"]);
    let book = make_source_file(tmp.path(), "A", "검마법.epub");
    let model = FixedModel(vec![("D", 1.0)]);

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[("C", &["검마법"])]), Some(&model));

    assert_eq!(candidate_categories(&proposal), ["C"]);
    assert_eq!(proposal.score, Some(1.0));
    assert!(proposal.model_category.is_none());
}

#[test]
fn model_candidates_without_directory_are_dropped() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    make_dirs(tmp.path(), &["D"]);
    let book = make_source_file(tmp.path(), "A", "책.epub");
    index_doc(&es, &book, json!({"title": "검과 마법", "file_path": "A/책.epub"}));
    let model = FixedModel(vec![("없는곳", 0.7), ("D", 0.3)]);

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[]), Some(&model));

    assert_eq!(candidate_categories(&proposal), ["D"]);
    // 남은 카테고리끼리 합 1로 다시 정규화한다.
    assert!((proposal.score.unwrap() - 1.0).abs() < 1e-9);
    assert_eq!(proposal.model_category.as_deref(), Some("D"));
}

// ---- 서점 근거 ----

/// 정해 둔 결과를 돌려주고 호출 수를 센다.
struct FixedLookup {
    results: Vec<StoreResult>,
    network_used: bool,
    calls: Mutex<Vec<(i64, BookQuery)>>,
}

impl FixedLookup {
    fn new(status: LookupStatus, category: &str) -> Self {
        let results =
            ["yes24", "aladin", "kyobo"].iter().map(|store| StoreResult { store: store.to_string(), status, rows: vec![StoreRow { category: category.to_string(), ..StoreRow::default() }] }).collect();
        FixedLookup { results, network_used: true, calls: Mutex::new(Vec::new()) }
    }
}

impl BookstoreLookup for FixedLookup {
    fn lookup(&self, inode: i64, query: &BookQuery) -> LookupOutcome {
        self.calls.lock().unwrap().push((inode, query.clone()));
        LookupOutcome { results: self.results.clone(), network_used: self.network_used }
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn three_agreeing_stores_make_the_bookstore_the_top_evidence() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    make_dirs(tmp.path(), &["3_SF", "5_음악"]);
    let book = make_source_file(tmp.path(), "A", "[홍길동] 검과 마법 1권.epub");
    index_doc(&es, &book, json!({"title": "검과 마법", "isbn": "0-306-40615-2", "file_path": "A/[홍길동] 검과 마법 1권.epub"}));
    let lookup = FixedLookup::new(LookupStatus::Found, "소설/시/희곡 > SF > 한국SF");

    let (proposal, network_used) = manager.propose_category_for_file_with(&book, "A", &mappings(&[]), None, Some(&lookup), &names(&["3_SF", "5_음악", "A"]));

    assert!(network_used);
    assert_eq!(proposal.target_category.as_deref(), Some("3_SF"));
    assert_eq!(proposal.score, Some(1.0));
    assert_eq!(candidate_sources(&proposal), ["bookstore"]);
    assert_eq!(proposal.bookstore_votes.iter().filter(|v| v.category.as_deref() == Some("3_SF")).count(), 3);
    let calls = lookup.calls.lock().unwrap();
    assert_eq!(calls[0].1, BookQuery { isbn13: Some("9780306406157".to_string()), title: "검과 마법".to_string(), author: "홍길동".to_string() });
}

#[test]
fn all_blocked_stores_leave_the_score_to_other_evidence() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    make_dirs(tmp.path(), &["3_SF", "5_음악"]);
    let book = make_source_file(tmp.path(), "A", "음악책.epub");
    let lookup = FixedLookup::new(LookupStatus::Blocked, "SF > 한국SF");

    let (proposal, _) = manager.propose_category_for_file_with(&book, "A", &mappings(&[("5_음악", &["음악책"])]), None, Some(&lookup), &names(&["3_SF", "5_음악", "A"]));

    assert_eq!(candidate_categories(&proposal), ["5_음악"]);
    assert_eq!(proposal.score, Some(1.0));
    assert!(proposal.bookstore_votes.iter().all(|v| v.category.is_none() && v.abstain.as_deref() == Some("blocked")));
}

fn manager_with_lookup(tmp: &Path, es: &Arc<FakeEsManager>, lookup: Arc<FixedLookup>) -> BookManager {
    let factory_lookup = lookup.clone();
    BookManager::with_deps(deps(tmp, es).with_bookstore_lookup_factory(Arc::new(move || Some(factory_lookup.clone() as Arc<dyn BookstoreLookup>))))
}

fn propose_with(manager: &BookManager, content_type: &str, use_bookstore: bool) {
    let options = ProposeOptions { content_type: Some(content_type), delay: Some(0.0), use_bookstore: Some(use_bookstore), ..ProposeOptions::default() };
    let (_, error) = outcome(block_on(manager.propose_category_changes("A", None, options)));
    assert!(error.is_none(), "{error:?}");
}

#[test]
fn proposals_query_bookstores_only_for_books_with_the_option_on() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    make_dirs(tmp.path(), &["3_SF"]);
    make_source_file(tmp.path(), "A", "책.epub");
    let lookup = Arc::new(FixedLookup::new(LookupStatus::Found, "SF > 한국SF"));
    let manager = manager_with_lookup(tmp.path(), &es, lookup.clone());

    propose_with(&manager, "comic", true);
    propose_with(&manager, "book", false);
    assert_eq!(lookup.calls.lock().unwrap().len(), 0);

    propose_with(&manager, "book", true);
    assert_eq!(lookup.calls.lock().unwrap().len(), 1);
}

// ---- 0_ 카테고리는 추천 대상이 아니다 ----

#[test]
fn zero_prefixed_model_candidates_are_not_recommended() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    make_dirs(tmp.path(), &["0_보관", "D"]);
    let book = make_source_file(tmp.path(), "A", "책.epub");
    index_doc(&es, &book, json!({"title": "검과 마법", "file_path": "A/책.epub"}));
    let model = FixedModel(vec![("0_보관", 0.7), ("D", 0.3)]);

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[]), Some(&model));

    assert_eq!(candidate_categories(&proposal), ["D"]);
    assert!((proposal.score.unwrap() - 1.0).abs() < 1e-9);
    assert_eq!(proposal.model_category.as_deref(), Some("D"));
}

#[test]
fn zero_prefixed_keyword_categories_are_not_recommended() {
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    let book = make_source_file(tmp.path(), "A", "과학소설 클래식 모음.epub");
    let mappings = mappings(&[("0_작업", &["과학소설"]), ("5_음악", &["클래식"])]);

    let proposal = manager.propose_category_for_file(&book, "A", &mappings, None);

    assert_eq!(candidate_categories(&proposal), ["5_음악"]);
    assert_eq!(proposal.score, Some(1.0));
    assert_eq!(proposal.matched_keywords, ["클래식"]);
}

#[test]
fn zero_prefixed_isbn_directories_are_not_recommended() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let book = make_source_file(tmp.path(), "A", "책.epub");
    index_doc(&es, &book, json!({"title": "책", "isbn": "555", "file_path": "A/책.epub"}));
    es.put_doc(999_999_005, json!({"isbn": "555", "file_path": "0_보관/사본.epub"}));
    es.put_doc(999_999_006, json!({"isbn": "555", "file_path": "B/사본.epub"}));

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[]), None);

    assert_eq!(candidate_categories(&proposal), ["B"]);
    assert_eq!(proposal.score, Some(1.0));
}

#[test]
fn empty_summary_and_title_skip_model() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    make_dirs(tmp.path(), &["D"]);
    let book = make_source_file(tmp.path(), "A", "책.epub");
    index_doc(&es, &book, json!({"title": "  ", "summary": "", "author": "누군가", "file_path": "A/책.epub"}));
    let model = FixedModel(vec![("D", 1.0)]);

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[]), Some(&model));

    assert!(proposal.target_category.is_none());
    assert!(proposal.model_category.is_none());
}

#[test]
fn es_lookup_failure_falls_back_to_keywords_and_is_noted() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    es.on_search_by_id(|_| Err(search_manager::EsError::Transport("연결 끊김".into())));
    let manager = make_manager(tmp.path(), &es);
    make_dirs(tmp.path(), &["C"]);
    let book = make_source_file(tmp.path(), "A", "검마법.epub");

    let proposal = manager.propose_category_for_file(&book, "A", &mappings(&[("C", &["검마법"])]), Some(&FixedModel(vec![("C", 1.0)])));

    assert_eq!(proposal.target_category.as_deref(), Some("C"));
    assert_eq!(candidate_sources(&proposal), ["keyword"]);
    assert!(proposal.reason.contains("ES 조회 실패"), "{}", proposal.reason);
}

#[test]
fn no_model_factory_returns_none_and_proposal_still_runs() {
    // 팩토리는 제안 한 번에 한 번만 부른다. None이면 모델 없이 키워드로 진행한다.
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let manager = BookManager::with_deps(deps(tmp.path(), &es).with_classifier_factory(Arc::new(move || {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        None
    })));
    make_dirs(tmp.path(), &["C"]);
    make_source_file(tmp.path(), "A", "검마법 1.epub");
    make_source_file(tmp.path(), "A", "검마법 2.epub");

    let (result, error) = block_on(propose_opts(&manager, "A", &mappings(&[("C", &["검마법"])]), ProposeOpts { use_bookstore: false, use_content_meta: true, on_progress: None }));

    assert!(error.is_none());
    assert!(result["items"].as_array().unwrap().iter().all(|i| i["target_category"] == "C"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn placeholder_rows_are_not_classified_and_finished_rows_are() {
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    make_source_file(tmp.path(), "A", "가.epub");

    let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = progress.clone();
    block_on(propose_opts(&manager, "A", &mappings(&[]), ProposeOpts { use_bookstore: false, use_content_meta: false, on_progress: Some(Box::new(move |call| sink.lock().unwrap().push(call))) }));

    let progress = progress.lock().unwrap();
    let placeholder = progress.iter().find_map(|call| call.get("new_items")).unwrap();
    let finished = progress.iter().find_map(|call| call.get("updated_items")).unwrap();
    assert_eq!(placeholder[0]["classified"], false);
    assert_eq!(finished[0]["classified"], true);
}

#[test]
fn old_payload_with_grade_still_deserializes() {
    let old = json!({"target_category": "3_SF", "grade": "certain", "confidence": 0.9, "expected_accuracy": 0.8, "source": "model", "candidates": [{"category": "3_SF", "source": "model", "detail": "모델 판정"}]});

    let proposal: Proposal = serde_json::from_value(old).unwrap();

    assert_eq!(proposal.target_category.as_deref(), Some("3_SF"));
    assert!(!proposal.classified);
    assert!(proposal.score.is_none());
    assert_eq!(proposal.candidates[0].score, 0.0);
}

#[test]
fn test_propose_category_changes_lists_items_without_moving() {
    // 제안만 만들고 파일은 그대로 둔다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "과학소설 모음.epub");

    let (result, error) = manager_propose(&manager, "A", &mappings(&[("3_SF", &["과학소설"])]));

    assert!(error.is_none());
    assert_eq!(result["total_count"], 1);
    assert_eq!(result["items"][0]["file_path"], "A/과학소설 모음.epub");
    assert!(book.is_file());
}

#[test]
fn test_propose_category_changes_preserves_requested_content_type() {
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    make_source_file(tmp.path(), "A", "과학소설 모음.epub");
    let (result, error) = outcome(block_on(manager.propose_category_changes(
        "A",
        Some(&json!({})),
        ProposeOptions { content_type: Some("comic"), delay: Some(0.0), use_bookstore: Some(false), use_content_meta: Some(false), on_progress: None },
    )));
    assert!(error.is_none());
    assert_eq!(result["content_type"], "comic");
}

#[test]
fn test_propose_category_changes_progress_reports_new_items_only() {
    // 진행 보고의 new_items는 이번 틱에서 새로 생긴 항목만 담는다(누적이 아니다).
    //
    // 누적 목록(items)을 매번 통째로 실으면 보고 크기가 책 수의 제곱으로 커진다(과거
    // JSON 상태 파일 방식에서 실측된 문제 — 79,589권 카테고리에서 기록만 약 33시간).
    // new_items로 이번 틱의 델타만 보내야 한 틱의 비용이 책 수와 무관해진다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "가.epub");
    make_source_file(tmp_path, "A", "나.epub");

    let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = progress.clone();
    let (result, error) = block_on(propose_opts(
        &manager,
        "A",
        &mappings(&[("3_SF", &["과학소설"])]),
        ProposeOpts { use_bookstore: false, use_content_meta: false, on_progress: Some(Box::new(move |call| sink.lock().unwrap().push(call))) },
    ));

    assert!(error.is_none());
    let progress = progress.lock().unwrap();
    // 첫 보고는 대상 파일 전체의 자리(이름만)를 한 번에 깔고, 이후 보고는 분류가 끝난
    // 항목만 하나씩 채운다. 어느 쪽도 누적 목록을 통째로 싣지 않는다.
    assert!(progress.iter().all(|call| call.get("items").is_none()));
    let updates: Vec<&Value> = progress.iter().filter(|call| call.get("updated_items").is_some()).collect();
    assert_eq!(updates.len(), 2);
    assert!(updates.iter().all(|call| call["updated_items"].as_array().unwrap().len() == 1));
    let updated_paths: Vec<&Value> = updates.iter().map(|call| &call["updated_items"][0]["file_path"]).collect();
    let result_paths: Vec<&Value> = result["items"].as_array().unwrap().iter().map(|item| &item["file_path"]).collect();
    assert_eq!(updated_paths, result_paths);
}

#[test]
fn test_propose_category_changes_lists_every_target_before_classifying() {
    // 분류를 시작하기 전에 대상 파일 전체가 이름만이라도 먼저 보고된다.
    //
    // 한 권에 서점 조회까지 하면 수 초가 든다. 다 끝난 뒤에 목록을 주면 관리자는 그동안
    // 무엇이 대상인지조차 알 수 없다. 첫 보고에 전체 목록이 실려야 화면이 이름부터
    // 채우고 분류 결과를 뒤이어 메울 수 있다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "가.epub");
    make_source_file(tmp_path, "A", "나.epub");

    let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = progress.clone();
    block_on(propose_opts(&manager, "A", &mappings(&[]), ProposeOpts { use_bookstore: false, use_content_meta: false, on_progress: Some(Box::new(move |call| sink.lock().unwrap().push(call))) }));

    let progress = progress.lock().unwrap();
    let first = &progress[0];
    assert_eq!(first["processed_count"], 0, "분류가 시작되기 전에 나와야 한다");
    assert_eq!(first["total_count"], 2);
    let mut paths: Vec<&str> = first["new_items"].as_array().unwrap().iter().map(|item| item["file_path"].as_str().unwrap()).collect();
    paths.sort();
    assert_eq!(paths, ["A/가.epub", "A/나.epub"]);
    // 아직 분류 전이라 목적지가 없고 classified도 false다. 화면은 이것으로 "분류 중"을 구분한다.
    assert!(first["new_items"].as_array().unwrap().iter().all(|item| item["classified"] == false && item["target_category"].is_null()));
    assert!(first["new_items"].as_array().unwrap().iter().all(|item| item["title"].as_str().is_some_and(|t| !t.is_empty())));
}

#[test]
fn test_propose_category_changes_result_items_still_has_everything() {
    // 함수 반환값 result["items"]는 진행 보고와 무관하게 여전히 전체 항목을 담는다.
    //
    // 진행 보고는 델타(new_items)만 실어도, 최종 저장은 호출자가 반환값을 보고 하므로
    // 반환값 자체는 지금처럼 전체 목록이어야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    for i in 0..3 {
        make_source_file(tmp_path, "A", &format!("책{i}.epub"));
    }

    let (result, error) = manager_propose(&manager, "A", &mappings(&[("3_SF", &["과학소설"])]));

    assert!(error.is_none());
    assert_eq!(result["items"].as_array().unwrap().len(), 3);
}

#[test]
fn test_apply_category_changes_rejects_unknown_file() {
    // 제안에 없던 파일은 옮기지 않는다. 화면이 보낸 경로를 그대로 믿으면 안 된다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "book.epub");

    let (result, error) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "3_SF"})], ApplyOpts::new(BTreeSet::new())));

    assert!(error.is_none());
    assert_eq!(result["applied_count"], 0);
    assert_eq!(result["results"][0]["apply_error"], "제안 목록에 없는 파일입니다");
}

#[test]
fn test_apply_category_changes_rejects_unsafe_target() {
    // 사용자가 고른 목적지도 검증한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "book.epub");
    let allowed = allowed_set(&["A/book.epub"]);

    let (result, _error) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "../밖"})], ApplyOpts::new(allowed)));

    assert_eq!(result["applied_count"], 0);
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("카테고리"));
}

#[test]
fn test_apply_category_changes_uses_user_chosen_target() {
    // 사용자가 제안과 다른 목적지를 고르면 그쪽으로 옮긴다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "book.epub");
    let allowed = allowed_set(&["A/book.epub"]);

    let (result, _error) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(allowed)));

    assert_eq!(result["applied_count"], 1);
    assert!(tmp_path.join("5_음악").join("book.epub").is_file());
}

#[test]
fn test_apply_category_changes_records_missing_file() {
    // 제안을 만든 뒤 파일이 사라졌으면 실패로 남기고 나머지를 계속 처리한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("A")).unwrap();
    let allowed = allowed_set(&["A/사라진책.epub"]);

    let (result, _error) = block_on(apply_opts(&manager, &[json!({"file_path": "A/사라진책.epub", "target_category": "3_SF"})], ApplyOpts::new(allowed)));

    assert_eq!(result["failed_count"], 1);
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("파일"));
}

#[test]
fn test_apply_category_changes_rejects_top_level_but_unsafe_target() {
    // ".."는 슬래시가 없어 최상위 검사는 통과하지만 안전한 이름 검사에서 걸러야 한다.
    //
    // 기존 "../밖" 케이스는 슬래시가 있어 _is_top_level_target_category에서 이미
    // 걸러지므로 _is_safe_category_name이 실제로 호출되는지 증명하지 못한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "book.epub");
    let allowed = allowed_set(&["A/book.epub"]);

    let (result, _error) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": ".."})], ApplyOpts::new(allowed)));

    assert_eq!(result["applied_count"], 0);
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("카테고리"));
}

#[test]
fn test_apply_category_changes_rejects_symlinked_source() {
    // 제안 시점 이후 파일이 심볼릭 링크로 바뀌면 corpus 밖 파일 유출을 막는다.
    //
    // allowed_file_paths에 있던 경로라도 실제 파일이 corpus 밖을 가리키는 링크로
    // 치환됐을 수 있다. is_file()은 링크를 따라가 True를 주므로 별도로 걸러야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("A")).unwrap();
    let outside = tmp_path.parent().unwrap().join("outside_secret.epub");
    fs::write(&outside, "secret").unwrap();
    let link_path = tmp_path.join("A").join("book.epub");
    symlink(&outside, &link_path).unwrap();
    let allowed = allowed_set(&["A/book.epub"]);

    let (result, _error) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "3_SF"})], ApplyOpts::new(allowed)));

    assert_eq!(result["applied_count"], 0);
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("링크"));
    assert!(outside.is_file());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "secret");
    // 원본 테스트는 공유 임시 디렉토리에 파일을 남기므로 여기서 치운다.
    fs::remove_file(&outside).unwrap();
}

#[test]
fn test_apply_category_changes_calls_on_item_done_per_item() {
    // 항목마다 on_item_done이 file_path/apply_status/apply_error와 함께 불린다.
    //
    // 성공(moved)/실패(파일 없음) 양쪽 다 확인한다. book_manager는 DB를 모르므로
    // 이 콜백을 통해서만 호출자(라우트)가 건별로 기록할 수 있다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "ok.epub");
    let allowed = allowed_set(&["A/ok.epub", "A/missing.epub"]);
    let done_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = done_calls.clone();

    let (result, _error) = block_on(apply_opts(
        &manager,
        &[json!({"file_path": "A/ok.epub", "target_category": "5_음악"}), json!({"file_path": "A/missing.epub", "target_category": "5_음악"})],
        ApplyOpts::new(allowed).on_item_done(Box::new(move |call| sink.lock().unwrap().push(call))),
    ));

    assert_eq!(result["applied_count"], 1);
    assert_eq!(result["failed_count"], 1);
    // ok.epub은 실제 이동 직전에 "moving"으로 한 번 더 불린 뒤 "moved"로 마무리된다.
    // missing.epub은 파일이 없어 이동을 시도조차 못 하므로 moving 없이 failed 한 번뿐이다.
    let done_calls = done_calls.lock().unwrap();
    assert_eq!(done_calls.len(), 3);
    assert_eq!(done_calls[0], json!({"file_path": "A/ok.epub", "apply_status": "moving", "apply_error": null}));
    assert_eq!(done_calls[1], json!({"file_path": "A/ok.epub", "apply_status": "moved", "apply_error": null}));
    assert_eq!(done_calls[2]["file_path"], "A/missing.epub");
    assert_eq!(done_calls[2]["apply_status"], "failed");
    assert!(done_calls[2]["apply_error"].as_str().unwrap().contains("파일"));
}

#[test]
fn test_apply_category_changes_stops_before_next_item_when_should_continue_is_false() {
    // should_continue가 False를 돌려주는 순간 그 이후 항목은 아예 손대지 않는다.
    //
    // 다른 승인 작업이 이 작업을 토큰으로 대체했을 때, 진 쪽이 계속 파일을 옮기며
    // 행을 덮어쓰지 못하게 막는 장치다. 첫 항목은 정상 처리되고, 두 번째 항목
    // "앞"에서 중단되어 파일도 옮겨지지 않고 결과에도 없어야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "first.epub");
    make_source_file(tmp_path, "A", "second.epub");
    let allowed = allowed_set(&["A/first.epub", "A/second.epub"]);
    let calls = Arc::new(Mutex::new(0usize));
    let counter = calls.clone();

    let (result, _error) = block_on(apply_opts(
        &manager,
        &[json!({"file_path": "A/first.epub", "target_category": "5_음악"}), json!({"file_path": "A/second.epub", "target_category": "5_음악"})],
        ApplyOpts::new(allowed).should_continue(Box::new(move || {
            let mut n = counter.lock().unwrap();
            *n += 1;
            *n <= 1 // 첫 항목 앞에서만 True, 두 번째 항목 앞에서 False
        })),
    ));

    assert_eq!(result["total_count"], 2);
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert_eq!(result["results"][0]["file_path"], "A/first.epub");
    assert_eq!(result["applied_count"], 1);
    assert!(tmp_path.join("5_음악").join("first.epub").is_file());
    // 두 번째는 손대지 않았다 - 여전히 원래 자리에 그대로 있다
    assert!(tmp_path.join("A").join("second.epub").is_file());
    assert_eq!(*calls.lock().unwrap(), 2);
}

#[test]
fn test_apply_category_changes_retry_of_failed_row_succeeds() {
    // C1: 첫 시도에서 allowed 밖이라 거부된 행도, 재시도 때 allowed에 다시 담기면
    // 실제로 이동에 성공한다.
    //
    // 라우트의 허용 집합 계산이 apply_status == "pending"만 담던 버그를 재현한다 —
    // 거부됐던(failed) 행을 재시도해도 허용 집합에 안 들어가면 여기(book_manager)까지
    // 오지도 못하고 "제안 목록에 없는 파일입니다"만 영원히 반복된다. 이 테스트는 그
    // 행이 다시 allowed에 담기기만 하면 재시도가 실제로 동작함을 보인다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "book.epub");

    // 1차 시도: allowed에 없어 거부된다(버그가 있던 상태를 재현).
    let (first, _) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(BTreeSet::new())));
    assert_eq!(first["results"][0]["apply_status"], "failed");
    assert_eq!(first["results"][0]["apply_error"], "제안 목록에 없는 파일입니다");
    assert!(tmp_path.join("A").join("book.epub").is_file());

    // 재시도: failed 행이 allowed에 다시 담기면(고친 뒤 동작) 실제로 이동한다.
    let (second, _) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(allowed_set(&["A/book.epub"]))));
    assert_eq!(second["results"][0]["apply_status"], "moved");
    assert!(!tmp_path.join("A").join("book.epub").exists());
    assert!(tmp_path.join("5_음악").join("book.epub").is_file());
}

#[test]
fn test_apply_category_changes_retrying_moving_row_when_file_already_moved() {
    // I4 outcome 1: moving으로 남았지만 실제로는 이미 옮겨져 있던 행을 재시도하면,
    // 원본을 못 찾아 이중 이동 없이 안전하게 failed로 기록된다.
    //
    // apply_category_changes가 이동 직전 반드시 source 파일 존재를 다시 확인하므로,
    // moving을 pending과 똑같이 취급해도 이미 끝난 이동을 또 옮기는 사고는 안 난다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("A")).unwrap();
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    // 원본은 이미 없고(실제로는 이동이 끝남), 목적지에만 파일이 있다.
    fs::write(tmp_path.join("5_음악").join("book.epub"), "x").unwrap();

    let (result, _) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(allowed_set(&["A/book.epub"]))));

    assert_eq!(result["results"][0]["apply_status"], "failed");
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("파일을 찾을 수 없습니다"));
    // 이미 옮겨져 있던 목적지 파일은 건드리지 않는다 — 덮어쓰거나 지우지 않는다.
    assert!(tmp_path.join("5_음악").join("book.epub").is_file());
}

#[test]
fn test_apply_category_changes_retrying_moving_row_when_move_never_happened() {
    // I4 outcome 2: moving으로 남았지만 실제로는 이동이 시작되지 않았던(원본이
    // 그대로인) 행을 재시도하면, 이번에는 정상적으로 이동이 완료된다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "book.epub");

    let (result, _) = block_on(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(allowed_set(&["A/book.epub"]))));

    assert_eq!(result["results"][0]["apply_status"], "moved");
    assert!(!tmp_path.join("A").join("book.epub").exists());
    assert!(tmp_path.join("5_음악").join("book.epub").is_file());
}

mod candidate_building {
    use super::common;
    use crate::common::dyn_es::make_manager;
    use crate::common::shared_fixtures::{category_with_files, outcome, split};
    use book_manager::{MoveAction, MoveOpts, MoveStatus};
    use common::es_fake::FakeEsManager;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    // ---- 분류 후보 조립 ----

    // ---- 분류 제안 ----

    #[tokio::test]
    async fn test_propose_rejects_unsafe_category_name() {
        let tmp = TempDir::new().unwrap();
        let manager = make_manager(tmp.path(), FakeEsManager::new());
        let (result, err) = outcome(manager.propose_category_changes("../밖", None, Default::default()).await);
        assert_eq!(result, json!({}));
        assert_eq!(err.as_deref(), Some("잘못된 카테고리 경로입니다"));
    }

    // ---- 중복 판정/번호 붙이기/이동 ----

    #[test]
    fn test_is_duplicate_content_treats_unreadable_file_as_not_duplicate() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("a.txt");
        fs::write(&source, "x").unwrap();
        assert!(!book_manager::category_proposer::is_duplicate_content(&source, &tmp.path().join("없음.txt")));
    }

    #[test]
    fn test_next_numbered_path_gives_up_when_all_slots_taken() {
        // 상수 재정의 setter 대신 상한을 인자로 받는 `next_numbered_path_with`를 쓴다.
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("book.txt");
        fs::write(tmp.path().join("book (1).txt"), "x").unwrap();
        let result = book_manager::category_proposer::next_numbered_path_with(&target, 1);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_move_removes_identical_duplicate_when_clean_existing() {
        let tmp = TempDir::new().unwrap();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp.path(), es.clone());
        category_with_files(tmp.path(), "A", &["book.txt"]);
        category_with_files(tmp.path(), "B", &["book.txt"]);

        let opts = MoveOpts { clean_existing: true, source_category: Some("A".to_string()), ..Default::default() };
        let (result, err) = split(manager.move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", opts).await);

        assert_eq!(err, None);
        assert_eq!(result.unwrap().status, MoveStatus::DuplicateCleaned);
        assert!(!tmp.path().join("A").join("book.txt").exists());
        assert_eq!(es.deleted_file_paths(), vec![(vec!["A/book.txt".to_string()], Vec::<i64>::new())]);
    }

    #[tokio::test]
    async fn test_move_dry_run_reports_plain_move_without_touching_files() {
        let tmp = TempDir::new().unwrap();
        let manager = make_manager(tmp.path(), FakeEsManager::new());
        category_with_files(tmp.path(), "A", &["book.txt"]);
        fs::create_dir(tmp.path().join("B")).unwrap();

        let opts = MoveOpts { dry_run: true, ..Default::default() };
        let (result, err) = split(manager.move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", opts).await);

        assert_eq!(err, None);
        assert_eq!(result.unwrap().action, Some(MoveAction::Move));
        assert!(tmp.path().join("A").join("book.txt").exists());
    }
}

// ---------------------------------------------------------------------------
// (b) 분류기를 꽂지 않았을 때
// ---------------------------------------------------------------------------

#[test]
fn without_an_injected_classifier_proposals_use_keywords_only() {
    let s = setup(&["달빛조각사.txt", "다른책.txt"]);
    let logs = LogCapture::start(log::Level::Info);

    let m = manager(&s, |d| d);
    let result = propose(&m, ProposeOptions::default());

    assert_eq!(result["items"].as_array().unwrap().len(), 2);
    assert!(result["items"].as_array().unwrap().iter().all(|i| i["classified"] == true && i["target_category"].is_null() && i["score"].is_null()));
    assert!(logs.records().iter().all(|r| r.level != log::Level::Warn), "{}", logs.text());
}

#[test]
fn the_model_directory_variable_is_ignored() {
    let s = setup(&["달빛조각사.txt"]);
    let mut env = EnvGuard::new();
    env.set("TM_CLASSIFIER_MODEL_DIR", s.meta.to_str().unwrap());
    let m = manager(&s, |d| d);
    let result = propose(&m, ProposeOptions { use_bookstore: Some(false), ..Default::default() });
    assert!(result["items"][0]["model_category"].is_null(), "{}", result["items"][0]);
    drop(env);
}

mod classified_file_moves {
    use super::common;
    use super::common::{es_fake::FakeEsManager, faulty_fs::FaultyFs};
    use crate::common::file_reader_fakes::FnFileReader;
    use crate::common::shared_fixtures::{category_with_files, make_deps, outcome, split};
    use book_manager::{BookManager, MoveOpts};
    use common::es_fake::Docs;
    use serde_json::json;
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_apply_rejects_file_when_resolve_raises_oserror() {
        let tmp = TempDir::new().unwrap();
        category_with_files(tmp.path(), "1_A", &["a.txt"]);
        fs::create_dir(tmp.path().join("2_B")).unwrap();

        // 경로 정규화(canonicalize)만 실패시킨다. a.txt만 EIO로 실패하고 나머지는 실제 파일 시스템에 맡긴다.
        let fs_ops = FaultyFs::new();
        fs_ops.set_canonicalize(|p: &Path| if p.file_name().unwrap() == "a.txt" { Some(std::io::Error::from_raw_os_error(5)) } else { None });
        let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_fs(fs_ops));
        let allowed: HashSet<String> = ["1_A/a.txt".to_string()].into();

        let (result, _) = outcome(manager.apply_category_changes(&[json!({"file_path": "1_A/a.txt", "target_category": "2_B"})], &allowed, Default::default()).await);

        assert_eq!(result["results"][0]["apply_error"], "파일 경로가 corpus 밖입니다");
        assert!(tmp.path().join("1_A").join("a.txt").exists());
    }

    #[tokio::test]
    async fn test_move_reports_when_numbered_names_are_exhausted() {
        let tmp = TempDir::new().unwrap();
        // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 값 설정을 Deps 빌더로 옮겼다): 번호 붙이기 상한을 1로 둔다
        let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_duplicate_name_max_index(1));
        category_with_files(tmp.path(), "A", &["book.txt"]);
        category_with_files(tmp.path(), "B", &["book.txt", "book (1).txt"]);
        fs::write(tmp.path().join("A").join("book.txt"), "다른 내용입니다").unwrap();

        let (result, err) = split(manager.move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", Default::default()).await);

        assert!(result.is_none());
        assert!(err.unwrap().contains("너무 많습니다"));
    }

    #[tokio::test]
    async fn test_move_reports_duplicate_cleanup_failure() {
        let tmp = TempDir::new().unwrap();
        category_with_files(tmp.path(), "A", &["book.txt"]);
        category_with_files(tmp.path(), "B", &["book.txt"]);

        // 파일 삭제(unlink)가 항상 PermissionError로 실패하는 파일 시스템
        let fs_ops = FaultyFs::new();
        fs_ops.set_remove_file(|_p: &Path| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "삭제 거부")));
        let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_fs(fs_ops));

        let opts = MoveOpts { clean_existing: true, ..Default::default() };
        let (result, err) = split(manager.move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", opts).await);

        assert!(result.is_none());
        assert!(err.unwrap().starts_with("중복 파일 정리 실패"));
    }

    #[tokio::test]
    async fn test_move_rolls_back_when_loader_returns_no_single_document() {
        let tmp = TempDir::new().unwrap();
        // Loader.read_file이 빈 dict를 돌려주는 파일 읽기 대역
        let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_file_reader(FnFileReader::new(|_path: &Path, _prefix: Option<&Path>| Docs::new())));
        category_with_files(tmp.path(), "A", &["book.txt"]);
        fs::create_dir(tmp.path().join("B")).unwrap();

        let (result, err) = split(manager.move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", Default::default()).await);

        assert!(result.is_none());
        assert!(err.unwrap().contains("지원하지 않는 파일 형식"));
        assert!(tmp.path().join("A").join("book.txt").exists());
    }
}

#[test]
fn test_move_classified_file_falls_back_to_copy_and_remove_across_devices() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    let source = tmp_path.join("A/f1.txt");
    fs::write(&source, "content").unwrap();
    let es = FakeEsManager::new();
    let faulty = FaultyFs::new();
    faulty.set_rename(|_src, _dst| Some(std::io::Error::from(std::io::ErrorKind::CrossesDevices)));
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_fs(faulty));

    let _ = block_on(manager.move_classified_file(&source, "B", "book", MoveOpts::default()));

    assert_eq!(fs::read_to_string(tmp_path.join("B/f1.txt")).unwrap(), "content");
    assert!(!source.exists(), "복사한 뒤 원본을 지운다");
}

// ---- 적용·이동의 남은 분기 ----

#[test]
fn test_apply_category_changes_rejects_items_without_a_path_or_a_target() {
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    make_source_file(tmp.path(), "A", "book.epub");

    let items = [json!({"target_category": "3_SF"}), json!({"file_path": "A/book.epub"})];
    let (result, error) = block_on(apply_opts(&manager, &items, ApplyOpts::new(allowed_set(&["A/book.epub"]))));

    assert!(error.is_none());
    assert_eq!(result["applied_count"], 0);
    assert_eq!(result["results"][0]["apply_error"], "제안 목록에 없는 파일입니다");
    assert_eq!(result["results"][1]["apply_error"], "옮길 수 없는 카테고리입니다");
}

#[test]
fn test_iter_category_indexable_files_descends_only_when_recursive_and_skips_links() {
    let tmp = tmp_dir();
    let dir = tmp.path().join("A");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("a.txt"), "a").unwrap();
    fs::write(dir.join("sub").join("b.txt"), "b").unwrap();
    symlink(dir.join("a.txt"), dir.join("link.txt")).unwrap();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());

    let names = |recursive: bool| -> Vec<String> { manager.iter_category_indexable_files("A", recursive).iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned()).collect() };

    assert_eq!(names(false), ["a.txt"]);
    assert_eq!(names(true), ["a.txt", "sub/b.txt"]);
}

#[test]
fn test_move_classified_file_reports_an_es_failure_while_cleaning_a_duplicate() {
    let tmp = tmp_dir();
    let source = make_source_file(tmp.path(), "A", "f1.txt");
    fs::create_dir_all(tmp.path().join("B")).unwrap();
    fs::copy(&source, tmp.path().join("B").join("f1.txt")).unwrap();
    let es = FakeEsManager::new();
    es.fail("delete_by_file_paths", "down");
    let manager = make_manager(tmp.path(), &es);

    let (result, err) = split(block_on(manager.move_classified_file(&source, "B", "book", MoveOpts { clean_existing: true, ..Default::default() })));

    assert!(result.is_none());
    assert!(err.expect("err").starts_with("중복 파일 정리 실패"));
}

/// 원본의 기존 ES 문서를 심어 둔 매니저. `insert`의 응답은 `ids`가 정한다.
fn manager_with_existing_doc(tmp_path: &Path, source: &Path, ids: impl Fn(usize, &search_manager::Docs) -> Vec<i64> + Send + Sync + 'static) -> BookManager {
    use std::os::unix::fs::MetadataExt;
    let es = FakeEsManager::new();
    es.put_doc(fs::metadata(source).unwrap().ino() as i64, make_doc("A/f1.txt", ".txt"));
    let calls = std::sync::atomic::AtomicUsize::new(0);
    es.on_insert(move |docs| Ok(ids(calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst), docs)));
    make_manager(tmp_path, &es)
}

#[test]
fn test_move_classified_file_restores_the_old_document_when_the_new_one_is_indexed_under_another_id() {
    let tmp = tmp_dir();
    let source = make_source_file(tmp.path(), "A", "f1.txt");
    // 새 문서는 엉뚱한 id로 들어가고, 기존 문서 복구는 제 id로 들어간다.
    let manager = manager_with_existing_doc(tmp.path(), &source, |call, docs| if call == 0 { vec![123_456] } else { docs.keys().copied().collect() });

    let (result, err) = split(block_on(manager.move_classified_file(&source, "B", "book", MoveOpts::default())));

    assert!(result.is_none());
    let err = err.expect("err");
    assert!(err.contains("ES 적재 실패") && !err.contains("기존 ES 문서 복구 실패"), "{err}");
}

#[test]
fn test_move_classified_file_reports_an_old_document_that_could_not_be_restored() {
    let tmp = tmp_dir();
    let source = make_source_file(tmp.path(), "A", "f1.txt");
    let manager = manager_with_existing_doc(tmp.path(), &source, |_, _| vec![123_456]);

    let (result, err) = split(block_on(manager.move_classified_file(&source, "B", "book", MoveOpts::default())));

    assert!(result.is_none());
    assert!(err.expect("err").contains("기존 ES 문서 복구 실패: None"));
}

#[test]
fn test_next_numbered_path_picks_a_free_sibling_name() {
    let tmp = tmp_dir();
    let target = tmp.path().join("book.epub");
    fs::write(&target, "x").unwrap();
    let next = book_manager::category_proposer::next_numbered_path(&target).expect("빈 이름이 있다");
    assert_ne!(next, target);
    assert_eq!(next.parent(), target.parent());
    assert!(!next.exists());
}

#[test]
fn test_move_classified_file_moves_even_when_the_source_metadata_cannot_be_read() {
    // 원본의 inode를 못 읽으면 기존 ES 문서를 모르는 채로 옮긴다.
    let tmp = tmp_dir();
    let source = make_source_file(tmp.path(), "A", "f1.txt");
    let faulty = FaultyFs::new();
    faulty.set_metadata(|p| p.ends_with("A/f1.txt").then(|| other_err("stat failed")));
    let manager = BookManager::with_deps(deps(tmp.path(), &FakeEsManager::new()).with_fs(faulty));

    let _ = block_on(manager.move_classified_file(&source, "B", "book", MoveOpts::default()));

    assert!(tmp.path().join("B").join("f1.txt").is_file());
    assert!(!source.exists());
}
