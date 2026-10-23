//! utils/search.py 테스트 이식 (tests/test_search.py).

use std::cell::RefCell;

use es_client::{BookSearch, EsError, Hit};
use search::run;
use serde_json::json;

#[derive(Default)]
struct Dummy {
    title_calls: RefCell<Vec<(String, String, i64)>>,
    similar_calls: RefCell<Vec<String>>,
}

impl BookSearch for Dummy {
    fn search_by_title(&self, title: &str, file_type: &str, file_size: i64) -> Result<Vec<Hit>, EsError> {
        self.title_calls.borrow_mut().push((title.into(), file_type.into(), file_size));
        Ok(vec![Hit { id: 1, source: json!({"summary": "line1\nline2", "title": title}), score: 100.0 }])
    }
    fn search_similar_docs(&self, summary: &str) -> Result<Vec<Hit>, EsError> {
        self.similar_calls.borrow_mut().push(summary.into());
        Ok(vec![Hit { id: 2, source: json!({"summary": "content summary", "title": "similar"}), score: 88.0 }])
    }
}

fn go(args: &[&str], es: &Dummy) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let code = run("search", &args, &mut out, &|| Ok(es as &dyn BookSearch));
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn without_title_prints_usage_and_exits_zero() {
    let es = Dummy::default();
    let (code, out) = go(&[], &es);
    assert_eq!(code, 0);
    assert!(out.contains("Usage:") && out.contains("search"));
    assert!(es.title_calls.borrow().is_empty());
}

#[test]
fn searches_by_title_and_content() {
    let es = Dummy::default();
    let (code, out) = go(&["hello", "body", "epub", "7"], &es);
    assert_eq!(code, 0);
    assert_eq!(*es.title_calls.borrow(), [("hello".to_string(), "epub".to_string(), 7)]);
    assert_eq!(*es.similar_calls.borrow(), ["body"]);
    assert!(out.contains("----------------------------------------"));
    assert!(out.contains("item: {'summary': 'line1\\nline2', 'title': 'hello'}"), "{out}");
    assert!(out.contains("line1 line2") && out.contains("content summary"));
}

#[test]
fn title_only_uses_default_ext_and_size() {
    let es = Dummy::default();
    assert_eq!(go(&["hello"], &es).0, 0);
    assert_eq!(*es.title_calls.borrow(), [("hello".to_string(), String::new(), 0)]);
    assert!(es.similar_calls.borrow().is_empty());
}

#[test]
fn summary_is_cut_to_200_chars_and_missing_summary_is_empty() {
    struct Long;
    impl BookSearch for Long {
        fn search_by_title(&self, _: &str, _: &str, _: i64) -> Result<Vec<Hit>, EsError> {
            Ok(vec![Hit { id: 1, source: json!({"summary": "가".repeat(300)}), score: 1.0 }, Hit { id: 2, source: json!({"title": "요약 없음"}), score: 1.0 }])
        }
        fn search_similar_docs(&self, _: &str) -> Result<Vec<Hit>, EsError> {
            Ok(vec![])
        }
    }
    let mut out = Vec::new();
    let args = vec!["x".to_string()];
    assert_eq!(run("search", &args, &mut out, &|| Ok(&Long as &dyn BookSearch)), 0);
    let text = String::from_utf8(out).unwrap();
    // `item:` 줄은 소스 전체를 그대로 찍는다. 잘리는 것은 그 아래 요약 줄이다.
    let summary_lines: Vec<&str> = text.lines().filter(|l| !l.starts_with("item:") && !l.starts_with("---")).collect();
    assert_eq!(summary_lines, ["가".repeat(200).as_str(), ""]);
}

#[test]
fn invalid_size_exits_one() {
    let es = Dummy::default();
    assert_eq!(go(&["t", "", "", "abc"], &es).0, 1);
}

#[test]
fn connection_setup_failure_exits_255_for_missing_env_and_one_otherwise() {
    let args = vec!["t".to_string()];
    let mut out = Vec::new();
    assert_eq!(run("search", &args, &mut out, &|| Err(EsError::MissingEnv("TM_ES_URL".into()))), 255);
    assert_eq!(run("search", &args, &mut out, &|| Err(EsError::Unsupported("x".into()))), 1);
}

#[test]
fn search_failure_exits_one() {
    struct Down;
    impl BookSearch for Down {
        fn search_by_title(&self, _: &str, _: &str, _: i64) -> Result<Vec<Hit>, EsError> {
            Err(EsError::Unsupported("down".into()))
        }
        fn search_similar_docs(&self, _: &str) -> Result<Vec<Hit>, EsError> {
            Ok(vec![])
        }
    }
    let mut out = Vec::new();
    assert_eq!(run("search", &["t".to_string()], &mut out, &|| Ok(&Down as &dyn BookSearch)), 1);
}

#[test]
fn python_repr_of_values() {
    use search::py_repr;
    assert_eq!(py_repr(&json!({"a": 1, "b": [true, null, 1.5, "x'y"], "c": "한글"})), "{'a': 1, 'b': [True, None, 1.5, \"x'y\"], 'c': '한글'}");
    assert_eq!(py_repr(&json!("a\"b'c")), "'a\"b\\'c'");
    assert_eq!(py_repr(&json!(2.0)), "2.0");
    assert_eq!(py_repr(&json!("tab\there\\")), "'tab\\there\\\\'");
}
