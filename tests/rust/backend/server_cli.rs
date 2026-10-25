//! 서버 바이너리를 loopback 임시 포트에서 실행해 HTTP와 종료 코드를 검증한다.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_backend"))
}

#[test]
fn usage_and_invalid_arguments_have_stable_exit_codes() {
    for args in [vec![], vec!["--help"], vec!["serve", "--help"]] {
        let output = command().args(args).output().unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("backend serve"));
    }
    for args in [vec!["other"], vec!["serve", "--unknown"], vec!["serve", "--addr"], vec!["serve", "--addr", "invalid"], vec!["serve", "--addr", "127.0.0.1:70000"]] {
        let output = command().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
}

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start() -> (Server, String) {
    let mut child = Server(command().args(["serve", "--addr", "127.0.0.1:0"]).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap();
            if let Some(addr) = line.strip_prefix("Listening on ") {
                let _ = tx.send(addr.to_string());
                break;
            }
        }
    });
    let addr = rx.recv_timeout(Duration::from_secs(10)).expect("server did not announce a listening address");
    (child, addr)
}
fn request(addr: &str, path: &str) -> String {
    let mut socket = TcpStream::connect(addr).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    write!(socket, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn serves_real_http_and_terminates_cleanly() {
    let (mut child, addr) = start();
    assert!(request(&addr, "/auth/me").starts_with("HTTP/1.1 401"));
    let response = request(&addr, "/does-not-exist");
    assert!(response.starts_with("HTTP/1.1 404"));
    assert!(response.contains("Not Found"));
    assert!(Command::new("kill").args(["-TERM", &child.0.id().to_string()]).status().unwrap().success());
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "server did not finish graceful shutdown");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn occupied_port_is_reported_without_starting_a_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let output = command().args(["serve", "--addr", &listener.local_addr().unwrap().to_string()]).output().unwrap();
    assert_eq!(output.status.code(), Some(255));
    assert!(String::from_utf8_lossy(&output.stderr).contains("bind"));
}
