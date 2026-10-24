//! 서점 신호를 쓰는 규칙. Python `backend/classifier/bookstore_policy.py` 대응.
//!
//! 1. 매핑 - 서점 카테고리 문자열을 내부 카테고리로 옮긴다. DB(`category_keywords`)를 먼저 보고, 없으면 하드코딩 규칙이다.
//! 2. 결합 - 모델과 서점 중 어느 쪽을 따를지. `bookstore-policy` 명령이 잰 경계 하나를 남기고, 그 미만이면 서점이 먼저다.
//!
//! DB 가 없거나 죽어 있어도 판정은 계속된다. 그때는 하드코딩 규칙만 쓴다.
//! 정책 파일은 신뢰할 수 없는 입력처럼 읽는다: 크기 상한, 심볼릭 링크 거부, JSON 깊이 한도, 로그에 내용 없음.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use super::model::model_path;
use super::tokenize::is_py_space;
use crate::book_classifier::map_category;

/// DB 를 파일마다 조회하면 재분류 2만 건에 2만 번 연결한다. 캐시하고 주기적으로만 다시 읽는다.
pub const KEYWORD_CACHE_SECONDS: u64 = 300;
/// 정책 파일 최대 크기. 정상 파일은 수 KB 다.
pub const MAX_POLICY_BYTES: u64 = 1 << 20;

static KEYWORDS: Mutex<Option<(Value, Instant)>> = Mutex::new(None);

fn strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// 카테고리-키워드 매핑(`{카테고리: [키워드...]}`)을 `{키워드: [카테고리...]}` 로 뒤집는다.
///
/// 한 키워드가 여러 카테고리에 붙는 것은 정상이다(`클래식` 은 `1_서양고전` 이자 `5_음악`). 그래서 버리지 않고 후보를 전부
/// 남긴다. 어느 쪽인지는 서점 다수결이 가른다.
pub fn _load_from_db_with(get_all_mappings: &dyn Fn(&str) -> Value) -> Value {
    let mut out: Map<String, Value> = Map::new();
    let mappings = get_all_mappings("book");
    for (category, keywords) in mappings.as_object().into_iter().flatten() {
        for keyword in keywords.as_array().into_iter().flatten() {
            let word = strip(keyword.as_str().unwrap_or(""));
            if word.is_empty() {
                continue;
            }
            let bucket = out.entry(word.to_string()).or_insert_with(|| Value::Array(Vec::new()));
            if let Some(list) = bucket.as_array_mut()
                && !list.iter().any(|c| c.as_str() == Some(category))
            {
                list.push(Value::String(category.clone()));
            }
        }
    }
    Value::Object(out)
}

/// DB 에서 키워드 맵을 읽는다. 연결이나 질의가 실패하면 오류 문자열이다.
pub fn _load_from_db() -> Result<Value, String> {
    let mapping = crate::category_mapping::CategoryMapping::new().map_err(|e| e.to_string())?;
    Ok(_load_from_db_with(&|content_type| {
        let all = mapping.get_all_mappings(content_type);
        Value::Object(all.into_iter().map(|(k, v)| (k, Value::Array(v.into_iter().map(Value::String).collect()))).collect())
    }))
}

/// DB 키워드 맵. 5분간 캐시한다. DB 를 못 읽으면 빈 맵을 준다(하드코딩 규칙으로 넘어간다).
pub fn keyword_map(loader: Option<&dyn Fn() -> Result<Value, String>>, force: bool) -> Value {
    let mut guard = KEYWORDS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached, at)) = guard.as_ref()
        && !force
        && at.elapsed() < Duration::from_secs(KEYWORD_CACHE_SECONDS)
    {
        return cached.clone();
    }
    let loaded = match loader {
        Some(f) => f(),
        None => _load_from_db(),
    };
    let value = match loaded {
        Ok(v) => {
            log::info!("서점 매핑 키워드 {}개를 DB 에서 읽었다", v.as_object().map_or(0, Map::len));
            v
        }
        Err(e) => {
            // DB 가 죽어도 판정은 계속돼야 한다. 다음 호출에서 다시 시도한다.
            log::warn!("DB 에서 키워드를 못 읽었다. 하드코딩 규칙만 쓴다: {e}");
            Value::Object(Map::new())
        }
    };
    *guard = Some((value.clone(), Instant::now()));
    value
}

