use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_classify_books_client"))
}

#[test]
fn help_option_prints_the_usage_and_exits_zero() {
    let out = bin().arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("사용법") && text.contains("classify_books_client --api-url"), "{text}");
}

#[test]
fn direct_mode_runs_against_the_given_library_root() {
    let root = std::env::temp_dir().join(format!("classify-books-client-bin-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&root).unwrap();
    let out = bin().args(["--library-root", root.to_str().unwrap(), "--category", "_root", "--limit", "10", "--dry-run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!String::from_utf8(out.stderr).unwrap().contains("classify_books_client.py"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_token_environment_variable_is_accepted_and_an_unreachable_server_exits_one() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let out = bin().args(["--api-url", &url]).env("TM_API_TOKEN", "tok-dummy").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!String::from_utf8(out.stderr).unwrap().contains("tok-dummy"), "토큰을 로그에 찍지 않는다");
}
