use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use ureq::tls::{PemItem, parse_pem};

pub fn serve_tls_once() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("https://{}", listener.local_addr().unwrap());
    let cert = include_bytes!("tls/local-es-server.crt");
    let key_pem = include_bytes!("tls/local-es.key");
    let certs: Vec<CertificateDer<'static>> = parse_pem(cert)
        .filter_map(|item| match item.unwrap() {
            PemItem::Certificate(cert) => Some(CertificateDer::from(cert.der().to_vec())),
            _ => None,
        })
        .collect();
    let key = parse_pem(key_pem)
        .find_map(|item| match item.unwrap() {
            PemItem::PrivateKey(key) => Some(PrivateKeyDer::try_from(key.as_ref().to_vec()).unwrap()),
            _ => None,
        })
        .unwrap();
    let config = ServerConfig::builder().with_no_client_auth().with_single_cert(certs, key).unwrap();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut tls = rustls::StreamOwned::new(rustls::ServerConnection::new(Arc::new(config)).unwrap(), stream);
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            let n = match tls.read(&mut chunk) {
                Ok(n) if n > 0 => n,
                Ok(_) => return,
                Err(_) => return,
            };
            request.extend_from_slice(&chunk[..n]);
            let Some(headers_end) = request.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4) else { continue };
            let headers = String::from_utf8_lossy(&request[..headers_end]).to_ascii_lowercase();
            let content_length = headers.lines().find_map(|line| line.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
            if request.len() >= headers_end + content_length {
                break;
            }
        }
        let body = r#"{"version":{"number":"9.1.0"}}"#;
        if let Err(error) = write!(tls, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()) {
            eprintln!("local TLS test server write failed: {error}");
            return;
        }
        tls.flush().unwrap();
    });
    (url, handle)
}
