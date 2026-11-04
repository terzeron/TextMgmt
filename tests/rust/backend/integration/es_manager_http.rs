//! `HttpTransport`(실제 HTTP 전송)와 `ESManager`의 통합 테스트.
//!
//! - 로컬 TCP 서버 테스트: 실제 ES 없이 `op`가 어떤 REST 요청(메서드, 경로, 쿼리 문자열, 본문)이 되는지 확인한다.
//! - Docker 테스트(fixture가 컨테이너를 띄운다): 일회용 ES 9.1.0 컨테이너(nori 분석기 설치, 보안 끔)에 붙어 Python `ESManager`의
//!   주요 동작을 실제로 확인한다. `cargo test -p backend --test es_manager_http`
//!   이미지는 환경 변수 `TM_TEST_ES_IMAGE`로 바꿀 수 있다(기본 `docker.elastic.co/elasticsearch/elasticsearch:9.1.0`).
//!   컨테이너는 테스트가 끝나면(실패해도) `docker rm -f`로 지운다. 운영 ES와 `.env`는 쓰지 않는다.
//!
//! 가정한 API:
//! - `backend::es_manager::HttpTransport::new(url: &str, user: &str, password: &str, index: &str, timeout: Duration)` (`EsTransport` 구현, http 전용)
//! - `backend::es_manager::RealEsConnector`(`EsConnector` 구현)와 `ESManager::connect`

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use backend::es_manager::{ESManager, EsError, EsTransport, HttpTransport, RealEsConnector};
use serde_json::{Value, json};
use sysdeps::SystemClock;

// ---------------------------------------------------------------------------
// 로컬 서버
// ---------------------------------------------------------------------------

struct Captured {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Captured {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// 연결 하나당 `responses`를 차례로 돌려주고, 받은 요청을 `Captured`로 보낸다.
fn serve(responses: Vec<(u16, String)>) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (status, payload) in responses {
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
            let body = String::from_utf8_lossy(&buf[header_end..header_end + len]).into_owned();
            let content_length = if request_line.starts_with("HEAD") { String::new() } else { format!("Content-Length: {}\r\n", payload.len()) };
            let payload = if request_line.starts_with("HEAD") { String::new() } else { payload };
            write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{content_length}Connection: close\r\n\r\n{payload}").unwrap();
            // 읽지 않은 입력(예: 본문 없는 POST의 chunked 끝 표시)을 남긴 채 닫으면 커널이 RST를 보내 클라이언트가
            // 응답 대신 "Connection reset"을 받는다. 쓰기를 닫고 상대가 닫을 때까지 남은 입력을 비운다.
            let _ = stream.shutdown(std::net::Shutdown::Write);
            stream.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
            while matches!(stream.read(&mut chunk), Ok(n) if n > 0) {}
            tx.send(Captured { request_line, headers, body }).unwrap();
        }
    });
    (url, rx)
}

fn transport(url: &str) -> HttpTransport {
    HttpTransport::new(url, "elastic", "pw", "books", Duration::from_secs(5))
}

#[test]
fn search_becomes_post_search_with_body_fields_and_scroll_query() {
    let (url, rx) = serve(vec![(200, json!({"hits": {"hits": []}}).to_string())]);
    let kwargs = json!({"index": "books", "query": {"match_all": {}}, "from_": 2, "size": 3, "source": ["title"], "scroll": "10m", "track_scores": true, "sort": [{"file_path": "asc"}]});

    let response = transport(&url).call("search", &kwargs).unwrap();

    assert_eq!(response, json!({"hits": {"hits": []}}));
    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "POST /books/_search?scroll=10m HTTP/1.1");
    // Basic 인증과 JSON 본문. Python 클라이언트 인자 이름(`from_`, `source`)은 REST 필드(`from`, `_source`)가 된다.
    assert_eq!(request.header("authorization"), Some("Basic ZWxhc3RpYzpwdw=="));
    assert!(request.header("content-type").is_some_and(|v| v.starts_with("application/json")));
    assert_eq!(request.json(), json!({"query": {"match_all": {}}, "from": 2, "size": 3, "_source": ["title"], "track_scores": true, "sort": [{"file_path": "asc"}]}));
}

