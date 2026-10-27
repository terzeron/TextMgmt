//! 외부 명령 대역. 설계: docs/rust-test-seams.md 3.6. book_manager_1의 `FakeRunner`를 `sysdeps::CommandRunner` 모양으로 옮기고
//! libreoffice 변환 대역(`override_convert_with_libreoffice`, `set_convert_with_libreoffice_hook`, `set_convert_override`)을 흡수했다.
//!
//! 가정한 API (아직 구현 전이다):
//! - `sysdeps::CommandRunner { fn run(&self, argv: &[String], timeout: Option<Duration>) -> Result<CommandOutput, CommandError>; fn which(&self, program: &str) -> Option<PathBuf>; }`
//! - `sysdeps::CommandOutput { returncode: i32, stdout: Vec<u8>, stderr: Vec<u8> }`
//! - `sysdeps::CommandError { Timeout, NotFound, Failed(String) }`
//!
//! 사용: `let runner = FakeRunner::new(|argv| Ok(ok_output(b"...")));` `deps.with_runner(runner.clone())`

#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysdeps::{CommandError, CommandOutput, CommandRunner};

pub type RunResult = Result<CommandOutput, CommandError>;
type Handler = Box<dyn Fn(&[String]) -> RunResult + Send + Sync>;

/// 종료 코드 0의 출력.
pub fn ok_output(stdout: impl AsRef<[u8]>) -> CommandOutput {
    CommandOutput { returncode: 0, stdout: stdout.as_ref().to_vec(), stderr: Vec::new() }
}

/// 종료 코드를 정한 출력.
pub fn output(returncode: i32, stdout: impl AsRef<[u8]>, stderr: impl AsRef<[u8]>) -> CommandOutput {
    CommandOutput { returncode, stdout: stdout.as_ref().to_vec(), stderr: stderr.as_ref().to_vec() }
}

/// `subprocess.run` / `asyncio.create_subprocess_exec` 대역. 인자 목록을 받아 결과나 오류를 돌려준다.
/// 호출한 `(argv, timeout)`을 모두 기록한다.
pub struct FakeRunner {
    handler: Mutex<Handler>,
    queue: Mutex<VecDeque<RunResult>>,
    which: Mutex<HashMap<String, PathBuf>>,
    calls: Mutex<Vec<(Vec<String>, Option<Duration>)>>,
}

impl FakeRunner {
    /// 모든 호출을 `f`가 답한다.
    pub fn new(f: impl Fn(&[String]) -> RunResult + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { handler: Mutex::new(Box::new(f)), queue: Mutex::new(VecDeque::new()), which: Mutex::new(HashMap::new()), calls: Mutex::new(Vec::new()) })
    }

    /// 항상 같은 결과. `Python subprocess.run`이 `CompletedProcess`를 돌려주는 경우다.
    pub fn returning(result: RunResult) -> Arc<Self> {
        Self::new(move |_| match &result {
            Ok(o) => Ok(CommandOutput { returncode: o.returncode, stdout: o.stdout.clone(), stderr: o.stderr.clone() }),
            Err(e) => Err(clone_error(e)),
        })
    }

    /// 호출할 때마다 큐에서 결과를 하나씩 꺼낸다. 큐가 비면 `Failed("FakeRunner 큐가 비었다")`다.
    pub fn scripted(results: Vec<RunResult>) -> Arc<Self> {
        let runner = Self::new(|_| Err(CommandError::Failed("FakeRunner 큐가 비었다".to_string())));
        *runner.queue.lock().unwrap() = results.into();
        runner
    }

    /// 변환기가 문자열을 돌려주는 단순 대역(libreoffice 텍스트 변환). stdout에 `text`를 담아 성공으로 답한다.
    pub fn office_output(text: &str) -> Arc<Self> {
        let text = text.to_string();
        Self::new(move |_| Ok(ok_output(&text)))
    }

    /// 실행 파일이 없는 환경. `run`은 `NotFound`, `which`는 `None`이다.
    pub fn not_found() -> Arc<Self> {
        Self::new(|_| Err(CommandError::NotFound))
    }

    /// 시간 초과로 끝나는 명령.
    pub fn timeout() -> Arc<Self> {
        Self::new(|_| Err(CommandError::Timeout))
    }

    /// `which(program)`이 `path`를 돌려주게 한다. 등록하지 않은 이름은 `None`이다.
    pub fn with_which(self: Arc<Self>, program: &str, path: &str) -> Arc<Self> {
        self.which.lock().unwrap().insert(program.to_string(), PathBuf::from(path));
        self
    }

    /// 호출한 `(argv, timeout)` 전체.
    pub fn calls(&self) -> Vec<(Vec<String>, Option<Duration>)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn argvs(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().iter().map(|(a, _)| a.clone()).collect()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// 마지막 호출의 argv. 호출이 없으면 panic한다.
    pub fn last_argv(&self) -> Vec<String> {
        self.calls.lock().unwrap().last().map(|(a, _)| a.clone()).expect("명령이 호출되지 않았다")
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> RunResult {
        self.calls.lock().unwrap().push((argv.to_vec(), timeout));
        if let Some(result) = self.queue.lock().unwrap().pop_front() {
            return result;
        }
        (self.handler.lock().unwrap())(argv)
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        self.which.lock().unwrap().get(program).cloned()
    }
}

/// `CommandError`가 `Clone`인지 모르므로 변형별로 복제한다.
fn clone_error(e: &CommandError) -> CommandError {
    match e {
        CommandError::Timeout => CommandError::Timeout,
        CommandError::NotFound => CommandError::NotFound,
        CommandError::Failed(m) => CommandError::Failed(m.clone()),
    }
}

/// `&str` 슬라이스를 argv(`Vec<String>`)로 바꾼다.
pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}
