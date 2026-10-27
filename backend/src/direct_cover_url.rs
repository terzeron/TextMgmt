//! 공개 이미지 URL을 검증한 IP에 직접 연결하여 내려받는다. Python `backend/direct_cover_url.py`에 대응한다.
//!
//! DNS 조회와 연결 풀은 [`Net`] trait 뒤에 둔다(설계: docs/rust-test-seams.md 3.7).
//! 연결 수준 계약이라 `sysdeps::HttpSession`과 합치지 않는다. 연결은 DNS로 검증한 IP에만 하고, TLS 호스트명은 원래 호스트로 확인한다.

use std::fmt;
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use crate::cover_sources::urlx::{Netloc, urljoin, urlsplit};
use crate::cover_sources::{is_global, system_resolve};
use sysdeps::HttpTimeout;

/// `backend.epub_cover.MAX_IMAGE_BYTES`와 같다.
pub const MAX_IMAGE_BYTES: usize = 15 * 1024 * 1024;

const MAX_REDIRECTS: usize = 5;

/// 내려받기 오류. `Value`는 사용자가 고칠 수 있는 입력·서버 문제(Python `CoverError`), `Os`는 DNS 같은 운영체제 오류다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    Value(String),
    Os(String),
    Other(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Value(message) | FetchError::Os(message) | FetchError::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FetchError {}

fn value_err(message: &str) -> FetchError {
    FetchError::Value(message.to_string())
}

/// 연결 풀을 만들 때 쓰는 값. `address`는 DNS 검증을 마친 IP다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectRequest {
    pub https: bool,
    pub address: String,
    pub port: u16,
    pub server_hostname: Option<String>,
    pub assert_hostname: Option<String>,
    pub cert_reqs: Option<String>,
    pub timeout: HttpTimeout,
}

pub trait PoolResponse {
    fn status(&self) -> u16;
    fn header(&self, name: &str) -> Option<String>;
    /// 본문을 `decode_content=False`로 청크 단위로 읽는다.
    fn stream(&mut self) -> Box<dyn Iterator<Item = Result<Vec<u8>, String>>>;
    /// 여러 번 불러도 안전해야 한다.
    fn close(&mut self);
}

pub trait Pool {
    fn urlopen(&mut self, method: &str, target: &str, headers: &[(String, String)]) -> Result<Box<dyn PoolResponse>, String>;
}

pub trait Net {
    fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>>;
    fn connect(&self, request: &ConnectRequest) -> Box<dyn Pool>;
}

/// 검증한 IP 하나만 ureq에 돌려주는 resolver.
#[derive(Debug)]
struct PinnedResolver(SocketAddr);

impl ureq::unversioned::resolver::Resolver for PinnedResolver {
    fn resolve(
        &self,
        _uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let mut addresses = ureq::unversioned::resolver::ResolvedSocketAddrs::from_fn(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        addresses.push(self.0);
        Ok(addresses)
    }
}

/// 시스템 DNS와 검증된 IP 고정 연결을 사용하는 실제 `Net`.
pub struct SystemNet;

struct UreqPool {
    agent: ureq::Agent,
    base_url: String,
    read_timeout: Duration,
}

struct UreqResponse {
    status: u16,
    headers: ureq::http::HeaderMap,
    body: Option<ureq::BodyReader<'static>>,
    agent: Option<ureq::Agent>,
    read_timeout: Duration,
}

/// ureq body deadline is total duration; this bounded reader channel resets an idle timeout per read.
struct BodyChunks {
    receiver: Receiver<Result<Vec<u8>, String>>,
    agent: Option<ureq::Agent>,
    read_timeout: Duration,
    finished: bool,
}

impl BodyChunks {
    fn new(body: ureq::BodyReader<'static>, agent: ureq::Agent, read_timeout: Duration) -> Self {
        let (sender, receiver) = sync_channel(1);
        std::thread::spawn(move || read_body_chunks(body, sender));
        Self { receiver, agent: Some(agent), read_timeout, finished: false }
    }
}

fn read_body_chunks(mut body: ureq::BodyReader<'static>, sender: SyncSender<Result<Vec<u8>, String>>) {
    loop {
        let mut chunk = vec![0; 8192];
        match body.read(&mut chunk) {
            Ok(0) => return,
            Ok(size) => {
                chunk.truncate(size);
                if sender.send(Ok(chunk)).is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
                return;
            }
        }
    }
}

impl Iterator for BodyChunks {
    type Item = Result<Vec<u8>, String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match self.receiver.recv_timeout(self.read_timeout) {
            Ok(chunk) => {
                if chunk.is_err() {
                    self.finished = true;
                }
                Some(chunk)
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.finished = true;
                None
            }
            Err(RecvTimeoutError::Timeout) => {
                self.finished = true;
                if let Some(agent) = self.agent.take() {
                    agent.shutdown();
                }
                Some(Err("response body read timed out".to_string()))
            }
        }
    }
}

