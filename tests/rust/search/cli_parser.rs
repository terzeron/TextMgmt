//! `search::cli_parser` 단위 테스트.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::es_dummy::{DummyEs, not_found_error};
use es_client::transport::EsTransport;
use search::cli_parser::{Command, World, parse_options, print_usage, run};
use search_manager::{ConnectOptions, ESManager};
use serde_json::{Value, json};

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn full_env() -> HashMap<String, String> {
    [("TM_ES_URL", "http://es.test:9200"), ("TM_ES_USER", "user"), ("TM_ES_PASSWORD", "dummy"), ("TM_ES_BOOK_INDEX", "book_idx"), ("TM_ES_COMICS_INDEX", "comics_idx")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

struct Outcome {
    code: i32,
    out: String,
    err: String,
    connected: Vec<ConnectOptions>,
}

/// `es`를 전송 계층으로 써서 `run`을 부른다.
fn run_with(argv: &[&str], env: HashMap<String, String>, es: Arc<DummyEs>) -> Outcome {
    let connected: Mutex<Vec<ConnectOptions>> = Mutex::new(Vec::new());
    let env_fn = |key: &str| env.get(key).cloned();
    let connect = |options: &ConnectOptions| -> Arc<dyn EsTransport> {
        connected.lock().unwrap().push(options.clone());
        es.clone()
    };
    let world = World { env: &env_fn, connect: &connect };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run("search", &args(argv), &world, &mut out, &mut err);
    Outcome { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap(), connected: connected.into_inner().unwrap() }
}

/// `op` 요청의 params를 기록하고 `response`를 돌려주는 대역.
fn capture(es: &DummyEs, op: &str, response: Value) -> Arc<Mutex<Vec<Value>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    es.on(op, move |params| {
        sink.lock().unwrap().push(params.clone());
        Ok(response.clone())
    });
    seen
}

fn two_hits() -> Value {
    json!({"hits": {"total": {"value": 42}, "max_score": 3.5, "hits": [
        {"_id": "101", "_score": 3.5, "_source": {"file_path": "/comics/a/001.pdf"}},
        {"_id": "x-2", "_score": null, "_source": {"file_path": "/comics/a/002.pdf"}}
    ]}})
}

fn lines(text: &str) -> Vec<Value> {
    text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

mod print_usage {
    use super::*;

    #[test]
    fn usage_lists_commands_and_exit_codes() {
        let mut out = Vec::new();
        print_usage("search", &mut out);
        let text = String::from_utf8(out).unwrap();
        for word in ["search", "id", "path", "keyword", "latest", "query", "--size", "--fields", "읽기 전용", "종료 코드"] {
            assert!(text.contains(word), "{word} 없음: {text}");
        }
    }
}

mod parse_options {
    use super::*;

    #[test]
    fn defaults() {
        let options = parse_options(&args(&["latest", "comics"])).unwrap();
        assert_eq!(options.command, Command::Latest);
        assert_eq!(options.index, "comics");
        assert_eq!(options.size, 20);
        assert_eq!(options.fields, None);
    }

    #[test]
    fn options_in_both_forms_and_anywhere() {
        let options = parse_options(&args(&["path", "--size", "5", "book", "/books/x", "--fields=title,file_path"])).unwrap();
        assert_eq!(options.command, Command::Path("/books/x".into()));
        assert_eq!(options.size, 5);
        assert_eq!(options.fields, Some(vec!["title".to_string(), "file_path".to_string()]));
        let options = parse_options(&args(&["--size=7", "--fields", "all", "keyword", "book", "삼국지"])).unwrap();
        assert_eq!(options.command, Command::Keyword("삼국지".into()));
        assert_eq!(options.size, 7);
        assert_eq!(options.fields, Some(Vec::new()));
    }

    #[test]
    fn double_dash_ends_options() {
        let options = parse_options(&args(&["keyword", "book", "--", "--size"])).unwrap();
        assert_eq!(options.command, Command::Keyword("--size".into()));
    }

    #[test]
    fn id_and_query_commands() {
        assert_eq!(parse_options(&args(&["id", "book", "123"])).unwrap().command, Command::Id("123".into()));
        let options = parse_options(&args(&["query", "comics", r#"{"query":{"match_all":{}}}"#])).unwrap();
        assert_eq!(options.command, Command::Query(json!({"query": {"match_all": {}}})));
    }

    #[test]
    fn rejects_bad_input() {
        let cases: [&[&str]; 13] = [
            &["latest"],
            &["latest", "novels"],
            &["nope", "book"],
            &["id", "book"],
            &["id", "book", "12a"],
            &["path", "book"],
            &["keyword", "book"],
            &["latest", "book", "extra"],
            &["query", "book", "{bad"],
            &["query", "book", "[1]"],
            &["--size", "0", "latest", "book"],
            &["--size=abc", "latest", "book"],
            &["--color", "latest", "book"],
        ];
        for case in cases {
            assert!(parse_options(&args(case)).is_err(), "{case:?}");
        }
        assert!(parse_options(&args(&["--size=10001", "latest", "book"])).is_err());
        assert!(parse_options(&args(&["latest", "book", "--size"])).is_err());
        assert!(parse_options(&args(&["latest", "book", "--fields="])).is_err());
    }
}

mod run {
    use super::*;

    #[test]
    fn no_args_and_help_print_usage_without_connecting() {
        for argv in [&[][..], &["-h"][..], &["--help"][..], &["latest", "book", "--help"][..]] {
            let outcome = run_with(argv, full_env(), DummyEs::new());
            assert_eq!(outcome.code, 0, "{argv:?}");
            assert!(outcome.out.contains("사용법"), "{}", outcome.out);
            assert!(outcome.connected.is_empty());
        }
    }

    #[test]
    fn usage_error_exits_2_without_connecting() {
        let outcome = run_with(&["latest", "novels"], full_env(), DummyEs::new());
        assert_eq!(outcome.code, 2);
        assert!(outcome.err.contains("ERROR"), "{}", outcome.err);
        assert!(outcome.err.contains("search -h"), "{}", outcome.err);
        assert!(outcome.connected.is_empty());
    }

    #[test]
    fn missing_env_exits_255_without_connecting() {
        for key in ["TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD", "TM_ES_COMICS_INDEX"] {
            let mut env = full_env();
            env.remove(key);
            let outcome = run_with(&["latest", "comics"], env, DummyEs::new());
            assert_eq!(outcome.code, 255, "{key}");
            assert!(outcome.err.contains(key), "{}", outcome.err);
            assert!(outcome.connected.is_empty());
        }
    }

    #[test]
    fn connect_options_come_from_env_and_index_name() {
        let es = DummyEs::new();
        capture(&es, "search", two_hits());
        let mut env = full_env();
        env.insert("TM_ES_CA_CERT".into(), "/etc/ca.pem".into());
        env.insert("TM_ES_URL".into(), "HTTPS://es.test".into());
        let outcome = run_with(&["latest", "comics"], env, es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        let options = &outcome.connected[0];
        assert_eq!(options.hosts, vec!["HTTPS://es.test".to_string()]);
        assert_eq!(options.basic_auth, ("user".to_string(), "dummy".to_string()));
        assert_eq!(options.index, "comics_idx");
        assert_eq!(options.verify_certs, Some(true));
        assert_eq!(options.ca_cert_path.as_deref(), Some(std::path::Path::new("/etc/ca.pem")));

        let es = DummyEs::new();
        capture(&es, "search", two_hits());
        let outcome = run_with(&["latest", "book"], full_env(), es);
        assert_eq!(outcome.connected[0].index, "book_idx");
        assert_eq!(outcome.connected[0].verify_certs, None);
        assert_eq!(outcome.connected[0].ca_cert_path, None);
    }

    #[test]
    fn latest_sorts_by_update_time_and_prints_json_lines() {
        let es = DummyEs::new();
        let seen = capture(&es, "search", two_hits());
        let outcome = run_with(&["latest", "comics", "--size", "2"], full_env(), es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        let body = &seen.lock().unwrap()[0];
        assert_eq!(body["query"], json!({"match_all": {}}));
        assert_eq!(body["sort"], ESManager::latest_sort());
        assert_eq!(body["size"], json!(2));
        assert_eq!(body["track_total_hits"], json!(true));
        assert_eq!(body["_source"], json!(es_client::LIST_SOURCE_FIELDS));
        assert_eq!(
            lines(&outcome.out),
            vec![json!({"id": 101, "score": 3.5, "source": {"file_path": "/comics/a/001.pdf"}}), json!({"id": "x-2", "score": null, "source": {"file_path": "/comics/a/002.pdf"}})]
        );
        assert!(outcome.err.contains("total: 42 (출력 2)"), "{}", outcome.err);
    }

    #[test]
    fn path_uses_prefix_on_file_path_sorted_by_path() {
        let es = DummyEs::new();
        let seen = capture(&es, "search", two_hits());
        let outcome = run_with(&["path", "comics", "/comics/a/", "--fields=file_path"], full_env(), es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        let body = &seen.lock().unwrap()[0];
        assert_eq!(body["query"], json!({"prefix": {"file_path": "/comics/a/"}}));
        assert_eq!(body["sort"], json!([{"file_path": "asc"}]));
        assert_eq!(body["_source"], json!(["file_path"]));
    }

    #[test]
    fn keyword_uses_web_keyword_query_and_all_fields() {
        let es = DummyEs::new();
        let seen = capture(&es, "search", two_hits());
        let outcome = run_with(&["keyword", "book", "삼국지", "--fields=all"], full_env(), es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        let body = &seen.lock().unwrap()[0];
        assert_eq!(body["query"], ESManager::keyword_query("삼국지"));
        assert!(body.get("sort").is_none());
        assert!(body.get("_source").is_none(), "{body}");
    }

    #[test]
    fn query_keeps_given_keys_and_fills_missing() {
        let es = DummyEs::new();
        let seen = capture(&es, "search", two_hits());
        let given = r#"{"query":{"term":{"file_type":"pdf"}},"size":3,"_source":["title"],"track_total_hits":false}"#;
        let outcome = run_with(&["query", "book", given, "--size=9", "--fields=file_path"], full_env(), es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        let body = &seen.lock().unwrap()[0];
        assert_eq!(body["size"], json!(3));
        assert_eq!(body["_source"], json!(["title"]));
        assert_eq!(body["track_total_hits"], json!(false));

        let es = DummyEs::new();
        let seen = capture(&es, "search", two_hits());
        let outcome = run_with(&["query", "book", r#"{"query":{"match_all":{}}}"#], full_env(), es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        let body = &seen.lock().unwrap()[0];
        assert_eq!(body["size"], json!(20));
        assert_eq!(body["track_total_hits"], json!(true));
        assert!(body.get("_source").is_none(), "query는 --fields가 없으면 _source를 건드리지 않는다: {body}");
    }

    #[test]
    fn search_response_without_total_still_prints_hits() {
        let es = DummyEs::new();
        capture(&es, "search", json!({"hits": {"hits": [{"_id": "5", "_source": {}}]}}));
        let outcome = run_with(&["latest", "book"], full_env(), es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        assert_eq!(lines(&outcome.out), vec![json!({"id": 5, "score": null, "source": {}})]);
        assert!(outcome.err.contains("total: ? (출력 1)"), "{}", outcome.err);
    }

    #[test]
    fn search_failure_exits_1() {
        let es = DummyEs::new();
        es.fail("search", "boom");
        let outcome = run_with(&["latest", "book"], full_env(), es);
        assert_eq!(outcome.code, 1);
        assert!(outcome.err.contains("boom"), "{}", outcome.err);
        assert!(outcome.out.is_empty());
    }

    #[test]
    fn id_prints_full_source_by_default() {
        let es = DummyEs::new();
        let seen = capture(&es, "get", json!({"_id": "77", "found": true, "_source": {"title": "t", "summary": "s"}}));
        let outcome = run_with(&["id", "book", "77"], full_env(), es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        assert_eq!(seen.lock().unwrap()[0]["id"], json!("77"));
        assert_eq!(lines(&outcome.out), vec![json!({"id": 77, "source": {"title": "t", "summary": "s"}})]);
    }

    #[test]
    fn id_filters_fields_when_given() {
        let es = DummyEs::new();
        capture(&es, "get", json!({"_source": {"title": "t", "summary": "s"}}));
        let outcome = run_with(&["id", "book", "77", "--fields", "title,missing"], full_env(), es);
        assert_eq!(outcome.code, 0, "{}", outcome.err);
        assert_eq!(lines(&outcome.out), vec![json!({"id": 77, "source": {"title": "t"}})]);
    }

    #[test]
    fn id_not_found_and_failure_exit_1() {
        let es = DummyEs::new();
        es.on("get", |_| Err(not_found_error("missing")));
        let outcome = run_with(&["id", "book", "77"], full_env(), es);
        assert_eq!(outcome.code, 1);
        assert!(outcome.err.contains("문서가 없다: 77"), "{}", outcome.err);

        let es = DummyEs::new();
        es.fail("get", "boom");
        let outcome = run_with(&["id", "book", "77"], full_env(), es);
        assert_eq!(outcome.code, 1);
        assert!(outcome.err.contains("boom"), "{}", outcome.err);
    }
}

mod closed_stdout {
    use super::*;

    /// `search ... | head`처럼 읽는 쪽이 먼저 닫힌 stdout.
    struct ClosedPipe;

    impl std::io::Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn run_closed(argv: &[&str], es: Arc<DummyEs>) -> i32 {
        let env = full_env();
        let env_fn = |key: &str| env.get(key).cloned();
        let connect = |_: &ConnectOptions| -> Arc<dyn EsTransport> { es.clone() };
        let world = World { env: &env_fn, connect: &connect };
        run("search", &args(argv), &world, &mut ClosedPipe, &mut Vec::new())
    }

    #[test]
    fn write_failure_exits_1() {
        let es = DummyEs::new();
        capture(&es, "search", two_hits());
        assert_eq!(run_closed(&["latest", "book"], es), 1);
        let es = DummyEs::new();
        capture(&es, "get", json!({"_source": {}}));
        assert_eq!(run_closed(&["id", "book", "1"], es), 1);
    }
}
