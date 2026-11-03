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
    serve_sequence(vec![(status, body)])
}

fn serve_sequence(responses: Vec<(u16, Value)>) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (status, body) in responses {
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
        }
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
fn search_scrolls_above_result_window_and_clears_context() {
    let first_rows = (1..=10_000).map(|id| json!({"_id":id.to_string(),"_score":2.0,"_source":{"title":"first"}})).collect::<Vec<_>>();
    let first = json!({"_scroll_id":"s1","hits":{"max_score":2.0,"hits":first_rows}});
    let second = json!({"_scroll_id":"s2","hits":{"max_score":4.0,"hits":[{"_id":"10001","_score":2.0,"_source":{"title":"second"}}]}});
    let (url, rx) = serve_sequence(vec![(200, first), (200, second), (200, json!({"succeeded":true}))]);
    let result = client(&url).search(json!({"match_all":{}}), 10_001, Some(&["title"])).unwrap();
    assert_eq!(result.len(), 10_001);
    assert_eq!(result.first().unwrap().id, 1);
    assert_eq!(result.last().unwrap().id, 10_001);
    assert_eq!(result.first().unwrap().score, 100.0);
    assert_eq!(result.last().unwrap().score, 50.0);
    let initial = rx.recv().unwrap();
    assert_eq!(initial.request_line, "POST /books/_search?scroll=10m HTTP/1.1");
    assert_eq!(initial.body["size"], 10_000);
    assert!(initial.body.get("_source").is_none(), "Python scroll path requests full source");
    let page = rx.recv().unwrap();
    assert_eq!(page.request_line, "POST /_search/scroll HTTP/1.1");
    assert_eq!(page.body, json!({"scroll":"10m","scroll_id":"s1"}));
    let clear = rx.recv().unwrap();
    assert_eq!(clear.request_line, "DELETE /_search/scroll HTTP/1.1");
    assert_eq!(clear.body, json!({"scroll_id":"s2"}));
}

#[test]
fn search_scroll_matches_python_empty_max_score_and_cleans_up_updated_id() {
    let first = json!({"_scroll_id":"s1","hits":{"max_score":1.0,"hits":[{"_id":"1","_score":1.0,"_source":{}}]}});
    let next = json!({"_scroll_id":"s2","hits":{"max_score":null,"hits":[{"_id":"2","_score":1.0,"_source":{}}]}});
    let (url, rx) = serve_sequence(vec![(200, first), (200, next), (200, json!({"succeeded":true}))]);
    assert!(client(&url).search(json!({}), 10_001, None).unwrap().is_empty());
    assert_eq!(rx.recv().unwrap().request_line, "POST /books/_search?scroll=10m HTTP/1.1");
    assert_eq!(rx.recv().unwrap().request_line, "POST /_search/scroll HTTP/1.1");
    let clear = rx.recv().unwrap();
    assert_eq!(clear.request_line, "DELETE /_search/scroll HTTP/1.1");
    assert_eq!(clear.body, json!({"scroll_id":"s2"}));
}

#[test]
fn search_scroll_clears_context_when_a_page_request_fails() {
    let first = json!({"_scroll_id":"s1","hits":{"max_score":1.0,"hits":[{"_id":"1","_score":1.0,"_source":{}}]}});
    let (url, rx) = serve_sequence(vec![(200, first), (500, json!({"error":"scroll failed"})), (200, json!({"succeeded":true}))]);
    assert!(matches!(client(&url).search(json!({}), 10_001, None), Err(EsError::Status { status: 500, .. })));
    assert_eq!(rx.recv().unwrap().request_line, "POST /books/_search?scroll=10m HTTP/1.1");
    assert_eq!(rx.recv().unwrap().request_line, "POST /_search/scroll HTTP/1.1");
    let clear = rx.recv().unwrap();
    assert_eq!(clear.request_line, "DELETE /_search/scroll HTTP/1.1");
    assert_eq!(clear.body, json!({"scroll_id":"s1"}));
}

#[test]
fn connect_with_retry_succeeds_when_the_server_answers() {
    let (url, rx) = serve_once(200, json!({"version": {"number": "8.0.0"}}));
    assert!(client(&url).connect_with_retry(5, &|_| panic!("쉬지 않는다")).is_ok());
    assert_eq!(rx.recv().unwrap().request_line, "GET / HTTP/1.1");
}

#[test]
fn malformed_responses_are_parse_errors() {
    let (url, _rx) = serve_once(200, json!({"hits": {"max_score": 1.0, "hits": "nope"}}));
    assert!(matches!(client(&url).search(json!({}), 10, None).unwrap_err(), EsError::Parse(_)));
    let (url, _rx) = serve_once(200, json!({"hits": {"max_score": 1.0, "hits": [{"_id": "abc", "_score": 1.0, "_source": {}}]}}));
    assert!(matches!(client(&url).search(json!({}), 10, None).unwrap_err(), EsError::Parse(_)));
}

#[test]
fn the_book_search_trait_uses_the_default_arguments() {
    use es_client::BookSearch;
    let (url, rx) = serve_once(200, hits(json!(1.0), &[("1", 1.0, json!({"title": "a"}))]));
    let r = BookSearch::search_by_title(&client(&url), "제목", "epub", 5).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(rx.recv().unwrap().body["size"], 10);

    let (url, rx) = serve_once(200, hits(json!(1.0), &[]));
    BookSearch::search_similar_docs(&client(&url), "요약").unwrap();
    let body = rx.recv().unwrap().body;
    assert_eq!(body["query"]["bool"]["should"][2]["more_like_this"]["like"], "요약");
    assert!(body["query"]["bool"].get("must_not").is_none());
}
