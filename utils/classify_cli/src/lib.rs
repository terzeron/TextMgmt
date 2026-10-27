//! Python `utils/classify_cli.py`의 Rust 실행 도구.

mod args;
mod commands;
mod data;
mod golden;
mod shuffle;

use std::io::Write;
use std::sync::Arc;

use backend::es_manager::{EsTransport, HttpTransport};

pub use data::{holdout_manifest_path, load_manifest_holdout};

pub trait BookstoreQuery: Send + Sync {
    fn query(&self, title: &str, author: &str, raw_title: &str, delay: f64) -> (serde_json::Value, serde_json::Value, serde_json::Value);
}

pub struct CliIo<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

pub struct Deps {
    pub(crate) es: Option<Arc<dyn EsTransport>>,
    pub(crate) bookstores: Option<Arc<dyn BookstoreQuery>>,
    pub(crate) config: Option<serde_json::Value>,
}

impl Deps {
    pub fn system() -> Self {
        Self { es: None, bookstores: None, config: None }
    }

    pub fn with_es(mut self, es: Arc<dyn EsTransport>) -> Self {
        self.es = Some(es);
        self
    }

    pub fn with_bookstores(mut self, bookstores: Arc<dyn BookstoreQuery>) -> Self {
        self.bookstores = Some(bookstores);
        self
    }

    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = Some(config);
        self
    }

    fn transport(&self, index: Option<&str>) -> Result<Arc<dyn EsTransport>, String> {
        if let Some(es) = &self.es {
            return Ok(es.clone());
        }
        let url = std::env::var("TM_ES_URL").map_err(|_| "TM_ES_URL이 없다. 파일만 읽으려면 --no-es를 쓸 것")?;
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("ES 주소는 http:// 또는 https://여야 한다".into());
        }
        let index = index.map(str::to_owned).or_else(|| std::env::var("TM_ES_BOOK_INDEX").ok()).ok_or("TM_ES_BOOK_INDEX가 없다")?;
        let ca_cert_path = std::env::var_os("TM_ES_CA_CERT").map(std::path::PathBuf::from);
        Ok(Arc::new(HttpTransport::new_with_ca(
            &url,
            &std::env::var("TM_ES_USER").unwrap_or_default(),
            &std::env::var("TM_ES_PASSWORD").unwrap_or_default(),
            &index,
            std::time::Duration::from_secs(120),
            ca_cert_path.as_deref(),
        )))
    }
}

pub fn commands() -> &'static [&'static str] {
    &["collect", "train", "evaluate", "score-calibration", "bookstore-policy", "classify", "reclassify", "info", "import-joblib"]
}

pub fn print_usage(out: &mut dyn Write, command: Option<&str>) -> std::io::Result<()> {
    writeln!(
        out,
        "도서 자동분류 모델 관리 도구\n사용법: classify_cli <명령> [옵션]\n명령: {}\n인자가 없거나 -h/--help이면 도움말을 출력한다.\n종료 코드: 0 성공, 1 실행 실패, 2 잘못된 인자",
        commands().join(", ")
    )?;
    if let Some(command) = command {
        writeln!(out, "\n{command}: {}", args::usage(command))?;
    }
    writeln!(out, "\n일반 분류 명령의 --model은 Rust export 디렉토리다. import-joblib은 --model joblib 파일을 --out export 디렉토리로 변환한다.")
}

pub fn main_with(args: &[&str], io: &mut CliIo<'_>, deps: &Deps) -> i32 {
    if args.is_empty() {
        let _ = print_usage(io.out, None);
        return 1;
    }
    let parsed = match args::parse(args) {
        Ok(parsed) => parsed,
        Err(e) => {
            let _ = writeln!(io.err, "{e}");
            return 2;
        }
    };
    if parsed.help {
        let no_command = parsed.command.is_none() && !args.iter().any(|arg| matches!(*arg, "-h" | "--help"));
        return if print_usage(io.out, parsed.command.as_deref()).is_ok() && !no_command { 0 } else { 1 };
    }
    match commands::execute(&parsed, io, deps) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(io.err, "{e}");
            1
        }
    }
}
