//! HTTP 요청과 응답.
//!
//! 서비스 코드는 [`HttpSession`]을 받고, 테스트는 대역으로 바꿔 끼운다(설계: docs/rust-test-seams.md 3.1).
//! 실제 구현은 `ureq`다. agent 설정, 본문 idle timeout, 응답 크기 상한을 여기서 한 번만 정한다.

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;

#[derive(Debug)]
pub enum HttpError {
    Timeout,
    Connection(String),
    Other(String),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Timeout => write!(f, "HTTP 요청 시간 초과"),
            HttpError::Connection(msg) => write!(f, "HTTP 연결 실패: {msg}"),
            HttpError::Other(msg) => write!(f, "HTTP 오류: {msg}"),
        }
    }
}

impl std::error::Error for HttpError {}

pub trait BodyStream: Send {
    fn next_chunk(&mut self) -> Option<Result<Vec<u8>, HttpError>>;
    /// 여러 번 불러도 안전해야 한다.
    fn close(&mut self);
}

pub struct HttpResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    body: Box<dyn BodyStream>,
}

struct ChunkBody {
    chunks: std::vec::IntoIter<Vec<u8>>,
    closed: Arc<AtomicBool>,
}

impl BodyStream for ChunkBody {
    fn next_chunk(&mut self) -> Option<Result<Vec<u8>, HttpError>> {
        self.chunks.next().map(Ok)
    }

    fn close(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

impl HttpResponse {
    pub fn new(status_code: u16, headers: Vec<(String, String)>, body: Box<dyn BodyStream>) -> Self {
        Self { status_code, headers, body }
    }

    /// fake 생성용: 본문이 한 덩어리인 응답.
    pub fn from_text(status: u16, text: &str) -> Self {
        Self::from_chunks(status, Vec::new(), vec![text.as_bytes().to_vec()], Arc::new(AtomicBool::new(false)))
    }

    /// fake 생성용: 청크 본문. 본문을 닫으면 `closed`가 `true`가 된다.
    pub fn from_chunks(status: u16, headers: Vec<(String, String)>, chunks: Vec<Vec<u8>>, closed: Arc<AtomicBool>) -> Self {
        Self::new(status, headers, Box::new(ChunkBody { chunks: chunks.into_iter(), closed }))
    }

    pub fn next_chunk(&mut self) -> Option<Result<Vec<u8>, HttpError>> {
        self.body.next_chunk()
    }

    /// 본문 전체를 UTF-8로 읽는다(잘못된 바이트는 대체 문자). 읽은 뒤 본문을 닫는다.
    pub fn text(mut self) -> String {
        let mut buf = Vec::new();
        while let Some(Ok(chunk)) = self.body.next_chunk() {
            buf.extend_from_slice(&chunk);
        }
        self.body.close();
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// 본문을 JSON으로 해석한다. 해석할 수 없으면 `Null`이다.
    pub fn json(self) -> Value {
        serde_json::from_str(&self.text()).unwrap_or(Value::Null)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn is_redirect(&self) -> bool {
        matches!(self.status_code, 301 | 302 | 303 | 307 | 308)
    }

    pub fn location(&self) -> Option<&str> {
        self.header("location")
    }
}

impl Drop for HttpResponse {
    fn drop(&mut self) {
        self.body.close();
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse").field("status_code", &self.status_code).field("headers", &self.headers).finish_non_exhaustive()
    }
}

/// Per-request network timeouts. `read` bounds response-header and per-read body inactivity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpTimeout {
    pub connect: Duration,
    pub read: Duration,
}

impl HttpTimeout {
    pub const fn from_secs(connect: u64, read: u64) -> Self {
        Self { connect: Duration::from_secs(connect), read: Duration::from_secs(read) }
    }
}

pub trait HttpSession: Send + Sync {
    fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError>;

    /// Request with explicit timeouts. Implementors may keep the existing `get` behavior.
    fn get_with_timeout(&self, url: &str, params: &[(String, String)], _timeout: HttpTimeout) -> Result<HttpResponse, HttpError> {
        self.get(url, params)
    }
}

// ---------------------------------------------------------------------------
// ureq 구현
// ---------------------------------------------------------------------------

/// 요청마다 timeout을 정하는 agent. 응답 상태를 오류로 바꾸지 않는다. `max_redirects`가 0이면 redirect를 따라가지 않는다.
pub fn http_client(max_redirects: u32) -> ureq::Agent {
    ureq::Agent::config_builder().max_redirects(max_redirects).http_status_as_error(false).build().into()
}

/// 요청 전체에 `timeout`을 거는 agent. 응답 상태를 오류로 바꾸지 않는다.
/// `ca_cert_path`를 주면 그 PEM의 인증서를 TLS trust root로 쓴다.
pub fn http_client_with_timeout(timeout: Duration, ca_cert_path: Option<&Path>) -> Result<ureq::Agent, String> {
    let mut builder = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false);
    if let Some(path) = ca_cert_path {
        let pem = fs::read(path).map_err(|error| format!("CA 인증서를 읽지 못했다 ({}): {error}", path.display()))?;
        let certs = ureq::tls::parse_pem(&pem)
            .filter_map(|item| match item {
                Ok(ureq::tls::PemItem::Certificate(cert)) => Some(Ok(cert)),
                Ok(ureq::tls::PemItem::PrivateKey(_)) => None,
                Err(error) => Some(Err(error)),
                _ => None,
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("CA 인증서 PEM을 해석하지 못했다: {error}"))?;
        if certs.is_empty() {
            return Err(format!("CA 인증서 PEM에 X.509 certificate가 없다: {}", path.display()));
        }
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::new_with_certs(&certs)).build();
        builder = builder.tls_config(tls);
    }
    Ok(builder.build().into())
}

/// 응답 본문을 조각 단위로 읽는다.
///
/// ureq의 본문 timeout은 전체 시간이라, 조각마다 대기 시간을 새로 세려고 별도 스레드에서 읽는다.
/// 조각 사이 대기가 `idle_timeout`을 넘으면 agent를 닫고 `TimedOut` 오류를 한 번 낸 뒤 끝난다.
/// 읽기 오류도 한 번 낸 뒤 끝난다.
pub struct BodyChunks {
    receiver: Receiver<io::Result<Vec<u8>>>,
    agent: Option<ureq::Agent>,
    idle_timeout: Duration,
    finished: bool,
}

impl BodyChunks {
    pub fn new(body: ureq::BodyReader<'static>, agent: ureq::Agent, idle_timeout: Duration) -> Self {
        let (sender, receiver) = sync_channel(1);
        std::thread::spawn(move || read_body_chunks(body, sender));
        Self { receiver, agent: Some(agent), idle_timeout, finished: false }
    }

    /// agent를 닫는다. 읽는 스레드는 이 값이 사라질 때 보내기에 실패해서 끝난다.
    pub fn shutdown(&mut self) {
        if let Some(agent) = self.agent.take() {
            agent.shutdown();
        }
    }

    /// agent를 돌려받는다. 시간 초과나 [`BodyChunks::shutdown`]으로 닫았으면 `None`이다.
    pub fn into_agent(self) -> Option<ureq::Agent> {
        self.agent
    }
}

fn read_body_chunks(mut body: ureq::BodyReader<'static>, sender: SyncSender<io::Result<Vec<u8>>>) {
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
                let _ = sender.send(Err(error));
                return;
            }
        }
    }
}

