//! 학습 쪽 결정적 도구. Python `backend/classifier/training.py` 대응.
//!
//! LinearSVC 학습은 Python 에 남는다(`backend/classifier/training.py`). 여기에는 홀드아웃 분할, 크기 필터, 판정률-정답률
//! 곡선, 평가, 리포트 출력처럼 값이 결정적인 부분만 옮겼다. `train` 은 입력 검사까지 하고 학습 자체는 거부한다.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::ValueError;
use super::model::{CategoryModel, softmax_confidence};
use super::softmax::fixed;

// ---------------------------------------------------------------------------
// BLAKE2b (RFC 7693). `hashlib.blake2b(data, digest_size=8, key=key)` 와 같은 값이다.
// 홀드아웃 소속이 Python 학습과 같아야 해서 직접 구현했다(새 crate 금지).
// ---------------------------------------------------------------------------

const IV: [u64; 8] = [0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179];
const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

fn compress(h: &mut [u64; 8], block: &[u8; 128], t: u128, last: bool) {
    let mut m = [0u64; 16];
    for (i, w) in m.iter_mut().enumerate() {
        *w = u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
    }
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(h);
    v[8..].copy_from_slice(&IV);
    v[12] ^= t as u64;
    v[13] ^= (t >> 64) as u64;
    if last {
        v[14] = !v[14];
    }
    fn g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
        v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
        v[d] = (v[d] ^ v[a]).rotate_right(32);
        v[c] = v[c].wrapping_add(v[d]);
        v[b] = (v[b] ^ v[c]).rotate_right(24);
        v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
        v[d] = (v[d] ^ v[a]).rotate_right(16);
        v[c] = v[c].wrapping_add(v[d]);
        v[b] = (v[b] ^ v[c]).rotate_right(63);
    }
    for round in 0..12 {
        let s = &SIGMA[round % 10];
        g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
        g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }
    for i in 0..8 {
        h[i] ^= v[i] ^ v[i + 8];
    }
}

/// 키가 있는 BLAKE2b. 출력은 `out_len`(1..=64) 바이트, 키는 64바이트 이하다.
pub fn blake2b_keyed(data: &[u8], key: &[u8], out_len: usize) -> Vec<u8> {
    assert!((1..=64).contains(&out_len) && key.len() <= 64);
    let mut h = IV;
    h[0] ^= 0x0101_0000 ^ ((key.len() as u64) << 8) ^ out_len as u64;
    let mut blocks: Vec<[u8; 128]> = Vec::new();
    if !key.is_empty() {
        let mut b = [0u8; 128];
        b[..key.len()].copy_from_slice(key);
        blocks.push(b);
    }
    for chunk in data.chunks(128) {
        let mut b = [0u8; 128];
        b[..chunk.len()].copy_from_slice(chunk);
        blocks.push(b);
    }
    if blocks.is_empty() {
        blocks.push([0u8; 128]);
    }
    let total = (data.len() + if key.is_empty() { 0 } else { 128 }) as u128;
    let n = blocks.len();
    let mut consumed: u128 = 0;
    for (i, block) in blocks.iter().enumerate() {
        let last = i == n - 1;
        // 마지막 블록은 실제 바이트 수까지만 센다.
        consumed = if last { total } else { consumed + 128 };
        compress(&mut h, block, consumed, last);
    }
    h.iter().flat_map(|w| w.to_le_bytes()).take(out_len).collect()
}

// ---------------------------------------------------------------------------
// 메모리 노트
// ---------------------------------------------------------------------------

/// 지금 이 프로세스가 쥔 메모리. 어느 단계가 피크인지 로그로 남긴다. 읽을 수 없으면 빈 문자열이다.
pub fn rss_note() -> String {
    rss_note_with(Path::new("/proc/self/status"))
}

/// `rss_note` 의 상태 파일 경로 주입 판.
pub fn rss_note_with(status_path: &Path) -> String {
    let Ok(text) = fs::read_to_string(status_path) else { return String::new() };
    let mut fields: HashMap<&str, f64> = HashMap::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else { continue };
        if matches!(key, "VmHWM" | "VmRSS" | "RssAnon" | "RssFile") {
            match rest.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()) {
                Some(kb) => {
                    fields.insert(key, kb);
                }
                None => return String::new(),
            }
        }
    }
    let gb = |k: &str| fields.get(k).copied().unwrap_or(0.0) / (1024.0 * 1024.0);
    format!("[익명 {}GB + 파일 {}GB = {}GB, 최대 {}GB]", fixed(gb("RssAnon"), 1), fixed(gb("RssFile"), 1), fixed(gb("VmRSS"), 1), fixed(gb("VmHWM"), 1))
}

// ---------------------------------------------------------------------------
// 분할과 크기 필터
// ---------------------------------------------------------------------------

