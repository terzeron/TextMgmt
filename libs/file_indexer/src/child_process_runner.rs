//! 파서를 자식 프로세스에서 돌린다. 파서가 멈추거나 죽어도 부모(loader, backend)는 시간 상한 뒤 프로세스째 끝낸다.
//!
//! 부모는 자기 실행 파일을 숨은 부명령으로 다시 띄운다. 그래서 이 부명령을 받는 쪽(`run_*_command`)도 여기에 둔다.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// 실행기를 받는 쪽(loader 등)이 doc_reader에 직접 의존하지 않게 다시 내보낸다.
pub use doc_reader::pdf::StageRunner;
use doc_reader::pdf::{PDF_STAGE_TIMEOUT, Stage, StageError};
use parse_stat_counter::Stat;
use sysdeps::timeout_runner::run_with_hard_timeout;

use crate::env_config::Config;
use crate::record_builder::ReadOutcome;

/// 자식 프로세스로 PDF 단계(본문·ISBN)를 돌리는 숨은 부명령.
pub const PDF_STAGE_COMMAND: &str = "__pdf-stage";
/// Backend의 격리된 파일 파서 부명령.
pub const READ_FILE_COMMAND: &str = "__read-file";

const STAGES: [Stage; 5] = [Stage::Poppler, Stage::Lopdf, Stage::Repair, Stage::HeadTail, Stage::HeadTailScan];

/// 부명령 인자로 쓰는 단계 키. 모두 `-`를 넣는다. 테스트 바이너리가 실수로 worker로 뜨면 인자를
/// 테스트 이름 필터로 읽는데, Rust 식별자에는 `-`가 없어 어떤 테스트도 고르지 않는다.
fn stage_key(stage: Stage) -> &'static str {
    match stage {
        Stage::Poppler => "text-poppler",
        Stage::Lopdf => "text-lopdf",
        Stage::Repair => "text-repair",
        Stage::HeadTail => "isbn-head-tail",
        Stage::HeadTailScan => "isbn-stream-scan",
    }
}

fn stage_from_key(key: &str) -> Option<Stage> {
    STAGES.into_iter().find(|s| stage_key(*s) == key)
}

/// PDF 단계를 자식 프로세스(이 실행 파일의 숨은 부명령)로 돌리고 시간 상한이 넘으면 프로세스 그룹째 종료한다.
pub struct ChildRunner {
    pub exe: PathBuf,
    pub timeout_seconds: f64,
}

impl ChildRunner {
    pub fn new(exe: PathBuf) -> Self {
        ChildRunner { exe, timeout_seconds: PDF_STAGE_TIMEOUT as f64 }
    }
}

impl StageRunner for ChildRunner {
    fn run(&self, stage: Stage, path: &Path) -> Result<(String, usize), StageError> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg(PDF_STAGE_COMMAND).arg(stage_key(stage)).arg(path);
        let output = run_with_hard_timeout(cmd, self.timeout_seconds).map_err(|_| StageError::Timeout)?;
        if !output.status.success() {
            return Err(StageError::Failed(String::from_utf8_lossy(&output.stderr).trim().to_owned()));
        }
        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| StageError::Failed(e.to_string()))?;
        Ok((parsed["text"].as_str().unwrap_or_default().to_owned(), parsed["pages"].as_u64().unwrap_or(0) as usize))
    }
}

/// 숨은 부명령 `__pdf-stage <단계> <경로>`: 단계를 실행해 JSON으로 쓴다.
pub fn run_pdf_stage_command(args: &[String], out: &mut dyn Write) -> i32 {
    let (Some(stage), Some(path)) = (args.first().and_then(|k| stage_from_key(k)), args.get(1)) else {
        let keys: Vec<&str> = STAGES.into_iter().map(stage_key).collect();
        eprintln!("usage: {PDF_STAGE_COMMAND} <{}> <path>", keys.join("|"));
        return 2;
    };
    match doc_reader::pdf::run_stage(stage, Path::new(path)) {
        Ok((text, pages)) => {
            let _ = writeln!(out, "{}", serde_json::json!({"text": text, "pages": pages}));
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[derive(Debug)]
pub enum ChildReadError {
    Timeout(sysdeps::timeout_runner::ParserTimeout),
    Other(String),
}

/// 파일 하나를 자식 프로세스에서 읽어 파서가 멈추면 프로세스 전체를 종료한다.
pub fn read_file_in_child(executable: &Path, cfg: &Config, path: &Path, path_prefix: Option<&Path>, skip_text: bool, timeout: Duration) -> Result<ReadOutcome, ChildReadError> {
    let mut command = Command::new(executable);
    command
        .arg(READ_FILE_COMMAND)
        .arg(&cfg.book_prefix)
        .arg(&cfg.comics_prefix)
        .arg(path)
        .arg(if path_prefix.is_some() { "1" } else { "0" })
        .arg(path_prefix.unwrap_or(Path::new("")))
        .arg(if skip_text { "1" } else { "0" });
    let output = run_with_hard_timeout(command, timeout.as_secs_f64()).map_err(ChildReadError::Timeout)?;
    if !output.status.success() {
        return Err(ChildReadError::Other(String::from_utf8_lossy(&output.stderr).trim().to_string()));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| ChildReadError::Other(e.to_string()))?;
    let item = match value.get("item") {
        Some(serde_json::Value::Array(parts)) if parts.len() == 2 => {
            let id = parts[0].as_i64().ok_or_else(|| ChildReadError::Other("worker returned invalid document id".into()))?;
            Some((id, parts[1].clone()))
        }
        Some(serde_json::Value::Null) | None => None,
        _ => return Err(ChildReadError::Other("worker returned invalid document".into())),
    };
    let messages = value.get("messages").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(serde_json::Value::as_str).map(str::to_owned).collect();
    Ok(ReadOutcome { item, messages })
}

/// 숨은 부명령 `__read-file`을 실행한다. 출력은 부모가 읽는 JSON 한 줄이다.
pub fn run_read_file_command(args: &[String], out: &mut dyn Write) -> i32 {
    if args.len() != 6 {
        eprintln!("usage: {READ_FILE_COMMAND} <book-root> <comics-root> <path> <has-prefix> <prefix> <skip-text>");
        return 2;
    }
    let cfg = Config::new(&args[0], &args[1]);
    let path = Path::new(&args[2]);
    let prefix = (args[3] == "1").then(|| Path::new(&args[4]));
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("backend"));
    let result = read_file_with_child_stages(&executable, &cfg, path, prefix, args[5] == "1");
    let _ = writeln!(out, "{}", serde_json::json!({"item": result.item, "messages": result.messages}));
    0
}

/// 파일 하나를 이 프로세스에서 읽되 PDF 단계는 `executable`의 자식 프로세스로 돌린다. 통계는 전역 [`Stat`]에 쌓는다.
/// 파일 전체 상한 없이 읽는 호출자도 멈춘 PDF 파서는 단계 상한에서 종료된다.
pub fn read_file_with_child_stages(executable: &Path, cfg: &Config, path: &Path, path_prefix: Option<&Path>, skip_text: bool) -> ReadOutcome {
    let runner = ChildRunner::new(executable.to_path_buf());
    crate::record_builder::read_file(cfg, path, None, skip_text, path_prefix, &runner, Stat::global())
}
