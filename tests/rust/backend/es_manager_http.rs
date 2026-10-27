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

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use backend::es_manager::{ESManager, EsError, EsTransport, HttpTransport, RealEsConnector, SimilarDocsArgs, UpdateFields};
use serde_json::{Value, json};
use sysdeps::SystemClock;

#[path = "../common/tls_server.rs"]
mod tls_server;

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
fn https_uses_the_configured_ca_and_unknown_op_is_unsupported() {
    let (url, server) = tls_server::serve_tls_once();
    let ca_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/rust/common/tls/local-es.crt");
    let t = HttpTransport::new_with_ca(&url, "u", "p", "books", Duration::from_secs(1), Some(&ca_path));
    assert_eq!(t.call("info", &json!({})).unwrap()["version"]["number"], "9.1.0");
    server.join().unwrap();

    let (untrusted_url, untrusted_server) = tls_server::serve_tls_once();
    let untrusted = HttpTransport::new(&untrusted_url, "u", "p", "books", Duration::from_secs(1));
    assert!(matches!(untrusted.call("info", &json!({})), Err(EsError::Transport(_))));
    untrusted_server.join().unwrap();
    assert_eq!(transport("http://127.0.0.1:1").call("no.such_op", &json!({})), Err(EsError::Unsupported));
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

// ---------------------------------------------------------------------------
// Docker: 일회용 ES 9.1.0
// ---------------------------------------------------------------------------

/// `docker rm -f`로 컨테이너를 지우는 가드. 테스트가 실패해도 drop에서 지운다.
struct EsContainer {
    name: String,
    url: String,
}

impl Drop for EsContainer {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.name]).output();
    }
}

