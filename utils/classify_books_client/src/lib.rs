//! 로컬 도서를 Rust 분류 서비스로 분류하거나 tm_dev REST API 작업을 기다린다.
//! Python `utils/classify_books_client.py`의 직접·REST 두 모드에 대응한다.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use backend::book_classifier::{BookClassifierService, ProcessOptions, get_effective_filename};
use serde_json::{Value, json};

pub const DEFAULT_CATEGORY: &str = "0_telegram";
pub const DEFAULT_DELAY: f64 = 1.2;
const TOKEN_ENV: &str = "TM_API_TOKEN";
/// 로그 한 줄을 받는 쪽. 첫 인자는 수준(`INFO`/`WARNING`/`ERROR`), 둘째는 메시지다.
pub type Log<'a> = &'a (dyn Fn(&str, &str) + Sync);

#[derive(Debug, Clone)]
pub struct Options {
    pub api_url: String,
    pub library_root: PathBuf,
    pub category: String,
    pub limit: usize,
    pub offset: usize,
    pub uncached_only: bool,
    pub re_evaluate: bool,
    pub delay: f64,
    pub dry_run: bool,
    pub no_recursive: bool,
    pub no_clean_existing: bool,
    pub no_bookstore: bool,
    pub no_content_meta: bool,
    pub token: Option<String>,
    pub poll_interval: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            api_url: String::new(),
            library_root: PathBuf::from("/mnt/data/text"),
            category: DEFAULT_CATEGORY.into(),
            limit: 1000,
            offset: 0,
            uncached_only: false,
            re_evaluate: false,
            delay: DEFAULT_DELAY,
            dry_run: false,
            no_recursive: false,
            no_clean_existing: false,
            no_bookstore: false,
            no_content_meta: false,
            token: None,
            poll_interval: Duration::from_secs(2),
        }
    }
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = write!(
        out,
        "{program} - 로컬 도서를 분류하거나 tm_dev REST API 작업을 요청한다.

서버의 /categories/auto-classify에 비동기 작업을 요청하고, /categories/auto-classify-status를
2초마다 조회해 진행 상황을 stderr에 남긴다. 서버가 소스 카테고리의 파일을 실제로 옮기고 지우므로
먼저 --dry-run으로 확인한다. 직접 모드는 Rust 분류 서비스와 로컬 파일을 사용한다.

사용법:
  {program} [직접 분류 옵션]
  {program} --api-url URL [REST 옵션]

옵션:
  --api-url URL         tm_dev 백엔드 주소 (지정하면 REST 모드)
  --library-root PATH   라이브러리 루트 (기본 /mnt/data/text)
  --category NAME       분류할 소스 카테고리 (기본 {DEFAULT_CATEGORY})
  --limit N             처리할 최대 도서 수 (기본 1000, 0은 제한 없음)
  --offset N            처리 시작 위치 (기본 0)
  --uncached-only       캐시에서 미분류된 도서를 건너뛴다
  --re-evaluate         서점 조회 없이 기존 캐시로 다시 판정한다
  --delay SEC           서점 요청 사이 대기 시간(초) (기본 {DEFAULT_DELAY})
  --dry-run             파일을 옮기지 않고 시뮬레이션만 한다
  --no-recursive        하위 디렉토리를 탐색하지 않는다
  --no-clean-existing   대상에 같은 파일이 있어도 원본 중복본을 지우지 않는다
  --no-bookstore        서점 3개사 검색을 끈다
  --no-content-meta     EPUB/TXT 메타데이터·본문 키워드 분석을 끈다
  --token TOKEN         Bearer 토큰. 명령행에 남으므로 환경변수 {TOKEN_ENV}를 권한다
  -h, --help            이 도움말

환경변수:
  {TOKEN_ENV}   --token이 없을 때 쓰는 Bearer 토큰

종료 코드: 0 작업 완료(done/idle), 1 요청 실패·작업 실패·통신 오류, 2 잘못된 인자
"
    );
}

/// 인자를 읽는다. `env_token`은 환경변수에서 읽은 토큰이고, `--token`이 있으면 그쪽이 앞선다.
pub fn parse_options(args: &[String], env_token: Option<String>) -> Result<Options, String> {
    parse_options_with_defaults(args, env_token, Options::default())
}

