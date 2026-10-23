//! tm_dev REST API로 도서 자동 분류를 요청하고 끝날 때까지 진행 상태를 지켜본다.
//! 원본: `utils/classify_books_client.py`의 REST 모드(`--api-url`).
//!
//! 로컬 엔진을 직접 부르는 직접 모드는 옮기지 않았다. 서점 크롤링과 분류기(`BookClassifierService`)가
//! 아직 Python이라서다. 직접 모드는 `utils/classify_books_client.py`를 쓴다.
//!
//! 내부망 http로만 통신한다. TLS 기능을 빌드에 넣지 않았으므로 https 주소는 요청 시 오류가 난다.

use std::io::Write;
use std::time::Duration;

use serde_json::{Value, json};

pub const DEFAULT_CATEGORY: &str = "0_telegram";
pub const DEFAULT_DELAY: f64 = 1.2;
/// 상태 조회가 연속으로 이만큼 실패하면 포기한다. 원본은 무한히 재시도했다.
pub const DEFAULT_MAX_STATUS_FAILURES: u32 = 30;
const TOKEN_ENV: &str = "TM_API_TOKEN";
const DIRECT_MODE_OPTIONS: [&str; 5] = ["--library-root", "--limit", "--offset", "--uncached-only", "--re-evaluate"];

/// 로그 한 줄을 받는 쪽. 첫 인자는 수준(`INFO`/`WARNING`/`ERROR`), 둘째는 메시지다.
pub type Log<'a> = &'a (dyn Fn(&str, &str) + Sync);

#[derive(Debug, Clone)]
pub struct Options {
    pub api_url: String,
    pub category: String,
    pub delay: f64,
    pub dry_run: bool,
    pub no_recursive: bool,
    pub no_clean_existing: bool,
    pub no_bookstore: bool,
    pub no_content_meta: bool,
    pub token: Option<String>,
    pub poll_interval: Duration,
    pub max_status_failures: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            api_url: String::new(),
            category: DEFAULT_CATEGORY.into(),
            delay: DEFAULT_DELAY,
            dry_run: false,
            no_recursive: false,
            no_clean_existing: false,
            no_bookstore: false,
            no_content_meta: false,
            token: None,
            poll_interval: Duration::from_secs(2),
            max_status_failures: DEFAULT_MAX_STATUS_FAILURES,
        }
    }
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = write!(
        out,
        "{program} - tm_dev REST API로 도서 자동 분류를 요청하고 끝날 때까지 지켜본다.

서버의 /categories/auto-classify에 비동기 작업을 요청하고, /categories/auto-classify-status를
2초마다 조회해 진행 상황을 stderr에 남긴다. 서버가 소스 카테고리의 파일을 실제로 옮기고 지우므로
먼저 --dry-run으로 확인한다. 로컬 엔진 직접 모드는 utils/classify_books_client.py를 쓴다.

사용법:
  {program} --api-url URL [옵션]

옵션:
  --api-url URL         tm_dev 백엔드 주소 (필수, 내부망 http)
  --category NAME       분류할 소스 카테고리 (기본 {DEFAULT_CATEGORY})
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
    let mut options = Options::default();
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
            "--category" => options.category = value(name)?,
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
            direct if DIRECT_MODE_OPTIONS.contains(&direct) => return Err(format!("{direct}는 직접 모드 옵션이다. 직접 모드는 utils/classify_books_client.py를 쓴다")),
            other => return Err(format!("unrecognized arguments: {other}")),
        }
    }
    options.api_url = api_url.ok_or("the following arguments are required: --api-url")?;
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
    let mut failures = 0;
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
            failures += 1;
            log("WARNING", &format!("상태 조회 실패: {}", reply.status));
            if failures >= options.max_status_failures {
                log("ERROR", &format!("상태 조회가 연속 {failures}번 실패해 중단한다"));
                return 1;
            }
            continue;
        }
        failures = 0;
        let parsed: Value = match serde_json::from_str(&reply.body) {
            Ok(v) => v,
            Err(e) => {
                log("ERROR", &format!("API 통신 예외 발생: {e}"));
                return 1;
            }
        };
        let status = parsed.get("result").cloned().unwrap_or(Value::Null);
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

/// 진입점. `env_token`은 환경변수에서 읽은 토큰이다. 종료 코드를 돌려준다.
pub fn run(program: &str, args: &[String], env_token: Option<String>, out: &mut dyn Write) -> i32 {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage(program, out);
        return 0;
    }
    match parse_options(args, env_token) {
        Ok(options) => run_api_client(&options, &|level, msg| eprintln!("{} {level} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"))),
        Err(msg) => {
            eprintln!("{program}: error: {msg}");
            2
        }
    }
}
