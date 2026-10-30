//! 명령행. 옵션은 argparse와 같은 규칙으로 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use scan_common::args::{long_option, number};
use scan_common::color::{ColorMode, enabled};
use scan_common::files::collect_files;
use scan_common::{Category, DEFAULT_WORKERS, Severity, limited, pool, scan_streaming, workers_within_memory};

use crate::pdf::security::MAX_PARALLEL_INPUT_MB;
use crate::report::{Aggregate, write_json_line, write_report_file, write_summary_line, write_text};
use crate::scan::scan_file;

const USAGE: &str = "usage: bookdoctor [-r] [--json] [--report PATH] [--workers N] [--limit N] [--max-total-mb N] [--only security,defect,render] [--min-severity low|medium|high|critical] [--color auto|always|never] [--pdf-text] [--no-poppler] PATH...";

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
    /// 이보다 낮은 심각도의 지표는 출력하지 않는다. 기본은 `High`다.
    pub min_severity: Severity,
    /// 텍스트 출력의 색. `--json`은 항상 색이 없다.
    pub color: ColorMode,
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
            min_severity: Severity::High,
            color: ColorMode::Auto,
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

fn parse_color(value: &str) -> Result<ColorMode, String> {
    ColorMode::parse(value).ok_or_else(|| format!("argument --color: invalid choice: '{value}' (choose from 'auto', 'always', 'never')"))
}

fn parse_min_severity(value: &str) -> Result<Severity, String> {
    Severity::parse(value).ok_or_else(|| format!("argument --min-severity: invalid choice: '{value}' (choose from 'low', 'medium', 'high', 'critical')"))
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
            _ => match long_option(arg, &["report", "workers", "limit", "max-total-mb", "only", "min-severity", "color"], &mut it)? {
                Some((name, value)) => match name.as_str() {
                    "report" => o.report = Some(value.into()),
                    "workers" => o.workers = number(&name, &value)?,
                    "limit" => o.limit = number(&name, &value)?,
                    "max-total-mb" => o.max_total_mb = number(&name, &value)?,
                    "color" => o.color = parse_color(&value)?,
                    "min-severity" => o.min_severity = parse_min_severity(&value)?,
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
            eprintln!("{USAGE}\nbookdoctor: error: {}", scan_common::text::sanitize_display(&msg, usize::MAX, None));
            return 2;
        }
    };
    match scan(&opts, out) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("bookdoctor: error: {}", scan_common::text::sanitize_display(&e.to_string(), usize::MAX, None));
            1
        }
    }
}

/// 실제로 쓰는 워커 수. lopdf는 입력보다 훨씬 많은 메모리를 쓰므로 동시에 해석하는 수를 메모리로 제한한다:
/// `workers <= MAX_PARALLEL_INPUT_MB / max-total-mb`, 최소 1.
pub fn effective_workers(opts: &Options) -> usize {
    workers_within_memory(opts.workers, opts.max_total_mb, MAX_PARALLEL_INPUT_MB)
}

/// 파일을 검사해 텍스트나 NDJSON으로 내보내고, 요청하면 리포트 파일을 쓴다.
fn scan(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let paths = limited(collect_files(&opts.paths, opts.recursive, wanted), opts.limit);
    let workers = effective_workers(opts);
    let pool = pool(workers);
    let color = enabled(opts.color, io::stdout().is_terminal(), std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()));
    let mut agg = Aggregate::new();
    let summary = scan_streaming(
        &paths,
        &pool,
        workers * WINDOW_PER_WORKER,
        |p| {
            // 점수 같은 판단은 scan_file 안에서 끝낸다. 출력 필터는 그 뒤에만 건다.
            let mut r = scan_file(p, opts);
            r.suppress_below(opts.min_severity);
            r
        },
        |r| {
            agg.add(r);
            if opts.json { write_json_line(r, out)? } else { write_text(r, out, color)? }
            out.flush()
        },
    )?;
    if !opts.json {
        write_summary_line(&agg, &summary, out)?;
    }
    if let Some(path) = &opts.report {
        let root = opts.paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join(", ");
        write_report_file(path, &agg, &summary, &root)?;
    }
    Ok(summary.exit_code())
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(out: &mut dyn Write) -> io::Result<()> {
    write!(
        out,
        "bookdoctor - EPUB·PDF·TXT 통합 진단. 파일을 렌더링하거나 실행하지 않고 보안, 결함, 렌더링 차단 여부를 본다.

사용법:
  bookdoctor [-r] [--json] [--report PATH] [--workers N] [--limit N] [--max-total-mb N]
             [--only security,defect,render] [--min-severity low|medium|high|critical]
             [--color auto|always|never] [--pdf-text] [--no-poppler] PATH...

인자:
  PATH   검사할 파일 또는 디렉토리 (여러 개 가능). 확장자 epub, pdf, txt만 모은다.

옵션:
  -r, --recursive     디렉토리를 하위까지 훑는다.
  --json              결과를 JSON으로 출력한다.
  --report PATH       리포트 파일 경로
  --workers N         병렬 작업 수 (기본 {DEFAULT_WORKERS}). 메모리 때문에 낮아질 수 있다:
                      실제 워커 수는 256 / max-total-mb 이하이고 최소 1이다
  --limit N           앞에서 N건만 처리한다 (0이면 전체)
  --max-total-mb N    파일 하나에서 읽는 최대 크기 MB (기본 256).
                      기본값이면 워커 1개, 128이면 2개까지 허용한다
  --only LIST         지정한 분류(security, defect, render)의 지표만 남긴다.
  --min-severity LVL  이 심각도 이상의 지표만 출력한다 (기본 high). 낮은 지표는 요약 줄에 건수만 남긴다.
                      low로 주면 모두 출력한다.
  --color MODE        auto(기본, 터미널이고 NO_COLOR가 없을 때만), always, never.
                      CRITICAL과 렌더링 구조 에러는 red, HIGH는 purple, 암호화·DRM은 cyan이다. --json에는 색이 없다.
  --pdf-text          PDF 본문 텍스트까지 검사한다.
  --no-poppler        poppler 외부 명령을 쓰지 않는다.
  -h, --help          이 도움말

주의:
  회전 디스크(HDD)에서는 워커 2개가 빠르고 8개는 오히려 느리다. 기본 메모리 상한(256MB)에서는 워커가 1개가 된다.
  PDF는 프로세스 안에서 메모리를 제한하지 못한다. 압축 객체 스트림(ObjStm)이 많은 PDF 하나가 RSS 2GB까지 쓴 것을 측정했고
  종료 코드는 0이다. 바깥에서 한도를 걸어 실행하라: systemd-run --user -p MemoryMax=4G bookdoctor ...

종료 코드:
  0 지표 없음, 1 지표 있음, 2 검사하지 못한 파일이 있고 지표는 없음 또는 잘못된 인자
"
    )
}
