use std::io::{Read, Write};

use bookstore::{Fetcher, HttpFetcher, Store};

fn mock_http(response: &'static [u8]) -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/page", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).unwrap();
        stream.write_all(response).unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });
    (url, handle)
}

#[test]
fn http_fetcher_sends_the_store_headers_and_decodes_utf8_leniently() {
    let (url, server) = mock_http(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 6\r\nConnection: close\r\n\r\nhi \xff!!");
    let html = HttpFetcher::new().get(Store::Kyobo, &url).unwrap();
    assert_eq!(html, "hi \u{fffd}!!");
    let request = server.join().unwrap().to_lowercase();
    assert!(request.contains("user-agent: mozilla/5.0"), "{request}");
    assert!(request.contains("referer: https://www.kyobobook.co.kr/"), "{request}");
    assert!(request.contains("accept-language: ko-kr"), "{request}");

    let (url, server) = mock_http(b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
    assert_eq!(HttpFetcher::new().get(Store::Yes24, &url).unwrap(), "nope", "상태 코드와 무관하게 본문을 쓴다(원본과 같다)");
    let request = server.join().unwrap().to_lowercase();
    assert!(request.contains("user-agent: curl/7.79.1") && request.contains("referer: https://www.yes24.com"), "{request}");
}

#[test]
fn http_fetcher_reports_connection_errors() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/x", listener.local_addr().unwrap());
    drop(listener);
    assert!(HttpFetcher::new().get(Store::Yes24, &url).is_err());
}
