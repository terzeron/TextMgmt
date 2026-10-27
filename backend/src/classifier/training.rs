//! Rust classifier 학습. 동작 기준은 Python `backend/classifier/training.py`다.
//!
//! 홀드아웃 분할, sparse TF-IDF 특징 행렬, One-vs-Rest squared-hinge 학습, 평가와
//! confidence calibration을 처리한다. Python sklearn과의 차이는 parity fixture로 확인한다.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};

use super::ValueError;
use super::corpus::IndexedJsonl;
use super::features::{Dtype, FeatureSpace, FitOptions, SparseMatrix};
use super::model::{CategoryModel, softmax_confidence};
use super::softmax::fixed;

// sklearn LinearSVC's default projected-gradient threshold. Golden fixtures guard
// decision-score parity when the solver stops at this tolerance.
const DUAL_MAX_PROJECTED_GRADIENT: f64 = 1e-4;
static HOLDOUT_SPILL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct SpilledHoldout {
    path: PathBuf,
    documents: usize,
    source_bytes: usize,
}

impl SpilledHoldout {
    fn store(docs: &mut [Value], indices: &[usize], fields: &Value, dir: &Path) -> Result<Self, ValueError> {
        fs::create_dir_all(dir).map_err(|error| ValueError(error.to_string()))?;
        let (path, file) = (0..100)
            .find_map(|_| {
                let sequence = HOLDOUT_SPILL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = dir.join(format!("classifier-holdout-{}-{sequence}.jsonl", std::process::id()));
                match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(file) => Some(Ok((path, file))),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .unwrap_or_else(|| Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "임시 holdout 파일 이름을 확보하지 못했다")))
            .map_err(|error| ValueError(error.to_string()))?;
        let sources: HashSet<String> = fields.as_object().into_iter().flat_map(|fields| fields.values()).filter_map(|spec| spec.get("source").and_then(Value::as_str).map(str::to_owned)).collect();
        let source_bytes =
            indices.iter().fold(0usize, |total, &index| total.saturating_add(sources.iter().filter_map(|source| docs[index].get(source).and_then(Value::as_str).map(str::len)).sum::<usize>()));
        let spilled = Self { path, documents: indices.len(), source_bytes };
        let mut writer = BufWriter::new(file);
        for &index in indices {
            serde_json::to_writer(&mut writer, &docs[index]).map_err(|error| ValueError(error.to_string()))?;
            writer.write_all(b"\n").map_err(|error| ValueError(error.to_string()))?;
        }
        writer.flush().map_err(|error| ValueError(error.to_string()))?;
        for &index in indices {
            if let Some(object) = docs[index].as_object_mut() {
                for source in &sources {
                    object.remove(source);
                }
            }
        }
        Ok(spilled)
    }

    fn load(&self) -> Result<Vec<Value>, ValueError> {
        super::corpus::read_jsonl(&self.path).map_err(|error| ValueError(error.to_string()))
    }
}

impl Drop for SpilledHoldout {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

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
    let (indices, dropped) = filter_indices_by_size(docs, min_per_category);
    (indices.into_iter().map(|i| docs[i].clone()).collect(), dropped)
}

fn filter_indices_by_size(docs: &[Value], min_per_category: usize) -> (Vec<usize>, Vec<String>) {
    let cat = |d: &Value| d["cat"].as_str().unwrap_or("").to_string();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for d in docs {
        *counts.entry(cat(d)).or_insert(0) += 1;
    }
    let floor = min_per_category.max(2);
    let mut dropped: Vec<String> = counts.iter().filter(|(_, n)| **n < floor).map(|(c, _)| c.clone()).collect();
    dropped.sort();
    let kept = docs.iter().enumerate().filter_map(|(i, d)| (counts[&cat(d)] >= floor).then_some(i)).collect();
    (kept, dropped)
}