/// 표본이 너무 적은 카테고리를 뺀다. 층화 분할이 성립하려면 최소 2건은 있어야 한다. 뺀 카테고리는 정렬해 돌려준다.
pub fn filter_by_size(docs: &[Value], min_per_category: usize) -> (Vec<Value>, Vec<String>) {
    let cat = |d: &Value| d["cat"].as_str().unwrap_or("").to_string();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for d in docs {
        *counts.entry(cat(d)).or_insert(0) += 1;
    }
    let floor = min_per_category.max(2);
    let mut dropped: Vec<String> = counts.iter().filter(|(_, n)| **n < floor).map(|(c, _)| c.clone()).collect();
    dropped.sort();
    let kept = docs.iter().filter(|d| counts[&cat(d)] >= floor).cloned().collect();
    (kept, dropped)
}

/// 분할에 쓸 문서의 고유 키. ES 의 `_id`(inode)가 없으면 경로, 이름 순이다.
pub fn document_key(doc: &Value) -> String {
    ["id", "path", "name"].iter().filter_map(|k| doc.get(*k).and_then(Value::as_str)).find(|s| !s.is_empty()).unwrap_or("").to_string()
}

/// Python `round()` 는 반올림에서 짝수 쪽을 고른다.
pub(crate) fn round_half_even(x: f64) -> i64 {
    let floor = x.floor();
    if x - floor == 0.5 {
        if floor % 2.0 == 0.0 { floor as i64 } else { floor as i64 + 1 }
    } else {
        x.round() as i64
    }
}

/// 문서 키의 해시로 홀드아웃을 뗀다. 카테고리별로 비율을 맞추고, 양쪽에 최소 1건씩 남긴다.
/// 키는 `blake2b(key=str(seed), digest_size=8)` 로 Python 과 같다. 반환은 (학습, 홀드아웃).
pub fn split_holdout(docs: &[Value], ratio: f64, seed: u64) -> (Vec<Value>, Vec<Value>) {
    let key = seed.to_string();
    let mut order: Vec<String> = Vec::new();
    let mut buckets: HashMap<String, Vec<(Vec<u8>, &Value)>> = HashMap::new();
    for doc in docs {
        let digest = blake2b_keyed(document_key(doc).as_bytes(), key.as_bytes(), 8);
        let cat = doc["cat"].as_str().unwrap_or("").to_string();
        if !buckets.contains_key(&cat) {
            order.push(cat.clone());
        }
        buckets.entry(cat).or_default().push((digest, doc));
    }
    let (mut train, mut hold) = (Vec::new(), Vec::new());
    for cat in order {
        let mut rows = buckets.remove(&cat).unwrap_or_default();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let n = rows.len() as i64;
        let n_hold = if n > 1 { round_half_even(n as f64 * ratio).max(1).min(n - 1) as usize } else { 0 };
        hold.extend(rows[..n_hold].iter().map(|(_, d)| (*d).clone()));
        train.extend(rows[n_hold..].iter().map(|(_, d)| (*d).clone()));
    }
    (train, hold)
}

// ---------------------------------------------------------------------------
// 곡선과 평가
// ---------------------------------------------------------------------------

fn order_desc(confidence: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..confidence.len()).collect();
    order.sort_by(|&a, &b| confidence[b].total_cmp(&confidence[a]));
    order
}

fn running_precision(correct: &[bool], order: &[usize]) -> Vec<f64> {
    let mut hits = 0usize;
    order
        .iter()
        .enumerate()
        .map(|(i, &j)| {
            hits += usize::from(correct[j]);
            hits as f64 / (i + 1) as f64
        })
        .collect()
}

/// 확신도가 높은 순으로 줄 세우고, 상위 몇 %까지 답했을 때의 정답률을 본다.
pub fn coverage_curve(correct: &[bool], confidence: &[f64], points: &[f64]) -> Vec<Value> {
    let order = order_desc(confidence);
    let running = running_precision(correct, &order);
    points
        .iter()
        .map(|&cov| {
            let k = ((order.len() as f64 * cov) as usize).saturating_sub(1);
            json!({"coverage": cov, "precision": running[k], "threshold": confidence[order[k]]})
        })
        .collect()
}

/// 기본 곡선 지점.
pub const DEFAULT_CURVE_POINTS: [f64; 6] = [0.5, 0.7, 0.8, 0.9, 0.95, 1.0];

/// 목표 정답률을 지키면서 답할 수 있는 최대 비율과, 그때의 임계값. 못 지키면 판정률 0, 임계값 1.
pub fn max_coverage_at(correct: &[bool], confidence: &[f64], target_precision: f64) -> Value {
    let order = order_desc(confidence);
    let running = running_precision(correct, &order);
    match running.iter().rposition(|p| *p >= target_precision) {
        None => json!({"coverage": 0.0, "precision": 0.0, "threshold": 1.0}),
        Some(k) => json!({"coverage": (k + 1) as f64 / order.len() as f64, "precision": running[k], "threshold": confidence[order[k]]}),
    }
}

