//! 명령행. `bookdoctor <command> [옵션] PATH...` 형태로 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.
//!
//! 옵션은 `scan`의 `--json`과 `--min-severity`뿐이다. 나머지 값(워커 수, 읽기 상한, 휴지통 위치 등)은 코드의 고정값이다.
//! 테스트는 `parse`가 만든 `Options`의 필드를 바꿔 `execute`에 넘긴다.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use corpus_common::DEFAULT_LIBRARY_ROOT;
use scan_common::args::long_option;
use scan_common::files::collect_files;
use scan_common::{pool, scan_streaming, workers_within_memory, Severity, DEFAULT_WORKERS};

use crate::exe::is_exec_extension;
use crate::pdf::security::MAX_PARALLEL_INPUT_MB;
use crate::prefetch::Prefetcher;
use crate::report::{write_json_line, write_summary_line, write_text, Aggregate};
use crate::rules::Compiled;
use crate::scan::scan_file;

const USAGE: &str = "usage: bookdoctor scan [--json] [--min-severity low|medium|high|critical] PATH... | sanitize PATH... | repair PATH... | quarantine PATH... | unquarantine";

/// 원본을 보관하는 디렉터리 이름(`sanitize`·`repair`). 코퍼스 루트 바로 아래에 있다.
pub const TRASH_DIR_NAME: &str = "0_trash";
/// 인코딩이 손상된 문서를 옮기는 디렉터리 이름(`quarantine`). 코퍼스 루트 바로 아래에 있다.
pub const BROKEN_DIR_NAME: &str = "0_broken";
/// 격리 매니페스트 파일 이름. 격리 디렉터리 안에 둔다.
pub const MANIFEST_NAME: &str = "manifest.json";

/// 파일 하나에서 읽는 최대 크기(MB).
const MAX_TOTAL_MB: u64 = 256;

/// 한 번에 검사하는 파일 수는 워커 수의 이 배수다.
const WINDOW_PER_WORKER: usize = 8;

/// 선읽기 한도: 소비자보다 이만큼(파일 수, 바이트)까지만 앞서가고, 이보다 큰 파일은 미리 읽지 않는다.
/// 소비자는 한 창(워커 수 × `WINDOW_PER_WORKER`개)이 끝나야 처리한 수를 알리므로, 한 창을 넘어서 읽을 수 있게 창 크기를 더해 준다.
const PREFETCH_FILES: usize = 16;
const PREFETCH_BYTES: u64 = 512 * 1024 * 1024;
const PREFETCH_MAX_FILE: u64 = 128 * 1024 * 1024;

/// 하위 명령.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Command {
    /// 진단만 한다. 파일을 바꾸지 않는다.
    #[default]
    Scan,
    /// 보안 지표(HIGH 이상)가 있는 EPUB·PDF를 정리한다.
    Sanitize,
    /// 결함을 고친다: EPUB 본문 XHTML, UTF-16LE로 잘못 저장된 바이너리.
    Repair,
    /// 인코딩이 손상된 TXT·EPUB을 격리 디렉터리로 옮긴다.
    Quarantine,
    /// 격리를 되돌린다.
    Unquarantine,
}

impl Command {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "scan" => Command::Scan,
            "sanitize" => Command::Sanitize,
            "repair" => Command::Repair,
            "quarantine" => Command::Quarantine,
            "unquarantine" => Command::Unquarantine,
            _ => return None,
        })
    }
}

