//! 파서에 벽시계 상한을 거는 도구.
//!
//! 실행 중인 코드를 밖에서 멈출 수는 없으므로 목적에 따라 둘로 나눈다.
//!
//! - [`run_with_timeout`]: 작업을 별도 스레드에서 돌리고 시간이 넘으면 기다리지 않고 돌아온다.
//!   스레드를 죽이지는 못하므로 작업은 백그라운드에서 끝까지 돈다.
//! - [`run_with_hard_timeout`]: 자식 프로세스로 실행하고 시간이 넘으면 강제 종료한다.
//!   멈춘 C 확장까지 끊어야 하는 경우에 쓴다. 실행은 [`crate::SystemRunner`]와 같은 엔진이 맡는다.

use std::fmt;
use std::os::unix::process::ExitStatusExt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::{CommandError, run_child};

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

/// `f`를 별도 스레드에서 실행한다. `seconds`를 넘기면 결과를 기다리지 않고 `ParserTimeout`을 돌려준다.
/// `f`가 panic하면 같은 panic을 호출한 스레드에서 다시 일으킨다.
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

/// `cmd`를 자식 프로세스로 실행한다. `timeout`초를 넘기면 강제 종료하고 `ParserTimeout`을 돌려준다.
/// 실행하지 못한 경우와 시그널에 죽은 경우도 `ParserTimeout`이다. 0이 아닌 종료 코드는 호출자가 보도록 `Output`으로 돌려준다.
pub fn run_with_hard_timeout(cmd: Command, timeout: f64) -> Result<Output, ParserTimeout> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let output = run_child(cmd, Some(Duration::from_secs_f64(timeout.max(0.0)))).map_err(|error| ParserTimeout {
        message: match error {
            CommandError::Timeout => format!("처리 시간 초과({timeout}초): {program}"),
            other => format!("프로세스를 실행하지 못했다: {program}: {other}"),
        },
    })?;
    if let Some(signal) = output.status.signal() {
        return Err(ParserTimeout { message: format!("처리가 비정상 종료되었습니다(signal {signal}): {program}") });
    }
    Ok(output)
}