/// 분할에 쓸 문서의 고유 키. ES 의 `_id`(inode)가 없으면 경로, 이름 순이다.
pub fn document_key(doc: &Value) -> String {
    ["id", "path", "name"].iter().filter_map(|k| doc.get(*k).and_then(Value::as_str)).find(|s| !s.is_empty()).unwrap_or("").to_string()
}

/// Python `round()` 는 반올림에서 짝수 쪽을 고른다.
pub(crate) fn round_half_even(x: f64) -> i64 {
    let floor = x.floor();
    if x - floor == 0.5 { if floor % 2.0 == 0.0 { floor as i64 } else { floor as i64 + 1 } } else { x.round() as i64 }
}

/// 문서 키의 해시로 홀드아웃을 뗀다. 카테고리별로 비율을 맞추고, 양쪽에 최소 1건씩 남긴다.
/// 키는 `blake2b(key=str(seed), digest_size=8)` 로 Python 과 같다. 반환은 (학습, 홀드아웃).
pub fn split_holdout(docs: &[Value], ratio: f64, seed: u64) -> (Vec<Value>, Vec<Value>) {
    let (train, hold) = split_holdout_indices(docs, &(0..docs.len()).collect::<Vec<_>>(), ratio, seed);
    (train.into_iter().map(|i| docs[i].clone()).collect(), hold.into_iter().map(|i| docs[i].clone()).collect())
}

/// 크기 필터 후 holdout에 속한 문서 키만 계산한다. 큰 corpus 문서 본문을 복제하지 않는다.
pub fn holdout_document_keys(docs: &[Value], min_per_category: usize, ratio: f64, seed: u64) -> Vec<String> {
    let (kept, _) = filter_indices_by_size(docs, min_per_category);
    let (_, holdout) = split_holdout_indices(docs, &kept, ratio, seed);
    holdout.into_iter().map(|index| document_key(&docs[index])).collect()
}

fn filter_indexed_by_size(docs: &IndexedJsonl, min_per_category: usize) -> (Vec<usize>, Vec<String>) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for index in 0..docs.len() {
        *counts.entry(docs.category(index).unwrap_or("").to_owned()).or_default() += 1;
    }
    let floor = min_per_category.max(2);
    let mut dropped: Vec<String> = counts.iter().filter(|(_, count)| **count < floor).map(|(category, _)| category.clone()).collect();
    dropped.sort();
    let kept = (0..docs.len()).filter(|&index| counts[docs.category(index).unwrap_or("")] >= floor).collect();
    (kept, dropped)
}

/// 인덱스 JSONL에서 holdout key만 산출한다. document body는 메모리에 올리지 않는다.
pub fn holdout_document_keys_indexed(docs: &IndexedJsonl, min_per_category: usize, ratio: f64, seed: u64) -> Vec<String> {
    holdout_indices_indexed(docs, min_per_category, ratio, seed).into_iter().map(|index| docs.document_key(index).unwrap_or("").to_owned()).collect()
}

/// 인덱스 JSONL에서 materialized 분할과 같은 holdout 문서 위치를 산출한다.
pub fn holdout_indices_indexed(docs: &IndexedJsonl, min_per_category: usize, ratio: f64, seed: u64) -> Vec<usize> {
    let (kept, _) = filter_indexed_by_size(docs, min_per_category);
    split_holdout_indices_indexed(docs, &kept, ratio, seed).1
}

fn split_holdout_indices(docs: &[Value], indices: &[usize], ratio: f64, seed: u64) -> (Vec<usize>, Vec<usize>) {
    split_holdout_indices_by(indices, ratio, seed, |index| (docs[index]["cat"].as_str().unwrap_or("").to_owned(), document_key(&docs[index])))
}

fn split_holdout_indices_indexed(docs: &IndexedJsonl, indices: &[usize], ratio: f64, seed: u64) -> (Vec<usize>, Vec<usize>) {
    split_holdout_indices_by(indices, ratio, seed, |index| (docs.category(index).unwrap_or("").to_owned(), docs.document_key(index).unwrap_or("").to_owned()))
}

