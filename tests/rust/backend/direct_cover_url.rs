//! 원본: tests/test_direct_cover_url.py
//!
//! 직접 표지 URL의 SSRF 차단과 제한된 다운로드를 검증한다. Python은 `socket.getaddrinfo`와 `urllib3` 연결 풀을
//! monkeypatch로 바꿨다. Rust에서는 DNS 조회와 연결 풀을 `Net` trait로 주입한다.
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
use std::net::{IpAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend::direct_cover_url::{ConnectRequest, MAX_IMAGE_BYTES, Net, Pool, PoolResponse, SystemNet, fetch_url_image_with};
use sysdeps::HttpTimeout;

type Request = (String, String, Vec<(String, String)>);

/// 응답 하나의 명세. `closed`는 같은 풀이 만든 모든 응답이 공유한다(Python `response.close.called`).
#[derive(Clone)]
struct Spec {
    status: u16,
    headers: Vec<(String, String)>,
    chunks: Vec<Result<Vec<u8>, String>>,
    closed: Arc<AtomicBool>,
}

impl Spec {
    fn new(status: u16, chunks: &[&[u8]]) -> Self {
        Self { status, headers: Vec::new(), chunks: chunks.iter().map(|c| Ok(c.to_vec())).collect(), closed: Arc::new(AtomicBool::new(false)) }
    }

    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

struct FakeResponse(Spec);

impl PoolResponse for FakeResponse {
    fn status(&self) -> u16 {
        self.0.status
    }

    fn header(&self, name: &str) -> Option<String> {
        self.0.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }

    fn stream(&mut self) -> Box<dyn Iterator<Item = Result<Vec<u8>, String>>> {
        Box::new(self.0.chunks.clone().into_iter())
    }

    fn close(&mut self) {
        self.0.closed.store(true, Ordering::SeqCst);
    }
}

/// `urlopen` 요청을 기록하고 항상 같은 명세의 응답을 돌려주는 풀. Python `MagicMock()` 풀.
struct FakePool {
    spec: Spec,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Pool for FakePool {
    fn urlopen(&mut self, method: &str, target: &str, headers: &[(String, String)]) -> Result<Box<dyn PoolResponse>, String> {
        self.requests.lock().unwrap().push((method.to_string(), target.to_string(), headers.to_vec()));
        Ok(Box::new(FakeResponse(self.spec.clone())))
    }
}

type Resolver = Box<dyn Fn(&str, u16) -> std::io::Result<Vec<IpAddr>> + Send + Sync>;

/// DNS와 연결 풀을 가짜로 바꾼 `Net`. 연결할 때마다 `pools`를 차례로 쓰고, 모자라면 마지막 풀을 다시 쓴다.
struct FakeNet {
    resolver: Resolver,
    pools: Vec<(Spec, Arc<Mutex<Vec<Request>>>)>,
    connects: Mutex<Vec<ConnectRequest>>,
}

impl FakeNet {
    fn new(resolver: Resolver, specs: Vec<Spec>) -> Self {
        Self { resolver, pools: specs.into_iter().map(|s| (s, Arc::new(Mutex::new(Vec::new())))).collect(), connects: Mutex::new(Vec::new()) }
    }

    fn requests(&self, pool: usize) -> Vec<Request> {
        self.pools[pool].1.lock().unwrap().clone()
    }

    fn connect_count(&self) -> usize {
        self.connects.lock().unwrap().len()
    }
}

impl Net for FakeNet {
    fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>> {
        (self.resolver)(host, port)
    }

    fn connect(&self, request: &ConnectRequest) -> Box<dyn Pool> {
        let mut connects = self.connects.lock().unwrap();
        let index = connects.len().min(self.pools.len() - 1);
        connects.push(request.clone());
        let (spec, requests) = &self.pools[index];
        Box::new(FakePool { spec: spec.clone(), requests: requests.clone() })
    }
}

fn addresses(list: &'static [&'static str]) -> Resolver {
    Box::new(move |_host, _port| Ok(list.iter().map(|a| a.parse().unwrap()).collect()))
}

/// Python `public_dns`: 어떤 호스트든 공인 주소 8.8.8.8 하나로 풀린다.
fn public_dns() -> Resolver {
    addresses(&["8.8.8.8"])
}

fn fetch(url: &str, net: &FakeNet) -> Result<Vec<u8>, String> {
    fetch_url_image_with(url, net, MAX_IMAGE_BYTES).map_err(|e| e.to_string())
}

#[test]
fn test_mixed_public_and_private_dns_is_rejected_before_connecting() {
    let net = FakeNet::new(addresses(&["8.8.8.8", "192.168.1.10"]), vec![Spec::new(200, &[])]);
    let err = fetch("https://images.example.org/cover.jpg", &net).unwrap_err();
    assert!(err.contains("내부 주소"), "{err}");
    assert_eq!(net.connect_count(), 0);
}

#[test]
fn test_ipv6_image_uses_bracketed_host_and_validated_address() {
    let address = "2001:4860:4860::8888";
    let spec = Spec::new(200, &[b"image"]);
    let net = FakeNet::new(addresses(&["2001:4860:4860::8888"]), vec![spec.clone()]);
    assert_eq!(fetch(&format!("https://[{address}]/cover.jpg"), &net).unwrap(), b"image");
    assert_eq!(net.connects.lock().unwrap()[0].address, address);
    let host = net.requests(0)[0].2.iter().find(|(k, _)| k == "Host").map(|(_, v)| v.clone());
    assert_eq!(host.as_deref(), Some(format!("[{address}]").as_str()));
    assert!(spec.closed());
}

#[test]
fn test_stream_limit_counts_chunks_instead_of_trusting_content_length() {
    let mut spec = Spec::new(200, &[b"abc", b"def"]).header("Content-Length", "1");
    // 제한을 넘은 뒤에도 읽으면 이 오류가 나서 테스트가 실패한다(Python의 `raise AssertionError`).
    spec.chunks.push(Err("읽기를 제한 뒤에 계속하면 안 된다".to_string()));
    let net = FakeNet::new(public_dns(), vec![spec.clone()]);
    let err = fetch_url_image_with("https://images.example.org/cover.jpg", &net, 5).unwrap_err().to_string();
    assert!(err.contains("크기"), "{err}");
    assert!(spec.closed());
}

#[test]
fn test_relative_redirect_downloads_image_and_closes_every_response() {
    for status in [301, 302, 303, 307, 308] {
        let redirect = Spec::new(status, &[]).header("Location", "/real.jpg?version=2");
        let image = Spec::new(200, &[b"image"]);
        let net = FakeNet::new(public_dns(), vec![redirect.clone(), image.clone()]);
        assert_eq!(fetch("https://example.org/cover.jpg", &net).unwrap(), b"image", "status={status}");
        let second = &net.requests(1)[0];
        assert_eq!((second.0.as_str(), second.1.as_str()), ("GET", "/real.jpg?version=2"), "status={status}");
        assert!(redirect.closed() && image.closed(), "status={status}");
    }
}

#[test]
fn test_https_connects_to_validated_ip_with_original_tls_hostname() {
    let spec = Spec::new(200, &[b"im", b"age"]);
    let net = FakeNet::new(public_dns(), vec![spec.clone()]);
    assert_eq!(fetch("https://images.example.org/a.jpg?q=1#ignored", &net).unwrap(), b"image");
    let connect = net.connects.lock().unwrap()[0].clone();
    assert_eq!(connect.address, "8.8.8.8");
    assert_eq!(connect.server_hostname.as_deref(), Some("images.example.org"));
    assert_eq!(connect.assert_hostname.as_deref(), Some("images.example.org"));
    assert_eq!(connect.cert_reqs.as_deref(), Some("CERT_REQUIRED"));
    assert_eq!(connect.timeout, HttpTimeout::from_secs(5, 15));
    assert!(spec.closed());
    let request = &net.requests(0)[0];
    assert_eq!((request.0.as_str(), request.1.as_str()), ("GET", "/a.jpg?q=1"));
}

#[test]
fn test_rejects_invalid_urls_and_private_addresses() {
    for url in ["file:///etc/passwd", "ftp://example.org/x", "http://u:p@example.org/x", "https://example.org:444/x", "https:///no-host", "http://127.0.0.1/x"] {
        let net = FakeNet::new(addresses(&["127.0.0.1"]), vec![Spec::new(200, &[b"data"])]);
        assert!(fetch(url, &net).is_err(), "url={url}");
    }
}

#[test]
fn test_redirect_is_revalidated_before_connecting() {
    let resolver: Resolver = Box::new(|host, _port| Ok(vec![(if host == "example.org" { "8.8.8.8" } else { "10.0.0.1" }).parse().unwrap()]));
    let spec = Spec::new(302, &[]).header("Location", "https://internal.example.org/a.jpg");
    let net = FakeNet::new(resolver, vec![spec.clone()]);
    assert!(fetch("https://example.org/a.jpg", &net).is_err());
    assert_eq!(net.connect_count(), 1);
    assert!(spec.closed());
}

#[test]
fn test_unicode_hostnames_match_python_idna_codec_before_dns_validation() {
    for (input, expected) in [("faß.de", "fass.de"), ("Ａ.com", "a.com"), ("exa\u{00ad}mple.org", "example.org"), ("a。com", "a.com")] {
        let resolved = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&resolved);
        let resolver: Resolver = Box::new(move |host, _| {
            observed.lock().unwrap().push(host.to_owned());
            Ok(vec!["8.8.8.8".parse().unwrap()])
        });
        let net = FakeNet::new(resolver, vec![Spec::new(200, &[b"image"])]);

        assert_eq!(fetch(&format!("https://{input}/cover.jpg"), &net).unwrap(), b"image");
        assert_eq!(*resolved.lock().unwrap(), [expected]);
        assert_eq!(net.connects.lock().unwrap()[0].server_hostname.as_deref(), Some(expected));
    }
}

#[test]
fn test_idna_rejects_overlong_and_empty_labels_before_dns_validation() {
    for host in [format!("{}.example", "a".repeat(64)), "a..example".to_string()] {
        let resolved = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&resolved);
        let resolver: Resolver = Box::new(move |name, _| {
            observed.lock().unwrap().push(name.to_owned());
            Ok(vec!["8.8.8.8".parse().unwrap()])
        });
        let net = FakeNet::new(resolver, vec![Spec::new(200, &[b"image"])]);

        assert!(fetch(&format!("https://{host}/cover.jpg"), &net).is_err(), "host={host:?}");
        assert!(resolved.lock().unwrap().is_empty(), "DNS lookup should not run for host={host:?}");
    }
}