impl PoolResponse for UreqResponse {
    fn status(&self) -> u16 {
        self.status
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
    }

    fn stream(&mut self) -> Box<dyn Iterator<Item = Result<Vec<u8>, String>>> {
        Box::new(BodyChunks::new(
            self.body.take().expect("response stream can only be consumed once"),
            self.agent.as_ref().expect("response agent is available until close").clone(),
            self.read_timeout,
        ))
    }

    fn close(&mut self) {
        if let Some(agent) = self.agent.take() {
            agent.shutdown();
        }
    }
}

impl Pool for UreqPool {
    fn urlopen(&mut self, method: &str, target: &str, headers: &[(String, String)]) -> Result<Box<dyn PoolResponse>, String> {
        let url = format!("{}{}", self.base_url, target);
        let mut request = ureq::http::Request::builder().method(method).uri(&url).body(()).map_err(|error| error.to_string())?;
        for (name, value) in headers {
            if !name.eq_ignore_ascii_case("host") {
                request.headers_mut().insert(
                    name.parse::<ureq::http::header::HeaderName>().map_err(|error| error.to_string())?,
                    value.parse().map_err(|error: ureq::http::header::InvalidHeaderValue| error.to_string())?,
                );
            }
        }
        let response = self.agent.run(request).map_err(|error| error.to_string())?;
        let (parts, body) = response.into_parts();
        Ok(Box::new(UreqResponse { status: parts.status.as_u16(), headers: parts.headers, body: Some(body.into_reader()), agent: Some(self.agent.clone()), read_timeout: self.read_timeout }))
    }
}

impl Net for SystemNet {
    fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>> {
        system_resolve(host, port)
    }

    fn connect(&self, request: &ConnectRequest) -> Box<dyn Pool> {
        let address: SocketAddr =
            format!("{}:{}", if request.address.contains(':') { format!("[{}]", request.address) } else { request.address.clone() }, request.port).parse().expect("validated socket address");
        let host = request.server_hostname.as_deref().or(request.assert_hostname.as_deref()).unwrap_or(&request.address);
        let url_host = if host.contains(':') { format!("[{host}]") } else { host.to_string() };
        let scheme = if request.https { "https" } else { "http" };
        let config = ureq::config::Config::builder()
            .proxy(None)
            .timeout_connect(Some(request.timeout.connect))
            .timeout_recv_response(Some(request.timeout.read))
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        let agent = ureq::Agent::with_parts(config, ureq::unversioned::transport::DefaultConnector::default(), PinnedResolver(address));
        Box::new(UreqPool { agent, base_url: format!("{scheme}://{url_host}"), read_timeout: request.timeout.read })
    }
}

pub fn fetch_url_image(url: &str) -> Result<Vec<u8>, FetchError> {
    fetch_url_image_with(url, &SystemNet, MAX_IMAGE_BYTES)
}

/// 공개 이미지 URL을 내려받는 능력. 테스트는 대역으로 바꾼다(설계: docs/rust-test-seams.md 3.7).
pub trait ImageFetcher: Send + Sync {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError>;
}

/// 실제 연결로 내려받는 구현.
pub struct RealImageFetcher;

impl ImageFetcher for RealImageFetcher {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        fetch_url_image(url)
    }
}

/// 응답을 버릴 때 반드시 닫는다.
struct Closing(Box<dyn PoolResponse>);