#[test]
fn search_with_body_kwarg_sends_that_body() {
    let (url, rx) = serve(vec![(200, "{}".to_string())]);
    transport(&url).call("search", &json!({"index": "books", "body": {"size": 1, "aggs": {"a": {"terms": {"field": "category"}}}}})).unwrap();
    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "POST /books/_search HTTP/1.1");
    assert_eq!(request.json(), json!({"size": 1, "aggs": {"a": {"terms": {"field": "category"}}}}));
}

#[test]
fn bulk_becomes_ndjson_post() {
    let (url, rx) = serve(vec![(200, json!({"errors": false, "items": []}).to_string())]);
    let body = json!([{"index": {"_index": "books", "_id": "1"}}, {"title": "한글 제목"}]);

    transport(&url).call("bulk", &json!({"body": body, "timeout": "60s", "refresh": false})).unwrap();

    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "POST /_bulk?timeout=60s&refresh=false HTTP/1.1");
    assert!(request.header("content-type").is_some_and(|v| v.starts_with("application/x-ndjson")));
    assert_eq!(request.body, "{\"index\":{\"_index\":\"books\",\"_id\":\"1\"}}\n{\"title\":\"한글 제목\"}\n");
}

#[test]
fn msearch_becomes_ndjson_post() {
    let (url, rx) = serve(vec![(200, json!({"responses": []}).to_string()), (200, json!({"responses": []}).to_string())]);
    let searches = json!([{}, {"query": {"match_all": {}}, "size": 1}]);

    transport(&url).call("msearch", &json!({"index": "books", "searches": searches.clone()})).unwrap();
    transport(&url).call("msearch", &json!({"searches": searches})).unwrap();

    let with_index = rx.recv().unwrap();
    assert_eq!(with_index.request_line, "POST /books/_msearch HTTP/1.1");
    assert_eq!(with_index.body, "{}\n{\"query\":{\"match_all\":{}},\"size\":1}\n");
    assert_eq!(rx.recv().unwrap().request_line, "POST /_msearch HTTP/1.1");
}

#[test]
fn single_document_and_index_ops_map_to_rest_paths() {
    let ok = |v: Value| (200, v.to_string());
    let (url, rx) = serve(vec![ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({}))]);
    let t = transport(&url);

    t.call("get", &json!({"index": "books", "id": "42"})).unwrap();
    t.call("update", &json!({"index": "books", "id": "42", "body": {"doc": {"title": "t"}}, "refresh": true})).unwrap();
    t.call("delete", &json!({"index": "books", "id": "42", "refresh": true})).unwrap();
    t.call("count", &json!({"index": "books", "query": {"term": {"category": "A"}}})).unwrap();
    t.call("delete_by_query", &json!({"index": "books", "body": {"query": {"ids": {"values": ["1"]}}}, "conflicts": "proceed", "refresh": false})).unwrap();
    t.call("update_by_query", &json!({"index": "books", "query": {"match_all": {}}, "script": {"source": "ctx.op='noop'"}, "conflicts": "abort", "refresh": true})).unwrap();
    t.call("indices.refresh", &json!({"index": "books"})).unwrap();
    t.call("indices.get_mapping", &json!({"index": "books"})).unwrap();
    t.call("indices.put_mapping", &json!({"index": "books", "properties": {"publisher": {"type": "keyword"}}})).unwrap();
    t.call("mget", &json!({"docs": [{"_index": "books", "_id": "1"}], "source": ["file_path", "title"]})).unwrap();
    t.call("scroll", &json!({"scroll_id": "abc", "scroll": "10m"})).unwrap();

    let lines: Vec<(String, Value)> = (0..11).map(|_| rx.recv().unwrap()).map(|c| (c.request_line.clone(), c.json())).collect();
    let line = |i: usize| lines[i].0.as_str();
    assert_eq!(line(0), "GET /books/_doc/42 HTTP/1.1");
    assert_eq!(line(1), "POST /books/_update/42?refresh=true HTTP/1.1");
    assert_eq!(lines[1].1, json!({"doc": {"title": "t"}}));
    assert_eq!(line(2), "DELETE /books/_doc/42?refresh=true HTTP/1.1");
    assert_eq!(line(3), "POST /books/_count HTTP/1.1");
    assert_eq!(lines[3].1, json!({"query": {"term": {"category": "A"}}}));
    assert_eq!(line(4), "POST /books/_delete_by_query?conflicts=proceed&refresh=false HTTP/1.1");
    assert_eq!(lines[4].1, json!({"query": {"ids": {"values": ["1"]}}}));
    assert_eq!(line(5), "POST /books/_update_by_query?conflicts=abort&refresh=true HTTP/1.1");
    assert_eq!(lines[5].1, json!({"query": {"match_all": {}}, "script": {"source": "ctx.op='noop'"}}));
    assert_eq!(line(6), "POST /books/_refresh HTTP/1.1");
    assert_eq!(line(7), "GET /books/_mapping HTTP/1.1");
    assert_eq!(line(8), "PUT /books/_mapping HTTP/1.1");
    assert_eq!(lines[8].1, json!({"properties": {"publisher": {"type": "keyword"}}}));
    assert_eq!(line(9), "POST /_mget?_source=file_path%2Ctitle HTTP/1.1");
    assert_eq!(lines[9].1, json!({"docs": [{"_index": "books", "_id": "1"}]}));
    assert_eq!(line(10), "POST /_search/scroll HTTP/1.1");
    assert_eq!(lines[10].1, json!({"scroll_id": "abc", "scroll": "10m"}));
}

