#[path = "../common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use backend::cover_sources::SearchHooks;

#[test]
fn default_search_hooks_fetch_cover_bytes_over_http() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap()).read_line(&mut request).unwrap();
        assert!(request.starts_with("GET /cover.jpg HTTP/1.1"), "{request}");
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\ncover").unwrap();
    });
    let hooks = SearchHooks::default().with_validate(|_store, url| Ok(url.to_string()));
    let bytes = hooks.fetch_bytes("yes24", &format!("http://{address}/cover.jpg"), Some(16)).unwrap();
    assert_eq!(bytes, b"cover");
    server.join().unwrap();
}

#[test]
fn default_cover_session_returns_redirect_for_each_target_to_be_validated() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap()).read_line(&mut request).unwrap();
        assert!(request.starts_with("GET /redirect HTTP/1.1"), "{request}");
        stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: http://{address}/unsafe\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    });
    let initial = format!("http://{address}/redirect");
    let checked = Arc::new(Mutex::new(Vec::new()));
    let checked_by_validator = checked.clone();
    let hooks = SearchHooks::default().with_validate(move |_store, url| {
        checked_by_validator.lock().unwrap().push(url.to_string());
        if url == initial { Ok(url.to_string()) } else { Err(backend::cover_sources::ValueError("redirect target blocked".into())) }
    });
    assert!(hooks.fetch_bytes("yes24", &format!("http://{address}/redirect"), Some(16)).is_err());
    assert_eq!(checked.lock().unwrap().len(), 2, "redirect target bypassed URL validation");
    server.join().unwrap();
}
