//! ES 갱신·조회·삭제 점검 테스트 (tests/test_update.py 이식).

use std::cell::RefCell;

use es_client::{BookStore, EsError, UpdateFields};
use serde_json::{Value, json};
use update::run;

struct Dummy {
    update_result: bool,
    delete_result: bool,
    updated: RefCell<Vec<(i64, UpdateFields)>>,
    deleted: RefCell<Vec<i64>>,
}

impl Dummy {
    fn new(update_result: bool, delete_result: bool) -> Self {
        Dummy { update_result, delete_result, updated: RefCell::new(vec![]), deleted: RefCell::new(vec![]) }
    }
}

impl BookStore for Dummy {
    fn update(&self, doc_id: i64, fields: &UpdateFields) -> Result<bool, EsError> {
        self.updated.borrow_mut().push((doc_id, fields.clone()));
        Ok(self.update_result)
    }
    fn search_by_id(&self, doc_id: i64) -> Result<Value, EsError> {
        Ok(json!({"book_id": doc_id, "title": "book"}))
    }
    fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        self.deleted.borrow_mut().push(doc_id);
        Ok(self.delete_result)
    }
}

fn go(args: &[&str], store: &Dummy) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let code = run("update", &args, &mut out, &|| Ok(store as &dyn BookStore));
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn run_updates_reads_and_deletes_the_test_document() {
    let es = Dummy::new(true, true);
    let (code, out) = go(&["run"], &es);
    assert_eq!(code, 0);
    let updated = es.updated.borrow();
    assert_eq!(updated[0].0, 3384);
    assert_eq!(
        updated[0].1,
        UpdateFields {
            category: "2_utf8".into(),
            title: "test title2".into(),
            author: "test author".into(),
            file_path: "/test/path".into(),
            file_type: "test_ext2".into(),
            file_size: 10_000_000,
            summary: "test content2".into(),
            updated_time: String::new()
        }
    );
    assert_eq!(*es.deleted.borrow(), [3384]);
    assert!(out.contains("{'book_id': 3384, 'title': 'book'}"), "{out}");
}

#[test]
fn a_custom_document_id_can_be_given() {
    let es = Dummy::new(true, true);
    assert_eq!(go(&["run", "--doc-id", "42"], &es).0, 0);
    assert_eq!(es.updated.borrow()[0].0, 42);
    assert_eq!(*es.deleted.borrow(), [42]);
    let es = Dummy::new(true, true);
    assert_eq!(go(&["run", "--doc-id=7"], &es).0, 0);
    assert_eq!(*es.deleted.borrow(), [7]);
}

#[test]
fn a_failed_update_stops_before_the_delete() {
    let es = Dummy::new(false, true);
    assert_eq!(go(&["run"], &es).0, 255);
    assert!(es.deleted.borrow().is_empty());
}

#[test]
fn a_failed_delete_is_reported_after_the_delete_was_attempted() {
    let es = Dummy::new(true, false);
    assert_eq!(go(&["run"], &es).0, 255);
    assert_eq!(*es.deleted.borrow(), [3384]);
}

#[test]
fn without_the_run_command_nothing_touches_the_index() {
    let es = Dummy::new(true, true);
    let (code, out) = go(&[], &es);
    assert_eq!(code, 0);
    for needle in ["update", "사용법:", "run", "--doc-id", "3384", "TM_ES_URL", "삭제"] {
        assert!(out.contains(needle), "{needle} 없음: {out}");
    }
    assert_eq!(go(&["--help"], &es).1, out);
    assert_eq!(go(&["-h"], &es).1, out);
    assert!(es.updated.borrow().is_empty() && es.deleted.borrow().is_empty());
}

#[test]
fn bad_arguments_exit_two_without_touching_the_index() {
    let es = Dummy::new(true, true);
    for bad in [&["bogus"][..], &["run", "--doc-id"], &["run", "--doc-id", "x"], &["run", "--nope"], &["run", "extra"]] {
        assert_eq!(go(bad, &es).0, 2, "{bad:?}");
    }
    assert!(es.updated.borrow().is_empty());
}

#[test]
fn connection_and_request_errors_map_to_exit_codes() {
    let args = vec!["run".to_string()];
    let mut out = Vec::new();
    assert_eq!(run("update", &args, &mut out, &|| Err(EsError::MissingEnv("TM_ES_URL".into()))), 255);
    assert_eq!(run("update", &args, &mut out, &|| Err(EsError::Unsupported("x".into()))), 1);

    struct Failing(&'static str);
    impl BookStore for Failing {
        fn update(&self, _: i64, _: &UpdateFields) -> Result<bool, EsError> {
            if self.0 == "update" { Err(EsError::Status { status: 404, body: "missing".into() }) } else { Ok(true) }
        }
        fn search_by_id(&self, _: i64) -> Result<Value, EsError> {
            if self.0 == "get" { Err(EsError::Parse("bad".into())) } else { Ok(json!({})) }
        }
        fn delete(&self, _: i64) -> Result<bool, EsError> {
            Err(EsError::Transport("down".into()))
        }
    }
    for which in ["update", "get", "delete"] {
        let f = Failing(which);
        assert_eq!(run("update", &args, &mut out, &|| Ok(&f as &dyn BookStore)), 1, "{which}");
    }
}
