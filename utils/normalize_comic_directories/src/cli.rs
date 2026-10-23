//! 명령행: 사용법, 인자 읽기, 실행.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use bookstore::{Fetcher, HttpFetcher, Store};

use crate::cache::{BookstoreSearchCache, DEFAULT_NEGATIVE_CACHE_SECONDS, StoreRow};
use crate::plan::{Searcher, apply_renames, build_proposals, write_report};

const DEFAULT_ROOT: &str = "/mnt/data/만화";
const REPORT_NAME: &str = "comic-directory-normalization.tsv";
const CACHE_NAME: &str = "comic-bookstore-title-cache.jsonl";

/// 사용자별 기본 디렉토리: `$XDG_CACHE_HOME/tm_dev`, 없으면 `$HOME/.cache/tm_dev`. 공유 `/tmp`는 쓰지 않는다.
fn default_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| PathBuf::from(h).join(".cache")));
    base.unwrap_or_else(|| PathBuf::from(".")).join("tm_dev")
}

/// 캐시가 내 파일이고 그룹·타인이 쓸 수 없는지 확인한다. 캐시 내용이 이름 변경을 이끌기 때문이다.
pub fn check_cache_trust(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::metadata(path) else { return Ok(()) };
    let me = std::fs::metadata("/proc/self").map(|m| m.uid()).map_err(|e| format!("현재 사용자를 확인하지 못했다: {e}"))?;
    if meta.uid() != me {
        return Err(format!("캐시 소유자가 내가 아니다: {}", path.display()));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(format!("캐시를 그룹·타인이 쓸 수 있다(chmod go-w 필요): {}", path.display()));
    }
    Ok(())
}

/// 캐시에 있는 결과만 쓰는 검색. 캐시에 없으면 조회하지 않는다.
pub struct CacheOnly;

impl Searcher for CacheOnly {
    fn is_live(&self) -> bool {
        false
    }

    fn search(&self, _store: &str, _title: &str, _author: &str) -> Result<Vec<StoreRow>, String> {
        Err("서점 조회는 이 빌드에서 지원하지 않는다".into())
    }
}

/// 서점에 실제로 물어보는 검색. 서점 이름은 `yes24`, `aladin`, `kyobo`다.
pub struct LiveSearch<F: Fetcher> {
    pub fetcher: F,
}

impl<F: Fetcher> Searcher for LiveSearch<F> {
    fn is_live(&self) -> bool {
        true
    }

    fn search(&self, store: &str, title: &str, author: &str) -> Result<Vec<StoreRow>, String> {
        let store = Store::from_name(store).ok_or_else(|| format!("알 수 없는 서점: {store}"))?;
        let outcome = bookstore::search(store, &self.fetcher, "", title, author);
        Ok(outcome.rows.into_iter().map(|r| StoreRow { title: r.title, author: r.author, category: r.category, url: r.detail_url, isbn: r.isbn }).collect())
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub root: PathBuf,
    pub report: PathBuf,
    pub delay: f64,
    pub max_searches: usize,
    pub cache: PathBuf,
    pub negative_cache_seconds: u64,
    pub apply: bool,
    /// 서점에 묻지 않고 캐시만 쓴다.
    pub offline: bool,
    /// 시작 전에 캐시 파일을 비운다.
    pub refresh_cache: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            root: DEFAULT_ROOT.into(),
            report: default_dir().join(REPORT_NAME),
            delay: 1.2,
            max_searches: 0,
            cache: default_dir().join(CACHE_NAME),
            negative_cache_seconds: DEFAULT_NEGATIVE_CACHE_SECONDS,
            apply: false,
            offline: false,
            refresh_cache: false,
        }
    }
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let report_default = default_dir().join(REPORT_NAME).display().to_string();
    let cache_default = default_dir().join(CACHE_NAME).display().to_string();
    let _ = write!(
        out,
        "{program} - 만화 디렉토리 이름을 `[저자] 제목 (1-10권 완)` 꼴로 정리한다.

3_미정리, 4_대기, 9_temp 밑의 디렉토리마다 안의 파일 이름에서 제목·저자·권수 범위를 뽑고,
서점(Yes24, 알라딘, 교보문고) 검색 결과로 제목과 저자를 확인해 새 이름을 정한다. 기본은 리포트(TSV)만 쓰는 점검이다.
서점 결과는 캐시에 남기고, 같은 질의는 캐시에서 읽는다. 서점에는 HTTPS로 접속한다.
--offline이면 서점에 묻지 않고 캐시만 쓴다. 캐시에 없는 질의는 SKIP_NOT_CACHED로 건너뛴다.

사용법:
  {program} run [옵션]

명령:
  run   계획을 세워 리포트를 쓴다. --apply를 주면 디렉토리 이름도 바꾼다.

옵션:
  --root DIR                    만화 루트 (기본 {DEFAULT_ROOT})
  --report FILE                 리포트 경로 (기본 {report_default})
  --cache FILE                  서점 결과 캐시 JSONL (기본 {cache_default})
  --negative-cache-seconds N    빈 결과를 캐시에 두는 시간(초) (기본 {DEFAULT_NEGATIVE_CACHE_SECONDS})
  --max-searches N              새 질의 수 상한, 0이면 무제한 (기본 0)
  --delay SEC                   서점 요청 사이 대기(초) (기본 1.2)
  --offline                     서점에 묻지 않고 캐시만 쓴다
  --refresh-cache               시작 전에 캐시 파일을 비운다
  --apply                       RENAME 계획을 실제로 적용한다
  -h, --help                    이 도움말

종료 코드: 0 정상, 1 이름 바꾸기·리포트 쓰기 실패, 2 잘못된 인자
"
    );
}