/// 캐시를 비운다. 키워드를 고친 뒤와 테스트에서 쓴다.
pub fn reset_keyword_cache() {
    *KEYWORDS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// 서점 카테고리 문자열에 걸리는 DB 키워드의 카테고리 후보를 모은다.
///
/// 긴 키워드를 먼저 본다. '영어전문교육' 이 '영어' 보다 구체적이라 먼저 맞아야 한다. 첫 키워드가 후보를 하나만 주면 거기서
/// 끝내고, 여러 개를 주면 그대로 돌려준다.
pub fn match_keyword(cat_str: &str, keywords: &Value) -> Vec<String> {
    let Some(map) = keywords.as_object().filter(|m| !m.is_empty()) else { return Vec::new() };
    if cat_str.is_empty() {
        return Vec::new();
    }
    let mut words: Vec<&String> = map.keys().collect();
    // 길이 내림차순. 같으면 원래 순서(안정 정렬).
    words.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    for word in words {
        if cat_str.contains(word.as_str()) {
            return match &map[word.as_str()] {
                Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                Value::String(s) => vec![s.clone()],
                _ => Vec::new(),
            };
        }
    }
    Vec::new()
}

/// 서점이 준 카테고리를 내부 카테고리 하나로 옮긴다. 후보가 여럿이면 `None` 이다.
pub fn map_bookstore_category(cat_str: &str, raw_title: &str, raw_author: &str, keywords: Option<&Value>) -> Option<String> {
    let mut candidates = map_bookstore_candidates(cat_str, raw_title, raw_author, keywords);
    if candidates.len() == 1 { candidates.pop() } else { None }
}

/// 서점이 준 카테고리를 내부 카테고리 후보 목록으로 옮긴다. 순서가 중요하다.
///
/// 1. 저자·시리즈 전용 규칙(카테고리 문자열을 비우고 부르면 그 절만 탄다)
/// 2. DB 키워드 - 한 키워드가 여러 카테고리에 붙을 수 있다. 후보를 전부 돌려준다.
/// 3. 하드코딩 규칙 전체 - DB 가 안 덮는 카테고리를 메운다.
pub fn map_bookstore_candidates(cat_str: &str, raw_title: &str, raw_author: &str, keywords: Option<&Value>) -> Vec<String> {
    if let Some(specific) = map_category("", raw_title, raw_author) {
        return vec![specific];
    }
    let hits = match keywords {
        Some(k) => match_keyword(cat_str, k),
        None => match_keyword(cat_str, &keyword_map(None, false)),
    };
    if !hits.is_empty() {
        return hits;
    }
    map_category(cat_str, raw_title, raw_author).into_iter().collect()
}

/// 확신도 분포를 보고 구간 경계를 고른다. 분위수 자리 근처에서 이웃 값 사이가 가장 크게 벌어진 곳(밀도가 낮은 곳)으로 옮긴다.
pub fn quantile_bands(confidences: &[f64], count: usize) -> Vec<f64> {
    quantile_bands_with_window(confidences, count, 0.5)
}

/// `quantile_bands` 의 `window`(분위수 한 칸의 몇 배만큼 좌우를 살필지) 지정 판.
pub fn quantile_bands_with_window(confidences: &[f64], count: usize, window: f64) -> Vec<f64> {
    let mut values: Vec<f64> = confidences.to_vec();
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n < 2 || count < 2 {
        return Vec::new();
    }
    let step = n as f64 / count as f64;
    let reach = ((step * window) as usize).max(1);
    let mut cuts: Vec<f64> = Vec::new();
    for i in 1..count {
        let center = super::training::round_half_even(i as f64 * step).max(0) as usize;
        let lo = center.saturating_sub(reach).max(1);
        let hi = (center + reach).min(n - 1);
        if lo >= hi {
            continue;
        }
        let mut best = lo;
        for k in lo..=hi {
            if values[k] - values[k - 1] > values[best] - values[best - 1] {
                best = k;
            }
        }
        let cut = (values[best] + values[best - 1]) / 2.0;
        if cuts.last().is_none_or(|last| cut > *last) {
            cuts.push(cut);
        }
    }
    cuts
}

/// 경계값 목록을 (하한, 상한) 구간으로 바꾼다. 마지막 구간의 상한은 1 을 넘겨 둔다.
pub fn bands_from_cuts(cuts: &[f64]) -> Vec<(f64, f64)> {
    let mut edges = vec![0.0];
    edges.extend_from_slice(cuts);
    edges.push(1.01);
    edges.windows(2).map(|w| (w[0], w[1])).collect()
}

/// 모델과 서점 중 어느 쪽을 따를지 정하는 규칙. `override_below` 가 0 이면 서점이 이긴 구간이 없었다는 뜻이다.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookstorePolicy {
    pub override_below: f64,
    pub sample: i64,
    pub bands: Vec<Value>,
    pub source: Option<String>,
    /// 표본에서 실제로 본 가장 높은 확신도. 경계는 여기를 넘을 수 없다.
    pub measured_max: f64,
}