fn split_holdout_indices_by<F>(indices: &[usize], ratio: f64, seed: u64, mut category_and_key: F) -> (Vec<usize>, Vec<usize>)
where
    F: FnMut(usize) -> (String, String),
{
    let key = seed.to_string();
    let mut order: Vec<String> = Vec::new();
    let mut buckets: HashMap<String, Vec<(Vec<u8>, usize)>> = HashMap::new();
    for &i in indices {
        let (cat, document_key) = category_and_key(i);
        let digest = blake2b_keyed(document_key.as_bytes(), key.as_bytes(), 8);
        if !buckets.contains_key(&cat) {
            order.push(cat.clone());
        }
        buckets.entry(cat).or_default().push((digest, i));
    }
    let (mut train, mut hold) = (Vec::new(), Vec::new());
    for cat in order {
        let mut rows = buckets.remove(&cat).unwrap_or_default();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let n = rows.len() as i64;
        let n_hold = if n > 1 { round_half_even(n as f64 * ratio).max(1).min(n - 1) as usize } else { 0 };
        hold.extend(rows[..n_hold].iter().map(|(_, i)| *i));
        train.extend(rows[n_hold..].iter().map(|(_, i)| *i));
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
    let refs: Vec<&Value> = docs.iter().collect();
    evaluate_refs(model, &refs, accept_parent)
}

/// 인덱스 corpus의 holdout 문서를 하나씩 읽어 평가하고 calibration 입력만 보존한다.
pub fn evaluate_indexed(model: &CategoryModel, docs: &mut IndexedJsonl, indices: &[usize], accept_parent: Option<&HashMap<String, String>>) -> Result<(Value, Value), ValueError> {
    let empty = HashMap::new();
    let parents = accept_parent.unwrap_or(&empty);
    let mut correct = Vec::with_capacity(indices.len());
    let mut confidence = Vec::with_capacity(indices.len());
    for &index in indices {
        let doc = docs.read_document(index).map_err(|error| ValueError(format!("holdout JSONL 문서를 읽지 못했다: {error}")))?;
        let scores = model.scores(&doc);
        let probabilities = softmax_confidence(std::slice::from_ref(&scores), model.temperature());
        let best = scores.iter().enumerate().fold(0, |best, (i, score)| if *score > scores[best] { i } else { best });
        let predicted = &model.classes[best];
        let truth = doc["cat"].as_str().unwrap_or("");
        confidence.push(probabilities[0][best]);
        correct.push(parent_of(parents, predicted) == parent_of(parents, truth));
    }
    let report = report_from_evaluation(&correct, &confidence);
    let calibration = super::calibration::fit_expected_accuracy(&confidence, &correct)?.unwrap_or(Value::Null);
    Ok((report, calibration))
}

fn evaluate_refs(model: &CategoryModel, docs: &[&Value], accept_parent: Option<&HashMap<String, String>>) -> Value {
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
    report_from_evaluation(&correct, &conf)
}

/// Holdout 문서에서 confidence→accuracy calibration을 학습한다. Python `fit_confidence_calibration` 대응.
pub fn fit_confidence_calibration(model: &CategoryModel, docs: &[Value], accept_parent: Option<&HashMap<String, String>>) -> Result<Option<Value>, ValueError> {
    let refs: Vec<&Value> = docs.iter().collect();
    fit_confidence_calibration_refs(model, &refs, accept_parent)
}

fn fit_confidence_calibration_refs(model: &CategoryModel, docs: &[&Value], accept_parent: Option<&HashMap<String, String>>) -> Result<Option<Value>, ValueError> {
    let empty = HashMap::new();
    let parents = accept_parent.unwrap_or(&empty);
    let scores: Vec<Vec<f64>> = docs.iter().map(|doc| model.scores(doc)).collect();
    let probabilities = softmax_confidence(&scores, model.temperature());
    let (confidences, correct): (Vec<f64>, Vec<bool>) = docs
        .iter()
        .zip(&scores)
        .zip(&probabilities)
        .map(|((doc, row), probability)| {
            // np.argmax: 최댓값이 여럿이면 첫 번째.
            let best = row.iter().enumerate().fold(0, |best, (i, score)| if *score > row[best] { i } else { best });
            let predicted = &model.classes[best];
            let truth = doc["cat"].as_str().unwrap_or("");
            (probability[best], parent_of(parents, predicted) == parent_of(parents, truth))
        })
        .unzip();
    super::calibration::fit_expected_accuracy(&confidences, &correct)
}

fn report_from_evaluation(correct: &[bool], confidence: &[f64]) -> Value {
    let top1 = if correct.is_empty() { 0.0 } else { correct.iter().filter(|c| **c).count() as f64 / correct.len() as f64 };
    json!({"n": correct.len(), "top1_accuracy": top1, "curve": coverage_curve(correct, confidence, &DEFAULT_CURVE_POINTS), "at_precision_90": max_coverage_at(correct, confidence, 0.90), "at_precision_95": max_coverage_at(correct, confidence, 0.95)})
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
    let mut lines = vec![
        format!("홀드아웃 {}건  전체 top-1 정답률 {}", thousands(report["n"].as_u64().unwrap_or(0)), percent(f(&report["top1_accuracy"]), 1, 0)),
        String::new(),
        "  판정률   정답률   임계값".to_string(),
    ];
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
    /// `spill_dir`에 holdout 문서를 저장하고 평가 전까지 메모리 원문을 비운다.
    pub release_holdout_text: bool,
}

/// `OneVsRestClassifier(LinearSVC(class_weight="balanced"))` 의 표본별 C 값.
/// sklearn 은 원래 다중 클래스 빈도가 아니라 각 이진 분류의 양성·음성 빈도를 사용한다.
pub fn one_vs_rest_sample_weights(labels: &[usize], class: usize, c: f64, balanced: bool) -> Vec<f64> {
    if !balanced || labels.is_empty() {
        return vec![c; labels.len()];
    }
    let positives = labels.iter().filter(|&&label| label == class).count();
    let negatives = labels.len() - positives;
    labels.iter().map(|&label| if label == class { c * labels.len() as f64 / (2 * positives) as f64 } else { c * labels.len() as f64 / (2 * negatives) as f64 }).collect()
}

/// 희소 TF-IDF 행렬에서 One-vs-Rest squared-hinge 선형 분류기를 학습한다.
pub fn train(docs: &mut [Value], config: &Value, opts: TrainOptions) -> Result<(CategoryModel, Value), ValueError> {
    let started = std::time::Instant::now();
    let min = config["corpus"]["min_per_category"].as_u64().unwrap_or(30) as usize;
    let (kept_indices, dropped) = filter_indices_by_size(docs, min);
    if kept_indices.is_empty() {
        return Err(ValueError("학습할 문서가 없다. min_per_category 를 낮추거나 collect 를 다시 하라".into()));
    }
    if !dropped.is_empty() {
        let shown: Vec<&str> = dropped.iter().take(10).map(String::as_str).collect();
        log::info!("표본 부족으로 제외한 카테고리 {}개: {}{}", dropped.len(), shown.join(", "), if dropped.len() > 10 { " ..." } else { "" });
    }
    let ratio = config["holdout"].as_f64().unwrap_or(0.2);
    let seed = config["seed"].as_u64().unwrap_or(20260914);
    let (train_indices, hold_indices) = split_holdout_indices(docs, &kept_indices, ratio, seed);
    log::info!("학습 분할 완료: 전체 {}건, 학습 {}건, 홀드아웃 {}건 {}", kept_indices.len(), train_indices.len(), hold_indices.len(), rss_note());
    let mut classes: Vec<String> = train_indices.iter().filter_map(|&i| docs[i]["cat"].as_str().map(str::to_owned)).collect();
    classes.sort();
    classes.dedup();
    if classes.len() < 2 {
        return Err(ValueError("학습에는 둘 이상의 카테고리가 필요하다".into()));
    }
    let fields = super::config::enabled_fields(config);
    if opts.release_holdout_text && opts.spill_dir.is_none() {
        return Err(ValueError("release_holdout_text requires spill_dir".into()));
    }
    let spilled_holdout = if opts.release_holdout_text { opts.spill_dir.as_deref().map(|dir| SpilledHoldout::store(docs, &hold_indices, &fields, dir)).transpose()? } else { None };
    if let Some(spilled) = &spilled_holdout {
        log::info!("holdout 원문 {}건, UTF-8 {} bytes 를 spill 했다 {}", spilled.documents, spilled.source_bytes, rss_note());
    }
    let mut space = FeatureSpace::new(&fields);
    let vectorize_started = std::time::Instant::now();
    let x = space.fit_transform_selected_with(docs, &train_indices, FitOptions { spill_dir: opts.spill_dir.clone(), dtype: Dtype::F64 })?.into_memory()?;
    let n_features = x.shape().1;
    log::info!("특징 {}개, 희소행렬 {} x {} / 비영요소 {}개, 벡터화 {:.0}초 {}", n_features, x.shape().0, x.shape().1, x.nnz(), vectorize_started.elapsed().as_secs_f64(), rss_note());
    let labels: Vec<usize> = train_indices.iter().map(|&i| classes.binary_search_by(|candidate| candidate.as_str().cmp(docs[i]["cat"].as_str().unwrap_or(""))).unwrap_or(0)).collect();
    let model_cfg = &config["model"];
    let c = model_cfg["C"].as_f64().unwrap_or(1.0).max(1e-12);
    let max_iter = model_cfg["max_iter"].as_u64().unwrap_or(1000).max(1) as usize;
    let requested_jobs = model_cfg["n_jobs"].as_i64().unwrap_or(2);
    if requested_jobs == 0 {
        return Err(ValueError("n_jobs must not be zero".into()));
    }
    let cpus = std::thread::available_parallelism().map_or(1, usize::from) as i64;
    let resolved_jobs = if requested_jobs < 0 { (cpus + 1 + requested_jobs).max(1) as usize } else { requested_jobs as usize };
    let n_jobs = resolved_jobs.min(classes.len()).max(1);
    let balanced = model_cfg["class_weight"].as_str() == Some("balanced");
    let mut coef = vec![0f32; n_features * classes.len()];
    let mut intercept = vec![0f64; classes.len()];
    if n_jobs == 1 {
        for class in 0..classes.len() {
            let weights = train_class_weights(&x, &labels, class, c, balanced, max_iter, n_features)?;
            store_class_weights(&mut coef, &mut intercept, classes.len(), n_features, class, &weights);
        }
    } else {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc::sync_channel;
        let next = AtomicUsize::new(0);
        let (sender, receiver) = sync_channel(n_jobs);
        let x = &x;
        let labels = &labels;
        let class_count = classes.len();
        std::thread::scope(|scope| -> Result<(), ValueError> {
            for _ in 0..n_jobs {
                let sender = sender.clone();
                let next = &next;
                scope.spawn(move || {
                    loop {
                        let class = next.fetch_add(1, Ordering::Relaxed);
                        if class >= class_count {
                            break;
                        }
                        let result = train_class_weights(x, labels, class, c, balanced, max_iter, n_features);
                        if sender.send((class, result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(sender);
            for _ in 0..class_count {
                let (class, result) = receiver.recv().map_err(|error| ValueError(format!("training worker channel closed: {error}")))?;
                let weights = result?;
                store_class_weights(&mut coef, &mut intercept, class_count, n_features, class, &weights);
            }
            Ok(())
        })?;
    }
    log::info!("SVM 학습 {:.0}초 {}", started.elapsed().as_secs_f64(), rss_note());
    // Spill 모드에서는 원문을 RAM 문서에서 지웠으므로 평가 직전에만 다시 읽는다.
    drop(x);
    let spilled_holdout_docs = spilled_holdout.as_ref().map(SpilledHoldout::load).transpose()?;
    let hold_docs: Vec<&Value> = match &spilled_holdout_docs {
        Some(spilled) => spilled.iter().collect(),
        None => hold_indices.iter().map(|&i| &docs[i]).collect(),
    };
    let blocks = space.describe();
    let meta = json!({"trained_at":Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),"config_hash":super::config::config_hash(config),"documents":kept_indices.len(),"categories":classes,"dropped_categories":dropped,"features":n_features,"blocks":blocks,"train_seconds":started.elapsed().as_secs_f64().round()});
    let mut model = CategoryModel::from_training(classes, config.clone(), meta, space.into_vectorizers(), coef, intercept).map_err(|e| ValueError(e.to_string()))?;
    let report = evaluate_refs(&model, &hold_docs, opts.accept_parent.as_ref());
    model.meta["holdout"] = report.clone();
    let target = config.pointer("/decision/target_accuracy").and_then(Value::as_f64).unwrap_or(0.90);
    let picked = if target >= 0.95 { &report["at_precision_95"] } else { &report["at_precision_90"] };
    model.meta["calibrated_threshold"] = picked["threshold"].clone();
    model.meta["calibrated_target"] = json!(target);
    model.meta["confidence_calibration"] = fit_confidence_calibration_refs(&model, &hold_docs, opts.accept_parent.as_ref())?.unwrap_or(Value::Null);
    Ok((model, report))
}

/// 인덱스 JSONL을 반복해서 읽어 학습한다. 전체 corpus 문서 본문은 RAM에 보유하지 않는다.
pub fn train_indexed(docs: &mut IndexedJsonl, config: &Value, opts: TrainOptions) -> Result<(CategoryModel, Value), ValueError> {
    let started = std::time::Instant::now();
    let min = config["corpus"]["min_per_category"].as_u64().unwrap_or(30) as usize;
    let (kept_indices, dropped) = filter_indexed_by_size(docs, min);
    if kept_indices.is_empty() {
        return Err(ValueError("학습할 문서가 없다. min_per_category 를 낮추거나 collect 를 다시 하라".into()));
    }
    if !dropped.is_empty() {
        let shown: Vec<&str> = dropped.iter().take(10).map(String::as_str).collect();
        log::info!("표본 부족으로 제외한 카테고리 {}개: {}{}", dropped.len(), shown.join(", "), if dropped.len() > 10 { " ..." } else { "" });
    }
    let ratio = config["holdout"].as_f64().unwrap_or(0.2);
    let seed = config["seed"].as_u64().unwrap_or(20260914);
    let (train_indices, hold_indices) = split_holdout_indices_indexed(docs, &kept_indices, ratio, seed);
    log::info!("학습 분할 완료: 전체 {}건, 학습 {}건, 홀드아웃 {}건 {}", kept_indices.len(), train_indices.len(), hold_indices.len(), rss_note());
    let mut classes: Vec<String> = train_indices.iter().filter_map(|&index| docs.category(index).map(str::to_owned)).collect();
    classes.sort();
    classes.dedup();
    if classes.len() < 2 {
        return Err(ValueError("학습에는 둘 이상의 카테고리가 필요하다".into()));
    }
    let labels: Vec<usize> = train_indices.iter().map(|&index| classes.binary_search_by(|candidate| candidate.as_str().cmp(docs.category(index).unwrap_or(""))).unwrap_or(0)).collect();
    let fields = super::config::enabled_fields(config);
    let mut space = FeatureSpace::new(&fields);
    let vectorize_started = std::time::Instant::now();
    let x = space
        .fit_transform_selected_by(&train_indices, FitOptions { spill_dir: opts.spill_dir.clone(), dtype: Dtype::F64 }, |index| {
            docs.read_document(index).map_err(|error| ValueError(format!("학습 JSONL 문서를 읽지 못했다: {error}")))
        })?
        .into_memory()?;
    let n_features = x.shape().1;
    log::info!("특징 {}개, 희소행렬 {} x {} / 비영요소 {}개, 벡터화 {:.0}초 {}", n_features, x.shape().0, x.shape().1, x.nnz(), vectorize_started.elapsed().as_secs_f64(), rss_note());
    let model_cfg = &config["model"];
    let c = model_cfg["C"].as_f64().unwrap_or(1.0).max(1e-12);
    let max_iter = model_cfg["max_iter"].as_u64().unwrap_or(1000).max(1) as usize;
    let requested_jobs = model_cfg["n_jobs"].as_i64().unwrap_or(2);
    if requested_jobs == 0 {
        return Err(ValueError("n_jobs must not be zero".into()));
    }
    let cpus = std::thread::available_parallelism().map_or(1, usize::from) as i64;
    let resolved_jobs = if requested_jobs < 0 { (cpus + 1 + requested_jobs).max(1) as usize } else { requested_jobs as usize };
    let n_jobs = resolved_jobs.min(classes.len()).max(1);
    let balanced = model_cfg["class_weight"].as_str() == Some("balanced");
    let mut coef = vec![0f32; n_features * classes.len()];
    let mut intercept = vec![0f64; classes.len()];
    if n_jobs == 1 {
        for class in 0..classes.len() {
            let weights = train_class_weights(&x, &labels, class, c, balanced, max_iter, n_features)?;
            store_class_weights(&mut coef, &mut intercept, classes.len(), n_features, class, &weights);
        }
    } else {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc::sync_channel;
        let next = AtomicUsize::new(0);
        let (sender, receiver) = sync_channel(n_jobs);
        let matrix = &x;
        let labels = &labels;
        let class_count = classes.len();
        std::thread::scope(|scope| -> Result<(), ValueError> {
            for _ in 0..n_jobs {
                let sender = sender.clone();
                let next = &next;
                scope.spawn(move || {
                    loop {
                        let class = next.fetch_add(1, Ordering::Relaxed);
                        if class >= class_count {
                            break;
                        }
                        let result = train_class_weights(matrix, labels, class, c, balanced, max_iter, n_features);
                        if sender.send((class, result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(sender);
            for _ in 0..class_count {
                let (class, result) = receiver.recv().map_err(|error| ValueError(format!("training worker channel closed: {error}")))?;
                let weights = result?;
                store_class_weights(&mut coef, &mut intercept, class_count, n_features, class, &weights);
            }
            Ok(())
        })?;
    }
    log::info!("SVM 학습 {:.0}초 {}", started.elapsed().as_secs_f64(), rss_note());
    drop(x);
    let blocks = space.describe();
    let meta = json!({"trained_at":Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),"config_hash":super::config::config_hash(config),"documents":kept_indices.len(),"categories":classes,"dropped_categories":dropped,"features":n_features,"blocks":blocks,"train_seconds":started.elapsed().as_secs_f64().round()});
    let mut model = CategoryModel::from_training(classes, config.clone(), meta, space.into_vectorizers(), coef, intercept).map_err(|error| ValueError(error.to_string()))?;
    let (report, calibration) = evaluate_indexed(&model, docs, &hold_indices, opts.accept_parent.as_ref())?;
    model.meta["holdout"] = report.clone();
    let target = config.pointer("/decision/target_accuracy").and_then(Value::as_f64).unwrap_or(0.90);
    let picked = if target >= 0.95 { &report["at_precision_95"] } else { &report["at_precision_90"] };
    model.meta["calibrated_threshold"] = picked["threshold"].clone();
    model.meta["calibrated_target"] = json!(target);
    model.meta["confidence_calibration"] = calibration;
    Ok((model, report))
}

fn store_class_weights(coef: &mut [f32], intercept: &mut [f64], class_count: usize, n_features: usize, class: usize, weights: &[f64]) {
    for (feature, value) in weights[..n_features].iter().enumerate() {
        coef[feature * class_count + class] = *value as f32;
    }
    intercept[class] = weights[n_features];
}

fn next_solver_random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn shuffle_indices(indices: &mut [usize], state: &mut u64) {
    for end in (1..indices.len()).rev() {
        let index = (next_solver_random(state) % (end as u64 + 1)) as usize;
        indices.swap(end, index);
    }
}

fn shuffled_indices(len: usize, seed: u64) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..len).collect();
    let mut state = seed;
    shuffle_indices(&mut indices, &mut state);
    indices
}

fn train_class_weights(x: &SparseMatrix, labels: &[usize], class: usize, c: f64, balanced: bool, max_iter: usize, n_features: usize) -> Result<Vec<f64>, ValueError> {
    let weights = one_vs_rest_sample_weights(labels, class, c, balanced);
    let rows = x.memory_rows()?;
    let mut alpha = vec![0.0; labels.len()];
    let mut w = vec![0.0; n_features + 1];
    let qd: Vec<f64> = rows.iter().zip(&weights).map(|(row, ci)| row.iter().map(|(_, v)| v * v).sum::<f64>() + 1.0 + 1.0 / (2.0 * ci)).collect();
    let mut order = shuffled_indices(labels.len(), (class as u64).wrapping_add(1));
    let mut active_size = order.len();
    let mut rng_state = (class as u64).wrapping_add(1);
    let mut pgmax_old = f64::INFINITY;
    for _ in 0..max_iter {
        shuffle_indices(&mut order[..active_size], &mut rng_state);
        let mut pgmax_new = f64::NEG_INFINITY;
        let mut pgmin_new = f64::INFINITY;
        let mut position = 0;
        while position < active_size {
            let i = order[position];
            let row = rows.get(i).ok_or_else(|| ValueError("sparse matrix row 누락".into()))?;
            let y = if labels[i] == class { 1.0 } else { -1.0 };
            let dot = row.iter().map(|(col, v)| w[*col as usize] * v).sum::<f64>() + w[n_features];
            let g = y * dot - 1.0 + alpha[i] / (2.0 * weights[i]);
            let pg = if alpha[i] == 0.0 { g.min(0.0) } else { g };
            if alpha[i] == 0.0 && g > pgmax_old {
                order.swap(position, active_size - 1);
                active_size -= 1;
                continue;
            }
            pgmax_new = pgmax_new.max(pg);
            pgmin_new = pgmin_new.min(pg);
            let next = (alpha[i] - g / qd[i]).max(0.0);
            let delta = (next - alpha[i]) * y;
            if delta != 0.0 {
                for &(col, v) in row.iter() {
                    w[col as usize] += delta * v;
                }
                w[n_features] += delta;
                alpha[i] = next;
            }
            position += 1;
        }
        if active_size == 0 {
            break;
        }
        if pgmax_new - pgmin_new <= DUAL_MAX_PROJECTED_GRADIENT && pgmax_new.abs() <= DUAL_MAX_PROJECTED_GRADIENT && pgmin_new.abs() <= DUAL_MAX_PROJECTED_GRADIENT {
            if active_size == labels.len() {
                break;
            }
            active_size = labels.len();
            pgmax_old = f64::INFINITY;
            continue;
        }
        pgmax_old = if pgmax_new <= 0.0 { f64::INFINITY } else { pgmax_new };
    }
    Ok(w)
}

#[cfg(test)]
mod solver_tests {
    use super::shuffled_indices;

    #[test]
    fn solver_order_is_a_repeatable_permutation_per_class() {
        let first = shuffled_indices(32, 7);
        let second = shuffled_indices(32, 7);
        let other_class = shuffled_indices(32, 8);
        let mut sorted = first.clone();
        sorted.sort_unstable();

        assert_eq!(first, second);
        assert_ne!(first, other_class);
        assert_ne!(first, (0..32).collect::<Vec<_>>());
        assert_eq!(sorted, (0..32).collect::<Vec<_>>());
    }
}
