//! 파서 시간 상한 테스트 (tests/test_parser_timeout.py 이식).
//! Python은 SIGALRM으로 실행 중인 코드에 예외를 던지고 fork한 자식을 죽인다. Rust는 같은 목적을
//! 협력적 `Deadline`, 스레드 기반 `run_with_timeout`, 자식 프로세스 강제 종료 `run_with_hard_timeout`로 나눈다.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use parser_timeout::*;

#[test]
fn time_limit_returns_the_value_when_the_work_finishes_in_time() {
    assert_eq!(run_with_timeout(1.0, "Timeout", || 1 + 1), Ok(2));
}

#[test]
fn time_limit_reports_the_message_when_the_work_runs_too_long() {
    let start = Instant::now();
    let err = run_with_timeout(0.1, "Exceeded limit", || thread::sleep(Duration::from_millis(600))).unwrap_err();
    assert!(err.to_string().contains("Exceeded limit"));
    assert_eq!(err.message, "Exceeded limit");
    assert!(start.elapsed() < Duration::from_millis(500), "기다리지 않고 바로 돌아온다");
}

#[test]
fn time_limit_works_from_a_worker_thread_too() {
    let handle = thread::spawn(|| run_with_timeout(1.0, "Worker timeout", || 42));
    assert_eq!(handle.join().unwrap(), Ok(42));
}

#[test]
fn a_panic_inside_the_work_is_passed_through_like_the_child_process_error() {
    let caught = catch_unwind(AssertUnwindSafe(|| run_with_timeout(2.0, "t", || -> i32 { panic!("Something went wrong") })));
    let payload = caught.unwrap_err();
    assert_eq!(payload.downcast_ref::<&str>().copied(), Some("Something went wrong"));
}

#[test]
fn deadline_checks_are_cooperative() {
    let d = Deadline::new(0.05, "slow parser");
    assert!(d.check().is_ok());
    assert!(!d.expired());
    thread::sleep(Duration::from_millis(80));
    assert!(d.expired());
    assert_eq!(d.check().unwrap_err().message, "slow parser");
    assert!(d.remaining() == Duration::ZERO);
    assert!(Deadline::new(10.0, "x").remaining() > Duration::from_secs(5));
}

#[test]
fn the_retry_interval_matches_the_python_constant() {
    assert_eq!(PARSER_TIMEOUT_RETRY_INTERVAL, 0.5);
}

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