fn read_policy_file(path: &Path) -> Result<Value, String> {
    if fs::symlink_metadata(path).map_err(|e| e.to_string())?.file_type().is_symlink() {
        return Err("심볼릭 링크는 따르지 않는다".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_POLICY_BYTES {
        return Err("일반 파일이 아니거나 너무 크다".into());
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_POLICY_BYTES + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_slice(&buf).map_err(|e| e.to_string())?;
    if value.is_object() { Ok(value) } else { Err("JSON 객체가 아니다".into()) }
}

impl BookstorePolicy {
    /// 정책 파일을 읽는다. 경로를 안 주면 모델 디렉토리 옆 `bookstore_policy.json` 이다. 없거나 깨졌으면 기본값이다.
    pub fn load(path: Option<&Path>) -> Self {
        let p = path.map(Path::to_path_buf).unwrap_or_else(default_policy_path);
        if !p.exists() {
            log::info!("서점 규칙 파일이 없다. 서점은 모델이 거부한 건에만 쓴다");
            return Self::default();
        }
        let data = match read_policy_file(&p) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("서점 규칙 파일을 못 읽었다: {e}");
                return Self::default();
            }
        };
        let num = |key: &str| data.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0);
        let measured_max = num("measured_max_confidence");
        let mut override_below = num("bookstore_override_below");
        if measured_max != 0.0 && override_below > measured_max {
            // 옛 파일이거나 잘못 계산된 경우. 재지 않은 구간까지 서점에 넘기지 않는다.
            log::warn!("서점 경계 {override_below:.3} 가 측정 범위 {measured_max:.3} 를 넘어 잘라 쓴다");
            override_below = measured_max;
        }
        Self {
            override_below,
            sample: data.get("sample").and_then(Value::as_f64).unwrap_or(0.0) as i64,
            bands: data.get("bands").and_then(Value::as_array).cloned().unwrap_or_default(),
            source: Some(p.to_string_lossy().into_owned()),
            measured_max,
        }
    }

    /// 이 확신도에서는 모델보다 서점을 믿는 것이 나은가.
    pub fn prefers_bookstore(&self, confidence: f64) -> bool {
        self.override_below > 0.0 && confidence < self.override_below
    }
}

/// 모델 디렉토리 옆에 둔다. 학습 머신과 pod 가 같은 파일을 본다.
pub fn default_policy_path() -> std::path::PathBuf {
    model_path().parent().map(Path::to_path_buf).unwrap_or_default().join("bookstore_policy.json")
}