fn parse_options_with_defaults(args: &[String], env_token: Option<String>, mut options: Options) -> Result<Options, String> {
    let mut api_url = None;
    let mut cli_token = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |name: &str| inline.clone().or_else(|| it.next().cloned()).ok_or_else(|| format!("argument {name}: expected one argument"));
        match name {
            "--api-url" => api_url = Some(value(name)?),
            "--library-root" => {
                options.library_root = PathBuf::from(value(name)?);
            }
            "--category" => options.category = value(name)?,
            "--limit" => {
                let raw = value(name)?;
                options.limit = raw.parse().map_err(|_| format!("argument --limit: 0 이상의 정수여야 한다: '{raw}'"))?;
            }
            "--offset" => {
                let raw = value(name)?;
                options.offset = raw.parse().map_err(|_| format!("argument --offset: 0 이상의 정수여야 한다: '{raw}'"))?;
            }
            "--uncached-only" => options.uncached_only = true,
            "--re-evaluate" => options.re_evaluate = true,
            "--delay" => {
                let raw = value(name)?;
                options.delay = raw.parse::<f64>().ok().filter(|d| d.is_finite() && *d >= 0.0).ok_or_else(|| format!("argument --delay: 0 이상의 숫자여야 한다: '{raw}'"))?;
            }
            "--token" => cli_token = Some(value(name)?),
            "--dry-run" => options.dry_run = true,
            "--no-recursive" => options.no_recursive = true,
            "--no-clean-existing" => options.no_clean_existing = true,
            "--no-bookstore" => options.no_bookstore = true,
            "--no-content-meta" => options.no_content_meta = true,
            other => return Err(format!("unrecognized arguments: {other}")),
        }
    }
    options.api_url = api_url.unwrap_or_default();
    options.token = cli_token.or(env_token).filter(|t| !t.is_empty());
    Ok(options)
}

struct Reply {
    status: u16,
    body: String,
}

fn send(agent: &ureq::Agent, url: &str, token: Option<&str>, payload: Option<&Value>) -> Result<Reply, String> {
    let auth = token.map(|t| format!("Bearer {t}"));
    let result = match payload {
        Some(body) => {
            let mut request = agent.post(url);
            if let Some(auth) = &auth {
                request = request.header("Authorization", auth);
            }
            request.send_json(body)
        }
        None => {
            let mut request = agent.get(url);
            if let Some(auth) = &auth {
                request = request.header("Authorization", auth);
            }
            request.call()
        }
    };
    let mut response = result.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(Reply { status, body })
}

fn count(status: &Value, key: &str) -> i64 {
    status.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn python_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|n| n != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn skip_cached_entry<'a>(entries: impl IntoIterator<Item = Option<&'a Value>>) -> bool {
    entries.into_iter().flatten().find(|entry| python_truthy(entry)).is_some_and(|entry| entry.get("target_category").is_none_or(Value::is_null))
}

