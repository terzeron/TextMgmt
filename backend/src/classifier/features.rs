//! 필드별 특징 추출. Python `backend/classifier/features.py` 대응.
//!
//! 학습은 Python 에 남지만, 특징 행렬을 만드는 이 부분은 결정적 코드라 같은 규칙으로 옮겼다. 행렬 값은
//! float64 로 계산하고 `Dtype::F32` 를 주면 float32 로 둥글린다(sklearn 의 float32 행렬과 같은 값).
//! 분석기는 `tokenize` 를 쓴다.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::ValueError;
use super::tokenize::{AnalyzerKind, AnalyzerSpec, for_each_term};
use super::vectorize::{FieldVectorizer, field_text as source_text};

/// 행렬 원소 자료형.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dtype {
    F32,
    #[default]
    F64,
}

/// `fit_transform_with` 옵션. `spill_dir` 을 주면 블록을 그 디렉토리로 흘려보냈다가 다시 합친다.
#[derive(Debug, Clone, Default)]
pub struct FitOptions {
    pub spill_dir: Option<PathBuf>,
    pub dtype: Dtype,
}

/// 희소 행렬(행마다 `(열, 값)`, 열 오름차순).
#[derive(Debug, Clone, PartialEq)]
pub struct SparseMatrix {
    rows: Vec<Vec<(u32, f64)>>,
    n_cols: usize,
    dtype: Dtype,
}

impl SparseMatrix {
    pub fn shape(&self) -> (usize, usize) {
        (self.rows.len(), self.n_cols)
    }

    pub fn nnz(&self) -> usize {
        self.rows.iter().map(Vec::len).sum()
    }

    pub fn dtype(&self) -> Dtype {
        self.dtype
    }

    pub fn rows(&self) -> &[Vec<(u32, f64)>] {
        &self.rows
    }

    pub fn to_dense(&self) -> Vec<Vec<f64>> {
        self.rows
            .iter()
            .map(|row| {
                let mut dense = vec![0.0; self.n_cols];
                for &(c, v) in row {
                    dense[c as usize] = v;
                }
                dense
            })
            .collect()
    }
}

fn spec_str<'a>(spec: &'a Value, key: &str) -> Option<&'a str> {
    spec.get(key).and_then(Value::as_str)
}

fn spec_usize(spec: &Value, key: &str) -> usize {
    spec.get(key).and_then(Value::as_u64).unwrap_or(0) as usize
}

/// 레코드 하나에서 한 필드의 원문을 꺼낸다. `spec` 은 `{"source", "max_chars"}` 를 가진다.
pub fn field_text(doc: &Value, spec: &Value) -> String {
    source_text(doc, spec_str(spec, "source").unwrap_or(""), spec_usize(spec, "max_chars"))
}

/// 어휘를 배우고 변환하는 벡터라이저(`TfidfVectorizer`). word 분석기는 한 글자 토큰도 살린다.
#[derive(Debug)]
pub struct Vectorizer {
    spec: AnalyzerSpec,
    min_df: usize,
    max_features: Option<usize>,
    vocabulary: HashMap<String, usize>,
    idf: Vec<f32>,
}

/// 설정의 필드 하나로 벡터라이저를 만든다. `analyzer`(기본 word), `ngram`(기본 [1,1]), `min_df`(기본 1), `max_features`.
pub fn make_vectorizer(spec: &Value) -> Vectorizer {
    let ngram = spec.get("ngram").and_then(Value::as_array).filter(|a| a.len() == 2).map(|a| (a[0].as_u64().unwrap_or(1) as usize, a[1].as_u64().unwrap_or(1) as usize)).unwrap_or((1, 1));
    let kind = AnalyzerKind::parse(spec_str(spec, "analyzer").unwrap_or("word")).unwrap_or(AnalyzerKind::Word);
    Vectorizer {
        spec: AnalyzerSpec { kind, ngram_lo: ngram.0.max(1), ngram_hi: ngram.1.max(ngram.0.max(1)), lowercase: true },
        min_df: spec.get("min_df").and_then(Value::as_u64).unwrap_or(1) as usize,
        max_features: spec.get("max_features").and_then(Value::as_u64).filter(|n| *n > 0).map(|n| n as usize),
        vocabulary: HashMap::new(),
        idf: Vec::new(),
    }
}