impl Drop for Closing {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Python `str.encode("idna")`와 같은 transitional IDNA mapping으로 hostname을 ASCII로 바꾼다.
fn idna_ascii(host: &str) -> Result<String, ()> {
    #[allow(deprecated)]
    let ascii = idna::Config::default().transitional_processing(true).to_ascii(host).map_err(|_| ())?;
    let labels: Vec<_> = ascii.split('.').collect();
    for (index, label) in labels.iter().enumerate() {
        let root_label = index + 1 == labels.len() && label.is_empty() && labels.len() > 1;
        if !root_label && (label.is_empty() || label.len() >= 64) {
            return Err(());
        }
    }
    Ok(ascii)
}

/// `url`의 이미지를 받는다. redirect는 최대 5번 따라가며 매번 DNS와 주소를 다시 검증한다.
pub fn fetch_url_image_with(url: &str, net: &dyn Net, max_image_bytes: usize) -> Result<Vec<u8>, FetchError> {
    let mut url = url.to_string();
    for _ in 0..MAX_REDIRECTS {
        let parsed = urlsplit(&url);
        let netloc = Netloc::parse(parsed.netloc.as_deref().unwrap_or(""));
        let https = parsed.scheme == "https";
        let default_port = if https { 443 } else { 80 };
        let invalid_url = || value_err("올바른 이미지 URL을 입력하세요");
        let port = netloc.port().map_err(|_| invalid_url())?.filter(|p| *p != 0).unwrap_or(default_port);
        let host = idna_ascii(netloc.hostname.as_deref().unwrap_or("")).map_err(|()| invalid_url())?;
        if !matches!(parsed.scheme.as_str(), "http" | "https") || host.is_empty() || netloc.username.is_some() || netloc.password.is_some() || port != default_port {
            return Err(value_err("공개 HTTP/HTTPS 이미지 URL만 사용할 수 있습니다"));
        }
        let addresses = net.resolve(&host, port).map_err(|e| FetchError::Os(format!("이미지 서버 주소를 확인하지 못했습니다: {e}")))?;
        if addresses.is_empty() || !addresses.iter().all(is_global) {
            return Err(value_err("내부 주소의 이미지는 사용할 수 없습니다"));
        }
        // 연결 대상 IP는 DNS 검증 결과로 고정한다. TLS SNI와 인증서는 원래 host로 검증한다.
        let request = ConnectRequest {
            https,
            address: addresses[0].to_string(),
            port,
            server_hostname: https.then(|| host.clone()),
            assert_hostname: https.then(|| host.clone()),
            cert_reqs: https.then(|| "CERT_REQUIRED".to_string()),
            timeout: HttpTimeout::from_secs(5, 15),
        };
        let mut pool = net.connect(&request);
        let mut target = if parsed.path.is_empty() { "/".to_string() } else { parsed.path.clone() };
        if !parsed.query.is_empty() {
            target.push('?');
            target.push_str(&parsed.query);
        }
        let headers = vec![
            ("Host".to_string(), if host.contains(':') { format!("[{host}]") } else { host.clone() }),
            ("User-Agent".to_string(), "TextMgmt/cover".to_string()),
            ("Accept".to_string(), "image/*".to_string()),
            ("Accept-Encoding".to_string(), "identity".to_string()),
        ];
        let connect_failed = || value_err("이미지 서버에 연결하지 못했습니다. URL을 확인하세요");
        let mut response = Closing(pool.urlopen("GET", &target, &headers).map_err(|_| connect_failed())?);
        let status = response.0.status();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let location = response.0.header("Location").filter(|l| !l.is_empty()).ok_or_else(|| value_err("이미지 redirect 주소가 없습니다"))?;
            url = urljoin(&url, &location);
            continue;
        }
        if status != 200 {
            return Err(value_err("이미지 서버에서 파일을 받지 못했습니다"));
        }
        let mut output = Vec::new();
        for chunk in response.0.stream() {
            output.extend_from_slice(&chunk.map_err(|_| connect_failed())?);
            if output.len() > max_image_bytes {
                return Err(value_err("표지 이미지 크기가 제한을 초과했습니다"));
            }
        }
        return Ok(output);
    }
    Err(value_err("이미지 redirect 횟수를 초과했습니다"))
}
