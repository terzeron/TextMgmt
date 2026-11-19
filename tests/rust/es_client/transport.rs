//! `es_client::transport` 단위 테스트: 요청 이름·펼친 보기, HTTP 응답 해석.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use es_client::EsError;
use es_client::transport::{EsRequest, EsTransport, HttpTransport};
use serde_json::{Value, json};

/// 요청 하나를 받고 `status`와 날 본문 `body`로 응답하는 서버의 주소.
fn serve_raw(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // 요청을 덜 읽고 닫으면 커널이 RST를 보내 클라이언트가 응답 대신 connection reset을 받는다.
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0, "요청 헤더가 끝나기 전에 연결이 닫혔다");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break p + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        while buf.len() < header_end + len {
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0, "요청 본문이 끝나기 전에 연결이 닫혔다");
            buf.extend_from_slice(&chunk[..n]);
        }
        write!(stream, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    url
}

fn transport(url: &str) -> HttpTransport {
    HttpTransport::new(url, "elastic", "pw", "books", Duration::from_secs(5))
}

#[test]
fn request_names_and_params_cover_delete_index_and_multi_get() {
    assert_eq!(EsRequest::DeleteIndex { index: None }.name(), "indices.delete");
    let mget = EsRequest::MultiGet { docs: vec![json!({"_id": "1"})], source: Some(vec!["file_path".into()]) };
    assert_eq!(mget.name(), "mget");
    assert_eq!(mget.params(), json!({"docs": [{"_id": "1"}], "_source": ["file_path"]}));
    // 객체가 아닌 검색 본문은 펼치지 않는다.
    assert_eq!(EsRequest::Search { index: None, body: json!([1]), scroll: None }.params(), json!({}));
}

#[test]
fn only_http_urls_are_sent_and_index_exists_rejects_unexpected_statuses() {
    assert!(matches!(transport("ftp://localhost").send(&EsRequest::Info), Err(EsError::Transport(message)) if message.contains("http")));
    let url = serve_raw(500, "boom");
    assert!(matches!(transport(&url).send(&EsRequest::IndexExists { index: None }), Err(EsError::Status { status: 500, .. })));
}

#[test]
fn an_empty_success_body_is_null() {
    let url = serve_raw(200, "");
    assert_eq!(transport(&url).send(&EsRequest::RefreshIndex { index: None }).unwrap(), Value::Null);
}