/// 분류 작업을 요청하고 끝날 때까지 지켜본다. 종료 코드를 돌려준다.
pub fn run_api_client(options: &Options, log: Log) -> i32 {
    let base = options.api_url.trim_end_matches('/');
    let endpoint = format!("{base}/categories/auto-classify");
    let status_endpoint = format!("{base}/categories/auto-classify-status");
    let token = options.token.as_deref();
    let payload = json!({
        "category": options.category,
        "recursive": !options.no_recursive,
        "dry_run": options.dry_run,
        "async_mode": true,
        "clean_existing": !options.no_clean_existing,
        "use_bookstore": !options.no_bookstore,
        "use_content_meta": !options.no_content_meta,
        "delay": options.delay,
    });
    // 토큰은 로그에 남기지 않는다. 페이로드에는 들어 있지 않다.
    log("INFO", &format!("tm_dev API 호출: {endpoint} (payload: {payload})"));

    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into();
    let started = match send(&agent, &endpoint, token, Some(&payload)) {
        Ok(reply) => reply,
        Err(e) => {
            log("ERROR", &format!("API 통신 예외 발생: {e}"));
            return 1;
        }
    };
    if started.status != 200 {
        log("ERROR", &format!("API 요청 실패: status={}, body={}", started.status, started.body));
        return 1;
    }
    let data: Value = match serde_json::from_str(&started.body) {
        Ok(v) => v,
        Err(e) => {
            log("ERROR", &format!("API 통신 예외 발생: {e}"));
            return 1;
        }
    };
    if data.get("status").and_then(Value::as_str) != Some("success") {
        log("ERROR", &format!("API 오류: {}", data.get("error").map_or("None".to_owned(), Value::to_string)));
        return 1;
    }

    log("INFO", "작업 시작됨. 진행 상태 폴링 중...");
    loop {
        std::thread::sleep(options.poll_interval);
        let reply = match send(&agent, &status_endpoint, token, None) {
            Ok(r) => r,
            Err(e) => {
                log("ERROR", &format!("API 통신 예외 발생: {e}"));
                return 1;
            }
        };
        if reply.status != 200 {
            log("WARNING", &format!("상태 조회 실패: {}", reply.status));
            continue;
        }
        let parsed: Value = match serde_json::from_str(&reply.body) {
            Ok(v) => v,
            Err(e) => {
                log("ERROR", &format!("API 통신 예외 발생: {e}"));
                return 1;
            }
        };
        let status = match parsed.get("result") {
            None => Value::Object(serde_json::Map::new()),
            Some(Value::Object(status)) => Value::Object(status.clone()),
            Some(_) => {
                log("ERROR", "API 통신 예외 발생: 상태 응답의 result가 객체가 아닙니다");
                return 1;
            }
        };
        let name = status.get("status").and_then(Value::as_str).unwrap_or("None");
        log(
            "INFO",
            &format!(
                "진행 중: [{name}] {}/{} (이동: {}, 중복정리: {}, 스킵: {}, 실패: {})",
                count(&status, "processed_count"),
                count(&status, "total_count"),
                count(&status, "moved_count"),
                count(&status, "duplicate_cleaned_count"),
                count(&status, "skipped_count"),
                count(&status, "failed_count"),
            ),
        );
        match name {
            "failed" => {
                log("ERROR", &format!("작업 실패: {}", status.get("error").map_or("None".to_owned(), Value::to_string)));
                return 1;
            }
            "done" | "idle" => {
                log("INFO", "작업 완료!");
                return 0;
            }
            _ => {}
        }
    }
}

fn collect_files(dir: &Path, recursive: bool, found: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() && recursive {
            collect_files(&entry.path(), true, found)?;
        } else if kind.is_file() || kind.is_symlink() && entry.path().is_file() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| matches!(extension.to_string_lossy().to_ascii_lowercase().as_str(), "epub" | "pdf" | "txt" | "html" | "htm")) {
                found.push(path);
            }
        }
    }
    Ok(())
}

fn counts_as_classified(result: &Value, source_category: &str) -> bool {
    let Some(target) = result.get("target_category").and_then(Value::as_str) else { return false };
    if target == source_category {
        return false;
    }
    matches!(result.get("action").and_then(Value::as_str), Some("classified" | "moved" | "cleaned_duplicate" | "skipped_already_exists" | "move_failed"))
}

/// Python direct mode를 Rust BookClassifierService에 연결해 로컬 파일을 분류한다.
pub fn run_direct_classifier(options: &Options, log: Log) -> i32 {
    let source_dir = if options.category == "_root" { options.library_root.clone() } else { options.library_root.join(&options.category) };
    if !source_dir.is_dir() {
        log("ERROR", &format!("디렉토리를 찾을 수 없습니다: {}", source_dir.display()));
        return 1;
    }
    let mut files = Vec::new();
    if let Err(error) = collect_files(&source_dir, !options.no_recursive, &mut files) {
        log("ERROR", &format!("파일 목록을 읽지 못했습니다: {error}"));
        return 1;
    }
    files.retain(|path| !path.components().any(|part| part.as_os_str().to_string_lossy().starts_with('.')));
    let mut service = BookClassifierService::new(&options.library_root, None, None).with_delay(options.delay);
    if options.uncached_only {
        files.retain(|path| {
            let rel = path.strip_prefix(&source_dir).unwrap_or(path).to_string_lossy().into_owned();
            let effective = get_effective_filename(path, &source_dir);
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let cache = service.cache();
            !skip_cached_entry([cache.get(&rel), cache.get(&effective), cache.get(name.as_ref())])
        });
    }
    let total = files.len();
    let files: Vec<_> = files.into_iter().skip(options.offset).take(if options.limit == 0 { usize::MAX } else { options.limit }).collect();
    log("INFO", &format!("분류 대상 도서 수집 완료: {}권 (전체 {}, limit={}, offset={}, uncached_only={})", files.len(), total, options.limit, options.offset, options.uncached_only));

    let mut stats = [0_u64; 6]; // processed, classified, moved, duplicate-cleaned, skipped, failed
    for (index, path) in files.iter().enumerate() {
        stats[0] += 1;
        let result = service.process_file(
            path,
            &source_dir,
            ProcessOptions {
                auto_move: true,
                clean_existing: !options.no_clean_existing,
                dry_run: options.dry_run,
                use_bookstore: !options.no_bookstore && !options.re_evaluate,
                use_content_meta: !options.no_content_meta,
                ..Default::default()
            },
        );
        let action = result.get("action").and_then(Value::as_str).unwrap_or("none");
        if counts_as_classified(&result, &options.category) {
            stats[1] += 1;
        }
        match action {
            "moved" => stats[2] += 1,
            "cleaned_duplicate" => stats[3] += 1,
            "classified" | "skipped_already_exists" | "skipped_same_category" | "none" => stats[4] += 1,
            _ => stats[5] += 1,
        }
        if index % 200 == 199 {
            log("INFO", &format!("{}/{} 진행 중: 이동 {}, 중복정리 {}, 스킵 {}, 실패 {}", index + 1, files.len(), stats[2], stats[3], stats[4], stats[5]));
        }
        if index % 500 == 499 && !options.dry_run {
            service.save_cache();
        }
    }
    if !options.dry_run {
        service.save_cache();
    }
    log("INFO", &format!("분류 완료: 총 {}권, 분류 {}, 이동 {}, 중복정리 {}, 스킵 {}, 실패 {}", stats[0], stats[1], stats[2], stats[3], stats[4], stats[5]));
    0
}