impl Vectorizer {
    /// 말뭉치로 어휘와 idf 를 배운다. 어휘는 사전순 번호이고, `max_features` 는 말뭉치 빈도 상위를 남긴다.
    pub fn fit(&mut self, texts: &[&str]) -> &Self {
        let mut stats: HashMap<String, (u64, usize)> = HashMap::new();
        for text in texts {
            let mut seen: HashMap<String, u64> = HashMap::new();
            for_each_term(text, &self.spec, |t| *seen.entry(t.to_string()).or_insert(0) += 1);
            for (term, count) in seen {
                let e = stats.entry(term).or_insert((0, 0));
                e.0 += count;
                e.1 += 1;
            }
        }
        let mut terms: Vec<(String, u64, usize)> = stats.into_iter().filter(|(_, (_, df))| *df >= self.min_df).map(|(t, (tf, df))| (t, tf, df)).collect();
        terms.sort_by(|a, b| a.0.cmp(&b.0));
        if let Some(limit) = self.max_features
            && terms.len() > limit
        {
            let mut by_tf: Vec<usize> = (0..terms.len()).collect();
            by_tf.sort_by_key(|&i| std::cmp::Reverse(terms[i].1));
            let mut keep = vec![false; terms.len()];
            by_tf.iter().take(limit).for_each(|&i| keep[i] = true);
            let mut i = 0;
            terms.retain(|_| {
                i += 1;
                keep[i - 1]
            });
        }
        let n = texts.len() as f64;
        self.idf = terms.iter().map(|(_, _, df)| (((1.0 + n) / (1.0 + *df as f64)).ln() + 1.0) as f32).collect();
        self.vocabulary = terms.into_iter().enumerate().map(|(i, (t, _, _))| (t, i)).collect();
        self
    }

    pub fn vocabulary(&self) -> &HashMap<String, usize> {
        &self.vocabulary
    }
}

/// 필드별 벡터라이저 묶음. 학습 때 `fit_transform` 하고, 판정 때는 `transform` 만 한다.
pub struct FeatureSpace {
    fields: Value,
    vectorizers: Vec<(String, FieldVectorizer)>,
    block_sizes: Vec<(String, usize)>,
}

impl FeatureSpace {
    pub fn new(fields: &Value) -> Self {
        Self { fields: fields.clone(), vectorizers: Vec::new(), block_sizes: Vec::new() }
    }

    pub fn has_vectorizer(&self, name: &str) -> bool {
        self.vectorizers.iter().any(|(n, _)| n == name)
    }

    pub fn vectorizer_names(&self) -> Vec<String> {
        self.vectorizers.iter().map(|(n, _)| n.clone()).collect()
    }

    pub fn block_sizes(&self) -> Vec<(String, usize)> {
        self.block_sizes.clone()
    }

    pub fn describe(&self) -> Value {
        let mut out = serde_json::Map::new();
        for (name, size) in &self.block_sizes {
            let weight = self.fields[name.as_str()].get("weight").and_then(Value::as_f64).unwrap_or(1.0);
            out.insert(name.clone(), json!({"features": size, "weight": weight}));
        }
        Value::Object(out)
    }

    pub fn fit_transform(&mut self, docs: &[Value]) -> Result<SparseMatrix, ValueError> {
        self.fit_transform_with(docs, FitOptions::default())
    }

    /// 필드마다 TF-IDF 블록을 만들어 가로로 잇는다. 어휘가 안 남는 필드는 조용히 버린다.
    pub fn fit_transform_with(&mut self, docs: &[Value], opts: FitOptions) -> Result<SparseMatrix, ValueError> {
        self.vectorizers.clear();
        self.block_sizes.clear();
        let fields = self.fields.as_object().cloned().unwrap_or_default();
        let mut blocks: Vec<Vec<Vec<(u32, f64)>>> = Vec::new();
        let mut n_cols = 0usize;
        for (name, spec) in &fields {
            let texts: Vec<String> = docs.iter().map(|d| field_text(d, spec)).collect();
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            let mut vec = make_vectorizer(spec);
            vec.fit(&refs);
            if vec.vocabulary.is_empty() {
                log::warn!("필드 '{name}' 에서 특징을 못 뽑아 건너뛴다");
                continue;
            }
            let mut vocab = vec![String::new(); vec.vocabulary.len()];
            for (t, i) in &vec.vocabulary {
                vocab[*i] = t.clone();
            }
            let weight = spec.get("weight").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            let size = vocab.len();
            let fv = FieldVectorizer::new(name, spec_str(spec, "source").unwrap_or(""), weight, spec_usize(spec, "max_chars"), vec.spec, true, n_cols as u32, vocab, vec.idf.clone()).map_err(|e| ValueError(e.to_string()))?;
            let rows: Vec<Vec<(u32, f64)>> = refs
                .iter()
                .map(|t| {
                    let mut row = Vec::new();
                    fv.transform_into_f64(t, &mut row);
                    row
                })
                .collect();
            n_cols += size;
            self.block_sizes.push((name.clone(), size));
            self.vectorizers.push((name.clone(), fv));
            blocks.push(match &opts.spill_dir {
                Some(dir) => spill_block(dir, name, &rows)?,
                None => rows,
            });
        }
        if blocks.is_empty() {
            return Err(ValueError("어느 필드에서도 특징을 못 뽑았다".into()));
        }
        Ok(assemble(blocks, docs.len(), n_cols, opts.dtype))
    }