fn docker(args: &[&str]) -> String {
    let out = Command::new("docker").args(args).output().expect("docker 실행");
    assert!(out.status.success(), "docker {args:?} 실패: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// nori 분석기를 설치한 ES를 띄우고 응답할 때까지 기다린다.
fn start_es() -> EsContainer {
    let image = std::env::var("TM_TEST_ES_IMAGE").unwrap_or_else(|_| "docker.elastic.co/elasticsearch/elasticsearch:9.1.0".to_string());
    let name = format!("tm-es-manager-http-{}", std::process::id());
    let _ = Command::new("docker").args(["rm", "-f", &name]).output();
    docker(&[
        "run",
        "-d",
        "--name",
        &name,
        "-p",
        "127.0.0.1::9200",
        "-e",
        "discovery.type=single-node",
        "-e",
        "xpack.security.enabled=false",
        "-e",
        "ES_JAVA_OPTS=-Xms512m -Xmx512m",
        "-e",
        "ES_SETTING_CLUSTER_ROUTING_ALLOCATION_DISK_THRESHOLD__ENABLED=false",
        "--entrypoint",
        "sh",
        &image,
        "-c",
        "elasticsearch-plugin install --batch analysis-nori && exec /usr/local/bin/docker-entrypoint.sh eswrapper",
    ]);
    let mut container = EsContainer { name: name.clone(), url: String::new() };
    let port = docker(&["port", &name, "9200/tcp"]).lines().next().and_then(|l| l.rsplit(':').next()).map(str::to_string).expect("호스트 포트");
    container.url = format!("http://127.0.0.1:{port}");
    let probe = transport(&container.url);
    let deadline = Instant::now() + Duration::from_secs(240);
    while Instant::now() < deadline {
        if probe.call("info", &json!({})).is_ok() {
            return container;
        }
        thread::sleep(Duration::from_secs(2));
    }
    let logs = docker(&["logs", "--tail", "40", &name]);
    panic!("ES가 시간 안에 뜨지 않았다:\n{logs}");
}

#[allow(clippy::too_many_arguments)]
fn book(category: &str, title: &str, author: &str, path: &str, file_type: &str, size: i64, pages: i64, updated: &str) -> Value {
    json!({"category": category, "title": title, "author": author, "file_path": path, "file_type": file_type, "file_size": size, "page_count": pages,
           "summary": format!("{title}에 대한 한국어 요약이다. {author}가 썼다."), "updated_time": updated})
}

fn ids(hits: &[(i64, Value, f64)]) -> Vec<i64> {
    hits.iter().map(|h| h.0).collect()
}

#[test]
fn docker_es_manager_flow_against_real_elasticsearch() {
    let container = start_es();
    let es: Arc<dyn EsTransport> = Arc::new(HttpTransport::new(&container.url, "", "", "tm_books", Duration::from_secs(30)));
    let manager = ESManager::with_transport(es.clone());

    // 1. create_index: nori 매핑 포함, 두 번째 호출은 기존 색인을 점검만 한다.
    assert!(!manager.do_exist_index().unwrap());
    assert_eq!(manager.create_index().unwrap()["acknowledged"], json!(true));
    assert!(manager.do_exist_index().unwrap());
    let mapping = es.call("indices.get_mapping", &json!({"index": "tm_books"})).unwrap();
    let props = &mapping["tm_books"]["mappings"]["properties"];
    assert_eq!(props["category"]["fields"]["nori"]["analyzer"], json!("nori_analyzer"));
    assert_eq!(props["title"]["analyzer"], json!("nori_analyzer"));
    assert_eq!(props["file_size"]["type"], json!("unsigned_long"));
    assert_eq!(manager.create_index().unwrap()["acknowledged"], json!(true));

    // 2. insert(bulk, 한글)
    let docs = vec![
        (1, book("소설/한국", "토지 첫째 권", "박경리", "소설/한국/토지1.epub", "epub", 1000, 0, "2026-01-03T00:00:00")),
        (2, book("소설/한국/현대", "채식주의자", "한강", "소설/한국/현대/채식주의자.epub", "epub", 2000, 0, "2026-01-02T00:00:00")),
        (3, book("과학", "코스모스 우주의 신비", "칼 세이건", "과학/코스모스.pdf", "pdf", 3000, 300, "2026-01-01T00:00:00")),
        (4, book("과학", "엘러건트 유니버스", "브라이언 그린", "과학/유니버스.pdf", "pdf", 4000, 400, "2026-01-04T00:00:00")),
        (5, book("과학/물리", "시간의 역사", "스티븐 호킹", "과학/물리/시간.txt", "txt", 500, 0, "2026-01-05T00:00:00")),
    ];
    assert_eq!(manager.insert(&docs, usize::MAX, 3).unwrap(), vec![1, 2, 3, 4, 5]);
    manager.refresh().unwrap();

    // 3. search_by_id
    assert_eq!(manager.search_by_id(1).unwrap()["title"], json!("토지 첫째 권"));
    assert_eq!(manager.search_by_id(999).unwrap(), json!({}));
    assert_eq!(manager.get_existing_paths(&[1, 999]).unwrap(), HashMap::from([(1, "소설/한국/토지1.epub".to_string())]));

    // 4. update, delete, 오류 상태 코드
    assert!(manager.update(1, &UpdateFields { title: "토지 제1권".into(), updated_time: "2026-02-01T00:00:00".into(), ..Default::default() }).unwrap());
    assert_eq!(manager.search_by_id(1).unwrap()["title"], json!("토지 제1권"));
    assert!(matches!(manager.update(999, &UpdateFields { title: "x".into(), ..Default::default() }), Err(EsError::Status { status: 404, .. })));
    assert!(matches!(manager.delete(999), Err(EsError::Status { status: 404, .. })));
    assert!(matches!(es.call("search", &json!({"index": "tm_books", "query": {"no_such_query": {}}})), Err(EsError::Status { status: 400, .. })));

    // 5. 카테고리 집계와 건수
    let aggregated = manager.search_and_aggregate_by_category().unwrap();
    assert_eq!(aggregated, json!({"과학": 2, "소설/한국": 1, "소설/한국/현대": 1, "과학/물리": 1}));
    assert_eq!(manager.count_by_category("소설/한국", false).unwrap(), 1);
    assert_eq!(manager.count_by_category("소설/한국", true).unwrap(), 2);
    assert_eq!(manager.count_by_categories(&["과학", "소설/한국"], true).unwrap(), json!({"과학": 3, "소설/한국": 2}));
    let covers = manager.aggregate_cover_categories();
    assert_eq!(covers.get("과학").map(|c| c.0), Some(2));
    assert!(!covers.contains_key("과학/물리"), "txt는 커버 형식이 아니다");
    assert_eq!(manager.search_category_pdf_stats("과학").unwrap(), json!({"file_count": 2, "page_count": 700, "total_file_size": 7000}));
    assert_eq!(manager.search_categories_pdf_stats(&["과학", "소설/한국"]).unwrap()["소설/한국"], json!({"file_count": 0, "page_count": 0, "total_file_size": 0}));
    let rows: Vec<(String, String, i64)> = manager.iter_all_category_file_paths(2).collect::<Result<_, _>>().unwrap();
    assert_eq!(rows.len(), 5);
    assert!(rows.contains(&("과학".to_string(), "과학/코스모스.pdf".to_string(), 1)));

    // 6. 키워드 검색(nori 분석기)과 정규화 점수
    let hits = manager.search_by_keyword("코스모스", 10).unwrap();
    assert_eq!(ids(&hits), vec![3]);
    assert_eq!(hits[0].2, 100.0);
    assert!(hits[0].1.get("summary").is_none(), "목록 검색은 summary를 받지 않는다");
    let (paged, total) = manager.search_by_keyword_paged("한국어", 2, 0, None, None).unwrap();
    assert_eq!(total, 5);
    assert_eq!(paged.len(), 2);
    let (filtered, total) = manager.search_by_keyword_paged("한국어", 10, 0, Some(&["과학"]), None).unwrap();
    assert_eq!((filtered.len(), total), (2, 2));
    assert_eq!(manager.search_by_title("토지 제1권", "epub", 1000, 5).unwrap()[0].0, 1);
    assert_eq!(ids(&manager.search_by_category("과학", 10).unwrap()), vec![4, 3]);
    assert_eq!(ids(&manager.search_similar_names("코스모스", "", 5).unwrap()), vec![3]);
    let similar =
        SimilarDocsArgs {
            title: "코스모스 유니버스".into(), author: "칼 세이건".into(), file_size: 3000, summary: "우주에 대한 한국어 요약".into(), exclude_id: Some(3), ..Default::default()
        };
    let (similar_hits, similar_total) = manager.search_similar_docs_paged(&similar).unwrap();
    assert!(similar_total >= 1 && !ids(&similar_hits).contains(&3));
    assert!(similar_hits.iter().all(|h| h.2 <= 100.0));
    assert!(manager.search_similar_docs(&similar).is_ok());
    // 10,000건 초과 요청은 scroll 경로이고, 끝나면 scroll을 지운다.
    assert_eq!(manager.search_by_keyword("한국어", 10_001).unwrap().len(), 5);

    // 7. search_after 페이징: 제목순 한 건씩
    let mut seen = Vec::new();
    let mut cursor: Option<Vec<Value>> = None;
    for _ in 0..4 {
        let (page, total, next) = manager.search_by_category_paged("과학", 1, cursor.as_deref());
        assert_eq!(total, 2);
        seen.extend(ids(&page));
        cursor = next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, vec![4, 3], "title.keyword 오름차순: 엘러건트, 코스모스");
    assert!(cursor.is_none() || manager.search_by_category_paged("과학", 1, cursor.as_deref()).0.is_empty());
    let (latest, latest_total) = manager.search_latest_docs(10, Some(&["소설"]));
    assert_eq!((ids(&latest), latest_total), (vec![4, 3], 2), "txt와 제외 카테고리는 빠진다");
    let (cover_files, cover_total) = manager.search_cover_files_paged(&["과학", "소설/한국"], 10, None, None);
    assert_eq!(cover_total, 3);
    assert_eq!(cover_files.iter().map(|f| f.0).collect::<Vec<_>>(), vec![1, 4, 3], "updated_time 내림차순");
    assert!(!cover_files[0].2.is_empty());
    let reps = manager.search_group_representatives(&["소설", "과학"], None);
    assert_eq!(reps["소설"].iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 2], "file_path 오름차순");
    assert_eq!(reps["과학"].iter().map(|r| r.0).collect::<Vec<_>>(), vec![4, 3]);

    // 8. scroll로 id 열거와 created_time 채우기
    assert_eq!(manager.get_doc_ids_by_path_prefix("과학"), [3, 4, 5].into_iter().collect());
    assert_eq!(manager.get_doc_ids_by_path_prefix("/").len(), 5);
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("과학")).unwrap();
    std::fs::write(tmp.path().join("과학/코스모스.pdf"), "x").unwrap();
    let report = manager.backfill_created_time(tmp.path(), 2);
    assert_eq!((report["updated"].clone(), report["skipped"].clone(), report["failed"].clone()), (json!(1), json!(4), json!(0)));
    assert!(manager.search_by_id(3).unwrap()["created_time_source"].as_str().is_some());

    // 9. 이름 바꾸기, 삭제
    let renamed = manager.rename_category("과학", "자연과학").unwrap();
    assert_eq!(renamed, json!({"updated": 3, "failures": []}));
    assert_eq!(manager.search_by_id(3).unwrap()["category"], json!("자연과학"));
    assert_eq!(manager.search_by_id(5).unwrap()["category"], json!("자연과학/물리"));
    assert_eq!(manager.search_by_id(5).unwrap()["file_path"], json!("자연과학/물리/시간.txt"), "file_path의 접두사도 함께 바뀐다");
    assert_eq!(manager.count_by_category("자연과학", true).unwrap(), 3);
    assert_eq!(manager.delete_by_category("자연과학/물리", false).unwrap(), json!({"deleted": 1, "failures": []}));
    assert_eq!(manager.delete_by_file_paths(&["소설/한국/토지1.epub"], Some(&[1])), 0);
    assert_eq!(manager.delete_by_file_paths(&["소설/한국/토지1.epub"], None), 1);
    assert_eq!(manager.delete_by_ids(&[2, 3], 1), 2);
    manager.refresh().unwrap();
    assert_eq!(manager.count_by_categories(&["자연과학", "소설/한국"], true).unwrap(), json!({"자연과학": 1, "소설/한국": 0}));
    assert!(manager.delete(4).unwrap());
    manager.delete_index().unwrap();
    assert!(!manager.do_exist_index().unwrap());
}