/// 진입점. `env_token`은 환경변수에서 읽은 토큰이다. 종료 코드를 돌려준다.
pub fn run(program: &str, args: &[String], env_token: Option<String>, out: &mut dyn Write) -> i32 {
    run_with_defaults(program, args, env_token, out, Options::default())
}

/// 기본 설정을 주입해 실행한다. 인자 없는 실행도 Python CLI처럼 기본 직접 분류를 수행한다.
pub fn run_with_defaults(program: &str, args: &[String], env_token: Option<String>, out: &mut dyn Write, defaults: Options) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage(program, out);
        return 0;
    }
    match parse_options_with_defaults(args, env_token, defaults) {
        Ok(options) if options.api_url.is_empty() => run_direct_classifier(&options, &|level, msg| eprintln!("{} {level} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"))),
        Ok(options) => run_api_client(&options, &|level, msg| eprintln!("{} {level} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"))),
        Err(msg) => {
            eprintln!("{program}: error: {msg}");
            2
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{collect_files, counts_as_classified, skip_cached_entry};
    use serde_json::json;
    use std::fs;

    #[test]
    fn file_discovery_includes_symlinks_to_supported_files() {
        let root =
            std::env::temp_dir().join(format!("classify-books-client-symlink-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        let target = root.join("book-content");
        fs::write(&target, "book").unwrap();
        let link = source.join("book.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let mut found = Vec::new();
        collect_files(&source, true, &mut found).unwrap();

        assert_eq!(found, vec![link]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn direct_summary_does_not_count_same_category_or_refused_targets_as_classified() {
        assert!(!counts_as_classified(&json!({"target_category":"0_telegram","action":"skipped_same_category"}), "0_telegram"));
        assert!(!counts_as_classified(&json!({"target_category":"../outside","action":"none"}), "0_telegram"));
        assert!(!counts_as_classified(&json!({"target_category":null,"action":"none"}), "0_telegram"));
        assert!(counts_as_classified(&json!({"target_category":"3_무협","action":"skipped_already_exists"}), "0_telegram"));
    }

    #[test]
    fn uncached_filter_matches_python_for_missing_target_category_keys() {
        let pending = json!({"status": "pending"});
        let unresolved = json!({"target_category": null});
        let empty = json!({});
        let categorized = json!({"target_category": "3_무협"});
        assert!(skip_cached_entry([Some(&pending)]));
        assert!(skip_cached_entry([Some(&unresolved)]));
        assert!(!skip_cached_entry([Some(&empty)]));
        assert!(!skip_cached_entry([Some(&categorized)]));
        assert!(!skip_cached_entry([None]));
        assert!(skip_cached_entry([Some(&empty), Some(&pending)]));
        assert!(!skip_cached_entry([Some(&empty), Some(&categorized)]));
    }
}