/// 인자를 읽는다.
pub fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |name: &str| inline.clone().or_else(|| it.next().cloned()).ok_or_else(|| format!("argument {name}: expected one argument"));
        match name {
            "--root" => options.root = value(name)?.into(),
            "--report" => options.report = value(name)?.into(),
            "--cache" => options.cache = value(name)?.into(),
            "--delay" => {
                let raw = value(name)?;
                options.delay = raw.parse::<f64>().ok().filter(|d| d.is_finite()).ok_or_else(|| format!("argument --delay: invalid float value: '{raw}'"))?.max(0.0);
            }
            "--max-searches" => {
                let raw = value(name)?;
                options.max_searches = raw.parse::<i64>().map_err(|_| format!("argument --max-searches: invalid int value: '{raw}'"))?.max(0) as usize;
            }
            "--negative-cache-seconds" => {
                let raw = value(name)?;
                options.negative_cache_seconds = raw.parse::<i64>().map_err(|_| format!("argument --negative-cache-seconds: invalid int value: '{raw}'"))?.max(0) as u64;
            }
            "--apply" => options.apply = true,
            "--offline" => options.offline = true,
            "--refresh-cache" => options.refresh_cache = true,
            other => return Err(format!("unrecognized arguments: {other}")),
        }
    }
    Ok(options)
}

/// 진입점. 서점에는 HTTPS로 실제 접속한다(`--offline`이면 접속하지 않는다). 종료 코드를 돌려준다.
pub fn run(program: &str, args: &[String], out: &mut dyn Write) -> i32 {
    run_with_searcher(program, args, out, &LiveSearch { fetcher: HttpFetcher::new() })
}

/// `run`과 같고, 서점 검색을 바꿔 끼울 수 있다. `--offline`이면 `live`를 쓰지 않는다.
pub fn run_with_searcher(program: &str, args: &[String], out: &mut dyn Write, live: &dyn Searcher) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => {
            print_usage(program, out);
            return 0;
        }
        Some("run") => {}
        Some(other) => {
            eprintln!("{program}: error: argument command: invalid choice: '{other}' (choose from 'run')");
            return 2;
        }
    }
    let options = match parse_options(&args[1..]) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{program}: error: {msg}");
            return 2;
        }
    };
    let Ok(root) = options.root.canonicalize().and_then(|p| if p.is_dir() { Ok(p) } else { Err(std::io::ErrorKind::NotFound.into()) }) else {
        eprintln!("{program}: error: not a directory: {}", options.root.display());
        return 2;
    };

    if let Err(e) = check_cache_trust(&options.cache) {
        eprintln!("ERROR {e}");
        return 1;
    }
    let mut cache = BookstoreSearchCache::new(&options.cache, options.refresh_cache, options.negative_cache_seconds);
    let searcher: &dyn Searcher = if options.offline { &CacheOnly } else { live };
    let proposals = build_proposals(&root, options.delay, options.max_searches, &mut cache, searcher);
    if let Err(e) = write_report(&proposals, &options.report) {
        eprintln!("ERROR 리포트를 쓰지 못했다: {}: {e}", options.report.display());
        return 1;
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &proposals {
        *counts.entry(p.status.as_str()).or_insert(0) += 1;
    }
    let count = |status: &str| counts.get(status).copied().unwrap_or(0);
    let skipped: usize = counts.iter().filter(|(s, _)| s.starts_with("SKIP_")).map(|(_, n)| n).sum();
    let _ = writeln!(out, "Directories scanned: {}", proposals.len());
    let _ = writeln!(out, "Rename proposals: {}; unchanged: {}; skipped: {skipped}", count("RENAME"), count("UNCHANGED"));
    let _ = writeln!(out, "Report: {}", options.report.display());
    let _ = writeln!(out, "Bookstore cache: {}; store requests: {}; store cache hits: {}; bookstore time: {:.1}s", options.cache.display(), cache.searches, cache.hits, cache.search_seconds);
    if options.apply {
        match apply_renames(&proposals) {
            Ok(changed) => {
                let _ = writeln!(out, "Renamed: {changed}");
                let _ = writeln!(out, "Rebuild or refresh the comics Elasticsearch index after renaming.");
            }
            Err(e) => {
                eprintln!("ERROR {e}");
                return 1;
            }
        }
    } else {
        let _ = writeln!(out, "Dry run only. Pass --apply to rename directories.");
    }
    0
}
