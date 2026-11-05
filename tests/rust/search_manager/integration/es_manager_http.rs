//! `HttpTransport`(실제 HTTP 전송)와 `ESManager`의 통합 테스트.
//!
//! - 로컬 TCP 서버 테스트: 실제 ES 없이 `op`가 어떤 REST 요청(메서드, 경로, 쿼리 문자열, 본문)이 되는지 확인한다.
//! - Docker 테스트(fixture가 컨테이너를 띄운다): 일회용 ES 9.1.0 컨테이너(nori 분석기 설치, 보안 끔)에 붙어 `ESManager`의
//!   주요 동작을 실제로 확인한다. `cargo test -p backend --test es_manager_http`
//!   이미지는 환경 변수 `TM_TEST_ES_IMAGE`로 바꿀 수 있다(기본 `docker.elastic.co/elasticsearch/elasticsearch:9.1.0`).
//!   컨테이너는 테스트가 끝나면(실패해도) `docker rm -f`로 지운다. 운영 ES와 `.env`는 쓰지 않는다.
//!
//! 가정한 API:
//! - `search_manager::HttpTransport::new(url: &str, user: &str, password: &str, index: &str, timeout: Duration)` (`EsTransport` 구현, http 전용)
//! - `search_manager::RealEsConnector`(`EsConnector` 구현)와 `ESManager::connect`

#[path = "../common/mod.rs"]
mod common;

#[path = "../../shared/tls_server.rs"]
mod tls_server;
use crate::common::transport;
use search_manager::{Conflicts, ESManager, EsError, EsRequest, EsTransport, HttpTransport, RealEsConnector};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
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

#[test]
fn search_becomes_post_search_with_body_fields_and_scroll_query() {
    let (url, rx) = serve(vec![(200, json!({"hits": {"hits": []}}).to_string())]);
    let body = json!({"query": {"match_all": {}}, "from": 2, "size": 3, "_source": ["title"], "track_scores": true, "sort": [{"file_path": "asc"}]});

    let response = transport(&url).send(&EsRequest::Search { index: Some("books".into()), body, scroll: Some("10m".into()) }).unwrap();

    assert_eq!(response, json!({"hits": {"hits": []}}));
    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "POST /books/_search?scroll=10m HTTP/1.1");
    // Basic 인증과 JSON 본문.
    assert_eq!(request.header("authorization"), Some("Basic ZWxhc3RpYzpwdw=="));
    assert!(request.header("content-type").is_some_and(|v| v.starts_with("application/json")));
    assert_eq!(request.json(), json!({"query": {"match_all": {}}, "from": 2, "size": 3, "_source": ["title"], "track_scores": true, "sort": [{"file_path": "asc"}]}));
}

#[test]
fn search_without_scroll_has_no_query_string() {
    let (url, rx) = serve(vec![(200, "{}".to_string())]);
    transport(&url).send(&EsRequest::Search { index: Some("books".into()), body: json!({"size": 1, "aggs": {"a": {"terms": {"field": "category"}}}}), scroll: None }).unwrap();
    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "POST /books/_search HTTP/1.1");
    assert_eq!(request.json(), json!({"size": 1, "aggs": {"a": {"terms": {"field": "category"}}}}));
}

#[test]
fn bulk_becomes_ndjson_post() {
    let (url, rx) = serve(vec![(200, json!({"errors": false, "items": []}).to_string())]);
    let body = json!([{"index": {"_index": "books", "_id": "1"}}, {"title": "한글 제목"}]);

    transport(&url).send(&EsRequest::Bulk { lines: body.as_array().unwrap().clone(), timeout: Some("60s".into()), refresh: Some(false) }).unwrap();

    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "POST /_bulk?timeout=60s&refresh=false HTTP/1.1");
    assert!(request.header("content-type").is_some_and(|v| v.starts_with("application/x-ndjson")));
    assert_eq!(request.body, "{\"index\":{\"_index\":\"books\",\"_id\":\"1\"}}\n{\"title\":\"한글 제목\"}\n");
}

