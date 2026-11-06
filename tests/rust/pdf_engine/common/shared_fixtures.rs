//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]

use std::time::Duration;
use sysdeps::{CommandError, CommandRunner};

pub struct FakeRunner {
    pub outputs: std::sync::Mutex<Vec<sysdeps::CommandOutput>>,
    pub calls: std::sync::Mutex<Vec<Vec<String>>>,
}

impl CommandRunner for FakeRunner {
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> Result<sysdeps::CommandOutput, CommandError> {
        assert_eq!(timeout, Some(Duration::from_secs(30)));
        self.calls.lock().unwrap().push(argv.to_vec());
        Ok(self.outputs.lock().unwrap().remove(0))
    }

    fn which(&self, program: &str) -> Option<std::path::PathBuf> {
        Some(std::path::PathBuf::from(format!("/usr/bin/{program}")))
    }
}

pub fn runner(outputs: &[(&str, &str, i32)]) -> FakeRunner {
    FakeRunner {
        outputs: std::sync::Mutex::new(
            outputs.iter().map(|(stdout, stderr, returncode)| sysdeps::CommandOutput { stdout: stdout.as_bytes().to_vec(), stderr: stderr.as_bytes().to_vec(), returncode: *returncode }).collect(),
        ),
        calls: std::sync::Mutex::new(Vec::new()),
    }
}
