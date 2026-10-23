use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_classify_books_client"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("사용법") && text.contains("classify_books_client --api-url"), "{text}");
}

#[test]
fn direct_mode_options_exit_two_with_a_pointer_to_the_python_tool() {
    let out = bin().args(["--limit", "10"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stderr).unwrap().contains("classify_books_client.py"));
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