/// 실행 설정. 명령행에서 읽는 것은 `command`, `paths`, `json`, `min_severity`뿐이고 나머지는 고정값이다.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub command: Command,
    pub paths: Vec<PathBuf>,
    pub json: bool,
    /// 이보다 낮은 심각도의 지표는 출력하지 않는다. 기본은 `High`다.
    pub min_severity: Severity,
    /// 워커 수. 메모리 상한(`effective_workers`)으로 다시 줄어든다.
    pub workers: usize,
    /// 파일 하나에서 읽는 최대 크기(MB).
    pub max_total_mb: u64,
    /// ZIP 계열 멤버를 끝까지 풀어 CRC까지 확인한다. 기본은 멤버마다 앞부분만 풀어 본다.
    pub zip_deep: bool,
    /// 다음 파일을 미리 읽어 디스크 대기와 검사를 겹친다.
    pub prefetch: bool,
    /// MuPDF worker를 부르지 않는다(PDF 본문 표본 추출을 건너뛴다). 테스트용이다.
    pub no_pdf_worker: bool,
    /// ZIP 계열 안의 컨테이너 멤버를 재귀 검사한다.
    pub recurse: bool,
    /// 코퍼스 루트. 휴지통·격리 디렉터리와 매니페스트가 이 아래에 있다.
    pub library_root: PathBuf,
    /// 읽어서 컴파일한 규칙. `execute`가 내장 규칙으로 채운다. 없으면 규칙을 적용하지 않는다(비용 0).
    pub ruleset: Option<Arc<Compiled>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            command: Command::Scan,
            paths: Vec::new(),
            json: false,
            min_severity: Severity::High,
            workers: DEFAULT_WORKERS,
            max_total_mb: MAX_TOTAL_MB,
            zip_deep: false,
            prefetch: true,
            no_pdf_worker: false,
            recurse: true,
            library_root: PathBuf::from(DEFAULT_LIBRARY_ROOT),
            ruleset: None,
        }
    }
}

impl Options {
    /// `sanitize`·`repair`가 원본을 두는 디렉터리.
    pub fn trash_dir(&self) -> PathBuf {
        self.library_root.join(TRASH_DIR_NAME)
    }

    /// `quarantine`이 파일을 옮기는 디렉터리.
    pub fn broken_dir(&self) -> PathBuf {
        self.library_root.join(BROKEN_DIR_NAME)
    }

    /// 격리 매니페스트 경로.
    pub fn manifest_path(&self) -> PathBuf {
        self.broken_dir().join(MANIFEST_NAME)
    }
}

fn parse_min_severity(value: &str) -> Result<Severity, String> {
    Severity::parse(value).ok_or_else(|| format!("argument --min-severity: invalid choice: '{value}' (choose from 'low', 'medium', 'high', 'critical')"))
}

/// 명령행 인자를 읽는다. 첫 인자는 명령이다.
pub fn parse(argv: &[String]) -> Result<Options, String> {
    let Some((first, rest)) = argv.split_first() else {
        return Err("the following arguments are required: command".into());
    };
    if first.starts_with('-') {
        return Err("the following arguments are required: command".into());
    }
    let command = Command::parse(first).ok_or_else(|| format!("argument command: invalid choice: '{first}' (choose from 'scan', 'sanitize', 'repair', 'quarantine', 'unquarantine')"))?;
    let mut o = Options { command, ..Options::default() };
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        if command == Command::Scan {
            if arg == "--json" {
                o.json = true;
                continue;
            }
            if let Some((_, value)) = long_option(arg, &["min-severity"], &mut it)? {
                o.min_severity = parse_min_severity(&value)?;
                continue;
            }
        }
        if arg.starts_with('-') || command == Command::Unquarantine {
            return Err(format!("unrecognized arguments: {arg}"));
        }
        o.paths.push(arg.into());
    }
    if o.paths.is_empty() && command != Command::Unquarantine {
        return Err("the following arguments are required: PATH...".into());
    }
    Ok(o)
}

fn wanted(p: &Path) -> bool {
    p.is_file()
        && p.extension().is_some_and(|e| {
            ["epub", "pdf", "txt", "zip", "docx", "docm", "dotx", "dotm", "hwpx", "doc", "dot", "rtf", "hwp", "html", "htm", "xhtml"].iter().any(|x| e.eq_ignore_ascii_case(x))
                || is_exec_extension(&e.to_string_lossy())
        })
}

fn report_error(e: &dyn std::fmt::Display) {
    eprintln!("bookdoctor: error: {}", scan_common::text::sanitize_display(&e.to_string(), usize::MAX, None));
}

/// 진입점. 종료 코드를 돌려준다. 색은 실제 stdout이 터미널인지와 `NO_COLOR`로 정한다.
pub fn run(argv: &[String], out: &mut dyn Write) -> i32 {
    run_with_env(argv, out, io::stdout().is_terminal(), std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()))
}

