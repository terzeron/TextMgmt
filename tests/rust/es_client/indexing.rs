use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use es_client::indexing::{IndexStore, index_mappings, index_settings, path_prefix_query};
use es_client::{EsClient, EsError};
use serde_json::{Value, json};

#[path = "../common/tls_server.rs"]
mod tls_server;

#[derive(Debug, Clone)]
struct Req {
    method: String,
    path: String,
    body: String,
    content_type: String,
}

impl Req {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

struct Mock {
    url: String,
    seen: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Mock {
    fn requests(&self) -> Vec<Req> {
        self.seen.lock().unwrap().clone()
    }

    fn client(&self) -> EsClient {
        EsClient::new(&self.url, "u", "p", "books")
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Req> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 && data.is_empty() {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&data).into_owned();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let wanted = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
            if body.len() >= wanted {
                let mut first = head.lines().next().unwrap().split(' ');
                let content_type = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-type:").map(|v| v.trim().to_owned())).unwrap_or_default();
                return Some(Req { method: first.next().unwrap().to_owned(), path: first.next().unwrap().to_owned(), body: body.to_owned(), content_type });
            }
        }
        if n == 0 {
            return None;
        }
    }
}

fn mock(handler: impl Fn(&Req) -> (u16, String) + Send + 'static) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (seen2, stop2) = (seen.clone(), stop.clone());
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stop2.load(Ordering::SeqCst) {
                break;
            }
            let Ok(mut stream) = stream else { continue };
            let Some(req) = read_request(&mut stream) else { continue };
            let (status, body) = handler(&req);
            seen2.lock().unwrap().push(req);
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    Mock { url, seen, stop, handle: Some(handle) }
}

fn ok(v: Value) -> (u16, String) {
    (200, v.to_string())
}

#[test]
fn settings_and_mappings_describe_the_nori_index() {
    let settings = index_settings();
    assert_eq!(settings["analysis"]["analyzer"]["nori_analyzer"]["tokenizer"], "nori_tokenizer");
    assert_eq!(settings["analysis"]["filter"]["nori_posfilter"]["stoptags"].as_array().unwrap().len(), 26);
    assert_eq!(settings["index"]["similarity"]["default"]["type"], "BM25");
    let props = &index_mappings()["properties"];
    assert_eq!(props["file_path"]["type"], "keyword");
    assert_eq!(props["file_size"]["type"], "unsigned_long");
    assert_eq!(props["category"]["fields"]["nori"]["analyzer"], "nori_analyzer");
    assert_eq!(props["publisher"]["fields"]["keyword"]["type"], "keyword");
    assert_eq!(props.as_object().unwrap().len(), 14);
}

#[test]
fn path_prefix_query_matches_the_path_itself_and_everything_below() {
    assert_eq!(path_prefix_query(""), json!({"match_all": {}}));
    assert_eq!(path_prefix_query("."), json!({"match_all": {}}));
    assert_eq!(path_prefix_query("/"), json!({"match_all": {}}));
    assert_eq!(path_prefix_query("/3_판타지/"), json!({"bool": {"should": [{"term": {"file_path": "3_판타지"}}, {"prefix": {"file_path": "3_판타지/"}}], "minimum_should_match": 1}}));
}

#[test]
fn create_index_puts_settings_and_mappings_when_absent() {
    let m = mock(|r| if r.method == "GET" { (404, r#"{"error":"index_not_found_exception"}"#.into()) } else { ok(json!({"acknowledged": true})) });
    m.client().create_index().unwrap();
    let seen = m.requests();
    assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("GET", "/books"));
    assert_eq!((seen[1].method.as_str(), seen[1].path.as_str()), ("PUT", "/books"));
    let body = seen[1].json();
    assert_eq!(body["settings"], index_settings());
    assert_eq!(body["mappings"], index_mappings());
}

#[test]
fn create_index_adds_only_the_missing_fields_to_an_existing_index() {
    let old = json!({"books": {"mappings": {"properties": {"category": {"type": "keyword"}, "title": {"type": "text"}}}}});
    let m = mock(move |r| if r.path.ends_with("/_mapping") && r.method == "GET" { ok(old.clone()) } else { ok(json!({"acknowledged": true})) });
    m.client().create_index().unwrap();
    let puts: Vec<Value> = m.requests().iter().filter(|r| r.method == "PUT").map(Req::json).collect();
    assert_eq!(puts.len(), 3);
    assert!(puts[0]["properties"]["category"]["fields"]["nori"].is_object());
    assert_eq!(puts[1]["properties"].as_object().unwrap().keys().cloned().collect::<Vec<_>>(), ["created_time", "created_time_source"]);
    assert!(puts[2]["properties"]["publisher"].is_object());

    let full = json!({"books": {"mappings": {"properties": index_mappings()["properties"].clone()}}});
    let m2 = mock(move |r| if r.path.ends_with("/_mapping") && r.method == "GET" { ok(full.clone()) } else { ok(json!({})) });
    m2.client().create_index().unwrap();
    assert!(m2.requests().iter().all(|r| r.method != "PUT"), "빠진 필드가 없으면 매핑을 건드리지 않는다");
}

#[test]
fn create_index_tolerates_a_creation_race() {
    let m = mock(|r| match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/books") => (404, "{}".into()),
        ("PUT", "/books") => (400, r#"{"error":{"type":"resource_already_exists_exception"}}"#.into()),
        ("GET", p) if p.ends_with("/_mapping") => ok(json!({"books": {"mappings": {"properties": index_mappings()["properties"].clone()}}})),
        _ => ok(json!({})),
    });
    m.client().create_index().unwrap();

    let m = mock(|r| if r.method == "PUT" { (400, r#"{"error":"illegal_argument_exception"}"#.into()) } else { (404, "{}".into()) });
    assert!(matches!(m.client().create_index().unwrap_err(), EsError::Status { status: 400, .. }));
}

#[test]
fn delete_index_only_deletes_an_existing_index() {
    let m = mock(|r| if r.method == "GET" { ok(json!({})) } else { ok(json!({"acknowledged": true})) });
    m.client().delete_index().unwrap();
    assert_eq!(m.requests().iter().map(|r| r.method.as_str()).collect::<Vec<_>>(), ["GET", "DELETE"]);
    let m = mock(|_| (404, "{}".into()));
    assert!(!m.client().index_exists().unwrap());
    m.client().delete_index().unwrap();
    assert_eq!(m.requests().len(), 2, "없으면 지우지 않는다");
    assert!(m.client().ping().is_err());
}

#[test]
fn existing_paths_and_created_time_metadata_come_from_mget() {
    let m = mock(|r| {
        assert!(r.path.starts_with("/_mget?_source="), "{}", r.path);
        if r.path.contains("file_path") {
            ok(json!({"docs": [
                {"_id": "10", "found": true, "_source": {"file_path": "a/b.txt"}},
                {"_id": "11", "found": false},
                {"_id": "12", "found": true, "_source": {"file_path": "c.epub"}},
            ]}))
        } else {
            ok(json!({"docs": [
                {"_id": "10", "found": true, "_source": {"created_time": "2020-01-01T00:00:00", "updated_time": "2021-01-01T00:00:00", "other": 1}},
                {"_id": "11", "found": true, "_source": {}},
                {"_id": "12", "found": false},
            ]}))
        }
    });
    let c = m.client();
    let paths = c.get_existing_paths(&[10, 11, 12]).unwrap();
    assert_eq!((paths.len(), paths[&10].as_str(), paths[&12].as_str()), (2, "a/b.txt", "c.epub"));
    let meta = c.get_existing_created_time_metadata(&[10, 11, 12]).unwrap();
    assert_eq!(meta.len(), 1);
    assert_eq!(meta[&10]["created_time"], "2020-01-01T00:00:00");
    assert!(!meta[&10].contains_key("other") && !meta[&10].contains_key("created_time_source"));
    let body = m.requests()[0].json();
    assert_eq!(body["docs"][0], json!({"_index": "books", "_id": "10"}));
    assert!(c.get_existing_paths(&[]).unwrap().is_empty());
    assert!(c.get_existing_created_time_metadata(&[]).unwrap().is_empty());
    assert_eq!(m.requests().len(), 2, "빈 목록은 요청하지 않는다");
}

#[test]
fn delete_by_file_paths_excludes_ids_and_swallows_errors() {
    let m = mock(|_| ok(json!({"deleted": 4})));
    let c = m.client();
    assert_eq!(c.delete_by_file_paths(&["a".into(), "b".into()], &[1, 2]), 4);
    let r = &m.requests()[0];
    assert_eq!(r.path, "/books/_delete_by_query?conflicts=proceed&refresh=false");
    assert_eq!(r.json()["query"], json!({"bool": {"must": [{"terms": {"file_path": ["a", "b"]}}], "must_not": [{"ids": {"values": ["1", "2"]}}]}}));
    assert_eq!(c.delete_by_file_paths(&[], &[]), 0);
    assert_eq!(m.requests().len(), 1);
    c.delete_by_file_paths(&["x".into()], &[]);
    assert!(m.requests()[1].json()["query"]["bool"].get("must_not").is_none());

    let failing = mock(|_| (500, "{}".into()));
    assert_eq!(failing.client().delete_by_file_paths(&["a".into()], &[]), 0);
}

#[test]
fn doc_ids_by_path_prefix_scroll_through_all_pages_and_clear_the_scroll() {
    let page = Arc::new(Mutex::new(0));
    let m = mock(move |r| match (r.method.as_str(), r.path.as_str()) {
        ("POST", p) if p.contains("_search?scroll=") => ok(json!({"_scroll_id": "S1", "hits": {"hits": [{"_id": "1"}, {"_id": "2"}]}})),
        ("POST", "/_search/scroll") => {
            let mut n = page.lock().unwrap();
            *n += 1;
            if *n == 1 { ok(json!({"_scroll_id": "S2", "hits": {"hits": [{"_id": "3"}]}})) } else { ok(json!({"_scroll_id": "S2", "hits": {"hits": []}})) }
        }
        _ => ok(json!({})),
    });
    let ids = m.client().get_doc_ids_by_path_prefix("3_판타지");
    assert_eq!(ids, [1, 2, 3].into_iter().collect());
    let seen = m.requests();
    assert_eq!(seen[0].json()["_source"], json!(false));
    assert_eq!(seen[0].json()["size"], 5000);
    assert_eq!(seen[1].json(), json!({"scroll": "10m", "scroll_id": "S1"}));
    let last = seen.last().unwrap();
    assert_eq!((last.method.as_str(), last.path.as_str()), ("DELETE", "/_search/scroll"));
    assert_eq!(last.json()["scroll_id"], "S2");

    let failing = mock(|_| (500, "{}".into()));
    assert!(failing.client().get_doc_ids_by_path_prefix("x").is_empty());
}

#[test]
fn delete_by_ids_chunks_requests() {
    let m = mock(|_| ok(json!({"deleted": 7})));
    let ids: Vec<i64> = (0..10_001).collect();
    assert_eq!(m.client().delete_by_ids(&ids), 14);
    let seen = m.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].json()["query"]["ids"]["values"].as_array().unwrap().len(), 10_000);
    assert_eq!(seen[1].json()["query"]["ids"]["values"].as_array().unwrap().len(), 1);
    assert_eq!(m.client().delete_by_ids(&[]), 0);
    let failing = mock(|_| (500, "{}".into()));
    assert_eq!(failing.client().delete_by_ids(&[1]), 0);
}

#[test]
fn insert_sends_ndjson_in_chunks_of_one_hundred() {
    let m = mock(|_| ok(json!({"errors": false, "items": []})));
    let docs: Vec<(i64, Value)> = (0..250).map(|i| (i, json!({"title": format!("책 {i}"), "page_count": 1}))).collect();
    m.client().insert(&docs).unwrap();
    let seen = m.requests();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].path, "/_bulk?timeout=60s&refresh=false");
    assert!(seen[0].content_type.starts_with("application/x-ndjson"), "{}", seen[0].content_type);
    let lines: Vec<&str> = seen[0].body.lines().collect();
    assert_eq!(lines.len(), 200);
    assert_eq!(serde_json::from_str::<Value>(lines[0]).unwrap(), json!({"index": {"_index": "books", "_id": "0"}}));
    assert_eq!(serde_json::from_str::<Value>(lines[1]).unwrap()["title"], "책 0");
    assert!(seen[0].body.ends_with('\n'));
    assert_eq!(seen[2].body.lines().count(), 100);
    m.client().insert(&[]).unwrap();
    assert_eq!(m.requests().len(), 3);
}

#[test]
fn insert_reports_item_errors_but_keeps_going_and_fails_on_http_errors() {
    let m = mock(|_| ok(json!({"errors": true, "items": [{"index": {"error": {"type": "mapper_parsing_exception"}}}]})));
    m.client().insert(&[(1, json!({"a": 1}))]).unwrap();
    let failing = mock(|_| (500, r#"{"error":"boom"}"#.into()));
    assert!(matches!(failing.client().insert(&[(1, json!({}))]).unwrap_err(), EsError::Status { status: 500, .. }));
    assert_eq!(failing.requests().len(), 1, "HTTP 오류는 다시 시도하지 않는다");
}

#[test]
fn insert_retries_transport_errors_with_backoff_then_gives_up() {
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let waits = std::cell::RefCell::new(Vec::new());
    let c = EsClient::new(&dead, "u", "p", "books");
    let err = c.insert_with(&[(1, json!({}))], &|s| waits.borrow_mut().push(s)).unwrap_err();
    assert!(matches!(err, EsError::Transport(_)));
    assert_eq!(*waits.borrow(), [1, 2], "세 번째 실패 뒤에는 쉬지 않는다");
}

#[test]
fn refresh_posts_to_the_refresh_endpoint() {
    let m = mock(|_| ok(json!({"_shards": {}})));
    m.client().refresh().unwrap();
    let r = &m.requests()[0];
    assert_eq!((r.method.as_str(), r.path.as_str()), ("POST", "/books/_refresh"));
    assert_eq!(m.client().index_name(), "books");
}

#[test]
fn bulk_insert_uses_verified_https_with_configured_ca() {
    let (url, server) = tls_server::serve_tls_once();
    let ca = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/rust/common/tls/local-es.crt");
    let client = EsClient::new_with_ca(&url, "u", "p", "books", Some(&ca)).unwrap();
    client.insert(&[(1, json!({"title": "책"}))]).unwrap();
    server.join().unwrap();
}
