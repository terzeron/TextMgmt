//! 필드별 특징 추출. Python `backend/classifier/features.py` 대응.
//!
//! Python `backend/classifier/features.py`와 같은 규칙으로 특징 행렬을 만든다. 행렬 값은
//! float64 로 계산하고 `Dtype::F32` 를 주면 float32 로 둥글린다(sklearn 의 float32 행렬과 같은 값).
//! 분석기는 `tokenize` 를 쓴다.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

/// 메모리 또는 spill 파일에 저장한 희소 행렬.
#[derive(Debug)]
pub struct SparseMatrix {
    storage: SparseStorage,
    n_rows: usize,
    n_cols: usize,
    nnz: usize,
    dtype: Dtype,
}

#[derive(Debug)]
enum SparseStorage {
    Memory(Vec<Vec<(u32, f64)>>),
    Disk(DiskRows),
}

#[derive(Debug)]
struct DiskRows {
    path: PathBuf,
}

impl Drop for DiskRows {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub struct SparseRowIter<'a> {
    storage: &'a SparseStorage,
    memory_index: usize,
    reader: Option<BufReader<fs::File>>,
    open_error: Option<String>,
    remaining: usize,
}

impl SparseMatrix {
    pub fn shape(&self) -> (usize, usize) {
        (self.n_rows, self.n_cols)
    }

    pub fn nnz(&self) -> usize {
        self.nnz
    }

    pub fn dtype(&self) -> Dtype {
        self.dtype
    }

    pub fn row_iter(&self) -> SparseRowIter<'_> {
        let (reader, open_error) = match &self.storage {
            SparseStorage::Memory(_) => (None, None),
            SparseStorage::Disk(disk) => match fs::File::open(&disk.path) {
                Ok(file) => (Some(BufReader::new(file)), None),
                Err(error) => (None, Some(format!("spill 파일을 열지 못했다: {error}"))),
            },
        };
        SparseRowIter { storage: &self.storage, memory_index: 0, reader, open_error, remaining: self.n_rows }
    }

    pub fn to_dense(&self) -> Vec<Vec<f64>> {
        self.row_iter()
            .map(|result| {
                let row = result.expect("유효한 sparse matrix row");
                let mut dense = vec![0.0; self.n_cols];
                for &(c, v) in row.iter() {
                    dense[c as usize] = v;
                }
                dense
            })
            .collect()
    }

    /// Spill 행렬을 메모리 행렬로 옮긴다. 반복 solver가 매 epoch마다 행을 decode하지 않게 한다.
    pub(super) fn into_memory(self) -> Result<Self, ValueError> {
        if matches!(&self.storage, SparseStorage::Memory(_)) {
            return Ok(self);
        }
        let rows = self.row_iter().map(|row| row.map(|row| row.into_owned())).collect::<Result<Vec<_>, _>>()?;
        let Self { n_rows, n_cols, nnz, dtype, .. } = self;
        Ok(Self { storage: SparseStorage::Memory(rows), n_rows, n_cols, nnz, dtype })
    }

    pub(super) fn memory_rows(&self) -> Result<&[Vec<(u32, f64)>], ValueError> {
        match &self.storage {
            SparseStorage::Memory(rows) => Ok(rows),
            SparseStorage::Disk(_) => Err(ValueError("sparse matrix 는 메모리에 materialize되어야 한다".into())),
        }
    }
}