/// `run`과 같되 터미널 여부(`is_tty`)와 `NO_COLOR` 설정 여부를 호출자가 정한다.
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
    match parse(argv) {
        Ok(opts) => execute(&opts, out, is_tty, no_color),
        Err(msg) => {
            eprintln!("{USAGE}\nbookdoctor: error: {}", scan_common::text::sanitize_display(&msg, usize::MAX, None));
            2
        }
    }
}

/// 읽은 설정으로 명령을 실행한다. 내장 규칙을 읽어 `ruleset`에 넣는다.
pub fn execute(opts: &Options, out: &mut dyn Write, is_tty: bool, no_color: bool) -> i32 {
    let mut opts = opts.clone();
    match crate::rules::load(&[], true) {
        Ok(set) => opts.ruleset = set.map(Arc::new),
        Err(msg) => {
            report_error(&msg);
            return 2;
        }
    }
    let result = match opts.command {
        Command::Scan => scan(&opts, out, is_tty && !no_color),
        Command::Sanitize => crate::sanitize::run(&opts, out),
        Command::Repair => crate::repair::run(&opts, out),
        Command::Quarantine => crate::quarantine::run(&opts, out),
        Command::Unquarantine => crate::quarantine::undo(&opts, out),
    };
    result.unwrap_or_else(|e| {
        report_error(&e);
        1
    })
}

/// 실제로 쓰는 워커 수. lopdf는 입력보다 훨씬 많은 메모리를 쓰므로 동시에 해석하는 수를 메모리로 제한한다:
/// `workers <= MAX_PARALLEL_INPUT_MB / max_total_mb`, 최소 1.
pub fn effective_workers(opts: &Options) -> usize {
    workers_within_memory(opts.workers, opts.max_total_mb, MAX_PARALLEL_INPUT_MB)
}

/// 명령이 다룰 파일을 모은다. 디렉터리는 항상 하위까지 훑는다.
/// 파일을 바꾸는 명령은 코퍼스 루트 아래의 휴지통·격리 디렉터리를 건너뛴다. 보관한 원본을 다시 고치거나 옮기지 않는다.
pub(crate) fn collect(opts: &Options, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut paths = collect_files(&opts.paths, true, keep);
    if opts.command != Command::Scan {
        paths.retain(|p| !in_reserved_dir(p, &opts.library_root));
    }
    paths
}

/// `path`가 `library_root` 바로 아래의 휴지통이나 격리 디렉터리 안에 있는가.
pub(crate) fn in_reserved_dir(path: &Path, library_root: &Path) -> bool {
    path.strip_prefix(library_root).ok().and_then(|rel| rel.components().next()).is_some_and(|first| first.as_os_str() == TRASH_DIR_NAME || first.as_os_str() == BROKEN_DIR_NAME)
}

