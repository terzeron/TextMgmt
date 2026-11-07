//! `http_client::UreqHttpSession` 통합 테스트. 로컬 TCP 서버로 timeout, 크기 상한, 연결 재사용을 확인한다.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Duration;

use http_client::{HttpError, HttpSession, HttpTimeout, MAX_RESPONSE_BYTES, UreqHttpSession};

fn session() -> UreqHttpSession {
    UreqHttpSession::new(10, Vec::new())
}

#[test]
fn ureq_session_maps_connection_refusal_to_connection_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let error = session().get(&format!("http://{address}/refused"), &[]).unwrap_err();

    assert!(matches!(error, HttpError::Connection(_)));
}

#[test]
fn ureq_session_applies_request_body_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let _ = stream.write_all(b"late");
    });

    let timeout = HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(40) };
    let result = session().get_with_timeout(&format!("http://{address}/slow"), &[], timeout);

    assert!(matches!(result, Err(HttpError::Timeout)), "{result:?}");
    server.join().unwrap();
}

#[test]
fn ureq_session_recovers_after_an_idle_body_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut slow, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(slow.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        slow.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").unwrap();
        slow.flush().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let _ = slow.write_all(b"late");

        let (mut fast, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(fast.try_clone().unwrap());
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("GET /fast HTTP/1.1"), "{line}");
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        fast.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
    });

    let session = session();
    let slow_timeout = HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(40) };
    assert!(matches!(session.get_with_timeout(&format!("http://{address}/slow"), &[], slow_timeout), Err(HttpError::Timeout)));
    let response = session.get_with_timeout(&format!("http://{address}/fast"), &[], HttpTimeout::from_secs(1, 1)).unwrap();
    assert_eq!(response.text(), "ok");
    server.join().unwrap();
}

#[test]
fn ureq_session_resets_body_timeout_for_each_received_chunk() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
        for chunk in [b"a".as_slice(), b"b".as_slice(), b"c".as_slice()] {
            std::thread::sleep(Duration::from_millis(60));
            write!(stream, "1\r\n{}\r\n", chunk[0] as char).unwrap();
            stream.flush().unwrap();
        }
        stream.write_all(b"0\r\n\r\n").unwrap();
    });

    let timeout = HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(100) };
    let response = session().get_with_timeout(&format!("http://{address}/slow-stream"), &[], timeout);

    assert_eq!(response.unwrap().text(), "abc");
    server.join().unwrap();
}

#[test]
fn ureq_session_rejects_a_body_over_the_size_limit() {
    // 다 읽은 뒤가 아니라 읽는 도중에 끊어야 메모리를 지킨다.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        let size = MAX_RESPONSE_BYTES + 1024 * 1024;
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n");
        let block = vec![b'x'; 64 * 1024];
        let mut sent = 0;
        while sent < size {
            if stream.write_all(&block).is_err() {
                return;
            }
            sent += block.len();
        }
    });

    let error = session().get(&format!("http://{address}/huge"), &[]).unwrap_err();

    assert!(error.to_string().contains("상한"), "{error}");
    server.join().unwrap();
}

#[test]
fn ureq_session_reuses_a_connection_after_a_complete_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        for _ in 0..2 {
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert!(request.starts_with("GET / HTTP/1.1"), "{request}");
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").unwrap();
            stream.flush().unwrap();
        }
    });

    let session = session();
    for _ in 0..2 {
        let response = session.get_with_timeout(&format!("http://{address}/"), &[], HttpTimeout::from_secs(1, 1)).unwrap();
        assert_eq!(response.text(), "ok");
    }
    server.join().unwrap();
}

#[test]
fn ureq_session_applies_response_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });

    let timeout = HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(40) };
    let result = session().get_with_timeout(&format!("http://{address}/slow"), &[], timeout);

    assert!(matches!(result, Err(HttpError::Timeout)), "{result:?}");
    server.join().unwrap();
}

#[test]
fn ureq_session_reports_a_body_that_ends_before_content_length() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc").unwrap();
        stream.flush().unwrap();
    });

    let result = session().get_with_timeout(&format!("http://{address}/short"), &[], HttpTimeout::from_secs(1, 1));

    assert!(matches!(result, Err(HttpError::Connection(_))), "{result:?}");
    server.join().unwrap();
}

#[test]
fn body_chunks_stop_after_the_first_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc").unwrap();
    });
    let agent = http_client::http_client(0);
    let response = agent.get(&format!("http://{address}/short")).call().unwrap();
    let mut chunks = http_client::BodyChunks::new(response.into_body().into_reader(), agent, Duration::from_secs(1));
    let mut saw_error = false;
    for chunk in chunks.by_ref() {
        if chunk.is_err() {
            saw_error = true;
            break;
        }
    }
    assert!(saw_error);
    // 오류를 한 번 돌려준 뒤에는 더 읽지 않는다.
    assert!(chunks.next().is_none());
    server.join().unwrap();
}
