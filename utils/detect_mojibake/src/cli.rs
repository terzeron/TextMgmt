//! 명령행. argparse와 같은 규칙으로 옵션을 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{
    DEFAULT_LIBRARY_ROOT, DEFAULT_WORKERS, Log, ScanOptions, default_report_path, inspect_epub_structure, inspect_file, print_epub_structure_summary, print_summary, scan, scan_epub_structure,
};

const USAGE: &str = "usage: detect_mojibake {scan,check,epub-structure} [--root ROOT] [--file FILE] [--workers WORKERS] [--limit LIMIT] [--report REPORT]";

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
    if !["scan", "check", "epub-structure"].contains(&command.as_str()) {
        return Err(format!("argument command: invalid choice: '{command}' (choose from 'scan', 'check', 'epub-structure')"));
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
    let args = match parse(argv) {
        Ok(Some(a)) => a,
        Ok(None) => {
            let _ = writeln!(out, "{USAGE}");
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
        "epub-structure" => {
            if !args.files.is_empty() {
                for f in &args.files {
                    writeln!(out, "{}", serde_json::to_string(&inspect_epub_structure(&f.to_string_lossy())).map_err(std::io::Error::other)?)?;
                }
                return Ok(0);
            }
            let report = scan_epub_structure(&args.root, args.workers, args.limit, log);
            let path = args.report.clone().unwrap_or_else(|| default_report_path("epub_structure", &args.root, repo_root));
            write_report(&path, &report)?;
            print_epub_structure_summary(&report, out)?;
            writeln!(out, "\n리포트: {}", path.display())?;
        }
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