#[test]
fn clear_scroll_is_delete_with_body() {
    let (url, rx) = serve(vec![(200, json!({"succeeded": true}).to_string())]);
    transport(&url).call("clear_scroll", &json!({"scroll_id": "abc"})).unwrap();
    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "DELETE /_search/scroll HTTP/1.1");
    assert_eq!(request.json(), json!({"scroll_id": "abc"}));
}

#[test]
fn indices_exists_uses_head_and_maps_404_to_false() {
    let (url, rx) = serve(vec![(200, String::new()), (404, String::new())]);
    let t = transport(&url);
    assert_eq!(t.call("indices.exists", &json!({"index": "books"})).unwrap(), json!(true));
    assert_eq!(t.call("indices.exists", &json!({"index": "books"})).unwrap(), json!(false));
    assert_eq!(rx.recv().unwrap().request_line, "HEAD /books HTTP/1.1");
}

#[test]
fn error_status_becomes_status_error_with_body() {
    let (url, _rx) = serve(vec![(404, json!({"found": false}).to_string()), (400, json!({"error": {"type": "parsing_exception"}}).to_string())]);
    let t = transport(&url);
    match t.call("get", &json!({"index": "books", "id": "1"})) {
        Err(EsError::Status { status: 404, body }) => assert!(body.contains("found")),
        other => panic!("404를 기대했다: {other:?}"),
    }
    match t.call("search", &json!({"index": "books", "query": {"bad": {}}})) {
        Err(e @ EsError::Status { status: 400, .. }) => assert!(!e.is_not_found()),
        other => panic!("400을 기대했다: {other:?}"),
    }
}

#[test]
fn non_json_response_is_parse_error_and_refused_connection_is_transport_error() {
    let (url, _rx) = serve(vec![(200, "<html>proxy</html>".to_string())]);
    assert!(matches!(transport(&url).call("info", &json!({})), Err(EsError::Parse(_))));

    // 방금 닫은 포트로 연결한다.
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    assert!(matches!(transport(&closed).call("info", &json!({})), Err(EsError::Transport(_))));
}

#[test]
fn index_name_comes_from_constructor_and_trailing_slash_is_trimmed() {
    let (url, rx) = serve(vec![(200, "{}".to_string())]);
    let t = HttpTransport::new(&format!("{url}/"), "", "", "my_books", Duration::from_secs(5));
    assert_eq!(t.index_name(), "my_books");
    t.call("info", &json!({})).unwrap();
    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "GET / HTTP/1.1");
    assert_eq!(request.header("authorization"), None, "사용자가 비어 있으면 인증 헤더를 보내지 않는다");
}

#[test]
fn real_connector_connects_through_http_and_uses_env_index() {
    let (url, rx) = serve(vec![(200, json!({"version": {"number": "9.1.0"}}).to_string())]);
    let env = move |key: &str| match key {
        "TM_ES_URL" => Some(url.clone()),
        "TM_ES_BOOK_INDEX" => Some("env_books".to_string()),
        "TM_ES_USER" => Some("user".to_string()),
        "TM_ES_PASSWORD" => Some("pw".to_string()),
        _ => None,
    };
    let manager = ESManager::connect(&env, &RealEsConnector, &SystemClock).unwrap();
    assert_eq!(rx.recv().unwrap().request_line, "GET / HTTP/1.1");
    drop(manager);
}
