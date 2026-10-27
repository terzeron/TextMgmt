//! 명령행. `structure`는 뷰어가 렌더링하지 못할 구조를, `scan`은 스크립트 실행 코드와 위험 구조를 찾는다.
//! 옵션은 argparse와 같은 규칙으로 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use corpus_common::{DEFAULT_LIBRARY_ROOT, Log, default_report_path};
use scan_common::args::{long_option, number};
use scan_common::files::{collect_files, has_extension};
use scan_common::{DEFAULT_WORKERS, limited, pool, scan_streaming, workers_within_memory};

use crate::security::{Limits, SecurityReport, Status, scan_epub};
use crate::structure::{inspect_epub_structure, print_epub_structure_summary, scan_epub_structure};

const USAGE: &str = "usage: epubscan structure [--root ROOT] [--file FILE]... [--workers N] [--limit N] [--report PATH]\n       epubscan scan [-r] [--json] [--limit N] [--max-member-mb N] [--max-total-mb N] [--max-files N] [--workers N] PATH...";

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
            _ => match long_option(arg, &["root", "file", "workers", "limit", "report", "max-member-mb", "max-total-mb", "max-files"], &mut it)? {
                Some((name, value)) => match name.as_str() {
                    "root" => a.root = value.into(),
                    "file" => a.files.push(value.into()),
                    "report" => a.report = Some(value.into()),
                    "workers" => a.workers = number(&name, &value)?,
                    "limit" => a.limit = number(&name, &value)?,
                    "max-member-mb" => a.max_member_mb = number(&name, &value)?,
                    "max-total-mb" => a.max_total_mb = number(&name, &value)?,
                    _ => a.max_files = number(&name, &value)?,
                },
                None if command.is_none() => command = Some(arg.clone()),
                None => a.paths.push(arg.into()),
            },
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
    has_extension(p, "epub")
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
        let sev = f.severity.label();
        writeln!(out, "[{sev}] {} | {} | {}", f.code, f.location, f.detail)?;
    }
    for n in &r.notes {
        writeln!(out, "note: {n}")?;
    }
    Ok(())
}

/// 한 번에 검사하는 파일 수는 워커 수의 이 배수다. 보고서는 이 창 안에서만 메모리에 머문다.
/// 창이 워커 수와 같으면 가장 느린 파일 하나를 기다리느라 놀고, 너무 크면 보고서가 쌓인다.
const WINDOW_PER_WORKER: usize = 8;

/// 동시에 해석하는 멤버들이 합쳐서 쓸 수 있는 메모리의 어림 상한(MB).
const MEMORY_BUDGET_MB: u64 = 2048;
/// 멤버 1MB를 해석하는 데 드는 메모리(MB). 마크업은 태그 목록과 소문자 사본 때문에 입력의 약 21배가 들고(19MB XHTML이 401MB),
/// 원본 한 벌이 더 있다.
const PARSE_MEMORY_FACTOR: u64 = 22;

/// 워커 수를 메모리로 제한한다. 워커마다 최악의 멤버(`--max-member-mb`)를 동시에 해석할 수 있고, 스레드별 malloc 아레나는
/// 한 번 늘린 메모리를 쥐고 있어서 큰 작업이 돌아가며 지나가도 최대 상주 메모리가 줄지 않는다. 엄밀한 한도가 아니라 보수적인 동시성 상한이다.
pub fn effective_workers(workers: usize, max_member_mb: u64) -> usize {
    workers_within_memory(workers, max_member_mb * PARSE_MEMORY_FACTOR, MEMORY_BUDGET_MB)
}

/// 보고서 하나를 JSON 배열의 요소로 쓴다. 한꺼번에 `to_string_pretty(&Vec)`로 내보낼 때와 같은 모양이다.
fn write_json_element(r: &SecurityReport, first: bool, out: &mut dyn Write) -> io::Result<()> {
    let pretty = serde_json::to_string_pretty(r).map_err(io::Error::other)?;
    let indented = pretty.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n");
    write!(out, "{}{indented}", if first { "[\n" } else { ",\n" })
}

fn scan(args: &Args, out: &mut dyn Write) -> io::Result<i32> {
    let paths = limited(collect_files(&args.paths, args.recursive, is_epub), args.limit);
    if paths.is_empty() {
        eprintln!("No EPUB files found");
        return Ok(2);
    }
    let limits = Limits { max_member: args.max_member_mb * 1024 * 1024, max_total: args.max_total_mb * 1024 * 1024, max_files: args.max_files };
    let workers = effective_workers(args.workers, args.max_member_mb);
    if workers < args.workers {
        eprintln!("epubscan: workers reduced from {} to {workers} to bound memory (--max-member-mb {})", args.workers, args.max_member_mb);
    }
    let pool = pool(workers);
    let mut first = true;
    // 모든 보고서를 모았다가 마지막에 내보내면 지표 수에 비례해 메모리가 늘어 OOM으로 죽는다. 창 단위로 쓰고 버린다.
    let summary = scan_streaming(
        &paths,
        &pool,
        workers * WINDOW_PER_WORKER,
        |p| scan_epub(p, &limits),
        |r| {
            let written = if args.json { write_json_element(r, first, out) } else { print_text(r, out) };
            first = false;
            written.and_then(|()| out.flush())
        },
    )?;
    if args.json {
        writeln!(out, "\n]")?;
    } else {
        eprintln!("scanned {} file(s); workers: {workers}; findings by severity: {:?}", summary.scanned, summary.by_severity);
    }
    Ok(summary.exit_code())
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
  epubscan scan [-r] [--json] [--limit N] [--max-member-mb N] [--max-total-mb N] [--max-files N] [--workers N] PATH...

명령:
  structure  라이브러리 또는 지정한 파일의 EPUB 구조 문제를 찾는다.
  scan       EPUB 파일이나 디렉토리의 스크립트 실행 코드와 위험 구조를 찾는다.

인자:
  PATH   scan 대상 EPUB 파일 또는 디렉토리 (여러 개 가능)

옵션:
  --root DIR          structure 대상 디렉토리 (기본 {DEFAULT_LIBRARY_ROOT})
  --file PATH         structure 대상 파일. 지정하면 파일당 JSON 한 줄을 출력한다.
  --report PATH       structure 리포트 경로 (기본 <repo>/tmp/epub_structure_report_<디렉토리>.json)
  --limit N           앞에서 N건만 처리한다 (0이면 전체). structure와 scan에서 쓴다.
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