#[test]
fn msearch_becomes_ndjson_post() {
    let (url, rx) = serve(vec![(200, json!({"responses": []}).to_string()), (200, json!({"responses": []}).to_string())]);
    let searches = json!([{}, {"query": {"match_all": {}}, "size": 1}]);

    let searches = searches.as_array().unwrap().clone();
    transport(&url).send(&EsRequest::MultiSearch { index: Some("books".into()), searches: searches.clone() }).unwrap();
    transport(&url).send(&EsRequest::MultiSearch { index: None, searches }).unwrap();

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

    let books = || Some("books".into());
    t.send(&EsRequest::Get { index: books(), id: "42".into() }).unwrap();
    t.send(&EsRequest::Update { index: books(), id: "42".into(), body: json!({"doc": {"title": "t"}}), refresh: Some(true) }).unwrap();
    t.send(&EsRequest::Delete { index: books(), id: "42".into(), refresh: Some(true) }).unwrap();
    t.send(&EsRequest::Count { index: books(), body: json!({"query": {"term": {"category": "A"}}}) }).unwrap();
    t.send(&EsRequest::DeleteByQuery { index: books(), body: json!({"query": {"ids": {"values": ["1"]}}}), conflicts: Some(Conflicts::Proceed), refresh: Some(false) }).unwrap();
    t.send(&EsRequest::UpdateByQuery { index: books(), body: json!({"query": {"match_all": {}}, "script": {"source": "ctx.op='noop'"}}), conflicts: Some(Conflicts::Abort), refresh: Some(true) })
        .unwrap();
    t.send(&EsRequest::RefreshIndex { index: books() }).unwrap();
    t.send(&EsRequest::GetMapping { index: books() }).unwrap();
    t.send(&EsRequest::PutMapping { index: books(), properties: json!({"publisher": {"type": "keyword"}}) }).unwrap();
    t.send(&EsRequest::MultiGet { docs: vec![json!({"_index": "books", "_id": "1"})], source: Some(vec!["file_path".into(), "title".into()]) }).unwrap();
    t.send(&EsRequest::Scroll { scroll_id: "abc".into(), scroll: "10m".into() }).unwrap();

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
    transport(&url).send(&EsRequest::ClearScroll { scroll_id: "abc".into() }).unwrap();
    let request = rx.recv().unwrap();
    assert_eq!(request.request_line, "DELETE /_search/scroll HTTP/1.1");
    assert_eq!(request.json(), json!({"scroll_id": "abc"}));
}

#[test]
fn indices_exists_uses_head_and_maps_404_to_false() {
    let (url, rx) = serve(vec![(200, String::new()), (404, String::new())]);
    let t = transport(&url);
    assert_eq!(t.send(&EsRequest::IndexExists { index: Some("books".into()) }).unwrap(), json!(true));
    assert_eq!(t.send(&EsRequest::IndexExists { index: Some("books".into()) }).unwrap(), json!(false));
    assert_eq!(rx.recv().unwrap().request_line, "HEAD /books HTTP/1.1");
}

#[test]
fn error_status_becomes_status_error_with_body() {
    let (url, _rx) = serve(vec![(404, json!({"found": false}).to_string()), (400, json!({"error": {"type": "parsing_exception"}}).to_string())]);
    let t = transport(&url);
    match t.send(&EsRequest::Get { index: Some("books".into()), id: "1".into() }) {
        Err(EsError::Status { status: 404, body }) => assert!(body.contains("found")),
        other => panic!("404를 기대했다: {other:?}"),
    }
    match t.send(&EsRequest::Search { index: Some("books".into()), body: json!({"query": {"bad": {}}}), scroll: None }) {
        Err(e @ EsError::Status { status: 400, .. }) => assert!(!e.is_not_found()),
        other => panic!("400을 기대했다: {other:?}"),
    }
}

#[test]
fn non_json_response_is_parse_error_and_refused_connection_is_transport_error() {
    let (url, _rx) = serve(vec![(200, "<html>proxy</html>".to_string())]);
    assert!(matches!(transport(&url).send(&EsRequest::Info), Err(EsError::Parse(_))));

    // 방금 닫은 포트로 연결한다.
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    assert!(matches!(transport(&closed).send(&EsRequest::Info), Err(EsError::Transport(_))));
}

