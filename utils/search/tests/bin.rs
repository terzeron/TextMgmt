//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_search"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("search") && text.contains("사용법") || text.contains("Usage:"), "{text}");
}

#[test]
fn missing_environment_variables_exit_255() {
    let out = bin().arg("제목").env_remove("TM_ES_URL").env_remove("TM_ES_USER").env_remove("TM_ES_PASSWORD").env_remove("TM_ES_BOOK_INDEX").output().unwrap();
    assert_eq!(out.status.code(), Some(255));
    assert!(String::from_utf8(out.stderr).unwrap().contains("TM_ES_BOOK_INDEX"));
}

#[test]
fn a_title_search_runs_end_to_end_against_a_mock_server() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).unwrap();
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let body = if head.starts_with("GET / ") {
                r#"{"version": {}}"#.to_string()
            } else {
                r#"{"hits": {"max_score": 2.0, "hits": [{"_id": "7", "_score": 2.0, "_source": {"title": "삼국지", "summary": "요약 본문"}}]}}"#.to_string()
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let out = bin().arg("삼국지").env("TM_ES_URL", &url).env("TM_ES_USER", "u").env("TM_ES_PASSWORD", "p").env("TM_ES_BOOK_INDEX", "books").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("item: {'title': '삼국지', 'summary': '요약 본문'}") && text.contains("요약 본문"), "{text}");
    server.join().unwrap();
}