/// 파일을 검사해 텍스트나 NDJSON으로 내보낸다.
fn scan(opts: &Options, out: &mut dyn Write, color: bool) -> io::Result<i32> {
    let paths = collect(opts, wanted);
    let workers = effective_workers(opts);
    let pool = pool(workers);
    // 처리한 파일 수를 선읽기 스레드에 알려 소비자보다 너무 앞서가지 않게 한다. 파일이 하나뿐이면 겹칠 것이 없다.
    let prefetcher = (opts.prefetch && paths.len() > 1).then(|| Prefetcher::start(paths.clone(), PREFETCH_FILES + workers * WINDOW_PER_WORKER, PREFETCH_BYTES, PREFETCH_MAX_FILE));
    let mut emitted = 0usize;
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
            emitted += 1;
            if let Some(p) = &prefetcher {
                p.advance(emitted);
            }
            agg.add(r);
            if opts.json {
                write_json_line(r, out)?
            } else {
                write_text(r, out, color && !opts.json)?
            }
            out.flush()
        },
    )?;
    if opts.json {
        // NDJSON stdout에는 파일 레코드만 둔다. 요약 줄은 stderr에 남겨 "깨끗함"과 "전부 억제됨"을 가를 수 있게 한다.
        write_summary_line(&agg, &summary, &mut io::stderr().lock())?;
    } else {
        write_summary_line(&agg, &summary, out)?;
    }
    Ok(summary.exit_code())
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 명령·옵션을 보여 준다.
pub fn print_usage(out: &mut dyn Write) -> io::Result<()> {
    write!(
        out,
        "bookdoctor - EPUB·PDF·TXT 등 문서 파일의 보안, 결함, 렌더링 차단 여부를 진단하고 고친다.

사용법:
  bookdoctor scan [--json] [--min-severity LVL] PATH...
  bookdoctor sanitize PATH...
  bookdoctor repair PATH...
  bookdoctor quarantine PATH...
  bookdoctor unquarantine

명령:
  scan          진단만 한다. 파일을 바꾸지 않는다. 디렉터리는 하위까지 훑는다.
                확장자 epub, pdf, txt, zip, docx(docm·dotx·dotm), hwpx, hwp, doc(dot), rtf, html(htm·xhtml)와
                실행 파일 확장자(exe, dll, so, dylib 등)를 모은다. 내장 규칙(강한 IOC)을 적용하고,
                PDF는 앞 {TEXT_PROBE}쪽과 끝 {TEXT_PROBE}쪽의 본문을 뽑아 글자층 없음·글자 깨짐을 본다.
  sanitize      보안 지표(HIGH 이상)가 있는 EPUB·PDF를 정리한다. EPUB은 허용 목록으로 재구성해 스크립트, 실행 파일,
                원격 로드, 외부 개체를 지우고, PDF는 Launch·JavaScript·RichMediaExecute 동작과 /JS 키를 지운다.
                본문·구조 검증과 재검사를 통과해야 적용한다. 암호화·128MB 초과 PDF는 건너뛴다.
  repair        EPUB 본문 XHTML(XHTML_PARSE_FAILED·HEAD_MISSING)을 고치고, UTF-16LE로 잘못 저장된 바이너리
                (HWP·PDF·PNG가 .txt로 남은 것)를 원래 바이트와 확장자로 되돌린다. 많이 깨진 EPUB은 DELETE_CANDIDATE로만 알린다.
  quarantine    인코딩이 손상된(MOJIBAKE) TXT·EPUB을 {library}/{broken}로 옮긴다. 원래 카테고리를 하위 디렉터리로 유지하고
                옮긴 내역을 {library}/{broken}/{manifest}에 누적한다.
  unquarantine  매니페스트대로 격리한 파일을 원위치시킨다.

  sanitize와 repair는 원본을 {library}/{trash}에 두고 같은 inode에 덮어쓰며 mtime을 유지한다.

scan 옵션:
  --json              결과를 파일마다 JSON 한 줄로 출력한다. 요약 줄은 stderr로 간다.
  --min-severity LVL  이 심각도 이상의 지표만 출력한다 (low|medium|high|critical, 기본 high).
                      낮은 지표는 요약 줄에 건수만 남긴다.
  -h, --help          이 도움말

주의:
  색은 터미널에서만 쓰고 NO_COLOR가 있으면 끈다. 워커는 메모리 상한 안에서 최대 {DEFAULT_WORKERS}개다.
  PDF는 프로세스 안에서 메모리를 제한하지 못한다. 압축 객체 스트림(ObjStm)이 많은 PDF 하나가 RSS 2GB까지 쓴 것을 측정했고
  종료 코드는 0이다. 바깥에서 한도를 걸어 실행하라: systemd-run --user -p MemoryMax=4G bookdoctor scan ...

종료 코드 (scan은 출력된 지표 기준. --min-severity보다 낮은 지표는 세지 않는다):
  0 지표 없음, 1 지표 있음 또는 쓰기 실패, 2 검사하지 못한 파일이 있고 지표는 없음 또는 잘못된 인자
",
        TEXT_PROBE = crate::pdf::structure::TEXT_PROBE_PAGES,
        library = DEFAULT_LIBRARY_ROOT,
        trash = TRASH_DIR_NAME,
        broken = BROKEN_DIR_NAME,
        manifest = MANIFEST_NAME,
    )
}
