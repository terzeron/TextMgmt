//! 파서 시간 상한 테스트. 자식 프로세스를 상한에서 강제 종료하는 `run_with_hard_timeout`만 있다.

use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use sysdeps::timeout_runner::*;

#[test]
fn hard_timeout_returns_the_output_of_a_command_that_finishes() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "echo hi"]);
    let out = run_with_hard_timeout(cmd, 2.0).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    assert!(out.status.success());
}

#[test]
fn hard_timeout_passes_a_failing_exit_code_to_the_caller() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "echo bad >&2; exit 3"]);
    let out = run_with_hard_timeout(cmd, 2.0).unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("bad"));
}

#[test]
fn hard_timeout_kills_a_command_that_runs_too_long() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "sleep 5"]);
    let start = Instant::now();
    let err = run_with_hard_timeout(cmd, 0.2).unwrap_err();
    assert!(err.message.contains("처리 시간 초과"), "{}", err.message);
    assert!(start.elapsed() < Duration::from_secs(3), "자식을 죽이고 돌아온다");
}

#[test]
fn hard_timeout_reports_a_command_killed_by_a_signal_as_abnormal() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "kill -9 $$"]);
    let err = run_with_hard_timeout(cmd, 2.0).unwrap_err();
    assert!(err.message.contains("처리가 비정상 종료되었습니다"), "{}", err.message);
}

#[test]
fn hard_timeout_reports_a_command_that_cannot_start() {
    let err = run_with_hard_timeout(Command::new("/nonexistent/binary"), 1.0).unwrap_err();
    assert!(err.message.contains("실행하지 못했다"), "{}", err.message);
}

#[test]
fn hard_timeout_collects_large_output_without_deadlocking() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "head -c 300000 /dev/zero | tr '\\0' 'x'"]);
    let out = run_with_hard_timeout(cmd, 5.0).unwrap();
    assert_eq!(out.stdout.len(), 300_000);
}

#[test]
fn hard_timeout_bounds_pipe_drain_when_a_grandchild_inherits_output_pipes() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let marker = std::env::temp_dir().join(format!("parser-timeout-grandchild-{nonce}"));
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "(sleep 0.4; touch \"$1\") & exit 0", "sh", marker.to_str().unwrap()]);
    let start = Instant::now();
    let error = run_with_hard_timeout(cmd, 0.2).unwrap_err();
    assert!(error.message.contains("처리 시간 초과"));
    assert!(start.elapsed() < Duration::from_millis(500), "reader waited for the grandchild pipe");
    thread::sleep(Duration::from_millis(450));
    assert!(!marker.exists(), "timed-out process group must not leave a grandchild running");
}
