//!
//! 직접 표지 URL의 SSRF 차단과 제한된 다운로드를 검증한다.
//! DNS 조회와 연결 풀은 `Net` trait로 주입한다.
//!
//! 가정한 API (아직 구현 전이다. 설계: docs/rust-test-seams.md 3.7. `Net`/`Pool`/`PoolResponse`는 연결 수준 계약이라 `HttpSession`과 합치지 않고 그대로 둔다):
//! - `backend::direct_cover_url::fetch_url_image_with(url: &str, net: &dyn Net, max_image_bytes: usize) -> Result<Vec<u8>, FetchError>`
//! - `backend::direct_cover_url::MAX_IMAGE_BYTES: usize`
//! - `backend::direct_cover_url::ConnectRequest { https, address, port, server_hostname, assert_hostname, cert_reqs, timeout: HttpTimeout }`
//!   (Clone + Debug. 검증된 IP로만 연결하고, TLS 호스트명은 원래 호스트로 확인한다)
//! - `trait Net { fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>>; fn connect(&self, request: &ConnectRequest) -> Box<dyn Pool>; }`
//! - `trait Pool { fn urlopen(&mut self, method: &str, target: &str, headers: &[(String, String)]) -> Result<Box<dyn PoolResponse>, String>; }`
//! - `trait PoolResponse { fn status(&self) -> u16; fn header(&self, name: &str) -> Option<String>; fn stream(&mut self) -> Box<dyn Iterator<Item = Result<Vec<u8>, String>>>; fn close(&mut self); }`
//! - `backend::direct_cover_url::FetchError { Value(String), Os(String), Other(String) }` (`impl Display`. 구 `CoverError`)

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use ::common::http_client::HttpTimeout;
use backend::direct_cover_url::{ConnectRequest, Net, SystemNet};

#[test]
fn test_system_net_uses_pinned_socket_and_streams_http_response() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let count = stream.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /image HTTP/1.1"));
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nimage").unwrap();
    });

    let net = SystemNet;
    let request = ConnectRequest { https: false, address: "127.0.0.1".to_string(), port, server_hostname: None, assert_hostname: None, cert_reqs: None, timeout: HttpTimeout::from_secs(5, 15) };
    let mut pool = net.connect(&request);
    let mut response = pool.urlopen("GET", "/image", &[]).unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Length").as_deref(), Some("5"));
    let body = response.stream().collect::<Result<Vec<_>, _>>().unwrap().concat();
    assert_eq!(body, b"image");
    response.close();
    server.join().unwrap();
}

#[test]
fn test_system_net_applies_response_header_timeout() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request);
        std::thread::sleep(Duration::from_millis(250));
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });

    let request = ConnectRequest {
        https: false,
        address: "127.0.0.1".to_string(),
        port,
        server_hostname: None,
        assert_hostname: None,
        cert_reqs: None,
        timeout: HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(40) },
    };
    let mut pool = SystemNet.connect(&request);

    assert!(pool.urlopen("GET", "/slow", &[]).is_err());
    server.join().unwrap();
}

#[test]
fn test_system_net_resets_read_timeout_after_each_body_chunk() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\n").unwrap();
        stream.flush().unwrap();
        for byte in b"abc" {
            std::thread::sleep(Duration::from_millis(100));
            stream.write_all(&[*byte]).unwrap();
            stream.flush().unwrap();
        }
    });

    let request = ConnectRequest {
        https: false,
        address: "127.0.0.1".to_string(),
        port,
        server_hostname: None,
        assert_hostname: None,
        cert_reqs: None,
        timeout: HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(150) },
    };
    let mut pool = SystemNet.connect(&request);
    let mut response = pool.urlopen("GET", "/slow-stream", &[]).unwrap();
    let chunks = response.stream().collect::<Result<Vec<_>, _>>();

    assert_eq!(chunks.unwrap().concat(), b"abc");
    server.join().unwrap();
}

#[test]
fn test_system_net_enforces_body_idle_timeout() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let _ = stream.write_all(b"late");
    });

    let request = ConnectRequest {
        https: false,
        address: "127.0.0.1".to_string(),
        port,
        server_hostname: None,
        assert_hostname: None,
        cert_reqs: None,
        timeout: HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(40) },
    };
    let mut pool = SystemNet.connect(&request);
    let mut response = pool.urlopen("GET", "/idle", &[]).unwrap();
    let chunks = response.stream().collect::<Result<Vec<_>, _>>();
    response.close();

    assert!(chunks.is_err(), "idle body read must time out");
    server.join().unwrap();
}
