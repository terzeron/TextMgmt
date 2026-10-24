//! Python `backend/classifier/export.py` 가 쓴 모델 디렉토리의 로더와 검증.
//!
//! 디렉토리 안의 파일은 신뢰할 수 없는 입력처럼 다룬다. 크기를 먼저 보고 할당하며, 길이 곱셈은
//! 넘침을 검사한다. sha256, 길이, 어휘·열 수 합계, 어휘 중복, NaN·Inf 를 모두 오류로 처리한다.
//! pickle(joblib)은 읽지 않는다.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::tokenize::{AnalyzerKind, AnalyzerSpec};
use super::vectorize::FieldVectorizer;

/// 지원하는 `format_version`.
pub const SUPPORTED_FORMAT_VERSION: u64 = 1;
const WORD_TOKEN_PATTERN: &str = r"(?u)\b\w+\b";
/// `model.json` 최대 크기. 홀드아웃 곡선까지 담아도 수 MB 다.
const MAX_MANIFEST_BYTES: u64 = 128 << 20;
/// 어휘 파일 하나의 최대 크기.
const MAX_VOCAB_BYTES: u64 = 512 << 20;
const MAX_NGRAM: usize = 32;
const COEF_CHUNK_BYTES: usize = 4 << 20;

/// 모델 로드 오류. 문서 본문 같은 사용자 데이터는 담지 않는다.
#[derive(Debug)]
pub enum ModelError {
    /// 파일을 열거나 읽지 못했다(없는 파일 포함).
    Io { file: String, source: std::io::Error },
    /// JSON 으로 해석하지 못했다.
    Json { file: String, message: String },
    UnsupportedVersion(u64),
    /// 파일 이름이 디렉토리 밖을 가리킬 수 있다.
    UnsafePath(String),
    LengthMismatch { file: String, expected: u64, actual: u64 },
    Sha256Mismatch { file: String },
    NotFinite { file: String },
    DuplicateTerm { field: String },
    /// 그 밖의 구조 불일치(열 수 합계, 필드 옵션 등).
    Invalid(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { file, source } => write!(f, "모델 파일을 읽지 못했다 ({file}): {source}"),
            Self::Json { file, message } => write!(f, "모델 JSON 을 해석하지 못했다 ({file}): {message}"),
            Self::UnsupportedVersion(v) => write!(f, "지원하지 않는 format_version {v} (지원: {SUPPORTED_FORMAT_VERSION})"),
            Self::UnsafePath(name) => write!(f, "안전하지 않은 파일 이름: {name:?}"),
            Self::LengthMismatch { file, expected, actual } => write!(f, "파일 길이가 맞지 않다 ({file}): 기대 {expected}바이트, 실제 {actual}바이트"),
            Self::Sha256Mismatch { file } => write!(f, "sha256 이 맞지 않다 ({file})"),
            Self::NotFinite { file } => write!(f, "NaN 또는 Inf 값이 있다 ({file})"),
            Self::DuplicateTerm { field } => write!(f, "필드 '{field}' 의 어휘에 중복 항목이 있다"),
            Self::Invalid(message) => write!(f, "모델 형식이 맞지 않다: {message}"),
        }
    }
}

impl std::error::Error for ModelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ModelError> {
    Err(ModelError::Invalid(message.into()))
}

/// `model.json` 의 필드 항목.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldManifest {
    pub name: String,
    pub source: String,
    pub weight: f64,
    pub max_chars: u64,
    pub analyzer: String,
    pub ngram_range: [u64; 2],
    pub lowercase: bool,
    pub sublinear_tf: bool,
    pub norm: String,
    pub token_pattern: String,
    pub offset: u64,
    pub size: u64,
    pub vocab_file: String,
    pub idf_file: String,
    pub idf_sha256: String,
    pub vocab_sha256: String,
}

/// `model.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub format_version: u64,
    pub classes: Vec<String>,
    pub config: Value,
    #[serde(default)]
    pub meta: Value,
    pub n_features: u64,
    pub n_classes: u64,
    pub fields: Vec<FieldManifest>,
    pub coef_file: String,
    pub coef_sha256: String,
    pub intercept: Vec<f64>,
}

/// 로더가 돌려주는 부품. `CategoryModel` 이 이것으로 조립된다.
#[derive(Debug)]
pub struct LoadedModel {
    pub manifest: Manifest,
    pub fields: Vec<FieldVectorizer>,
    /// 전치 레이아웃 `[n_features, n_classes]` 행 우선.
    pub coef: Vec<f32>,
}

fn io_err(file: &str, source: std::io::Error) -> ModelError {
    ModelError::Io { file: file.to_string(), source }
}

