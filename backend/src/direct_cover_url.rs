//! 공개 이미지 URL을 검증한 IP에 직접 연결하여 내려받는다. Python `backend/direct_cover_url.py`에 대응한다.
//!
//! DNS 조회와 연결 풀은 [`Net`] trait 뒤에 둔다(설계: docs/rust-test-seams.md 3.7).
//! 연결 수준 계약이라 `sysdeps::HttpSession`과 합치지 않는다. 연결은 DNS로 검증한 IP에만 하고, TLS 호스트명은 원래 호스트로 확인한다.

use std::fmt;
use std::net::IpAddr;

use crate::cover_sources::urlx::{Netloc, urljoin, urlsplit};
use crate::cover_sources::{is_global, system_resolve};

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

/// 시스템 DNS를 쓰는 실제 `Net`.
///
/// TLS 라이브러리가 아직 이 crate에 없어 `connect`가 만든 풀은 요청마다 연결 실패를 돌려준다.
pub struct SystemNet;

struct UnconnectedPool;

impl Pool for UnconnectedPool {
    fn urlopen(&mut self, _method: &str, _target: &str, _headers: &[(String, String)]) -> Result<Box<dyn PoolResponse>, String> {
        Err("실제 연결 풀(TLS 포함)이 아직 연결되지 않았습니다".to_string())
    }
}

impl Net for SystemNet {
    fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>> {
        system_resolve(host, port)
    }

    fn connect(&self, _request: &ConnectRequest) -> Box<dyn Pool> {
        Box::new(UnconnectedPool)
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

/// `str.encode("idna")`에 대응한다. ASCII 라벨은 길이만 검사하고, 비ASCII 라벨은 금지 문자를 거른 뒤 소문자로 바꿔 punycode로 만든다.
/// (nameprep의 NFKC 정규화는 하지 않는다.)
fn idna_ascii(host: &str) -> Result<String, ()> {
    let labels: Vec<&str> = host.split('.').collect();
    let last = labels.len() - 1;
    let mut out = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        if label.is_empty() {
            if i == last && labels.len() > 1 {
                out.push(String::new());
                continue;
            }
            return Err(());
        }
        let encoded = if label.is_ascii() {
            (*label).to_string()
        } else {
            if label.chars().any(prohibited_in_label) {
                return Err(());
            }
            let lowered = label.to_lowercase();
            format!("xn--{}", punycode_encode(&lowered).ok_or(())?)
        };
        if encoded.len() > 63 {
            return Err(());
        }
        out.push(encoded);
    }
    Ok(out.join("."))
}

fn prohibited_in_label(c: char) -> bool {
    let code = c as u32;
    c.is_control()
        || c.is_whitespace()
        || (code & 0xfffe) == 0xfffe
        || (0xfdd0..=0xfdef).contains(&code)
        || (0xe000..=0xf8ff).contains(&code)
        || code >= 0xf0000
        || (0xfff9..=0xffff).contains(&code)
        || (0x2028..=0x202e).contains(&code)
}

/// RFC 3492 punycode 인코딩.
fn punycode_encode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const T_MIN: u32 = 1;
    const T_MAX: u32 = 26;
    const SKEW: u32 = 38;
    const DAMP: u32 = 700;
    let digit = |d: u32| -> char { if d < 26 { (b'a' + d as u8) as char } else { (b'0' + (d - 26) as u8) as char } };
    let adapt = |mut delta: u32, points: u32, first: bool| -> u32 {
        delta = if first { delta / DAMP } else { delta / 2 };
        delta += delta / points;
        let mut k = 0;
        while delta > ((BASE - T_MIN) * T_MAX) / 2 {
            delta /= BASE - T_MIN;
            k += BASE;
        }
        k + (BASE - T_MIN + 1) * delta / (delta + SKEW)
    };
    let chars: Vec<u32> = input.chars().map(|c| c as u32).collect();
    let mut output: String = input.chars().filter(char::is_ascii).collect();
    let basic = output.len() as u32;
    let mut handled = basic;
    if basic > 0 {
        output.push('-');
    }
    let (mut n, mut delta, mut bias) = (128u32, 0u32, 72u32);
    while (handled as usize) < chars.len() {
        let m = chars.iter().copied().filter(|c| *c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(handled + 1)?)?;
        n = m;
        for &c in &chars {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        T_MIN
                    } else if k >= bias + T_MAX {
                        T_MAX
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    output.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                output.push(digit(q));
                bias = adapt(delta, handled + 1, handled == basic);
                delta = 0;
                handled += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    Some(output)
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
