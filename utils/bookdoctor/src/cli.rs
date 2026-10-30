//! 명령행. 옵션은 argparse와 같은 규칙으로 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.

use std::sync::Arc;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use scan_common::args::{long_option, number};
use scan_common::color::{ColorMode, enabled};
use scan_common::files::collect_files;
use scan_common::{Category, DEFAULT_WORKERS, Severity, limited, pool, scan_streaming, workers_within_memory};

use crate::exe::is_exec_extension;
use crate::pdf::security::MAX_PARALLEL_INPUT_MB;
use crate::prefetch::Prefetcher;
use crate::report::{Aggregate, write_json_line, write_report_file, write_summary_line, write_text};
use crate::isolate::scan_isolated;
use crate::rules::Compiled;
use crate::scan::scan_file;

const USAGE: &str = "usage: bookdoctor [-r] [--json] [--report PATH] [--workers N] [--limit N] [--max-total-mb N] [--only security,defect,render] [--min-severity low|medium|high|critical] [--color auto|always|never] [--zip-deep] [--no-prefetch] [--no-recurse] [--isolate] [--isolate-mem-mb N] [--isolate-timeout-s N] [--rules PATH]... [--builtin-rules] [--pdf-text] [--no-poppler] PATH...";

/// 한 번에 검사하는 파일 수는 워커 수의 이 배수다.
const WINDOW_PER_WORKER: usize = 8;

/// 선읽기 한도: 소비자보다 이만큼(파일 수, 바이트)까지만 앞서가고, 이보다 큰 파일은 미리 읽지 않는다.
/// 소비자는 한 창(워커 수 × `WINDOW_PER_WORKER`개)이 끝나야 처리한 수를 알리므로, 한 창을 넘어서 읽을 수 있게 창 크기를 더해 준다.
const PREFETCH_FILES: usize = 16;
const PREFETCH_BYTES: u64 = 512 * 1024 * 1024;
const PREFETCH_MAX_FILE: u64 = 128 * 1024 * 1024;

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
    /// ZIP 계열 멤버를 끝까지 풀어 CRC까지 확인한다. 기본은 멤버마다 앞부분만 풀어 본다.
    pub zip_deep: bool,
    /// 다음 파일을 미리 읽어 디스크 대기와 검사를 겹친다. `--no-prefetch`로 끈다.
    pub prefetch: bool,
    pub no_poppler: bool,
    /// ZIP 계열 안의 컨테이너 멤버를 재귀 검사한다. `--no-recurse`로 끈다.
    pub recurse: bool,
    /// 파일마다 하위 프로세스에서 메모리·시간 상한을 걸고 검사한다. `--isolate`.
    pub isolate: bool,
    /// `--isolate`의 자식 프로세스 주소 공간 상한(MiB).
    pub isolate_mem_mb: u64,
    /// `--isolate`의 자식 프로세스 시간 상한(초).
    pub isolate_timeout_s: u64,
    /// 내부용: 한 파일을 검사해 필터 없는 JSON 한 줄로 낸다. `--isolate`의 자식이 쓴다.
    pub child: bool,
    /// `--rules`로 준 규칙 파일·디렉터리.
    pub rule_files: Vec<PathBuf>,
    /// `--builtin-rules`: 내장 소규모 규칙을 켠다.
    pub builtin_rules: bool,
    /// 읽어서 컴파일한 규칙. `run`이 `rule_files`와 `builtin_rules`로 채운다. 없으면 규칙을 적용하지 않는다(비용 0).
    pub ruleset: Option<Arc<Compiled>>,
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
            zip_deep: false,
            prefetch: true,
            no_poppler: false,
            recurse: true,
            isolate: false,
            isolate_mem_mb: 4096,
            isolate_timeout_s: 120,
            child: false,
            rule_files: Vec::new(),
            builtin_rules: false,
            ruleset: None,
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
            "--zip-deep" => o.zip_deep = true,
            "--no-prefetch" => o.prefetch = false,
            "--no-poppler" => o.no_poppler = true,
            "--no-recurse" => o.recurse = false,
            "--isolate" => o.isolate = true,
            "--builtin-rules" => o.builtin_rules = true,
            "--child" => o.child = true,
            _ => match long_option(arg, &["report", "workers", "limit", "max-total-mb", "only", "min-severity", "color", "isolate-mem-mb", "isolate-timeout-s", "rules"], &mut it)? {
                Some((name, value)) => match name.as_str() {
                    "report" => o.report = Some(value.into()),
                    "workers" => o.workers = number(&name, &value)?,
                    "limit" => o.limit = number(&name, &value)?,
                    "max-total-mb" => o.max_total_mb = number(&name, &value)?,
                    "rules" => o.rule_files.push(value.into()),
                    "isolate-mem-mb" => o.isolate_mem_mb = number(&name, &value)?,
                    "isolate-timeout-s" => o.isolate_timeout_s = number(&name, &value)?,
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
    if o.max_total_mb == 0 || o.isolate_mem_mb == 0 || o.isolate_timeout_s == 0 {
        return Err("Limits must be positive".into());
    }
    Ok(o)
}

fn wanted(p: &Path) -> bool {
    p.is_file() && p.extension().is_some_and(|e| ["epub", "pdf", "txt", "zip", "docx", "docm", "dotx", "dotm", "hwpx", "doc", "dot", "rtf", "hwp", "html", "htm", "xhtml"].iter().any(|x| e.eq_ignore_ascii_case(x)) || is_exec_extension(&e.to_string_lossy()))
}

/// 진입점. 종료 코드를 돌려준다. 색은 실제 stdout이 터미널인지와 `NO_COLOR`로 정한다.
pub fn run(argv: &[String], out: &mut dyn Write) -> i32 {
    run_with_env(argv, out, io::stdout().is_terminal(), std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()))
}