/// 디렉토리 안의 이름 하나만 허용한다. `/`, `\`, `..`, 절대 경로, 숨김 파일, NUL 은 거부한다.
fn check_file_name(name: &str) -> Result<(), ModelError> {
    let mut comps = Path::new(name).components();
    let single_normal = matches!((comps.next(), comps.next()), (Some(Component::Normal(_)), None));
    if name.is_empty() || name.len() > 255 || !single_normal || name.contains(['/', '\\', '\0']) || name.starts_with('.') {
        return Err(ModelError::UnsafePath(name.chars().take(64).collect()));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 바이트열의 sha256 을 소문자 16진수로 돌려준다.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn check_sha(file: &str, expected: &str, actual: &str) -> Result<(), ModelError> {
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) || !expected.eq_ignore_ascii_case(actual) {
        return Err(ModelError::Sha256Mismatch { file: file.to_string() });
    }
    Ok(())
}

fn open_regular(dir: &Path, name: &str) -> Result<(File, u64), ModelError> {
    check_file_name(name)?;
    let file = File::open(dir.join(name)).map_err(|e| io_err(name, e))?;
    let meta = file.metadata().map_err(|e| io_err(name, e))?;
    if !meta.is_file() {
        return invalid(format!("{name} 은 일반 파일이 아니다"));
    }
    Ok((file, meta.len()))
}

/// 파일 전체를 읽는다. 길이가 `expected_len` 과 다르거나 `max_len` 을 넘으면 읽기 전에 거부한다.
fn read_bounded(dir: &Path, name: &str, expected_len: Option<u64>, max_len: u64) -> Result<Vec<u8>, ModelError> {
    let (file, len) = open_regular(dir, name)?;
    if let Some(expected) = expected_len
        && len != expected
    {
        return Err(ModelError::LengthMismatch { file: name.to_string(), expected, actual: len });
    }
    if len > max_len {
        return Err(ModelError::LengthMismatch { file: name.to_string(), expected: max_len, actual: len });
    }
    let mut buf = Vec::with_capacity(len as usize);
    // 열린 뒤에 파일이 자라도 상한을 넘겨 읽지 않는다.
    file.take(max_len + 1).read_to_end(&mut buf).map_err(|e| io_err(name, e))?;
    if buf.len() as u64 != len {
        return Err(ModelError::LengthMismatch { file: name.to_string(), expected: len, actual: buf.len() as u64 });
    }
    Ok(buf)
}

fn le_f32s(file: &str, bytes: &[u8]) -> Result<Vec<f32>, ModelError> {
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let v = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !v.is_finite() {
            return Err(ModelError::NotFinite { file: file.to_string() });
        }
        out.push(v);
    }
    Ok(out)
}

/// 계수 파일을 4MB 씩 읽어 sha256 을 갱신하며 `f32` 로 옮긴다. 바이트 사본을 따로 두지 않는다.
fn read_coef(dir: &Path, name: &str, expected_sha: &str, n_values: u64) -> Result<Vec<f32>, ModelError> {
    let bytes = n_values.checked_mul(4).ok_or_else(|| ModelError::Invalid("계수 파일 크기가 u64 를 넘는다".into()))?;
    let (mut file, len) = open_regular(dir, name)?;
    if len != bytes {
        return Err(ModelError::LengthMismatch { file: name.to_string(), expected: bytes, actual: len });
    }
    let n = usize::try_from(n_values).map_err(|_| ModelError::Invalid("계수 개수가 usize 를 넘는다".into()))?;
    n.checked_mul(4).filter(|b| *b <= isize::MAX as usize).ok_or_else(|| ModelError::Invalid("계수 파일이 너무 크다".into()))?;
    let mut coef = vec![0f32; n];
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; COEF_CHUNK_BYTES];
    let mut at = 0usize;
    while at < n {
        let take = (n - at).min(COEF_CHUNK_BYTES / 4);
        let chunk = &mut buf[..take * 4];
        file.read_exact(chunk).map_err(|e| io_err(name, e))?;
        hasher.update(&*chunk);
        for (dst, src) in coef[at..at + take].iter_mut().zip(chunk.chunks_exact(4)) {
            *dst = f32::from_le_bytes([src[0], src[1], src[2], src[3]]);
        }
        at += take;
    }
    // 읽는 동안 파일이 자랐다면 거부한다.
    let mut probe = [0u8; 1];
    if file.read(&mut probe).map_err(|e| io_err(name, e))? != 0 {
        return Err(ModelError::LengthMismatch { file: name.to_string(), expected: bytes, actual: bytes + 1 });
    }
    check_sha(name, expected_sha, &hex(&hasher.finalize()))?;
    if coef.iter().any(|v| !v.is_finite()) {
        return Err(ModelError::NotFinite { file: name.to_string() });
    }
    Ok(coef)
}

