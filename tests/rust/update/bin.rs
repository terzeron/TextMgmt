//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_update"))
}

#[test]
fn no_arguments_runs_update_get_delete_against_a_mock_server() {
    let (url, server) = mock_es(4);
    let out = bin().env("TM_ES_URL", &url).env("TM_ES_USER", "u").env("TM_ES_PASSWORD", "p").env("TM_ES_BOOK_INDEX", "books").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8(out.stdout).unwrap().contains("{'book_id': 3384, 'title': 'test title2'}"));
    let seen = server.join().unwrap();
    assert_eq!(seen[1], "POST /books/_update/3384?refresh=true HTTP/1.1");
    assert_eq!(seen[2], "GET /books/_doc/3384 HTTP/1.1");
    assert_eq!(seen[3], "DELETE /books/_doc/3384?refresh=true HTTP/1.1");
}

#[test]
fn no_arguments_without_the_environment_exits_255_before_connecting() {
    let out = bin().env_remove("TM_ES_URL").env_remove("TM_ES_USER").env_remove("TM_ES_PASSWORD").env_remove("TM_ES_BOOK_INDEX").output().unwrap();
    assert_eq!(out.status.code(), Some(255));
    assert!(String::from_utf8(out.stderr).unwrap().contains("TM_ES_BOOK_INDEX"));
}

/// 머리와 Content-Length만큼의 본문을 모두 읽는다. 머리와 본문이 따로 도착해도 안전하다.
fn read_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).unwrap();
        data.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&data).into_owned();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let wanted = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
            if body.len() >= wanted {
                return text;
            }
        }
        assert!(n > 0, "요청이 끝나기 전에 연결이 닫혔다");
    }
}

/// 요청 줄만 보고 ES처럼 답하는 서버. 요청 수만큼 연결을 받는다.
fn mock_es(requests: usize) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let head = read_request(&mut stream);
            let line = head.lines().next().unwrap().to_owned();
            let body = if line.starts_with("GET / ") {
                r#"{"version": {"number": "8.0.0"}}"#
            } else if line.contains("_update") {
                r#"{"_shards": {"failed": 0}}"#
            } else if line.starts_with("GET") {
                r#"{"_source": {"book_id": 3384, "title": "test title2"}}"#
            } else {
                r#"{"result": "deleted"}"#
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            seen.push(line);
        }
        seen
    });
    (url, handle)
}

#[test]
fn run_walks_through_update_get_and_delete_against_a_mock_server() {
    let (url, server) = mock_es(4);
    let out = bin().arg("run").env("TM_ES_URL", &url).env("TM_ES_USER", "u").env("TM_ES_PASSWORD", "p").env("TM_ES_BOOK_INDEX", "books").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8(out.stdout).unwrap().contains("{'book_id': 3384, 'title': 'test title2'}"));
    assert!(String::from_utf8(out.stderr).unwrap().contains("Elasticsearch 연결 성공"));
    let seen = server.join().unwrap();
    assert_eq!(seen[1], "POST /books/_update/3384?refresh=true HTTP/1.1");
    assert_eq!(seen[2], "GET /books/_doc/3384 HTTP/1.1");
    assert_eq!(seen[3], "DELETE /books/_doc/3384?refresh=true HTTP/1.1");
}