fn parent_of<'a>(map: &'a HashMap<String, String>, cat: &'a str) -> &'a str {
    map.get(cat).map(String::as_str).unwrap_or(cat)
}

/// 홀드아웃으로 판정률-정답률 곡선을 낸다. `accept_parent` 는 하위 카테고리를 상위 장르로 접는 표다.
pub fn evaluate(model: &CategoryModel, docs: &[Value], accept_parent: Option<&HashMap<String, String>>) -> Value {
    let empty = HashMap::new();
    let parent = accept_parent.unwrap_or(&empty);
    let scores: Vec<Vec<f64>> = docs.iter().map(|d| model.scores(d)).collect();
    let prob = softmax_confidence(&scores, model.temperature());
    let mut correct = Vec::with_capacity(docs.len());
    let mut conf = Vec::with_capacity(docs.len());
    for ((doc, row), p) in docs.iter().zip(&scores).zip(&prob) {
        // np.argmax: 최댓값이 여럿이면 첫 번째.
        let best = row.iter().enumerate().fold(0, |b, (i, v)| if *v > row[b] { i } else { b });
        let pred = model.classes[best].as_str();
        let truth = doc["cat"].as_str().unwrap_or("");
        correct.push(pred == truth || parent_of(parent, pred) == parent_of(parent, truth));
        conf.push(p[best]);
    }
    let top1 = if docs.is_empty() { 0.0 } else { correct.iter().filter(|c| **c).count() as f64 / docs.len() as f64 };
    json!({"n": docs.len(), "top1_accuracy": top1, "curve": coverage_curve(&correct, &conf, &DEFAULT_CURVE_POINTS), "at_precision_90": max_coverage_at(&correct, &conf, 0.90), "at_precision_95": max_coverage_at(&correct, &conf, 0.95)})
}

/// Python `f"{n:,}"`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `f"{x:>{width}.{digits}%}"`.
fn percent(x: f64, digits: usize, width: usize) -> String {
    format!("{:>width$}", format!("{}%", fixed(x * 100.0, digits)))
}

pub fn format_report(report: &Value) -> String {
    let f = |v: &Value| v.as_f64().unwrap_or(0.0);
    let mut lines = vec![format!("홀드아웃 {}건  전체 top-1 정답률 {}", thousands(report["n"].as_u64().unwrap_or(0)), percent(f(&report["top1_accuracy"]), 1, 0)), String::new(), "  판정률   정답률   임계값".to_string()];
    for row in report["curve"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        lines.push(format!("  {}   {}   {}", percent(f(&row["coverage"]), 0, 5), percent(f(&row["precision"]), 1, 6), fixed(f(&row["threshold"]), 3)));
    }
    let (a90, a95) = (&report["at_precision_90"], &report["at_precision_95"]);
    lines.push(String::new());
    lines.push(format!("  정답률 90% 유지 최대 판정률: {}  (임계값 {})", percent(f(&a90["coverage"]), 1, 0), fixed(f(&a90["threshold"]), 3)));
    lines.push(format!("  정답률 95% 유지 최대 판정률: {}  (임계값 {})", percent(f(&a95["coverage"]), 1, 0), fixed(f(&a95["threshold"]), 3)));
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// 학습 진입점
// ---------------------------------------------------------------------------

/// `train` 옵션.
#[derive(Debug, Clone, Default)]
pub struct TrainOptions {
    pub accept_parent: Option<HashMap<String, String>>,
    pub spill_dir: Option<PathBuf>,
}

/// 학습은 Python 에 남는다. 입력 검사(표본 부족 카테고리 제외)까지 Python 과 같게 하고, 학습 자체는 거부한다.
///
/// 모델은 `python -m backend.classifier.export` 로 내보낸 디렉토리를 `CategoryModel::load` 로 읽는다.
pub fn train(docs: &mut [Value], config: &Value, _opts: TrainOptions) -> Result<(CategoryModel, Value), ValueError> {
    let min = config["corpus"]["min_per_category"].as_u64().unwrap_or(30) as usize;
    let (kept, dropped) = filter_by_size(docs, min);
    if kept.is_empty() {
        return Err(ValueError("학습할 문서가 없다. min_per_category 를 낮추거나 collect 를 다시 하라".into()));
    }
    if !dropped.is_empty() {
        let shown: Vec<&str> = dropped.iter().take(10).map(String::as_str).collect();
        log::info!("표본 부족으로 제외한 카테고리 {}개: {}{}", dropped.len(), shown.join(", "), if dropped.len() > 10 { " ..." } else { "" });
    }
    Err(ValueError("학습은 Python 에 남는다: backend/classifier/training.py 로 학습하고 backend.classifier.export 로 내보낸다".into()))
}