fn analyzer_spec(field: &FieldManifest) -> Result<AnalyzerSpec, ModelError> {
    let name = &field.name;
    let Some(kind) = AnalyzerKind::parse(&field.analyzer) else {
        return invalid(format!("필드 '{name}': 지원하지 않는 analyzer"));
    };
    let (lo, hi) = (field.ngram_range[0] as usize, field.ngram_range[1] as usize);
    if lo < 1 || lo > hi || hi > MAX_NGRAM || field.ngram_range[1] > MAX_NGRAM as u64 {
        return invalid(format!("필드 '{name}': ngram_range 가 1 <= lo <= hi <= {MAX_NGRAM} 이 아니다"));
    }
    if field.norm != "l2" {
        return invalid(format!("필드 '{name}': norm 은 l2 만 지원한다"));
    }
    if kind == AnalyzerKind::Word && field.token_pattern != WORD_TOKEN_PATTERN {
        return invalid(format!("필드 '{name}': word 분석기는 token_pattern {WORD_TOKEN_PATTERN} 만 지원한다"));
    }
    Ok(AnalyzerSpec { kind, ngram_lo: lo, ngram_hi: hi, lowercase: field.lowercase })
}

fn validate_manifest(m: &Manifest) -> Result<(), ModelError> {
    if m.classes.is_empty() || m.classes.len() as u64 != m.n_classes {
        return invalid("classes 개수와 n_classes 가 맞지 않다");
    }
    if m.intercept.len() != m.classes.len() || m.intercept.iter().any(|v| !v.is_finite()) {
        return invalid("intercept 길이가 클래스 수와 다르거나 유한하지 않다");
    }
    if m.fields.is_empty() {
        return invalid("fields 가 비어 있다");
    }
    let mut expected_offset = 0u64;
    for f in &m.fields {
        if f.offset != expected_offset {
            return invalid(format!("필드 '{}': offset 이 앞 필드들의 크기 합과 다르다", f.name));
        }
        expected_offset = expected_offset.checked_add(f.size).ok_or_else(|| ModelError::Invalid("열 수 합계가 넘친다".into()))?;
        if !f.weight.is_finite() {
            return invalid(format!("필드 '{}': weight 가 유한하지 않다", f.name));
        }
    }
    if expected_offset != m.n_features {
        return invalid(format!("필드 크기의 합 {expected_offset} 이 n_features {} 와 다르다", m.n_features));
    }
    if m.n_features > u32::MAX as u64 {
        return invalid("n_features 가 u32 범위를 넘는다");
    }
    Ok(())
}

/// 모델 디렉토리를 읽고 모든 무결성 검사를 거친다.
pub fn load_dir(dir: &Path) -> Result<LoadedModel, ModelError> {
    let raw = read_bounded(dir, "model.json", None, MAX_MANIFEST_BYTES)?;
    let value: Value = serde_json::from_slice(&raw).map_err(|e| ModelError::Json { file: "model.json".into(), message: e.to_string() })?;
    let version = value.get("format_version").and_then(Value::as_u64).ok_or_else(|| ModelError::Invalid("format_version 이 없다".into()))?;
    if version != SUPPORTED_FORMAT_VERSION {
        return Err(ModelError::UnsupportedVersion(version));
    }
    let manifest: Manifest = serde_json::from_value(value).map_err(|e| ModelError::Json { file: "model.json".into(), message: e.to_string() })?;
    validate_manifest(&manifest)?;

    let mut fields = Vec::with_capacity(manifest.fields.len());
    for f in &manifest.fields {
        let spec = analyzer_spec(f)?;
        let idf_bytes = f.size.checked_mul(4).ok_or_else(|| ModelError::Invalid("idf 크기가 넘친다".into()))?;
        let idf_raw = read_bounded(dir, &f.idf_file, Some(idf_bytes), idf_bytes)?;
        check_sha(&f.idf_file, &f.idf_sha256, &sha256_hex(&idf_raw))?;
        let idf = le_f32s(&f.idf_file, &idf_raw)?;
        let vocab_raw = read_bounded(dir, &f.vocab_file, None, MAX_VOCAB_BYTES)?;
        check_sha(&f.vocab_file, &f.vocab_sha256, &sha256_hex(&vocab_raw))?;
        let vocab: Vec<String> = serde_json::from_slice(&vocab_raw).map_err(|e| ModelError::Json { file: f.vocab_file.clone(), message: e.to_string() })?;
        drop(vocab_raw);
        if vocab.len() as u64 != f.size {
            return invalid(format!("필드 '{}': 어휘 {}개가 size {} 와 다르다", f.name, vocab.len(), f.size));
        }
        let max_chars = usize::try_from(f.max_chars).map_err(|_| ModelError::Invalid(format!("필드 '{}': max_chars 가 너무 크다", f.name)))?;
        fields.push(FieldVectorizer::new(&f.name, &f.source, f.weight as f32, max_chars, spec, f.sublinear_tf, f.offset as u32, vocab, idf)?);
    }

    let n_values = manifest.n_features.checked_mul(manifest.n_classes).ok_or_else(|| ModelError::Invalid("계수 개수가 넘친다".into()))?;
    let coef = read_coef(dir, &manifest.coef_file, &manifest.coef_sha256, n_values)?;
    Ok(LoadedModel { manifest, fields, coef })
}