impl<'a> Iterator for SparseRowIter<'a> {
    type Item = Result<Cow<'a, [(u32, f64)]>, ValueError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.open_error.take() {
            self.remaining = 0;
            return Some(Err(ValueError(error)));
        }
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        match self.storage {
            SparseStorage::Memory(rows) => {
                let row = rows.get(self.memory_index).map(|row| Cow::Borrowed(row.as_slice())).ok_or_else(|| ValueError("sparse matrix row 누락".into()));
                self.memory_index += 1;
                Some(row)
            }
            SparseStorage::Disk(_) => {
                let reader = self.reader.as_mut().expect("disk row reader는 열려 있어야 한다");
                let result = (|| {
                    let mut len = [0u8; 8];
                    reader.read_exact(&mut len).map_err(|e| ValueError(format!("spill 행 길이를 읽지 못했다: {e}")))?;
                    let n = usize::try_from(u64::from_le_bytes(len)).map_err(|_| ValueError("spill 행이 너무 크다".into()))?;
                    let mut row = Vec::with_capacity(n.min(1 << 20));
                    for _ in 0..n {
                        let mut col = [0u8; 4];
                        let mut value = [0u8; 8];
                        reader.read_exact(&mut col).map_err(|e| ValueError(format!("spill 열을 읽지 못했다: {e}")))?;
                        reader.read_exact(&mut value).map_err(|e| ValueError(format!("spill 값을 읽지 못했다: {e}")))?;
                        row.push((u32::from_le_bytes(col), f64::from_le_bytes(value)));
                    }
                    Ok(Cow::Owned(row))
                })();
                Some(result)
            }
        }
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
        self.fit_iter(texts.iter().copied())
    }

    /// 텍스트를 한 건씩 읽어 어휘와 idf 를 배운다. 전체 필드 텍스트를 복제해 보유하지 않는다.
    fn fit_iter<I, T>(&mut self, texts: I) -> &Self
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        self.fit_try_iter(texts.into_iter().map(Ok::<T, ValueError>)).expect("메모리 텍스트 fitting 은 실패하지 않는다");
        self
    }

    fn fit_try_iter<I, T>(&mut self, texts: I) -> Result<(), ValueError>
    where
        I: IntoIterator<Item = Result<T, ValueError>>,
        T: AsRef<str>,
    {
        let mut stats: HashMap<String, (u64, usize)> = HashMap::new();
        let mut n = 0usize;
        for text in texts {
            let text = text?;
            n += 1;
            let mut seen: HashMap<String, u64> = HashMap::new();
            for_each_term(text.as_ref(), &self.spec, |t| *seen.entry(t.to_string()).or_insert(0) += 1);
            for (term, count) in seen {
                let e = stats.entry(term).or_insert((0, 0));
                e.0 += count;
                e.1 += 1;
            }
        }
        self.finish_fit(stats, n);
        Ok(())
    }

    fn finish_fit(&mut self, stats: HashMap<String, (u64, usize)>, n: usize) {
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
        let n = n as f64;
        self.idf = terms.iter().map(|(_, _, df)| (((1.0 + n) / (1.0 + *df as f64)).ln() + 1.0) as f32).collect();
        self.vocabulary = terms.into_iter().enumerate().map(|(i, (t, _, _))| (t, i)).collect();
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

    /// 설정에 적힌 필드 이름. 순서는 설정 순서다(Python `FeatureSpace.names`).
    pub fn names(&self) -> Vec<String> {
        self.fields.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default()
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
        let mut spill_blocks = Vec::new();
        let mut n_cols = 0usize;
        for (name, spec) in &fields {
            let mut vec = make_vectorizer(spec);
            vec.fit_iter(docs.iter().map(|doc| field_text(doc, spec)));
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
            let fv = FieldVectorizer::new(name, spec_str(spec, "source").unwrap_or(""), weight, spec_usize(spec, "max_chars"), vec.spec, true, n_cols as u32, vocab, vec.idf.clone())
                .map_err(|e| ValueError(e.to_string()))?;
            n_cols += size;
            self.block_sizes.push((name.clone(), size));
            let rows = docs.iter().map(|doc| {
                let text = field_text(doc, spec);
                let mut row = Vec::new();
                fv.transform_into_f64(&text, &mut row);
                row
            });
            if let Some(dir) = &opts.spill_dir {
                spill_blocks.push((spill_block_rows(dir, name, docs.len(), rows)?, size));
            } else {
                blocks.push(rows.collect());
            }
            self.vectorizers.push((name.clone(), fv));
        }
        if blocks.is_empty() && spill_blocks.is_empty() {
            return Err(ValueError("어느 필드에서도 특징을 못 뽑았다".into()));
        }
        match &opts.spill_dir {
            Some(dir) => assemble_spilled(dir, spill_blocks, docs.len(), n_cols, opts.dtype),
            None => Ok(assemble(blocks, docs.len(), n_cols, opts.dtype)),
        }
    }

    /// Training 전용 변형. spill 중 학습 행만 점차 줄여 본문 복사본이 block 결합 때 남지 않게 한다.
    pub(super) fn fit_transform_selected_with(&mut self, docs: &mut [Value], selected: &[usize], opts: FitOptions) -> Result<SparseMatrix, ValueError> {
        self.vectorizers.clear();
        self.block_sizes.clear();
        let fields: Vec<(String, Value)> = self.fields.as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
        let all_sources: Vec<String> = fields.iter().map(|(_, spec)| spec_str(spec, "source").unwrap_or("").to_owned()).collect();
        let source_refs: Vec<&str> = all_sources.iter().map(String::as_str).collect();
        let mut blocks: Vec<Vec<Vec<(u32, f64)>>> = Vec::new();
        let mut spill_blocks = Vec::new();
        let mut n_cols = 0usize;
        for (field_index, (name, spec)) in fields.iter().enumerate() {
            let mut vec = make_vectorizer(spec);
            vec.fit_iter(selected.iter().map(|&i| field_text(&docs[i], spec)));
            if vec.vocabulary.is_empty() {
                log::warn!("필드 '{name}' 에서 특징을 못 뽑아 건너뛴다");
            } else {
                let mut vocab = vec![String::new(); vec.vocabulary.len()];
                for (term, index) in &vec.vocabulary {
                    vocab[*index] = term.clone();
                }
                let weight = spec.get("weight").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                let size = vocab.len();
                let vectorizer = FieldVectorizer::new(name, spec_str(spec, "source").unwrap_or(""), weight, spec_usize(spec, "max_chars"), vec.spec, true, n_cols as u32, vocab, vec.idf.clone())
                    .map_err(|e| ValueError(e.to_string()))?;
                n_cols += size;
                self.block_sizes.push((name.clone(), size));
                let rows = selected.iter().map(|&i| {
                    let text = field_text(&docs[i], spec);
                    let mut row = Vec::new();
                    vectorizer.transform_into_f64(&text, &mut row);
                    row
                });
                if let Some(dir) = &opts.spill_dir {
                    spill_blocks.push((spill_block_rows(dir, name, selected.len(), rows)?, size));
                } else {
                    blocks.push(rows.collect());
                }
                self.vectorizers.push((name.clone(), vectorizer));
            }
            if opts.spill_dir.is_some() {
                let remaining: Vec<(&str, Value)> = fields[field_index + 1..].iter().map(|(n, spec)| (n.as_str(), spec.clone())).collect();
                for &i in selected {
                    shrink_sources(std::slice::from_mut(&mut docs[i]), &source_refs, &remaining);
                }
            }
        }
        if blocks.is_empty() && spill_blocks.is_empty() {
            return Err(ValueError("어느 필드에서도 특징을 못 뽑았다".into()));
        }
        if opts.spill_dir.is_some() {
            for &i in selected {
                if let Some(object) = docs[i].as_object_mut() {
                    for key in ["text", "name", "title", "author", "publisher"] {
                        object.remove(key);
                    }
                }
            }
        }
        match &opts.spill_dir {
            Some(dir) => assemble_spilled(dir, spill_blocks, selected.len(), n_cols, opts.dtype),
            None => Ok(assemble(blocks, selected.len(), n_cols, opts.dtype)),
        }
    }

    /// JSONL 인덱스 같은 외부 저장소에서 문서를 한 건씩 읽어 학습 행렬을 만든다.
    pub fn fit_transform_selected_by<F>(&mut self, selected: &[usize], opts: FitOptions, mut load_doc: F) -> Result<SparseMatrix, ValueError>
    where
        F: FnMut(usize) -> Result<Value, ValueError>,
    {
        self.vectorizers.clear();
        self.block_sizes.clear();
        let fields: Vec<(String, Value)> = self.fields.as_object().map(|map| map.iter().map(|(name, spec)| (name.clone(), spec.clone())).collect()).unwrap_or_default();
        let mut blocks: Vec<Vec<Vec<(u32, f64)>>> = Vec::new();
        let mut spill_blocks = Vec::new();
        let mut n_cols = 0usize;
        for (name, spec) in &fields {
            let mut vectorizer = make_vectorizer(spec);
            vectorizer.fit_try_iter(selected.iter().map(|&index| load_doc(index).map(|doc| field_text(&doc, spec))))?;
            if vectorizer.vocabulary.is_empty() {
                log::warn!("필드 '{name}' 에서 특징을 못 뽑아 건너뛴다");
                continue;
            }
            let mut vocabulary = vec![String::new(); vectorizer.vocabulary.len()];
            for (term, &index) in &vectorizer.vocabulary {
                vocabulary[index] = term.clone();
            }
            let weight = spec.get("weight").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            let size = vocabulary.len();
            let field_vectorizer =
                FieldVectorizer::new(name, spec_str(spec, "source").unwrap_or(""), weight, spec_usize(spec, "max_chars"), vectorizer.spec, true, n_cols as u32, vocabulary, vectorizer.idf.clone())
                    .map_err(|error| ValueError(error.to_string()))?;
            n_cols += size;
            self.block_sizes.push((name.clone(), size));
            if let Some(dir) = &opts.spill_dir {
                let block = spill_block_rows_by(dir, name, selected.len(), |row| {
                    let doc = load_doc(selected[row])?;
                    let mut values = Vec::new();
                    field_vectorizer.transform_into_f64(&field_text(&doc, spec), &mut values);
                    Ok(values)
                })?;
                spill_blocks.push((block, size));
            } else {
                let mut rows = Vec::with_capacity(selected.len());
                for &index in selected {
                    let doc = load_doc(index)?;
                    let mut values = Vec::new();
                    field_vectorizer.transform_into_f64(&field_text(&doc, spec), &mut values);
                    rows.push(values);
                }
                blocks.push(rows);
            }
            self.vectorizers.push((name.clone(), field_vectorizer));
        }
        if blocks.is_empty() && spill_blocks.is_empty() {
            return Err(ValueError("어느 필드에서도 특징을 못 뽑았다".into()));
        }
        match &opts.spill_dir {
            Some(dir) => assemble_spilled(dir, spill_blocks, selected.len(), n_cols, opts.dtype),
            None => Ok(assemble(blocks, selected.len(), n_cols, opts.dtype)),
        }
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

    /// 모델에 저장할 학습된 벡터라이저를 꺼낸다.
    pub fn into_vectorizers(self) -> Vec<FieldVectorizer> {
        self.vectorizers.into_iter().map(|(_, vectorizer)| vectorizer).collect()
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
    let nnz = rows.iter().map(Vec::len).sum();
    SparseMatrix { storage: SparseStorage::Memory(rows), n_rows, n_cols, nnz, dtype }
}

static SPILL_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct BlockFile(PathBuf);

impl Drop for BlockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl BlockFile {
    fn persist(self) -> PathBuf {
        let path = self.0.clone();
        std::mem::forget(self);
        path
    }
}

fn spill_path(dir: &Path, prefix: &str) -> PathBuf {
    let id = SPILL_FILE_ID.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{prefix}-{}-{id}.bin", std::process::id()))
}

/// 한 특징 block을 행 단위로 기록해 다음 행을 만들 때 원소 배열을 놓아준다.
fn spill_block_rows<I>(dir: &Path, name: &str, n_rows: usize, rows: I) -> Result<BlockFile, ValueError>
where
    I: IntoIterator<Item = Vec<(u32, f64)>>,
{
    let mut rows = rows.into_iter();
    spill_block_rows_by(dir, name, n_rows, |_| rows.next().ok_or_else(|| ValueError("spill block 행 개수가 맞지 않는다".into())))
}

fn spill_block_rows_by<F>(dir: &Path, name: &str, n_rows: usize, mut row_at: F) -> Result<BlockFile, ValueError>
where
    F: FnMut(usize) -> Result<Vec<(u32, f64)>, ValueError>,
{
    let io = |e: std::io::Error| ValueError(format!("스필 파일 오류: {e}"));
    fs::create_dir_all(dir).map_err(io)?;
    let path = spill_path(dir, &name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect::<String>());
    let block = BlockFile(path);
    let mut out = std::io::BufWriter::new(fs::File::create(&block.0).map_err(io)?);
    out.write_all(&(n_rows as u64).to_le_bytes()).map_err(io)?;
    let mut written = 0;
    while written < n_rows {
        let row = row_at(written)?;
        out.write_all(&(row.len() as u64).to_le_bytes()).map_err(io)?;
        for (c, v) in row {
            out.write_all(&c.to_le_bytes()).map_err(io)?;
            out.write_all(&v.to_le_bytes()).map_err(io)?;
        }
        written += 1;
    }
    out.flush().map_err(io)?;
    drop(out);
    Ok(block)
}

/// 한 번에 한 행만 모아 최종 CSR 유사 파일을 만든다.
fn assemble_spilled(dir: &Path, blocks: Vec<(BlockFile, usize)>, n_rows: usize, n_cols: usize, dtype: Dtype) -> Result<SparseMatrix, ValueError> {
    let io = |e: std::io::Error| ValueError(format!("스필 행렬 오류: {e}"));
    let mut readers = Vec::with_capacity(blocks.len());
    for (block, _) in &blocks {
        let mut reader = BufReader::new(fs::File::open(&block.0).map_err(io)?);
        let mut count = [0u8; 8];
        reader.read_exact(&mut count).map_err(io)?;
        if u64::from_le_bytes(count) != n_rows as u64 {
            return Err(ValueError("spill block 행 개수가 맞지 않는다".into()));
        }
        readers.push(reader);
    }
    let file = BlockFile(spill_path(dir, "matrix"));
    let mut out = BufWriter::new(fs::File::create(&file.0).map_err(io)?);
    let mut nnz = 0usize;
    for _ in 0..n_rows {
        let mut row = Vec::new();
        for reader in &mut readers {
            let mut count = [0u8; 8];
            reader.read_exact(&mut count).map_err(io)?;
            let len = usize::try_from(u64::from_le_bytes(count)).map_err(|_| ValueError("spill 행이 너무 크다".into()))?;
            for _ in 0..len {
                let mut col = [0u8; 4];
                let mut val = [0u8; 8];
                reader.read_exact(&mut col).map_err(io)?;
                reader.read_exact(&mut val).map_err(io)?;
                let value = f64::from_le_bytes(val);
                row.push((u32::from_le_bytes(col), if dtype == Dtype::F32 { f64::from(value as f32) } else { value }));
            }
        }
        nnz += row.len();
        out.write_all(&(row.len() as u64).to_le_bytes()).map_err(io)?;
        for (col, value) in row {
            out.write_all(&col.to_le_bytes()).map_err(io)?;
            out.write_all(&value.to_le_bytes()).map_err(io)?;
        }
    }
    out.flush().map_err(io)?;
    drop(out);
    drop(blocks);
    let path = file.persist();
    Ok(SparseMatrix { storage: SparseStorage::Disk(DiskRows { path }), n_rows, n_cols, nnz, dtype })
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
            Some((_, current)) => {
                *current = match (*current, want) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                }
            }
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

#[cfg(test)]
mod spill_tests {
    use super::{BlockFile, Dtype, FeatureSpace, FitOptions, SparseStorage, assemble_spilled, make_vectorizer, shrink_sources, spill_block_rows};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    struct ObserveRows {
        dir: PathBuf,
        next: usize,
    }

    impl Iterator for ObserveRows {
        type Item = Vec<(u32, f64)>;

        fn next(&mut self) -> Option<Self::Item> {
            if self.next == 1 {
                let first_file = fs::read_dir(&self.dir).unwrap().next().unwrap().unwrap().path();
                assert!(fs::metadata(first_file).unwrap().len() > 8, "첫 행을 다 쓰기 전에 다음 행을 생성하면 안 된다");
            }
            let row = self.next;
            self.next += 1;
            (row < 2).then(|| if row == 0 { (0..20_000).map(|column| (column, 1.0)).collect() } else { vec![(0, 1.0)] })
        }
    }

    #[test]
    fn spill_block_writes_each_row_before_requesting_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let block: BlockFile = spill_block_rows(dir.path(), "stream", 2, ObserveRows { dir: dir.path().to_path_buf(), next: 0 }).unwrap();

        drop(block);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn vectorizer_fit_iter_matches_slice_fit() {
        let spec = json!({"analyzer": "word", "ngram": [1, 2], "min_df": 1, "max_features": 4});
        let texts = vec!["무협 강호 무협", "판타지 마법", "무협 마법"];
        let mut expected = make_vectorizer(&spec);
        expected.fit(&texts);
        let mut streamed = make_vectorizer(&spec);
        streamed.fit_iter(texts.into_iter().map(str::to_string));

        assert_eq!(streamed.vocabulary(), expected.vocabulary());
        assert_eq!(streamed.idf, expected.idf);
    }

    #[test]
    fn materializing_spilled_matrix_preserves_rows() {
        let dir = tempfile::tempdir().unwrap();
        let block = spill_block_rows(dir.path(), "matrix", 2, vec![vec![(0, 0.5), (2, 1.5)], vec![(1, 2.5)]]).unwrap();
        let matrix = assemble_spilled(dir.path(), vec![(block, 3)], 2, 3, Dtype::F64).unwrap();

        let actual = matrix.into_memory().unwrap();
        let rows: Vec<Vec<(u32, f64)>> = actual.row_iter().map(|row| row.unwrap().into_owned()).collect();
        let memory_rows = actual.memory_rows().unwrap();

        assert_eq!(actual.shape(), (2, 3));
        assert_eq!(actual.nnz(), 3);
        assert_eq!(rows, [vec![(0, 0.5), (2, 1.5)], vec![(1, 2.5)]]);
        assert_eq!(memory_rows, rows);
        assert!(matches!(actual.storage, SparseStorage::Memory(_)));
    }

    #[test]
    fn sparse_matrix_reports_spill_open_failure_and_rejects_memory_rows() {
        let dir = tempfile::tempdir().unwrap();
        let block = spill_block_rows(dir.path(), "matrix", 1, vec![vec![(0, 1.0)]]).unwrap();
        let matrix = assemble_spilled(dir.path(), vec![(block, 1)], 1, 1, Dtype::F64).unwrap();
        assert!(matrix.memory_rows().is_err());

        let SparseStorage::Disk(rows) = &matrix.storage else { panic!("expected a spill-backed matrix") };
        fs::remove_file(&rows.path).unwrap();
        let mut iter = matrix.row_iter();
        assert!(iter.next().unwrap().is_err());
        assert!(iter.next().is_none());
    }

    #[test]
    fn spilled_assembly_rejects_blocks_with_different_row_counts() {
        let dir = tempfile::tempdir().unwrap();
        let block = spill_block_rows(dir.path(), "short", 1, vec![vec![(0, 1.0)]]).unwrap();
        let error = assemble_spilled(dir.path(), vec![(block, 1)], 2, 1, Dtype::F64).unwrap_err();
        assert!(error.0.contains("행 개수"));
    }

    #[test]
    fn selected_training_rejects_a_corpus_without_any_terms() {
        let mut space = FeatureSpace::new(&json!({"title": {"source": "title", "analyzer": "word"}}));
        let mut docs = vec![json!({"title": ""})];
        let error = space.fit_transform_selected_with(&mut docs, &[0], FitOptions::default()).unwrap_err();
        assert!(error.0.contains("특징을 못 뽑았다"));
    }

    #[test]
    fn shrink_sources_merges_duplicate_limits_and_skips_non_object_documents() {
        let mut docs = vec![json!({"text":"abcdef", "author":"삭제"}), json!("not an object")];
        let remaining = [("title", json!({"source":"text", "max_chars":2})), ("body", json!({"source":"text", "max_chars":0}))];
        shrink_sources(&mut docs, &["text", "author"], &remaining);

        assert_eq!(docs[0]["text"], "abcdef");
        assert!(docs[0].get("author").is_none());
        assert_eq!(docs[1], "not an object");
    }

    #[test]
    fn transform_requires_a_fitted_vectorizer_and_handles_fitted_documents() {
        let fields = json!({"title": {"source": "title", "analyzer": "word"}});
        let empty = FeatureSpace::new(&fields);
        assert!(empty.transform(&[]).unwrap_err().0.contains("학습된 벡터라이저"));

        let mut fitted = FeatureSpace::new(&fields);
        fitted.fit_transform(&[json!({"title":"alpha beta"})]).unwrap();
        let matrix = fitted.transform(&[json!({"title":"alpha"}), json!({"title":"beta"})]).unwrap();
        assert_eq!(matrix.shape(), (2, 2));
        assert_eq!(matrix.to_dense().len(), 2);
    }

    #[test]
    fn selected_by_builds_memory_and_spilled_matrices_and_rejects_empty_vocabulary() {
        let fields = json!({"title": {"source": "title", "analyzer": "word"}});
        let docs = [json!({"title":"alpha beta"}), json!({"title":"beta gamma"})];
        let mut memory = FeatureSpace::new(&fields);
        let matrix = memory.fit_transform_selected_by(&[0, 1], FitOptions::default(), |index| Ok(docs[index].clone())).unwrap();
        assert_eq!(matrix.shape(), (2, 3));

        let dir = tempfile::tempdir().unwrap();
        let mut spilled = FeatureSpace::new(&fields);
        let options = FitOptions { spill_dir: Some(dir.path().to_path_buf()), dtype: Dtype::F64 };
        let matrix = spilled.fit_transform_selected_by(&[0, 1], options, |index| Ok(docs[index].clone())).unwrap();
        assert_eq!(matrix.shape(), (2, 3));
        assert_eq!(matrix.to_dense(), memory.transform(&docs).unwrap().to_dense());

        let mut empty = FeatureSpace::new(&fields);
        let error = empty.fit_transform_selected_by(&[0], FitOptions::default(), |_| Ok(json!({"title":""}))).unwrap_err();
        assert!(error.0.contains("특징을 못 뽑았다"));
    }

    #[test]
    fn selected_spill_training_removes_consumed_document_text() {
        let fields = json!({
            "first": {"source":"title", "analyzer":"word", "max_chars":0},
            "second": {"source":"author", "analyzer":"word", "max_chars":0}
        });
        let mut docs = vec![json!({"title":"alpha", "author":"writer", "text":"unused"}), json!("not an object")];
        let dir = tempfile::tempdir().unwrap();
        let options = FitOptions { spill_dir: Some(dir.path().to_path_buf()), dtype: Dtype::F64 };
        let mut space = FeatureSpace::new(&fields);
        let matrix = space.fit_transform_selected_with(&mut docs, &[0, 1], options).unwrap();

        assert_eq!(matrix.shape(), (2, 2));
        assert!(docs[0].get("title").is_none());
        assert!(docs[0].get("author").is_none());
        assert!(docs[0].get("text").is_none());
        assert_eq!(docs[1], "not an object");
    }
}
