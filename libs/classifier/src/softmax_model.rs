//! 다중 클래스 softmax 선형 모델. 해시 특징을 받아 카테고리별 확률을 낸다.
//!
//! 파일 형식: 매직 8바이트 + 헤더 JSON 길이(u32 LE) + 헤더 JSON + bias(f32 LE × K) + weights(f32 LE × K × 2^hash_bits, 카테고리 major).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::feature_hasher::{BookFields, SparseVec, hash_features};
use crate::{Classify, Prediction};

const MAGIC: &[u8; 8] = b"TMCLS\0\x01\0";

/// 모델 메타데이터. 카테고리별 실제 권수와 학습 권수는 사전확률 보정에 쓴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHeader {
    pub version: u32,
    pub seed: u64,
    pub created_unix_secs: u64,
    pub hash_bits: u32,
    pub categories: Vec<String>,
    pub population_counts: Vec<u64>,
    pub sample_counts: Vec<u64>,
}

impl ModelHeader {
    fn to_json(&self) -> Value {
        json!({
            "version": self.version, "seed": self.seed, "created_unix_secs": self.created_unix_secs, "hash_bits": self.hash_bits,
            "categories": self.categories, "population_counts": self.population_counts, "sample_counts": self.sample_counts,
        })
    }

    fn from_json(v: &Value) -> Result<ModelHeader, String> {
        let num = |key: &str| v.get(key).and_then(Value::as_u64).ok_or_else(|| format!("헤더에 {key}가 없다"));
        let nums = |key: &str| -> Result<Vec<u64>, String> {
            v.get(key).and_then(Value::as_array).ok_or_else(|| format!("헤더에 {key}가 없다"))?.iter().map(|x| x.as_u64().ok_or_else(|| format!("{key} 값이 정수가 아니다"))).collect()
        };
        let categories = v
            .get("categories")
            .and_then(Value::as_array)
            .ok_or("헤더에 categories가 없다")?
            .iter()
            .map(|x| x.as_str().map(str::to_string).ok_or_else(|| "categories 값이 문자열이 아니다".to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let header = ModelHeader {
            version: num("version")? as u32,
            seed: num("seed")?,
            created_unix_secs: num("created_unix_secs")?,
            hash_bits: num("hash_bits")? as u32,
            categories,
            population_counts: nums("population_counts")?,
            sample_counts: nums("sample_counts")?,
        };
        let k = header.categories.len();
        if header.population_counts.len() != k || header.sample_counts.len() != k {
            return Err("카테고리 수와 권수 배열 길이가 다르다".into());
        }
        if header.hash_bits == 0 || header.hash_bits > 24 {
            return Err(format!("hash_bits가 범위 밖이다: {}", header.hash_bits));
        }
        Ok(header)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoftmaxModel {
    pub header: ModelHeader,
    /// `categories.len() × 2^hash_bits`, 카테고리 major.
    pub weights: Vec<f32>,
    pub bias: Vec<f32>,
}

impl SoftmaxModel {
    fn dim(&self) -> usize {
        1usize << self.header.hash_bits
    }

    /// 보정 없는 원점수.
    pub fn logits(&self, x: &SparseVec) -> Vec<f32> {
        let dim = self.dim();
        self.bias
            .iter()
            .enumerate()
            .map(|(k, b)| {
                let row = &self.weights[k * dim..(k + 1) * dim];
                b + x.iter().map(|(i, v)| row[*i as usize] * v).sum::<f32>()
            })
            .collect()
    }

    /// `ln(실제 권수 / 학습 권수)`. 학습 권수가 0이면 그 카테고리는 고를 수 없다.
    pub fn prior_offsets(&self) -> Vec<f32> {
        let h = &self.header;
        h.population_counts.iter().zip(&h.sample_counts).map(|(p, s)| if *s == 0 { f32::NEG_INFINITY } else { (*p as f32 / *s as f32).ln() }).collect()
    }

    /// softmax 확률 내림차순.
    pub fn predict_proba(&self, x: &SparseVec, prior_corrected: bool) -> Vec<(String, f64)> {
        let mut z: Vec<f64> = self.logits(x).into_iter().map(f64::from).collect();
        if prior_corrected {
            for (zi, offset) in z.iter_mut().zip(self.prior_offsets()) {
                *zi += f64::from(offset);
            }
        }
        let max = z.iter().copied().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
        let exp: Vec<f64> = z.iter().map(|v| if v.is_finite() { (v - max).exp() } else { 0.0 }).collect();
        let total: f64 = exp.iter().sum();
        let mut ranked: Vec<(String, f64)> = self.header.categories.iter().cloned().zip(exp.into_iter().map(|e| if total > 0.0 { e / total } else { 0.0 })).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked
    }

    /// 같은 디렉토리의 임시 파일에 다 쓴 뒤 rename한다. 읽는 쪽이 반쯤 쓰인 파일을 보지 않는다.
    pub fn save_atomic(&self, path: &Path) -> std::io::Result<()> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "model.bin".into());
        let tmp = path.with_file_name(format!(".{name}.tmp"));
        let header = serde_json::to_vec(&self.header.to_json()).map_err(std::io::Error::other)?;
        {
            let file = File::create(&tmp)?;
            let mut out = BufWriter::new(&file);
            out.write_all(MAGIC)?;
            out.write_all(&(header.len() as u32).to_le_bytes())?;
            out.write_all(&header)?;
            for v in self.bias.iter().chain(&self.weights) {
                out.write_all(&v.to_le_bytes())?;
            }
            out.flush()?;
            drop(out);
            file.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }

    pub fn load(path: &Path) -> Result<SoftmaxModel, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("모델 파일을 읽지 못했다: {e}"))?;
        if bytes.len() < 12 || &bytes[..8] != MAGIC {
            return Err("모델 파일 형식이 아니다".into());
        }
        let header_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap_or_default()) as usize;
        let header_end = 12usize.checked_add(header_len).filter(|end| *end <= bytes.len()).ok_or("헤더 길이가 파일보다 길다")?;
        let header_json: Value = serde_json::from_slice(&bytes[12..header_end]).map_err(|e| format!("헤더 JSON이 깨졌다: {e}"))?;
        let header = ModelHeader::from_json(&header_json)?;
        let k = header.categories.len();
        let count = k + (k << header.hash_bits);
        let body = &bytes[header_end..];
        if body.len() != count * 4 {
            return Err(format!("가중치 크기가 다르다: 기대 {} 바이트, 실제 {} 바이트", count * 4, body.len()));
        }
        let floats: Vec<f32> = body.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let (bias, weights) = floats.split_at(k);
        Ok(SoftmaxModel { header, weights: weights.to_vec(), bias: bias.to_vec() })
    }
}

impl Classify for SoftmaxModel {
    fn classify_document(&self, doc: &Value) -> Result<Prediction, String> {
        let x = hash_features(&BookFields::from_es_source(doc), self.header.hash_bits);
        Ok(Prediction { ranked: self.predict_proba(&x, true) })
    }
}
