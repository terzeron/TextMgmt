//! 명령행. `structure`는 뷰어가 렌더링하지 못할 구조를, `scan`은 스크립트 실행 코드와 위험 구조를 찾는다.
//! 옵션은 argparse와 같은 규칙으로 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use corpus_common::{DEFAULT_LIBRARY_ROOT, DEFAULT_WORKERS, Log, default_report_path, pool};
use rayon::prelude::*;

use crate::security::{Limits, SecurityReport, Status, scan_epub};
use crate::structure::{inspect_epub_structure, print_epub_structure_summary, scan_epub_structure};

const USAGE: &str = "usage: epubscan structure [--root ROOT] [--file FILE]... [--workers N] [--limit N] [--report PATH]\n       epubscan scan [-r] [--json] [--max-member-mb N] [--max-total-mb N] [--max-files N] [--workers N] PATH...";

struct Args {
    command: String,
    root: PathBuf,
    files: Vec<PathBuf>,
    paths: Vec<PathBuf>,
    workers: usize,
    limit: usize,
    report: Option<PathBuf>,
    recursive: bool,
    json: bool,
    max_member_mb: u64,
    max_total_mb: u64,
    max_files: usize,
}

fn number<T: std::str::FromStr>(name: &str, value: &str) -> Result<T, String> {
    value.parse().map_err(|_| format!("argument --{name}: invalid int value: '{value}'"))
}

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args {
        command: String::new(),
        root: PathBuf::from(DEFAULT_LIBRARY_ROOT),
        files: Vec::new(),
        paths: Vec::new(),
        workers: DEFAULT_WORKERS,
        limit: 0,
        report: None,
        recursive: false,
        json: false,
        max_member_mb: 20,
        max_total_mb: 200,
        max_files: 10_000,
    };
    let mut it = argv.iter();
    let mut command = None;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "-r" | "--recursive" => a.recursive = true,
            "--json" => a.json = true,
            _ if arg.starts_with("--") => {
                let (name, inline) = match arg[2..].split_once('=') {
                    Some((n, v)) => (n, Some(v.to_owned())),
                    None => (&arg[2..], None),
                };
                if !["root", "file", "workers", "limit", "report", "max-member-mb", "max-total-mb", "max-files"].contains(&name) {
                    return Err(format!("unrecognized arguments: {arg}"));
                }
                let value = inline.or_else(|| it.next().cloned()).ok_or_else(|| format!("argument --{name}: expected one argument"))?;
                match name {
                    "root" => a.root = value.into(),
                    "file" => a.files.push(value.into()),
                    "report" => a.report = Some(value.into()),
                    "workers" => a.workers = number(name, &value)?,
                    "limit" => a.limit = number(name, &value)?,
                    "max-member-mb" => a.max_member_mb = number(name, &value)?,
                    "max-total-mb" => a.max_total_mb = number(name, &value)?,
                    _ => a.max_files = number(name, &value)?,
                }
            }
            _ if command.is_none() => command = Some(arg.clone()),
            _ => a.paths.push(arg.into()),
        }
    }
    let command = command.ok_or("the following arguments are required: command")?;
    if command != "structure" && command != "scan" {
        return Err(format!("argument command: invalid choice: '{command}' (choose from 'structure', 'scan')"));
    }
    if command == "scan" && (a.max_member_mb == 0 || a.max_total_mb == 0 || a.max_files == 0) {
        return Err("Limits must be positive".into());
    }
    a.command = command;
    Ok(Some(a))
}

/// 진입점. 종료 코드를 돌려준다. `repo_root`는 구조 검사 리포트(`<repo>/tmp`)의 기준이다.
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
            eprintln!("{USAGE}\nepubscan: error: {msg}");
            return 2;
        }
    };
    let result = if args.command == "structure" { structure(&args, repo_root, out, log) } else { scan(&args, out) };
    match result {
        Ok(code) => code,
        Err(e) => {
            log(&format!("ERROR {e}"));
            1
        }
    }
}

fn structure(args: &Args, repo_root: &Path, out: &mut dyn Write, log: Log) -> io::Result<i32> {
    if !args.files.is_empty() {
        for f in &args.files {
            writeln!(out, "{}", serde_json::to_string(&inspect_epub_structure(&f.to_string_lossy())).map_err(io::Error::other)?)?;
        }
        return Ok(0);
    }
    let report = scan_epub_structure(&args.root, args.workers, args.limit, log);
    let path = args.report.clone().unwrap_or_else(|| default_report_path("epub_structure", &args.root, repo_root));
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&report).map_err(io::Error::other)?)?;
    print_epub_structure_summary(&report, out)?;
    writeln!(out, "\n리포트: {}", path.display())?;
    Ok(0)
}

fn is_epub(p: &Path) -> bool {
    p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("epub"))
}