/// `run`과 같되 터미널 여부(`is_tty`)와 `NO_COLOR` 설정 여부를 호출자가 정한다. `--color auto`가 이 값을 따른다.
/// `out`이 stdout이 아닐 수 있으므로 테스트가 실행 환경과 무관하게 같은 결과를 얻는다.
pub fn run_with_env(argv: &[String], out: &mut dyn Write, is_tty: bool, no_color: bool) -> i32 {
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
    let mut opts = opts;
    match crate::rules::load(&opts.rule_files, opts.builtin_rules) {
        Ok(set) => opts.ruleset = set.map(Arc::new),
        Err(msg) => {
            eprintln!("bookdoctor: error: {}", scan_common::text::sanitize_display(&msg, usize::MAX, None));
            return 2;
        }
    }
    if opts.child {
        return run_child(&opts, out);
    }
    match scan(&opts, out, enabled(opts.color, is_tty, no_color)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("bookdoctor: error: {}", scan_common::text::sanitize_display(&e.to_string(), usize::MAX, None));
            1
        }
    }
}

/// `--isolate`의 자식: 첫 경로 하나를 검사해 필터 없는 JSON 보고서를 한 줄로 낸다. 깨끗한 파일도 한 줄을 낸다(부모가 실패와 구분하려고).
fn run_child(opts: &Options, out: &mut dyn Write) -> i32 {
    let Some(path) = opts.paths.first() else { return 2 };
    let report = scan_file(path, opts);
    let written = serde_json::to_writer(&mut *out, &report).map_err(io::Error::other).and_then(|()| out.write_all(b"\n")).and_then(|()| out.flush());
    if written.is_ok() { 0 } else { 1 }
}

/// 실제로 쓰는 워커 수. lopdf는 입력보다 훨씬 많은 메모리를 쓰므로 동시에 해석하는 수를 메모리로 제한한다:
/// `workers <= MAX_PARALLEL_INPUT_MB / max-total-mb`, 최소 1.
pub fn effective_workers(opts: &Options) -> usize {
    workers_within_memory(opts.workers, opts.max_total_mb, MAX_PARALLEL_INPUT_MB)
}