#[test]
fn index_name_comes_from_constructor_and_trailing_slash_is_trimmed() {
    let (url, rx) = serve(vec![(200, "{}".to_string())]);
    let t = HttpTransport::new(&format!("{url}/"), "", "", "my_books", Duration::from_secs(5));
    assert_eq!(t.index_name(), "my_books");
    t.send(&EsRequest::Info).unwrap();
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

mod real_transport {
    use super::tls_server;
    use crate::common::transport;
    use search_manager::{
        BackfillReport, CoverFileHits, DeleteByQueryResult, ESManager, EsError, EsRequest, EsTransport, Hit, HttpTransport, PagedHits, PdfStats, SimilarDocsArgs, UpdateByQueryResult, UpdateFields,
    };
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, HashMap};
    use std::process::Command;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

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

    /// nori 를 미리 설치한 ES 9.1.0 이미지. 없을 때 한 번만 만든다. 컨테이너마다 플러그인을 내려받아 설치하면 실행마다 수십 초가 든다.
    const NORI_IMAGE: &str = "elasticsearch-nori:9.1.0";

    fn ensure_nori_image() {
        if Command::new("docker").args(["image", "inspect", NORI_IMAGE]).output().is_ok_and(|o| o.status.success()) {
            return;
        }
        let mut build = Command::new("docker").args(["build", "--tag", NORI_IMAGE, "-"]).stdin(std::process::Stdio::piped()).spawn().expect("docker build 실행");
        std::io::Write::write_all(
            &mut build.stdin.take().expect("docker build stdin"),
            b"FROM docker.elastic.co/elasticsearch/elasticsearch:9.1.0\nRUN bin/elasticsearch-plugin install analysis-nori --batch\n",
        )
        .expect("Dockerfile 전달");
        assert!(build.wait().expect("docker build 결과").success(), "nori ES 9.1.0 이미지 빌드 실패");
    }

    /// nori 분석기를 설치한 ES를 띄우고 응답할 때까지 기다린다.
    /// `TM_TEST_ES_IMAGE` 로 다른 이미지를 고르면 예전처럼 컨테이너 안에서 nori 를 설치한다.
    fn start_es() -> EsContainer {
        let custom = std::env::var("TM_TEST_ES_IMAGE").ok();
        if custom.is_none() {
            ensure_nori_image();
        }
        let name = format!("tm-es-manager-http-{}", std::process::id());
        let _ = Command::new("docker").args(["rm", "-f", &name]).output();
        let mut args: Vec<&str> = vec![
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
        ];
        match custom.as_deref() {
            Some(image) => args.extend(["--entrypoint", "sh", image, "-c", "elasticsearch-plugin install --batch analysis-nori && exec /usr/local/bin/docker-entrypoint.sh eswrapper"]),
            None => args.push(NORI_IMAGE),
        }
        docker(&args);
        let mut container = EsContainer { name: name.clone(), url: String::new() };
        let port = docker(&["port", &name, "9200/tcp"]).lines().next().and_then(|l| l.rsplit(':').next()).map(str::to_string).expect("호스트 포트");
        container.url = format!("http://127.0.0.1:{port}");
        let probe = transport(&container.url);
        let deadline = Instant::now() + Duration::from_secs(240);
        while Instant::now() < deadline {
            if probe.send(&EsRequest::Info).is_ok() {
                return container;
            }
            thread::sleep(Duration::from_millis(250));
        }
        let logs = docker(&["logs", "--tail", "40", &name]);
        panic!("ES가 시간 안에 뜨지 않았다:\n{logs}");
    }

    #[allow(clippy::too_many_arguments)]
    fn book(category: &str, title: &str, author: &str, path: &str, file_type: &str, size: i64, pages: i64, updated: &str) -> Value {
        json!({"category": category, "title": title, "author": author, "file_path": path, "file_type": file_type, "file_size": size, "page_count": pages,
               "summary": format!("{title}에 대한 한국어 요약이다. {author}가 썼다."), "updated_time": updated})
    }

    fn ids(hits: &[Hit]) -> Vec<i64> {
        hits.iter().map(|h| h.id).collect()
    }

    #[test]
    fn https_uses_the_configured_ca() {
        let (url, server) = tls_server::serve_tls_once();
        let ca_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/rust/shared/tls/local-es.crt");
        let t = HttpTransport::new_with_ca(&url, "u", "p", "books", Duration::from_secs(1), Some(&ca_path));
        assert_eq!(t.send(&EsRequest::Info).unwrap()["version"]["number"], "9.1.0");
        server.join().unwrap();

        let (untrusted_url, untrusted_server) = tls_server::serve_tls_once();
        let untrusted = HttpTransport::new(&untrusted_url, "u", "p", "books", Duration::from_secs(1));
        assert!(matches!(untrusted.send(&EsRequest::Info), Err(EsError::Transport(_))));
        untrusted_server.join().unwrap();
    }

    #[test]
    fn docker_es_manager_flow_against_real_elasticsearch() {
        let container = start_es();
        let es: Arc<dyn EsTransport> = Arc::new(HttpTransport::new(&container.url, "", "", "tm_books", Duration::from_secs(30)));
        let manager = ESManager::with_transport(es.clone());
        // 색인 존재 확인·조회·삭제는 loader와 같은 es_client 구현을 같은 전송 위에서 쓴다.
        let store = es_client::EsClient::from_transport(es.clone());
        use es_client::indexing::IndexStore;

        // 1. create_index: nori 매핑 포함, 두 번째 호출은 기존 색인을 점검만 한다.
        assert!(!store.index_exists().unwrap());
        assert_eq!(manager.create_index().unwrap()["acknowledged"], json!(true));
        assert!(store.index_exists().unwrap());
        let mapping = es.send(&EsRequest::GetMapping { index: Some("tm_books".into()) }).unwrap();
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
        assert_eq!(manager.insert(&docs).unwrap(), vec![1, 2, 3, 4, 5]);
        manager.refresh().unwrap();

        // 3. search_by_id
        assert_eq!(manager.search_by_id(1).unwrap()["title"], json!("토지 첫째 권"));
        assert_eq!(manager.search_by_id(999).unwrap(), json!({}));
        assert_eq!(store.get_existing_paths(&[1, 999]).unwrap(), HashMap::from([(1, "소설/한국/토지1.epub".to_string())]));

        // 4. update, delete, 오류 상태 코드
        assert!(manager.update(1, &UpdateFields { title: "토지 제1권".into(), updated_time: "2026-02-01T00:00:00".into(), ..Default::default() }).unwrap());
        assert_eq!(manager.search_by_id(1).unwrap()["title"], json!("토지 제1권"));
        assert!(matches!(manager.update(999, &UpdateFields { title: "x".into(), ..Default::default() }), Err(EsError::Status { status: 404, .. })));
        assert!(matches!(manager.delete(999), Err(EsError::Status { status: 404, .. })));
        assert!(matches!(es.send(&EsRequest::Search { index: Some("tm_books".into()), body: json!({"query": {"no_such_query": {}}}), scroll: None }), Err(EsError::Status { status: 400, .. })));

        // 5. 카테고리 집계와 건수
        let aggregated = manager.search_and_aggregate_by_category().unwrap();
        assert_eq!(aggregated, BTreeMap::from([("과학".to_string(), 2), ("소설/한국".to_string(), 1), ("소설/한국/현대".to_string(), 1), ("과학/물리".to_string(), 1)]));
        assert_eq!(manager.count_by_category("소설/한국", false).unwrap(), 1);
        assert_eq!(manager.count_by_category("소설/한국", true).unwrap(), 2);
        assert_eq!(manager.count_by_categories(&["과학", "소설/한국"], true).unwrap(), BTreeMap::from([("과학".to_string(), 3), ("소설/한국".to_string(), 2)]));
        let covers = manager.aggregate_cover_categories();
        assert_eq!(covers.get("과학").map(|c| c.0), Some(2));
        assert!(!covers.contains_key("과학/물리"), "txt는 커버 형식이 아니다");
        assert_eq!(manager.search_category_pdf_stats("과학").unwrap(), PdfStats { file_count: 2, page_count: 700, total_file_size: 7000 });
        assert_eq!(manager.search_categories_pdf_stats(&["과학", "소설/한국"]).unwrap()["소설/한국"], PdfStats::default());
        let rows: Vec<(String, String, i64)> = manager.iter_all_category_file_paths(2).collect::<Result<_, _>>().unwrap();
        assert_eq!(rows.len(), 5);
        assert!(rows.contains(&("과학".to_string(), "과학/코스모스.pdf".to_string(), 1)));

        // 6. 키워드 검색(nori 분석기)과 정규화 점수
        let hits = manager.search_by_keyword("코스모스", 10).unwrap();
        assert_eq!(ids(&hits), vec![3]);
        assert_eq!(hits[0].score, 100.0);
        assert!(hits[0].source.get("summary").is_none(), "목록 검색은 summary를 받지 않는다");
        let (paged, total) = manager.search_by_keyword_paged("한국어", 2, 0, None, None).unwrap();
        assert_eq!(total, 5);
        assert_eq!(paged.len(), 2);
        let (filtered, total) = manager.search_by_keyword_paged("한국어", 10, 0, Some(&["과학"]), None).unwrap();
        assert_eq!((filtered.len(), total), (2, 2));
        assert_eq!(manager.search_by_title("토지 제1권", "epub", 1000, 5).unwrap()[0].id, 1);
        assert_eq!(ids(&manager.search_by_category("과학", 10).unwrap()), vec![4, 3]);
        assert_eq!(ids(&manager.search_similar_names("코스모스", "", 5).unwrap()), vec![3]);
        let similar =
            SimilarDocsArgs {
                title: "코스모스 유니버스".into(), author: "칼 세이건".into(), file_size: 3000, summary: "우주에 대한 한국어 요약".into(), exclude_id: Some(3), ..Default::default()
            };
        let (similar_hits, similar_total) = manager.search_similar_docs_paged(&similar).unwrap();
        assert!(similar_total >= 1 && !ids(&similar_hits).contains(&3));
        assert!(similar_hits.iter().all(|h| h.score <= 100.0));
        assert!(manager.search_similar_docs(&similar).is_ok());
        // 10,000건 초과 요청은 scroll 경로이고, 끝나면 scroll을 지운다.
        assert_eq!(manager.search_by_keyword("한국어", 10_001).unwrap().len(), 5);

        // 7. search_after 페이징: 제목순 한 건씩
        let mut seen = Vec::new();
        let mut cursor: Option<Vec<Value>> = None;
        for _ in 0..4 {
            let PagedHits { hits: page, total, search_after: next } = manager.search_by_category_paged("과학", 1, cursor.as_deref());
            assert_eq!(total, 2);
            seen.extend(ids(&page));
            cursor = next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, vec![4, 3], "title.keyword 오름차순: 엘러건트, 코스모스");
        assert!(cursor.is_none() || manager.search_by_category_paged("과학", 1, cursor.as_deref()).hits.is_empty());
        let (latest, latest_total) = manager.search_latest_docs(10, Some(&["소설"]));
        assert_eq!((ids(&latest), latest_total), (vec![4, 3], 2), "txt와 제외 카테고리는 빠진다");
        let CoverFileHits { hits: cover_files, total: cover_total } = manager.search_cover_files_paged(&["과학", "소설/한국"], 10, None, None);
        assert_eq!(cover_total, 3);
        assert_eq!(cover_files.iter().map(|f| f.id).collect::<Vec<_>>(), vec![1, 4, 3], "updated_time 내림차순");
        assert!(!cover_files[0].sort.is_empty());
        let reps = manager.search_group_representatives(&["소설", "과학"], None);
        assert_eq!(reps["소설"].iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 2], "file_path 오름차순");
        assert_eq!(reps["과학"].iter().map(|r| r.0).collect::<Vec<_>>(), vec![4, 3]);

        // 8. scroll로 id 열거와 created_time 채우기
        assert_eq!(store.get_doc_ids_by_path_prefix("과학").unwrap(), [3, 4, 5].into_iter().collect());
        assert_eq!(store.get_doc_ids_by_path_prefix("/").unwrap().len(), 5);
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("과학")).unwrap();
        std::fs::write(tmp.path().join("과학/코스모스.pdf"), "x").unwrap();
        let report = manager.backfill_created_time(tmp.path(), 2);
        assert_eq!(report, BackfillReport { updated: 1, skipped: 4, failed: 0 });
        assert!(manager.search_by_id(3).unwrap()["created_time_source"].as_str().is_some());

        // 9. 이름 바꾸기, 삭제
        let renamed = manager.rename_category("과학", "자연과학").unwrap();
        assert_eq!(renamed, UpdateByQueryResult { updated: 3, failures: vec![] });
        assert_eq!(manager.search_by_id(3).unwrap()["category"], json!("자연과학"));
        assert_eq!(manager.search_by_id(5).unwrap()["category"], json!("자연과학/물리"));
        assert_eq!(manager.search_by_id(5).unwrap()["file_path"], json!("자연과학/물리/시간.txt"), "file_path의 접두사도 함께 바뀐다");
        assert_eq!(manager.count_by_category("자연과학", true).unwrap(), 3);
        assert_eq!(manager.delete_by_category("자연과학/물리", false).unwrap(), DeleteByQueryResult { deleted: 1, failures: vec![] });
        assert_eq!(manager.delete_by_file_paths(&["소설/한국/토지1.epub".to_string()], Some(&[1])).unwrap(), 0);
        assert_eq!(manager.delete_by_file_paths(&["소설/한국/토지1.epub".to_string()], None).unwrap(), 1);
        assert_eq!(manager.delete_by_ids(&[2, 3], 1).unwrap(), 2);
        manager.refresh().unwrap();
        assert_eq!(manager.count_by_categories(&["자연과학", "소설/한국"], true).unwrap(), BTreeMap::from([("자연과학".to_string(), 1), ("소설/한국".to_string(), 0)]));
        assert!(manager.delete(4).unwrap());
        store.delete_index().unwrap();
        assert!(!store.index_exists().unwrap());
    }
}