#[test]
fn test_download_limits_cleanup_and_url_validation() {
    for mode in ["http", "no_location", "bad_status", "size", "redirect_limit", "read_error", "bad_port", "bad_host", "empty_dns"] {
        let status = match mode {
            "no_location" | "redirect_limit" => 302,
            "bad_status" => 500,
            _ => 200,
        };
        let mut spec = Spec::new(status, &[b"data"]);
        if mode == "redirect_limit" {
            spec = spec.header("Location", "/next");
        }
        if mode == "read_error" {
            // urllib3 ReadTimeoutError
            spec.chunks = vec![Err("timeout".to_string())];
        }
        let resolver = if mode == "empty_dns" { addresses(&[]) } else { public_dns() };
        let net = FakeNet::new(resolver, vec![spec.clone()]);
        let max_bytes = if mode == "size" { 3 } else { MAX_IMAGE_BYTES };
        // Python은 lone surrogate("\ud800")로 잘못된 호스트를 만든다. Rust 문자열은 surrogate를 담을 수 없어 IDNA에 맞지 않는 문자로 대신한다.
        let url = match mode {
            "http" => "http://example.org",
            "bad_port" => "https://example.org:image/a",
            "bad_host" => "https://\u{fffe}.org/a",
            _ => "https://example.org/a",
        };
        let result = fetch_url_image_with(url, &net, max_bytes);
        if mode == "http" {
            assert_eq!(result.unwrap(), b"data", "mode={mode}");
        } else {
            assert!(result.is_err(), "mode={mode}");
        }
        if !["bad_port", "bad_host", "empty_dns"].contains(&mode) {
            assert!(spec.closed(), "mode={mode}");
        }
    }
}

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
