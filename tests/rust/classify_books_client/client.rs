use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;
use std::time::Duration;

use classify_books_client::{Options, parse_options, run, run_api_client};

/// 머리와 Content-Length만큼의 본문을 모두 읽는다. 머리와 본문이 따로 도착해도 안전하다.
fn read_request(stream: &mut std::net::TcpStream) -> String {
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

/// 준비한 응답을 순서대로 돌려주는 서버. 받은 요청(머리+본문)을 모아 돌려준다.
fn mock_api(responses: Vec<(u16, &'static str)>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            seen.push(read_request(&mut stream));
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(reply.as_bytes()).unwrap();
        }
        seen
    });
    (url, handle)
}

fn options(url: &str) -> Options {
    Options { api_url: url.to_owned(), poll_interval: Duration::from_millis(1), ..Options::default() }
}

fn quiet() -> impl Fn(&str, &str) + Sync {
    |_, _| {}
}

#[test]
fn polls_until_done_and_sends_the_payload_with_the_bearer_token() {
    let (url, server) = mock_api(vec![
        (200, r#"{"status": "success"}"#),
        (200, r#"{"result": {"status": "running", "processed_count": 1, "total_count": 4}}"#),
        (200, r#"{"result": {"status": "done", "processed_count": 4, "total_count": 4, "moved_count": 3}}"#),
    ]);
    let mut opts = options(&url);
    opts.token = Some("tok-dummy".into());
    opts.category = "9_temp".into();
    opts.dry_run = true;
    opts.no_bookstore = true;

    assert_eq!(run_api_client(&opts, &quiet()), 0);

    let seen = server.join().unwrap();
    assert!(seen[0].starts_with("POST /categories/auto-classify HTTP/1.1"), "{}", seen[0]);
    assert!(seen[0].to_lowercase().contains("authorization: bearer tok-dummy"));
    let body = seen[0].split("\r\n\r\n").nth(1).unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["category"], "9_temp");
    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["async_mode"], true);
    assert_eq!(payload["recursive"], true);
    assert_eq!(payload["clean_existing"], true);
    assert_eq!(payload["use_bookstore"], false);
    assert_eq!(payload["use_content_meta"], true);
    assert_eq!(payload["delay"], 1.2);
    assert!(seen[1].starts_with("GET /categories/auto-classify-status HTTP/1.1"));
}

#[test]
fn a_trailing_slash_in_the_api_url_is_ignored_and_no_token_means_no_header() {
    let (url, server) = mock_api(vec![(200, r#"{"status": "success"}"#), (200, r#"{"result": {"status": "idle"}}"#)]);
    let opts = options(&format!("{url}/"));
    assert_eq!(run_api_client(&opts, &quiet()), 0);
    let seen = server.join().unwrap();
    assert!(seen[0].starts_with("POST /categories/auto-classify HTTP/1.1"));
    assert!(!seen[0].to_lowercase().contains("authorization"));
}

#[test]
fn a_failed_job_exits_one_with_the_server_error() {
    let (url, server) = mock_api(vec![(200, r#"{"status": "success"}"#), (200, r#"{"result": {"status": "failed", "error": "boom"}}"#)]);
    let logs = std::sync::Mutex::new(Vec::new());
    let log = |level: &str, msg: &str| logs.lock().unwrap().push(format!("{level} {msg}"));
    assert_eq!(run_api_client(&options(&url), &log), 1);
    server.join().unwrap();
    assert!(logs.lock().unwrap().iter().any(|l| l.starts_with("ERROR") && l.contains("boom")));
}

#[test]
fn a_rejected_start_request_exits_one() {
    let (url, server) = mock_api(vec![(401, r#"{"detail": "no"}"#)]);
    assert_eq!(run_api_client(&options(&url), &quiet()), 1);
    server.join().unwrap();

    let (url, server) = mock_api(vec![(200, r#"{"status": "error", "error": "bad category"}"#)]);
    assert_eq!(run_api_client(&options(&url), &quiet()), 1);
    server.join().unwrap();

    let (url, server) = mock_api(vec![(200, "not json")]);
    assert_eq!(run_api_client(&options(&url), &quiet()), 1);
    server.join().unwrap();
}

#[test]
fn status_errors_are_retried_but_not_forever() {
    let (url, server) = mock_api(vec![(200, r#"{"status": "success"}"#), (503, "{}"), (200, r#"{"result": {"status": "done"}}"#)]);
    assert_eq!(run_api_client(&options(&url), &quiet()), 0);
    server.join().unwrap();

    let mut responses = vec![(200, r#"{"status": "success"}"#)];
    responses.extend(std::iter::repeat_n((500, "{}"), 3));
    let (url, server) = mock_api(responses);
    let mut opts = options(&url);
    opts.max_status_failures = 3;
    assert_eq!(run_api_client(&opts, &quiet()), 1);
    server.join().unwrap();
}

#[test]
fn an_unreachable_server_exits_one() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    assert_eq!(run_api_client(&options(&url), &quiet()), 1);
}

#[test]
fn parse_options_reads_every_flag_and_the_token_environment_fallback() {
    let args: Vec<String> =
        ["--api-url", "http://h:1", "--category", "4_대기", "--delay", "0.5", "--dry-run", "--no-recursive", "--no-clean-existing", "--no-bookstore", "--no-content-meta", "--token", "t"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let o = parse_options(&args, None).unwrap();
    assert_eq!((o.api_url.as_str(), o.category.as_str(), o.delay, o.token.as_deref()), ("http://h:1", "4_대기", 0.5, Some("t")));
    assert!(o.dry_run && o.no_recursive && o.no_clean_existing && o.no_bookstore && o.no_content_meta);

    let o = parse_options(&["--api-url=http://h:1".to_string(), "--category=x".to_string()], Some("env-tok".into())).unwrap();
    assert_eq!((o.category.as_str(), o.token.as_deref()), ("x", Some("env-tok")));
    let o = parse_options(&["--api-url=http://h:1".to_string(), "--token=cli".to_string()], Some("env-tok".into())).unwrap();
    assert_eq!(o.token.as_deref(), Some("cli"), "명령행이 환경변수보다 앞선다");

    assert!(parse_options(&[], None).unwrap_err().contains("--api-url"));
    assert!(parse_options(&["--api-url".into()], None).unwrap_err().contains("expected one argument"));
    assert!(parse_options(&["--api-url=h".into(), "--delay=x".into()], None).unwrap_err().contains("--delay"));
    assert!(parse_options(&["--api-url=h".into(), "--delay=-1".into()], None).unwrap_err().contains("--delay"));
    assert!(parse_options(&["--api-url=h".into(), "--limit".into(), "5".into()], None).unwrap_err().contains("직접 모드"));
    assert!(parse_options(&["--api-url=h".into(), "--bogus".into()], None).unwrap_err().contains("unrecognized"));
}

#[test]
fn run_prints_usage_without_arguments_and_rejects_bad_ones() {
    let mut out = Vec::new();
    assert_eq!(run("cbc", &[], None, &mut out), 0);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("사용법") && text.contains("--api-url"));
    assert_eq!(run("cbc", &["--help".into()], None, &mut Vec::new()), 0);
    assert_eq!(run("cbc", &["--bogus".into()], None, &mut Vec::new()), 2);
    assert_eq!(run("cbc", &["--limit".into(), "5".into()], None, &mut Vec::new()), 2);
}
