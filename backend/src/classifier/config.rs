//! 분류기 설정. Python `backend/classifier/config.py` 대응.
//!
//! 기본값은 `default_config.json` 에 Python `DEFAULT_CONFIG` 와 `SERIES_TO_PARENT` 를 그대로 옮겨 두었다.
//! 설정 파일은 신뢰할 수 없는 입력처럼 읽는다. 크기 상한을 읽기 전에 검사하고, JSON 깊이는 serde_json 기본 한도(128)를 따른다.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::sync::LazyLock;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::pyjson;

/// 설정 파일 최대 크기. 정상 설정은 수 KB 다.
pub const MAX_CONFIG_BYTES: u64 = 4 << 20;
/// 설정 파일 경로를 담는 환경변수. 없으면 기본값만 쓴다(Python 은 패키지 옆 `config.json`).
pub const CONFIG_ENV: &str = "TM_CLASSIFIER_CONFIG";

static DEFAULTS: LazyLock<Value> = LazyLock::new(|| serde_json::from_str(include_str!("default_config.json")).expect("default_config.json 은 유효한 JSON 이다"));

/// Python `DEFAULT_CONFIG`.
pub fn default_config() -> Value {
    DEFAULTS["config"].clone()
}

/// Python `SERIES_TO_PARENT`. 하위 카테고리(전집)를 상위 장르로 접는 표.
pub fn series_to_parent() -> HashMap<String, String> {
    DEFAULTS["series_to_parent"].as_object().map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|p| (k.clone(), p.to_string()))).collect()).unwrap_or_default()
}

/// 설정을 읽는다. 파일이 없거나 깨졌거나 너무 크면 기본값을 쓴다.
pub fn load_config(path: &Path) -> Value {
    let cfg = default_config();
    match read_json_object(path) {
        Some(user) => merge_config(&cfg, &user),
        None => cfg,
    }
}

/// `CONFIG_ENV` 가 가리키는 설정. 환경변수가 없으면 기본값이다.
pub fn load_config_from_env() -> Value {
    match std::env::var(CONFIG_ENV) {
        Ok(p) if !p.is_empty() => load_config(Path::new(&p)),
        _ => default_config(),
    }
}

fn read_json_object(path: &Path) -> Option<Value> {
    let file = fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > MAX_CONFIG_BYTES {
        log::warn!("설정 파일이 너무 커서 기본값을 쓴다 ({}바이트 > {MAX_CONFIG_BYTES})", meta.len());
        return None;
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut buf).ok()?;
    match serde_json::from_slice::<Value>(&buf) {
        Ok(v) if v.is_object() => Some(v),
        Ok(_) => {
            log::warn!("설정 파일이 JSON 객체가 아니라 기본값을 쓴다");
            None
        }
        Err(e) => {
            log::warn!("설정 파일을 읽지 못해 기본값을 쓴다: {e}");
            None
        }
    }
}

/// `over` 의 값으로 `base` 를 덮어쓴다. 객체는 한 단계씩 내려가며 합친다.
pub fn merge_config(base: &Value, over: &Value) -> Value {
    let (Some(b), Some(o)) = (base.as_object(), over.as_object()) else {
        return base.clone();
    };
    let mut out = b.clone();
    for (k, v) in o {
        let merged = match (out.get(k), v) {
            (Some(current), Value::Object(_)) if current.is_object() => merge_config(current, v),
            _ => v.clone(),
        };
        out.insert(k.clone(), merged);
    }
    Value::Object(out)
}

/// 설정을 `indent=2`, `ensure_ascii=False` 로 쓴다. 부모 디렉토리가 없으면 만든다.
pub fn save_config(cfg: &Value, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, pyjson::dumps_indent2(cfg))
}

/// 설정의 지문. Python `config_hash` 와 같은 값이다(`sort_keys=True` JSON 의 sha256 앞 16자).
pub fn config_hash(cfg: &Value) -> String {
    let digest = Sha256::digest(pyjson::dumps(cfg, true).as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `enabled` 가 참인 필드만 남긴다. 순서는 설정 순서다.
pub fn enabled_fields(cfg: &Value) -> Value {
    let fields = cfg["fields"].as_object().cloned().unwrap_or_default();
    Value::Object(fields.into_iter().filter(|(_, v)| v.get("enabled").is_some_and(truthy)).collect())
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// 하위 카테고리면 상위 장르를, 아니면 그대로 돌려준다.
pub fn resolve_parent(category: Option<&str>) -> Option<String> {
    let category = category?;
    if category.is_empty() {
        return Some(String::new());
    }
    Some(series_to_parent().get(category).cloned().unwrap_or_else(|| category.to_string()))
}