/// 파일을 검사해 텍스트나 NDJSON으로 내보내고, 요청하면 리포트 파일을 쓴다.
fn scan(opts: &Options, out: &mut dyn Write, color: bool) -> io::Result<i32> {
    let paths = limited(collect_files(&opts.paths, opts.recursive, wanted), opts.limit);
    let workers = effective_workers(opts);
    let pool = pool(workers);
    // 자식은 자기 자신이다. 실행 파일 경로를 알 수 없으면 격리 없이 검사한다.
    let exe = std::env::current_exe().ok();
    // 처리한 파일 수를 선읽기 스레드에 알려 소비자보다 너무 앞서가지 않게 한다. 파일이 하나뿐이면 겹칠 것이 없다.
    let prefetcher = (opts.prefetch && !opts.isolate && paths.len() > 1).then(|| Prefetcher::start(paths.clone(), PREFETCH_FILES + workers * WINDOW_PER_WORKER, PREFETCH_BYTES, PREFETCH_MAX_FILE));
    let mut emitted = 0usize;
    let mut agg = Aggregate::new();
    let summary = scan_streaming(
        &paths,
        &pool,
        workers * WINDOW_PER_WORKER,
        |p| {
            // 점수 같은 판단은 scan_file 안에서 끝낸다. 출력 필터는 그 뒤에만 건다.
            let mut r = match (&exe, opts.isolate) {
                (Some(exe), true) => scan_isolated(exe, p, opts),
                _ => scan_file(p, opts),
            };
            r.suppress_below(opts.min_severity);
            r
        },
        |r| {
            emitted += 1;
            if let Some(p) = &prefetcher {
                p.advance(emitted);
            }
            agg.add(r);
            if opts.json { write_json_line(r, out)? } else { write_text(r, out, color)? }
            out.flush()
        },
    )?;
    if opts.json {
        // NDJSON stdout에는 파일 레코드만 둔다. 요약 줄은 stderr에 남겨 "깨끗함"과 "전부 억제됨"을 가를 수 있게 한다.
        write_summary_line(&agg, &summary, &mut io::stderr().lock())?;
    } else {
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
        "bookdoctor - EPUB·PDF·TXT 통합 진단과 실행 파일 확인. 파일을 렌더링하거나 실행하지 않고 보안, 결함, 렌더링 차단 여부를 본다.

사용법:
  bookdoctor [-r] [--json] [--report PATH] [--workers N] [--limit N] [--max-total-mb N]
             [--only security,defect,render] [--min-severity low|medium|high|critical]
             [--color auto|always|never] [--zip-deep] [--no-prefetch] [--no-recurse] [--isolate] [--isolate-mem-mb N] [--isolate-timeout-s N] [--rules PATH]... [--builtin-rules] [--pdf-text] [--no-poppler] PATH...

인자:
  PATH   검사할 파일 또는 디렉토리 (여러 개 가능). 확장자 epub, pdf, txt, zip, docx(docm·dotx·dotm), hwpx, hwp, doc(dot), rtf, html(htm·xhtml)와 실행 파일 확장자(exe, dll, so, dylib 등)를 모은다.
         실행 파일은 확장자와 매직바이트(ELF·Mach-O·PE)가 모두 맞을 때 EXECUTABLE_FILE로 알린다.

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
  --zip-deep          ZIP 멤버를 끝까지 풀어 CRC까지 확인한다 (기본은 멤버마다 앞 4096바이트만 풀어 본다).
  --rules PATH        규칙 파일이나 디렉터리(*.rules)를 읽어 파일의 앞 16MiB에 적용한다. 여러 번 쓸 수 있다. 일치는 RULE_MATCH 지표다. 문법은 docs/bookdoctor-rules.md.
  --builtin-rules     내장 소규모 규칙(강한 IOC만)을 켠다. 기본은 규칙을 적용하지 않는다.
  --isolate           파일마다 하위 프로세스에서 검사한다. 자식이 죽거나 멈추거나 메모리를 넘으면 그 파일만 미완료로 보고하고 계속한다. 선읽기는 끈다.
  --isolate-mem-mb N  --isolate의 자식 주소 공간 상한(MiB, 기본 4096). 0은 허용하지 않는다.
  --isolate-timeout-s N  --isolate의 자식 시간 상한(초, 기본 120). 넘으면 프로세스 그룹 전체를 죽인다.
  --no-recurse        ZIP·DOCX·HWPX 안의 중첩 ZIP·PDF·RTF·Word·HWP를 재귀 검사하지 않는다. 기본은 깊이 3, 파일 64개, 바이트 합계는 --max-total-mb까지 검사한다.
  --no-prefetch       다음 파일을 미리 읽지 않는다. 기본은 검사하는 동안 다음 파일 최대 16개(512MiB)를 미리 읽는다.
  --pdf-text          PDF 본문 텍스트까지 검사한다.
  --no-poppler        poppler 외부 명령을 쓰지 않는다.
  -h, --help          이 도움말

주의:
  회전 디스크(HDD)에서는 워커 2개가 빠르고 8개는 오히려 느리다. 기본 메모리 상한(256MB)에서는 워커가 1개가 된다.
  PDF는 프로세스 안에서 메모리를 제한하지 못한다. 압축 객체 스트림(ObjStm)이 많은 PDF 하나가 RSS 2GB까지 쓴 것을 측정했고
  종료 코드는 0이다. 바깥에서 한도를 걸어 실행하라: systemd-run --user -p MemoryMax=4G bookdoctor ...

종료 코드 (출력된 지표 기준. --min-severity보다 낮은 지표는 세지 않는다):
  0 지표 없음, 1 지표 있음, 2 검사하지 못한 파일이 있고 지표는 없음 또는 잘못된 인자
"
    )
}
