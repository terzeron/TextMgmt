//! 명령행. 옵션은 argparse와 같은 규칙으로 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use scan_common::args::{long_option, number};
use scan_common::files::collect_files;
use scan_common::{Category, DEFAULT_WORKERS, limited, pool, scan_streaming};

use crate::scan::scan_file;

const USAGE: &str = "usage: bookdoctor [-r] [--json] [--report PATH] [--workers N] [--limit N] [--max-total-mb N] [--only security,defect,render] [--pdf-text] [--no-poppler] PATH...";

/// 한 번에 검사하는 파일 수는 워커 수의 이 배수다.
const WINDOW_PER_WORKER: usize = 8;

/// 명령행 옵션.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    pub json: bool,
    pub report: Option<PathBuf>,
    pub workers: usize,
    pub limit: usize,
    pub max_total_mb: u64,
    pub only: Vec<Category>,
    pub pdf_text: bool,
    pub no_poppler: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            paths: Vec::new(),
            recursive: false,
            json: false,
            report: None,
            workers: DEFAULT_WORKERS,
            limit: 0,
            max_total_mb: 256,
            only: Vec::new(),
            pdf_text: false,
            no_poppler: false,
        }
    }
}

fn parse_only(value: &str) -> Result<Vec<Category>, String> {
    let mut only = Vec::new();
    for word in value.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        let c = match word {
            "security" => Category::Security,
            "defect" => Category::Defect,
            "render" => Category::Render,
            _ => return Err(format!("argument --only: invalid choice: '{word}' (choose from 'security', 'defect', 'render')")),
        };
        if !only.contains(&c) {
            only.push(c);
        }
    }
    Ok(only)
}

/// 명령행 인자를 읽는다.
pub fn parse(argv: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-r" | "--recursive" => o.recursive = true,
            "--json" => o.json = true,
            "--pdf-text" => o.pdf_text = true,
            "--no-poppler" => o.no_poppler = true,
            _ => match long_option(arg, &["report", "workers", "limit", "max-total-mb", "only"], &mut it)? {
                Some((name, value)) => match name.as_str() {
                    "report" => o.report = Some(value.into()),
                    "workers" => o.workers = number(&name, &value)?,
                    "limit" => o.limit = number(&name, &value)?,
                    "max-total-mb" => o.max_total_mb = number(&name, &value)?,
                    _ => o.only = parse_only(&value)?,
                },
                None => o.paths.push(arg.into()),
            },
        }
    }
    if o.paths.is_empty() {
        return Err("the following arguments are required: PATH...".into());
    }
    if o.max_total_mb == 0 {
        return Err("Limits must be positive".into());
    }
    Ok(o)
}

fn wanted(p: &Path) -> bool {
    p.is_file() && p.extension().is_some_and(|e| ["epub", "pdf", "txt"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// 진입점. 종료 코드를 돌려준다.
pub fn run(argv: &[String], out: &mut dyn Write) -> i32 {
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        let _ = print_usage(out);
        return 0;
    }
    if argv.is_empty() {
        let _ = print_usage(out);
        return 2;
    }
    let opts = match parse(argv) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{USAGE}\nbookdoctor: error: {msg}");
            return 2;
        }
    };
    match scan(&opts, out) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("bookdoctor: error: {e}");
            1
        }
    }
}

/// 임시 출력. 지표나 미완료가 있는 파일마다 한 줄을 쓴다. 보고서 출력(report.rs)이 대신한다.
fn scan(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let paths = limited(collect_files(&opts.paths, opts.recursive, wanted), opts.limit);
    let pool = pool(opts.workers);
    let summary = scan_streaming(
        &paths,
        &pool,
        opts.workers * WINDOW_PER_WORKER,
        |p| scan_file(p, opts),
        |r| {
            if r.incomplete {
                writeln!(out, "{} INCOMPLETE {}", r.path, r.reasons.join("; "))?;
            }
            for f in &r.findings {
                writeln!(out, "{} [{}] {} {} {}", r.path, f.severity.label(), f.code, f.location, f.detail)?;
            }
            out.flush()
        },
    )?;
    Ok(summary.exit_code())
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(out: &mut dyn Write) -> io::Result<()> {
    write!(
        out,
        "bookdoctor - EPUB·PDF·TXT 통합 진단. 파일을 렌더링하거나 실행하지 않고 보안, 결함, 렌더링 차단 여부를 본다.

사용법:
  bookdoctor [-r] [--json] [--report PATH] [--workers N] [--limit N] [--max-total-mb N]
             [--only security,defect,render] [--pdf-text] [--no-poppler] PATH...

인자:
  PATH   검사할 파일 또는 디렉토리 (여러 개 가능). 확장자 epub, pdf, txt만 모은다.

옵션:
  -r, --recursive     디렉토리를 하위까지 훑는다.
  --json              결과를 JSON으로 출력한다.
  --report PATH       리포트 파일 경로
  --workers N         병렬 작업 수 (기본 {DEFAULT_WORKERS})
  --limit N           앞에서 N건만 처리한다 (0이면 전체)
  --max-total-mb N    파일 하나에서 읽는 최대 크기 MB (기본 256)
  --only LIST         지정한 분류(security, defect, render)의 지표만 남긴다.
  --pdf-text          PDF 본문 텍스트까지 검사한다.
  --no-poppler        poppler 외부 명령을 쓰지 않는다.
  -h, --help          이 도움말

종료 코드:
  0 지표 없음, 1 지표 있음, 2 검사하지 못한 파일이 있고 지표는 없음 또는 잘못된 인자
"
    )
}