impl Iterator for BodyChunks {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match self.receiver.recv_timeout(self.idle_timeout) {
            Ok(chunk) => {
                self.finished = chunk.is_err();
                Some(chunk)
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.finished = true;
                None
            }
            Err(RecvTimeoutError::Timeout) => {
                self.finished = true;
                self.shutdown();
                Some(Err(io::Error::new(io::ErrorKind::TimedOut, "response body read timed out")))
            }
        }
    }
}

/// 외부 응답 본문의 상한. 다 읽은 뒤에 검사하면 메모리를 지키지 못하므로 읽는 도중에 끊는다.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// `ureq` 기반 동기 HTTP GET 세션. 응답 상태와 본문을 그대로 돌려주고, 판정은 호출자에게 맡긴다.
pub struct UreqHttpSession {
    max_redirects: u32,
    agents: Mutex<Vec<ureq::Agent>>,
    headers: Vec<(String, String)>,
}

impl UreqHttpSession {
    /// 모든 요청에 `headers`를 붙인다. `max_redirects`가 0이면 redirect 응답을 그대로 돌려준다.
    pub fn new(max_redirects: u32, headers: Vec<(String, String)>) -> Self {
        Self { max_redirects, agents: Mutex::new(vec![http_client(max_redirects)]), headers }
    }

    /// 페이지 하나를 받는다. `timeout.read`는 응답 헤더 대기와 본문 조각 사이 대기의 상한이다.
    pub fn get_page(&self, url: &str, params: &[(String, String)], timeout: HttpTimeout) -> Result<HttpResponse, HttpError> {
        // idle timeout으로 agent를 닫아도 동시 요청이 끊기지 않게 요청마다 agent를 따로 빌린다.
        let agent = self.agents.lock().unwrap_or_else(PoisonError::into_inner).pop().unwrap_or_else(|| http_client(self.max_redirects));
        let mut request = agent.get(url);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        for (key, value) in params {
            request = request.query(key, value);
        }
        let request = request.config().timeout_connect(Some(timeout.connect)).timeout_recv_response(Some(timeout.read)).timeout_recv_body(None).build();
        let response = request.call().map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let headers = response.headers().iter().map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("").to_string())).collect();
        let (_, body) = response.into_parts();
        let mut chunks = BodyChunks::new(body.into_reader(), agent, timeout.read);
        let mut output = Vec::new();
        while let Some(chunk) = chunks.next() {
            let chunk = chunk.map_err(map_io_error)?;
            if output.len() + chunk.len() > MAX_RESPONSE_BYTES {
                chunks.shutdown();
                return Err(HttpError::Connection(format!("응답 본문이 상한 {MAX_RESPONSE_BYTES}바이트를 넘습니다")));
            }
            output.extend(chunk);
        }
        if let Some(agent) = chunks.into_agent() {
            self.agents.lock().unwrap_or_else(PoisonError::into_inner).push(agent);
        }
        Ok(HttpResponse::from_chunks(status, headers, vec![output], Arc::new(AtomicBool::new(false))))
    }
}

impl HttpSession for UreqHttpSession {
    fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        self.get_page(url, params, HttpTimeout::from_secs(10, 10))
    }

    fn get_with_timeout(&self, url: &str, params: &[(String, String)], timeout: HttpTimeout) -> Result<HttpResponse, HttpError> {
        self.get_page(url, params, timeout)
    }
}

fn map_ureq_error(error: ureq::Error) -> HttpError {
    match &error {
        ureq::Error::Timeout(_) => HttpError::Timeout,
        ureq::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut => HttpError::Timeout,
        _ => HttpError::Connection(error.to_string()),
    }
}

fn map_io_error(error: io::Error) -> HttpError {
    if error.kind() == io::ErrorKind::TimedOut { HttpError::Timeout } else { HttpError::Connection(error.to_string()) }
}
