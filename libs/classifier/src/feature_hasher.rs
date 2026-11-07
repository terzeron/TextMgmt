//! ES 문서의 `summary`·`title`·`author`를 사전 없는 해시 특징으로 바꾼다. 학습 CLI와 추론이 같은 코드를 쓴다.

use std::collections::BTreeMap;

use serde_json::Value;

/// `summary`에서 쓰는 앞부분 글자 수.
pub const SUMMARY_MAX_CHARS: usize = 1500;

/// 인덱스 오름차순, 중복 없음.
pub type SparseVec = Vec<(u32, f32)>;

/// 분류에 쓰는 ES 문서 필드.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookFields {
    pub summary: String,
    pub title: String,
    pub author: String,
}

impl BookFields {
    /// 없는 필드는 빈 문자열이다. `author`가 배열이면 쉼표로 잇는다.
    pub fn from_es_source(source: &Value) -> BookFields {
        let text = |key: &str| source.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let author = match source.get("author") {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            _ => text("author"),
        };
        BookFields { summary: text("summary"), title: text("title"), author }
    }

    /// `summary`와 `title`이 모두 공백뿐이면 본문 근거가 없다.
    pub fn is_empty(&self) -> bool {
        self.summary.trim().is_empty() && self.title.trim().is_empty()
    }
}

/// FNV-1a 64비트. 표준 `DefaultHasher`는 버전마다 바뀔 수 있어 모델 파일과 함께 쓸 수 없다.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn normalize(text: &str) -> String {
    text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn bucket(namespace: &str, token: &str, mask: u64) -> u32 {
    let mut bytes = Vec::with_capacity(namespace.len() + token.len());
    bytes.extend_from_slice(namespace.as_bytes());
    bytes.extend_from_slice(token.as_bytes());
    (fnv1a64(&bytes) & mask) as u32
}

fn char_ngram_counts(text: &str, namespace: &str, mask: u64) -> BTreeMap<u32, f32> {
    let chars: Vec<char> = text.chars().collect();
    let mut counts = BTreeMap::new();
    for n in [2usize, 3] {
        for gram in chars.windows(n) {
            let token: String = gram.iter().collect();
            *counts.entry(bucket(namespace, &token, mask)).or_insert(0.0) += 1.0;
        }
    }
    counts
}

fn author_counts(author: &str, mask: u64) -> BTreeMap<u32, f32> {
    let mut counts = BTreeMap::new();
    for name in author.split(',').map(normalize).filter(|n| !n.is_empty()) {
        *counts.entry(bucket("a:", &name, mask)).or_insert(0.0) += 1.0;
    }
    counts
}

/// `ln(1 + tf)` 뒤 L2 정규화한 값을 `out`에 더한다.
fn add_normalized(counts: BTreeMap<u32, f32>, out: &mut BTreeMap<u32, f32>) {
    let weighted: Vec<(u32, f32)> = counts.into_iter().map(|(i, tf)| (i, tf.ln_1p())).collect();
    let norm = weighted.iter().map(|(_, v)| v * v).sum::<f32>().sqrt();
    if norm == 0.0 {
        return;
    }
    for (i, v) in weighted {
        *out.entry(i).or_insert(0.0) += v / norm;
    }
}

/// 필드마다 namespace를 붙여 `2^hash_bits` 버킷으로 해시한다.
pub fn hash_features(fields: &BookFields, hash_bits: u32) -> SparseVec {
    let mask = (1u64 << hash_bits) - 1;
    let summary: String = normalize(&fields.summary).chars().take(SUMMARY_MAX_CHARS).collect();
    let mut merged = BTreeMap::new();
    add_normalized(char_ngram_counts(&summary, "s:", mask), &mut merged);
    add_normalized(char_ngram_counts(&normalize(&fields.title), "t:", mask), &mut merged);
    add_normalized(author_counts(&fields.author, mask), &mut merged);
    merged.into_iter().collect()
}