    /// 학습된 벡터라이저로 문서를 변환한다.
    pub fn transform(&self, docs: &[Value]) -> Result<SparseMatrix, ValueError> {
        if self.vectorizers.is_empty() {
            return Err(ValueError("학습된 벡터라이저가 없다".into()));
        }
        let n_cols = self.block_sizes.iter().map(|(_, s)| s).sum();
        let blocks: Vec<Vec<Vec<(u32, f64)>>> = self
            .vectorizers
            .iter()
            .map(|(name, fv)| {
                docs.iter()
                    .map(|d| {
                        let mut row = Vec::new();
                        fv.transform_into_f64(&field_text(d, &self.fields[name.as_str()]), &mut row);
                        row
                    })
                    .collect()
            })
            .collect();
        Ok(assemble(blocks, docs.len(), n_cols, Dtype::F64))
    }
}

/// 블록들을 행마다 이어 붙인다. 열 번호는 블록 생성 때 이미 offset 이 더해져 있다.
fn assemble(blocks: Vec<Vec<Vec<(u32, f64)>>>, n_rows: usize, n_cols: usize, dtype: Dtype) -> SparseMatrix {
    let mut rows: Vec<Vec<(u32, f64)>> = vec![Vec::new(); n_rows];
    for block in blocks {
        for (row, part) in rows.iter_mut().zip(block) {
            row.extend(part.into_iter().map(|(c, v)| (c, if dtype == Dtype::F32 { f64::from(v as f32) } else { v })));
        }
    }
    SparseMatrix { rows, n_cols, dtype }
}

/// 블록을 디스크에 쓰고 다시 읽어 메모리에서 내려놓는다. 읽은 뒤 파일은 지운다.
fn spill_block(dir: &Path, name: &str, rows: &[Vec<(u32, f64)>]) -> Result<Vec<Vec<(u32, f64)>>, ValueError> {
    let io = |e: std::io::Error| ValueError(format!("스필 파일 오류: {e}"));
    fs::create_dir_all(dir).map_err(io)?;
    // 필드 이름이 경로를 못 벗어나게 파일명에 쓰는 문자를 제한한다.
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    let path = dir.join(format!("{safe}.block"));
    let mut out = std::io::BufWriter::new(fs::File::create(&path).map_err(io)?);
    out.write_all(&(rows.len() as u64).to_le_bytes()).map_err(io)?;
    for row in rows {
        out.write_all(&(row.len() as u64).to_le_bytes()).map_err(io)?;
        for &(c, v) in row {
            out.write_all(&c.to_le_bytes()).map_err(io)?;
            out.write_all(&v.to_le_bytes()).map_err(io)?;
        }
    }
    out.flush().map_err(io)?;
    drop(out);
    let mut input = std::io::BufReader::new(fs::File::open(&path).map_err(io)?);
    let mut u64buf = [0u8; 8];
    input.read_exact(&mut u64buf).map_err(io)?;
    let n_rows = u64::from_le_bytes(u64buf) as usize;
    let mut back = Vec::with_capacity(n_rows.min(1 << 20));
    for _ in 0..n_rows {
        input.read_exact(&mut u64buf).map_err(io)?;
        let len = u64::from_le_bytes(u64buf) as usize;
        let mut row = Vec::with_capacity(len.min(1 << 20));
        for _ in 0..len {
            let mut c = [0u8; 4];
            input.read_exact(&mut c).map_err(io)?;
            input.read_exact(&mut u64buf).map_err(io)?;
            row.push((u32::from_le_bytes(c), f64::from_le_bytes(u64buf)));
        }
        back.push(row);
    }
    let _ = fs::remove_file(&path);
    Ok(back)
}

/// 남은 필드가 볼 만큼만 남기고 원문을 제자리에서 줄인다. 아무 필드도 안 보는 원문은 지운다.
/// `max_chars` 가 0(전부)인 필드가 남아 있으면 그 원문은 건드리지 않는다. `name` 은 숫자를 지운 뒤에 자르므로 건드리지 않는다.
pub fn shrink_sources(docs: &mut [Value], all_sources: &[&str], remaining: &[(&str, Value)]) {
    // 소스 → 남길 글자 수(None 이면 전부).
    let mut limits: Vec<(String, Option<usize>)> = Vec::new();
    for (_, spec) in remaining {
        let source = spec_str(spec, "source").unwrap_or("").to_string();
        let max = spec_usize(spec, "max_chars");
        let want = if source == "name" || max == 0 { None } else { Some(max) };
        match limits.iter_mut().find(|(s, _)| *s == source) {
            Some((_, current)) => *current = match (*current, want) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            },
            None => limits.push((source, want)),
        }
    }
    let drop: Vec<&str> = all_sources.iter().copied().filter(|s| !limits.iter().any(|(l, _)| l == s)).collect();
    let cut: Vec<(&str, usize)> = limits.iter().filter_map(|(s, n)| n.map(|n| (s.as_str(), n))).collect();
    if drop.is_empty() && cut.is_empty() {
        return;
    }
    for doc in docs.iter_mut() {
        let Some(obj) = doc.as_object_mut() else { continue };
        for source in &drop {
            obj.remove(*source);
        }
        for (source, limit) in &cut {
            if let Some(Value::String(s)) = obj.get_mut(*source)
                && let Some((byte, _)) = s.char_indices().nth(*limit)
            {
                s.truncate(byte);
            }
        }
    }
}
