//! 명령행. argparse와 같은 규칙으로 옵션을 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::{DEFAULT_LIBRARY_ROOT, DEFAULT_WORKERS, Log, ScanOptions, default_report_path, inspect_file, print_summary, scan};

const USAGE: &str = "usage: detect_mojibake {scan,check} [--root ROOT] [--file FILE] [--workers WORKERS] [--limit LIMIT] [--report REPORT]";

struct Args {
    command: String,
    root: PathBuf,
    files: Vec<PathBuf>,
    workers: usize,
    limit: usize,
    report: Option<PathBuf>,
}

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut command = None;
    let mut args = Args { command: String::new(), root: PathBuf::from(DEFAULT_LIBRARY_ROOT), files: Vec::new(), workers: DEFAULT_WORKERS, limit: 0, report: None };
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if a == "-h" || a == "--help" {
            return Ok(None);
        }
        if let Some(opt) = a.strip_prefix("--") {
            let (name, inline) = match opt.split_once('=') {
                Some((n, v)) => (n, Some(v.to_owned())),
                None => (opt, None),
            };
            let mut value = || inline.clone().or_else(|| it.next().cloned()).ok_or_else(|| format!("argument --{name}: expected one argument"));
            match name {
                "root" => args.root = value()?.into(),
                "file" => args.files.push(value()?.into()),
                "report" => args.report = Some(value()?.into()),
                "workers" | "limit" => {
                    let v = value()?;
                    let n: usize = v.parse().map_err(|_| format!("argument --{name}: invalid int value: '{v}'"))?;
                    if name == "workers" { args.workers = n } else { args.limit = n }
                }
                _ => return Err(format!("unrecognized arguments: {a}")),
            }
        } else if command.is_none() {
            command = Some(a.clone());
        } else {
            return Err(format!("unrecognized arguments: {a}"));
        }
    }
    let command = command.ok_or("the following arguments are required: command")?;
    if !["scan", "check"].contains(&command.as_str()) {
        return Err(format!("argument command: invalid choice: '{command}' (choose from 'scan', 'check')"));
    }
    args.command = command;
    Ok(Some(args))
}

fn write_report(path: &Path, report: &impl serde::Serialize) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(report).map_err(std::io::Error::other)?)
}

/// 진입점. 종료 코드를 돌려준다. `repo_root`는 기본 리포트 위치(`<repo>/tmp`)의 기준이다.
pub fn run(argv: &[String], repo_root: &Path, out: &mut dyn Write, log: Log) -> i32 {
    if argv.is_empty() {
        let _ = print_usage(out);
        return 0;
    }
    let args = match parse(argv) {
        Ok(Some(a)) => a,
        Ok(None) => {
            let _ = print_usage(out);
            return 0;
        }
        Err(msg) => {
            eprintln!("{USAGE}\ndetect_mojibake: error: {msg}");
            return 2;
        }
    };
    match execute(&args, repo_root, out, log) {
        Ok(code) => code,
        Err(e) => {
            log(&format!("ERROR {e}"));
            1
        }
    }
}

fn execute(args: &Args, repo_root: &Path, out: &mut dyn Write, log: Log) -> std::io::Result<i32> {
    match args.command.as_str() {
        "check" => {
            if args.files.is_empty() {
                log("ERROR --file 로 대상을 지정하라");
                return Ok(1);
            }
            for f in &args.files {
                writeln!(out, "{}", serde_json::to_string(&inspect_file(&f.to_string_lossy())).map_err(std::io::Error::other)?)?;
            }
        }
        _ => {
            let report = scan(&args.root, &ScanOptions { workers: args.workers, limit: args.limit, progress_every: 2000 }, log);
            let path = args.report.clone().unwrap_or_else(|| default_report_path("mojibake", &args.root, repo_root));
            write_report(&path, &report)?;
            print_summary(&report, out)?;
            writeln!(out, "\n리포트: {}", path.display())?;
        }
    }
    Ok(0)
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(out: &mut dyn Write) -> io::Result<()> {
    write!(
        out,
        "detect_mojibake - 라이브러리에서 인코딩이 손상된(mojibake) 문서를 찾는다.

원본 텍스트를 잘못된 코드페이지로 읽고 다시 UTF-8로 저장해 내용만 깨진 txt·epub 파일을
한글 음절 분포(korean_likeness)로 판정한다. 한문 원전과 영문 문서는 판정 대상에서 뺀다.

사용법:
  detect_mojibake scan  [--root DIR] [--workers N] [--limit N] [--report PATH]
  detect_mojibake check --file PATH [--file PATH ...]

명령:
  scan    디렉토리를 훑어 판정 결과를 JSON 리포트로 저장하고 요약을 출력한다.
  check   지정한 파일만 판정해 파일당 JSON 한 줄을 출력한다.

옵션:
  --root DIR      검사할 디렉토리 (기본 {DEFAULT_LIBRARY_ROOT})
  --file PATH     check 대상 파일. 여러 번 지정할 수 있다.
  --workers N     병렬 작업 수 (기본 {DEFAULT_WORKERS}). 회전 디스크라 늘려도 I/O에 막힌다.
  --limit N       앞에서 N건만 처리한다 (0이면 전체).
  --report PATH   리포트 경로 (기본 <repo>/tmp/mojibake_report_<디렉토리>.json)
  -h, --help      이 도움말

종료 코드: 0 정상, 1 실행 오류, 2 잘못된 인자
"
    )
}