fn walk(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() && !fs::symlink_metadata(&p).is_ok_and(|m| m.is_symlink()) {
            if recursive {
                walk(&p, recursive, out);
            }
        } else if is_epub(&p) {
            out.push(p);
        }
    }
}

fn collect(args: &Args) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for p in &args.paths {
        if p.is_file() {
            found.push(p.clone());
        } else if p.is_dir() {
            walk(p, args.recursive, &mut found);
        } else {
            eprintln!("Not found: {}", p.display());
        }
    }
    // 중복은 처음 것만 남긴다.
    let mut seen = std::collections::HashSet::new();
    found.retain(|p| seen.insert(p.clone()));
    found
}

fn print_text(r: &SecurityReport, out: &mut dyn Write) -> io::Result<()> {
    let label = match r.status {
        Status::None => "NONE",
        Status::Findings => "FINDINGS",
        Status::Incomplete => "INCOMPLETE",
    };
    writeln!(out, "\n== {} == [{label}] ({} entries)", r.file, r.entries)?;
    if r.findings.is_empty() {
        writeln!(out, "No matched indicators (not a safety guarantee)")?;
    }
    for f in &r.findings {
        let sev = format!("{:?}", f.severity).to_uppercase();
        writeln!(out, "[{sev}] {} | {} | {}", f.code, f.location, f.detail)?;
    }
    for n in &r.notes {
        writeln!(out, "note: {n}")?;
    }
    Ok(())
}

fn scan(args: &Args, out: &mut dyn Write) -> io::Result<i32> {
    let paths = collect(args);
    if paths.is_empty() {
        eprintln!("No EPUB files found");
        return Ok(2);
    }
    let limits = Limits { max_member: args.max_member_mb * 1024 * 1024, max_total: args.max_total_mb * 1024 * 1024, max_files: args.max_files };
    let reports: Vec<SecurityReport> = pool(args.workers).install(|| paths.par_iter().map(|p| scan_epub(p, &limits)).collect());
    if args.json {
        writeln!(out, "{}", serde_json::to_string_pretty(&reports).map_err(io::Error::other)?)?;
    } else {
        for r in &reports {
            print_text(r, out)?;
        }
    }
    // 지표가 하나라도 있으면 1이다. 검사하지 못한 파일이 있어도 지표를 2 뒤에 숨기지 않는다.
    Ok(if reports.iter().any(|r| !r.findings.is_empty()) {
        1
    } else if reports.iter().any(|r| r.status == Status::Incomplete) {
        2
    } else {
        0
    })
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(out: &mut dyn Write) -> io::Result<()> {
    write!(
        out,
        "epubscan - EPUB 정적 검사. 렌더링하거나 스크립트를 실행하지 않고 구조와 문자열만 본다.

구조 검사(structure)는 epubcheck는 통과하지만 뷰어가 렌더링하지 못하는 구조를 찾고,
위험 검사(scan)는 스크립트·활성 콘텐츠·위험한 ZIP 구조(경로 탈출, 실행 파일, XXE 등)를 찾는다.

사용법:
  epubscan structure [--root DIR] [--file PATH ...] [--workers N] [--limit N] [--report PATH]
  epubscan scan [-r] [--json] [--max-member-mb N] [--max-total-mb N] [--max-files N] [--workers N] PATH...

명령:
  structure  라이브러리 또는 지정한 파일의 EPUB 구조 문제를 찾는다.
  scan       EPUB 파일이나 디렉토리의 스크립트 실행 코드와 위험 구조를 찾는다.

인자:
  PATH   scan 대상 EPUB 파일 또는 디렉토리 (여러 개 가능)

옵션:
  --root DIR          structure 대상 디렉토리 (기본 {DEFAULT_LIBRARY_ROOT})
  --file PATH         structure 대상 파일. 지정하면 파일당 JSON 한 줄을 출력한다.
  --report PATH       structure 리포트 경로 (기본 <repo>/tmp/epub_structure_report_<디렉토리>.json)
  --limit N           structure에서 앞에서 N건만 처리한다 (0이면 전체).
  -r, --recursive     scan에서 디렉토리를 하위까지 훑는다.
  --json              scan 결과를 JSON으로 출력한다.
  --max-member-mb N   scan이 멤버 하나에서 읽는 최대 크기 MB (기본 20)
  --max-total-mb N    scan이 파일 하나에서 읽는 최대 크기 MB (기본 200)
  --max-files N       scan이 검사하는 최대 항목 수 (기본 10000)
  --workers N         병렬 작업 수 (기본 {DEFAULT_WORKERS})
  -h, --help          이 도움말

종료 코드:
  structure  0 정상, 1 실행 오류, 2 잘못된 인자
  scan       0 지표 없음, 1 지표 있음, 2 검사하지 못한 파일이 있고 지표는 없음 또는 잘못된 인자
"
    )
}
