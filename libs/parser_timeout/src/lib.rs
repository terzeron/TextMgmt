//! 외부 파서에 벽시계 상한을 거는 공용 유틸. 원본: `utils/parser_timeout.py`.
//!
//! Python은 SIGALRM으로 실행 중인 코드에 예외를 던지고, 손상 PDF처럼 시그널도 못 빠져나오는 경우는
//! fork한 자식 프로세스를 죽인다. Rust에서는 실행 중인 코드에 예외를 던질 수 없으므로 같은 목적을
//! 세 가지로 나눈다.
//!
//! - [`Deadline`]: 파서 루프가 스스로 확인하는 협력적 마감.
//! - [`run_with_timeout`]: 작업을 별도 스레드에서 돌리고 시간이 넘으면 기다리지 않고 돌아온다.
//!   스레드를 죽이지는 못하므로 작업은 백그라운드에서 끝까지 돈다.
//! - [`run_with_hard_timeout`]: 자식 프로세스로 실행하고 시간이 넘으면 강제 종료한다.
//!   멈춘 C 확장까지 끊어야 하는 경우에 쓴다.

use std::fmt;
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use std::os::unix::process::ExitStatusExt;

/// 파서가 제한 시간 안에 끝나지 않았다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserTimeout {
    pub message: String,
}

impl fmt::Display for ParserTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParserTimeout {}

/// Python 구현에서 삼켜진 알람을 다시 울리던 간격(초). 기존 설정과 맞추려고 값을 그대로 둔다.
pub const PARSER_TIMEOUT_RETRY_INTERVAL: f64 = 0.5;

/// 협력적 마감. 파서가 반복문 안에서 `check()`를 불러 제때 빠져나온다.
#[derive(Debug, Clone)]
pub struct Deadline {
    expires: Instant,
    message: String,
}

impl Deadline {
    pub fn new(seconds: f64, message: &str) -> Self {
        Deadline { expires: Instant::now() + Duration::from_secs_f64(seconds.max(0.0)), message: message.to_owned() }
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.expires
    }

    pub fn remaining(&self) -> Duration {
        self.expires.saturating_duration_since(Instant::now())
    }

    pub fn check(&self) -> Result<(), ParserTimeout> {
        if self.expired() { Err(ParserTimeout { message: self.message.clone() }) } else { Ok(()) }
    }
}

/// `f`를 별도 스레드에서 실행한다. `seconds`를 넘기면 결과를 기다리지 않고 `ParserTimeout`을 돌려준다.
/// `f`가 panic하면 같은 panic을 호출한 스레드에서 다시 일으킨다(Python이 자식의 예외를 그대로 던지는 것과 같다).
pub fn run_with_timeout<T: Send + 'static>(seconds: f64, message: &str, f: impl FnOnce() -> T + Send + 'static) -> Result<T, ParserTimeout> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(catch_unwind(AssertUnwindSafe(f)));
    });
    match rx.recv_timeout(Duration::from_secs_f64(seconds.max(0.0))) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(payload)) => resume_unwind(payload),
        Err(_) => Err(ParserTimeout { message: message.to_owned() }),
    }
}

fn read_all(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    })
}

/// `cmd`를 자식 프로세스로 실행한다. `timeout`초를 넘기면 강제 종료하고 `ParserTimeout`을 돌려준다.
/// 시그널에 죽은 경우도 `ParserTimeout`이다. 0이 아닌 종료 코드는 호출자가 보도록 `Output`으로 돌려준다.
pub fn run_with_hard_timeout(mut cmd: Command, timeout: f64) -> Result<Output, ParserTimeout> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child =
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| ParserTimeout { message: format!("프로세스를 실행하지 못했다: {program}: {e}") })?;
    // 출력이 파이프 버퍼보다 커도 막히지 않도록 읽는 스레드를 따로 둔다.
    let (stdout, stderr) = (child.stdout.take().map(read_all), child.stderr.take().map(read_all));

    let deadline = Instant::now() + Duration::from_secs_f64(timeout.max(0.0));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            // 시간 초과(또는 상태 확인 실패)면 강제 종료한다. 읽기 스레드는 기다리지 않는다:
            // 손자 프로세스가 파이프를 쥐고 있으면 끝나지 않을 수 있다.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ParserTimeout { message: format!("처리 시간 초과({timeout}초): {program}") });
            }
        }
    };
    let collect = |h: Option<thread::JoinHandle<Vec<u8>>>| h.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    let (stdout, stderr) = (collect(stdout), collect(stderr));
    if let Some(signal) = status.signal() {
        return Err(ParserTimeout { message: format!("처리가 비정상 종료되었습니다(signal {signal}): {program}") });
    }
    Ok(Output { status, stdout, stderr })
}
