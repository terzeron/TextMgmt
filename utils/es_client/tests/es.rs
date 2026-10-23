//! ES REST 클라이언트 테스트. 실제 ES 대신 한 번만 응답하는 로컬 HTTP 서버를 쓴다.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use es_client::{EsClient, EsError};
use serde_json::{Value, json};

struct Captured {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Captured {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// 요청 하나를 받아 `status`와 `body`로 응답하고, 받은 요청을 돌려주는 서버를 띄운다.
fn serve_once(status: u16, body: Value) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let n = stream.read(&mut chunk).unwrap();
            buf.extend_from_slice(&chunk[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break p + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let mut lines = head.lines();
        let request_line = lines.next().unwrap().to_owned();
        let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
        let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
        while buf.len() < header_end + len {
            let n = stream.read(&mut chunk).unwrap();
            buf.extend_from_slice(&chunk[..n]);
        }
        let parsed = serde_json::from_slice(&buf[header_end..header_end + len]).unwrap_or(Value::Null);
        let payload = body.to_string();
        write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
        tx.send(Captured { request_line, headers, body: parsed }).unwrap();
    });
    (url, rx)
}

fn client(url: &str) -> EsClient {
    EsClient::new(url, "elastic", "pw", "books")
}

fn hits(max_score: Value, rows: &[(&str, f64, Value)]) -> Value {
    json!({"hits": {"max_score": max_score, "hits": rows.iter().map(|(id, score, src)| json!({"_id": id, "_score": score, "_source": src})).collect::<Vec<_>>()}})
}

#[test]
fn search_sends_query_with_basic_auth_and_normalizes_scores() {
    let (url, rx) = serve_once(200, hits(json!(8.0), &[("11", 8.0, json!({"title": "a"})), ("12", 4.0, json!({"title": "b"}))]));
    let res = client(&url).search(json!({"match_all": {}}), 10, Some(&["title", "author"])).unwrap();

    let req = rx.recv().unwrap();
    assert_eq!(req.request_line, "POST /books/_search HTTP/1.1");
    // base64("elastic:pw")
    assert_eq!(req.header("authorization"), Some("Basic ZWxhc3RpYzpwdw=="));
    assert_eq!(req.body, json!({"query": {"match_all": {}}, "size": 10, "track_scores": true, "_source": ["title", "author"]}));

    assert_eq!(res.iter().map(|h| h.id).collect::<Vec<_>>(), [11, 12]);
    assert_eq!(res.iter().map(|h| h.score).collect::<Vec<_>>(), [100.0, 50.0]);
    assert_eq!(res[0].source["title"], "a");
}

#[test]
fn search_without_source_fields_omits_the_key_and_clamps_size() {
    let (url, rx) = serve_once(200, hits(json!(1.0), &[]));
    client(&url).search(json!({"match_all": {}}), 50_000, None).unwrap();
    let body = rx.recv().unwrap().body;
    assert!(body.get("_source").is_none());
    assert_eq!(body["size"], 10_000);
}

#[test]
fn search_with_null_max_score_is_empty_and_zero_max_score_gives_zero() {
    let (url, _rx) = serve_once(200, hits(Value::Null, &[]));
    assert!(client(&url).search(json!({}), 10, None).unwrap().is_empty());
    let (url, _rx) = serve_once(200, hits(json!(0.0), &[("1", 0.0, json!({}))]));
    assert_eq!(client(&url).search(json!({}), 10, None).unwrap()[0].score, 0.0);
}

#[test]
fn search_by_title_builds_the_python_query() {
    let (url, rx) = serve_once(200, hits(json!(1.0), &[]));
    client(&url).search_by_title("a b c", "epub", 7, None).unwrap();
    let body = rx.recv().unwrap().body;
    let boost = 1.2 + 3f64.log2();
    assert_eq!(
        body["query"],
        json!({"bool": {"should": [{"match": {"title": {"query": "a b c", "boost": boost}}}, {"match": {"file_type": {"query": "epub", "boost": 1}}}, {"match": {"file_size": {"query": 7, "boost": 1}}}]}})
    );
    // 기본 건수는 10건이고 목록용 필드만 받는다.
    assert_eq!(body["size"], 10);
    assert_eq!(body["_source"], json!(["category", "title", "author", "file_path", "file_type", "file_size", "line_count", "page_count", "isbn", "created_time", "updated_time"]));
}

#[test]
fn title_boost_counts_single_space_separated_words_like_python_split() {
    // "".split(" ") -> [''] 1개, "a  b".split(" ") -> ['a','','b'] 3개.
    for (title, words) in [("", 1.0), ("a  b", 3.0)] {
        let (url, rx) = serve_once(200, hits(json!(1.0), &[]));
        client(&url).search_by_title(title, "", 0, None).unwrap();
        let boost = rx.recv().unwrap().body["query"]["bool"]["should"][0]["match"]["title"]["boost"].as_f64().unwrap();
        assert_eq!(boost, 1.2 + f64::log2(words), "{title:?}");
    }
}

#[test]
fn search_similar_docs_builds_should_clauses_and_excludes_the_source_document() {
    let (url, rx) = serve_once(200, hits(json!(1.0), &[]));
    client(&url).search_similar_docs("", "", 1000, "요약 본문", None, Some(42)).unwrap();
    let q = rx.recv().unwrap().body["query"].clone();
    assert_eq!(q["bool"]["minimum_should_match"], 1);
    assert_eq!(q["bool"]["must_not"], json!([{"term": {"_id": "42"}}]));
    let should = q["bool"]["should"].as_array().unwrap();
    assert_eq!(should[0], json!({"match": {"title": {"query": "", "boost": 20}}}));
    assert_eq!(should[1], json!({"match": {"author": {"query": "", "boost": 15}}}));
    assert_eq!(should[2], json!({"more_like_this": {"fields": ["summary"], "like": "요약 본문", "max_query_terms": 25, "min_term_freq": 1, "min_doc_freq": 3, "boost": 3}}));
    assert_eq!(should[3], json!({"range": {"file_size": {"gte": 900.0, "lte": 1100.0, "boost": 1}}}));
}

#[test]
fn search_similar_docs_without_exclude_id_has_no_must_not() {
    let (url, rx) = serve_once(200, hits(json!(1.0), &[]));
    client(&url).search_similar_docs("", "", 0, "x", None, None).unwrap();
    assert!(rx.recv().unwrap().body["query"]["bool"].get("must_not").is_none());
}

#[test]
fn http_error_status_is_an_error_with_the_body() {
    let (url, _rx) = serve_once(500, json!({"error": "boom"}));
    let err = client(&url).search(json!({}), 10, None).unwrap_err();
    assert!(matches!(&err, EsError::Status { status: 500, .. }), "{err}");
    assert!(err.to_string().contains("boom"));
}

#[test]
fn max_result_count_above_the_window_is_rejected_because_scroll_is_not_supported() {
    let (url, _rx) = serve_once(200, hits(json!(1.0), &[]));
    let err = client(&url).search_by_title("a", "", 0, Some(10_001)).unwrap_err();
    assert!(matches!(err, EsError::Unsupported(_)));
}

#[test]
fn from_env_reports_the_first_missing_variable() {
    // 다른 테스트와 환경을 공유하지 않도록 이 테스트만 변수를 건드린다.
    for key in ["TM_ES_BOOK_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"] {
        unsafe { std::env::remove_var(key) };
    }
    assert!(matches!(EsClient::from_env(), Err(EsError::MissingEnv(k)) if k == "TM_ES_BOOK_INDEX"));
    unsafe {
        std::env::set_var("TM_ES_BOOK_INDEX", "books");
        std::env::set_var("TM_ES_URL", "http://127.0.0.1:1");
        std::env::set_var("TM_ES_USER", "u");
    }
    assert!(matches!(EsClient::from_env(), Err(EsError::MissingEnv(k)) if k == "TM_ES_PASSWORD"));
    unsafe { std::env::set_var("TM_ES_PASSWORD", "p") };
    assert!(EsClient::from_env().is_ok());
}

#[test]
fn https_urls_are_rejected_because_tls_is_not_built_in() {
    // 내부망 http 전용이다. 인증서 검증을 끄는 우회로를 두지 않고 요청 단계에서 거부한다.
    let err = EsClient::new("https://127.0.0.1:1", "u", "p", "books").search(json!({}), 10, None).unwrap_err();
    assert!(matches!(err, EsError::Transport(_)), "{err}");
}

#[test]
fn connect_with_retry_gives_up_after_the_configured_attempts_and_sleeps_between() {
    let waits = std::cell::RefCell::new(Vec::new());
    // 닫힌 포트: 연결이 항상 거부된다.
    let c = client("http://127.0.0.1:1");
    let err = c.connect_with_retry(3, &|secs| waits.borrow_mut().push(secs)).unwrap_err();
    assert!(matches!(err, EsError::Transport(_)), "{err}");
    // 시도 3번 사이에 2번 쉰다. 2**(n+1)초, 최대 10초.
    assert_eq!(*waits.borrow(), [2, 4]);
}

#[test]
fn connect_with_retry_succeeds_when_the_server_answers() {
    let (url, rx) = serve_once(200, json!({"version": {"number": "8.0.0"}}));
    assert!(client(&url).connect_with_retry(5, &|_| panic!("쉬지 않는다")).is_ok());
    assert_eq!(rx.recv().unwrap().request_line, "GET / HTTP/1.1");
}
