//! 학습 데이터 수집과 publisher 보강. Python `backend/classifier/corpus.py` 대응.
//!
//! JSONL 과 publisher 캐시는 Python 이 쓴 파일과 바이트 호환이다(`pyjson::dumps`). 코퍼스 경로와 파일은
//! 신뢰할 수 없는 입력처럼 다룬다: 상대 경로의 `..` 거부, 심볼릭 링크로 라이브러리 밖을 가리키면 건너뜀,
//! 줄·파일·캐시 크기 상한. 로그에는 제목·저자·본문을 남기지 않는다.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use fancy_regex::Regex;
use serde_json::{Map, Value, json};

use super::pyjson;
use super::reader::read_epub_publisher;
use crate::es_manager::EsTransport;

/// ES 에서 가져올 필드. `summary` 가 본문 앞부분이다.
pub const SOURCE_FIELDS: [&str; 7] = ["category", "summary", "title", "author", "publisher", "file_path", "file_type"];
/// 레이블은 `숫자_이름` 꼴 디렉토리다.
pub const DEFAULT_LABEL_PATTERN: &str = r"^[1-9]_";
pub const PUBLISHER_WORKERS: usize = 2;
/// JSONL 한 줄 최대 크기. 문서 하나는 본문 앞 4KB 와 메타데이터라 이보다 훨씬 작다.
pub const MAX_JSONL_LINE_BYTES: u64 = 16 << 20;
/// JSONL 파일 최대 크기. 23만 건이 1~2GB 다.
pub const MAX_JSONL_BYTES: u64 = 16 << 30;
/// publisher 캐시 최대 크기.
pub const MAX_CACHE_BYTES: u64 = 256 << 20;

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// ES 히트 하나를 학습/판정용 레코드로 바꾼다. 카테고리가 경로면 최상위만 레이블로 쓴다.
pub fn document(hit: &Value) -> Value {
    let src = hit.get("_source").filter(|s| s.is_object()).unwrap_or(&Value::Null);
    let category = text_of(src.get("category")).split('/').next().unwrap_or("").to_string();
    let path = text_of(src.get("file_path"));
    let id = match hit.get("_id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let name = if path.is_empty() { String::new() } else { Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() };
    json!({
        "id": id, "cat": category, "text": text_of(src.get("summary")), "title": text_of(src.get("title")), "author": text_of(src.get("author")),
        "publisher": text_of(src.get("publisher")), "type": text_of(src.get("file_type")), "name": name, "path": path,
    })
}

fn has_label_shape(cat: &str) -> bool {
    let mut chars = cat.chars();
    matches!((chars.next(), chars.next()), (Some('1'..='9'), Some('_')))
}

/// 학습에 쓸 수 있는 레코드인가. 레이블이 없거나 본문이 없으면 못 쓴다.
pub fn is_trainable(doc: &Value, min_chars: usize, excluded_prefixes: &[&str]) -> bool {
    let cat = doc.get("cat").and_then(Value::as_str).unwrap_or("");
    if cat.is_empty() || !has_label_shape(cat) || excluded_prefixes.iter().any(|p| cat.starts_with(p)) {
        return false;
    }
    doc.get("text").and_then(Value::as_str).unwrap_or("").chars().count() >= min_chars
}

/// `is_trainable` 의 사용자 지정 레이블 패턴 판. 패턴은 문자열 시작에 고정해 검사한다(`re.match`).
pub fn is_trainable_with(doc: &Value, min_chars: usize, excluded_prefixes: &[&str], label_pattern: &Regex) -> bool {
    let cat = doc.get("cat").and_then(Value::as_str).unwrap_or("");
    if cat.is_empty() || label_pattern.find(cat).ok().flatten().is_none_or(|m| m.start() != 0) || excluded_prefixes.iter().any(|p| cat.starts_with(p)) {
        return false;
    }
    doc.get("text").and_then(Value::as_str).unwrap_or("").chars().count() >= min_chars
}

/// 레코드의 `path` 를 실제 경로로 바꾼다. 상대 경로는 라이브러리 루트 기준이다.
///
/// 상대 경로에 `..` 가 있으면 루트 밖으로 나갈 수 있어 `None` 이다(Python 은 그대로 이어 붙인다).
pub fn absolute_path(doc: &Value, library_root: &str) -> Option<PathBuf> {
    let raw = doc.get("path").and_then(Value::as_str).unwrap_or("");
    if raw.is_empty() {
        return None;
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    Some(Path::new(library_root).join(p))
}

// ---------------------------------------------------------------------------
// ES 조회
// ---------------------------------------------------------------------------

/// 전체 훑기 옵션. `index` 가 없으면 전송 계층의 색인을 쓴다.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub batch: usize,
    pub index: Option<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { batch: 2000, index: None }
    }
}

fn scan_kwargs(es: &dyn EsTransport, opts: &ScanOptions) -> Value {
    json!({"index": opts.index.clone().unwrap_or_else(|| es.index_name().to_string()), "query": {"query": {"match_all": {}}}, "_source": SOURCE_FIELDS, "size": opts.batch, "preserve_order": false})
}

/// 인덱스 전체를 훑는다. `scan` 이 `helpers.scan` 자리다. 히트를 레코드로 바꿔 모은다.
pub fn scan_documents_with(es: &dyn EsTransport, opts: ScanOptions, scan: &dyn Fn(&Value) -> Vec<Value>) -> Vec<Value> {
    scan(&scan_kwargs(es, &opts)).iter().map(document).collect()
}

/// scroll 로 인덱스 전체를 훑으며 히트마다 `each` 를 부른다. 메모리에 다 올리지 않는다. 오류는 호출한 쪽에 돌려준다.
pub fn scan_each(es: &dyn EsTransport, opts: &ScanOptions, each: &mut dyn FnMut(Value)) -> Result<(), crate::es_manager::EsError> {
    scan_each_with_page(
        es,
        opts,
        &mut |hit| {
            each(hit);
            Ok(())
        },
        &mut || Ok(()),
    )
}

fn scan_each_with_page(
    es: &dyn EsTransport,
    opts: &ScanOptions,
    each: &mut dyn FnMut(Value) -> Result<(), crate::es_manager::EsError>,
    after_page: &mut dyn FnMut() -> Result<(), crate::es_manager::EsError>,
) -> Result<(), crate::es_manager::EsError> {
    let index = opts.index.clone().unwrap_or_else(|| es.index_name().to_string());
    let mut response = es.call("search", &json!({"index": index, "query": {"match_all": {}}, "_source": SOURCE_FIELDS, "size": opts.batch, "scroll": "10m"}))?;
    let mut scroll_id = response["_scroll_id"].as_str().map(str::to_string);
    let outcome = (|| loop {
        let hits = response["hits"]["hits"].as_array().cloned().unwrap_or_default();
        if hits.is_empty() {
            return Ok(());
        }
        for hit in hits {
            each(hit)?;
        }
        after_page()?;
        let Some(id) = scroll_id.clone() else { return Ok(()) };
        response = es.call("scroll", &json!({"scroll_id": id, "scroll": "10m"}))?;
        if let Some(next) = response["_scroll_id"].as_str() {
            scroll_id = Some(next.to_string());
        }
    })();
    if let Some(id) = scroll_id
        && let Err(e) = es.call("clear_scroll", &json!({"scroll_id": id}))
    {
        log::debug!("scroll 을 지우지 못했다: {e}");
    }
    outcome
}

/// inode 하나로 문서를 집어 온다. ES 의 `_id` 가 inode 다. 못 찾으면 `None`.
pub fn fetch_by_inode(es: &dyn EsTransport, inode: u64, index: Option<&str>) -> Option<Value> {
    let index = index.unwrap_or_else(|| es.index_name());
    match es.call("get", &json!({"index": index, "id": inode.to_string(), "_source": SOURCE_FIELDS})) {
        Ok(hit) => Some(document(&hit)),
        Err(e) => {
            log::debug!("inode {inode} 를 ES 에서 못 찾았다: {e}");
            None
        }
    }
}

/// 경로로 찾는다. inode 가 바뀐 경우의 대비책이다. `file_path` 는 keyword 타입이다.
pub fn fetch_by_path(es: &dyn EsTransport, file_path: &str, index: Option<&str>) -> Option<Value> {
    let index = index.unwrap_or_else(|| es.index_name());
    match es.call("search", &json!({"index": index, "query": {"term": {"file_path": file_path}}, "size": 1, "_source": SOURCE_FIELDS})) {
        Ok(res) => res.get("hits").and_then(|h| h.get("hits")).and_then(Value::as_array).and_then(|a| a.first()).map(document),
        Err(e) => {
            log::debug!("경로를 ES 에서 못 찾았다: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// JSONL
// ---------------------------------------------------------------------------

/// 레코드를 JSONL 로 쓴다(`ensure_ascii=False`). 부모 디렉토리가 없으면 만든다. 쓴 줄 수를 돌려준다.
pub fn write_jsonl(docs: &[Value], out_path: &Path) -> io::Result<usize> {
    write_jsonl_iter(docs.iter(), out_path)
}

fn write_jsonl_iter<'a>(docs: impl Iterator<Item = &'a Value>, out_path: &Path) -> io::Result<usize> {
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = io::BufWriter::new(fs::File::create(out_path)?);
    let mut n = 0;
    for d in docs {
        out.write_all(pyjson::dumps(d, false).as_bytes())?;
        out.write_all(b"\n")?;
        n += 1;
    }
    out.flush()?;
    Ok(n)
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// JSONL 을 읽는다. 빈 줄은 건너뛴다. 파일 크기와 줄 길이에 상한이 있고, 깨진 줄은 오류다.
pub fn read_jsonl(path: &Path) -> io::Result<Vec<Value>> {
    let file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > MAX_JSONL_BYTES {
        return Err(invalid(format!("JSONL 이 너무 크다 ({len}바이트 > {MAX_JSONL_BYTES})")));
    }
    let mut reader = BufReader::new(file);
    let mut docs = Vec::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = (&mut reader).take(MAX_JSONL_LINE_BYTES + 1).read_until(b'\n', &mut buf)?;
        if read == 0 {
            return Ok(docs);
        }
        if buf.len() as u64 > MAX_JSONL_LINE_BYTES {
            return Err(invalid(format!("JSONL {}번째 줄이 너무 길다", docs.len() + 1)));
        }
        if buf.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        docs.push(serde_json::from_slice(&buf).map_err(|e| invalid(format!("JSONL 해석 실패: {e}")))?);
    }
}

struct JsonlRecordIndex {
    offset: u64,
    length: usize,
    category: String,
    document_key: String,
}

/// JSONL 학습 corpus의 메타데이터와 줄 위치만 보유한다. 본문은 `read_document` 호출 때만 해석한다.
pub struct IndexedJsonl {
    reader: BufReader<fs::File>,
    records: Vec<JsonlRecordIndex>,
    has_publisher: bool,
}

impl IndexedJsonl {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = fs::File::open(path)?;
        let len = file.metadata()?.len();
        if len > MAX_JSONL_BYTES {
            return Err(invalid(format!("JSONL 이 너무 크다 ({len}바이트 > {MAX_JSONL_BYTES})")));
        }
        let mut reader = BufReader::new(file);
        let mut records = Vec::new();
        let mut has_publisher = false;
        let mut buf = Vec::new();
        let mut line_number = 0usize;
        loop {
            let offset = reader.stream_position()?;
            buf.clear();
            let read = (&mut reader).take(MAX_JSONL_LINE_BYTES + 1).read_until(b'\n', &mut buf)?;
            if read == 0 {
                break;
            }
            line_number += 1;
            if buf.len() as u64 > MAX_JSONL_LINE_BYTES {
                return Err(invalid(format!("JSONL {line_number}번째 줄이 너무 길다")));
            }
            if buf.iter().all(|byte| byte.is_ascii_whitespace()) {
                continue;
            }
            let doc: Value = serde_json::from_slice(&buf).map_err(|error| invalid(format!("JSONL 해석 실패: {error}")))?;
            has_publisher |= json_truthy(doc.get("publisher"));
            let document_key = ["id", "path", "name"].iter().filter_map(|key| doc.get(*key).and_then(Value::as_str)).find(|value| !value.is_empty()).unwrap_or("").to_owned();
            records.push(JsonlRecordIndex { offset, length: buf.len(), category: doc.get("cat").and_then(Value::as_str).unwrap_or("").to_owned(), document_key });
        }
        Ok(Self { reader, records, has_publisher })
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn category(&self, index: usize) -> Option<&str> {
        self.records.get(index).map(|record| record.category.as_str())
    }

    pub fn document_key(&self, index: usize) -> Option<&str> {
        self.records.get(index).map(|record| record.document_key.as_str())
    }

    pub fn has_publisher(&self) -> bool {
        self.has_publisher
    }

    pub fn read_document(&mut self, index: usize) -> io::Result<Value> {
        let record = self.records.get(index).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "JSONL 문서 인덱스가 범위를 벗어났다"))?;
        self.reader.seek(SeekFrom::Start(record.offset))?;
        let mut bytes = vec![0; record.length];
        self.reader.read_exact(&mut bytes)?;
        serde_json::from_slice(&bytes).map_err(|error| invalid(format!("JSONL 해석 실패: {error}")))
    }
}

fn json_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(value)) => !value.is_empty(),
        Some(Value::Object(value)) => !value.is_empty(),
    }
}

/// 수집 옵션. `excluded_prefixes` 로 시작하는 카테고리는 뺀다.
#[derive(Debug, Clone, Default)]
pub struct CollectOptions {
    pub min_chars: usize,
    pub excluded_prefixes: Vec<String>,
    pub index: Option<String>,
}

struct CorpusWriter {
    out: io::BufWriter<fs::File>,
    kept: usize,
    skipped: usize,
    order: Vec<String>,
    counts: HashMap<String, usize>,
}

impl CorpusWriter {
    fn create(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self { out: io::BufWriter::new(fs::File::create(path)?), kept: 0, skipped: 0, order: Vec::new(), counts: HashMap::new() })
    }

    fn push(&mut self, doc: Value, min_chars: usize, prefixes: &[&str], label_pattern: &Regex) -> io::Result<()> {
        if !is_trainable_with(&doc, min_chars, prefixes, label_pattern) {
            self.skipped += 1;
            return Ok(());
        }
        let cat = doc["cat"].as_str().unwrap_or("").to_string();
        if !self.counts.contains_key(&cat) {
            self.order.push(cat.clone());
        }
        *self.counts.entry(cat).or_default() += 1;
        self.out.write_all(pyjson::dumps(&doc, false).as_bytes())?;
        self.out.write_all(b"\n")?;
        self.kept += 1;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    fn finish(mut self) -> io::Result<Value> {
        self.flush()?;
        // Counter.most_common: 개수 내림차순, 같으면 처음 나온 순서(안정 정렬).
        self.order.sort_by_key(|cat| std::cmp::Reverse(self.counts[cat]));
        let per_category: Map<String, Value> = self.order.iter().map(|cat| (cat.clone(), json!(self.counts[cat]))).collect();
        Ok(json!({"kept": self.kept, "skipped": self.skipped, "categories": self.order.len(), "per_category": per_category}))
    }
}

/// ES 를 훑어 학습용 JSONL 을 만든다. 돌려주는 값은 요약 통계다. `scan` 이 `helpers.scan` 자리다.
pub fn collect_with(es: &dyn EsTransport, out_path: &Path, opts: CollectOptions, scan: &dyn Fn(&Value) -> Vec<Value>) -> io::Result<Value> {
    let prefixes: Vec<&str> = opts.excluded_prefixes.iter().map(String::as_str).collect();
    let pattern = Regex::new(DEFAULT_LABEL_PATTERN).expect("기본 레이블 정규식은 유효하다");
    let kwargs = scan_kwargs(es, &ScanOptions { index: opts.index.clone(), ..Default::default() });
    let mut writer = CorpusWriter::create(out_path)?;
    for hit in scan(&kwargs) {
        writer.push(document(&hit), opts.min_chars, &prefixes, &pattern)?;
    }
    writer.finish()
}

/// `collect_with` 의 실제 ES(scroll) 판.
pub fn collect(es: &dyn EsTransport, out_path: &Path, opts: CollectOptions) -> io::Result<Value> {
    collect_with_pattern(es, out_path, opts, DEFAULT_LABEL_PATTERN)
}

/// 설정에 지정된 레이블 정규식을 사용해 ES(scroll) 결과를 JSONL로 스트리밍한다.
pub fn collect_with_pattern(es: &dyn EsTransport, out_path: &Path, opts: CollectOptions, label_pattern: &str) -> io::Result<Value> {
    let prefixes: Vec<&str> = opts.excluded_prefixes.iter().map(String::as_str).collect();
    let pattern = Regex::new(label_pattern).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let writer = RefCell::new(CorpusWriter::create(out_path)?);
    scan_each_with_page(
        es,
        &ScanOptions { index: opts.index.clone(), ..Default::default() },
        &mut |hit| writer.borrow_mut().push(document(&hit), opts.min_chars, &prefixes, &pattern).map_err(|e| crate::es_manager::EsError::Other(e.to_string())),
        &mut || writer.borrow_mut().flush().map_err(|e| crate::es_manager::EsError::Other(e.to_string())),
    )
    .map_err(|e| io::Error::other(e.to_string()))?;
    writer.into_inner().finish()
}

// ---------------------------------------------------------------------------
// publisher 보강
// ---------------------------------------------------------------------------

/// 리더가 실패해도 빈 문자열로 삼킨다. 한 권이 깨져도 전체 뒷마감이 멈추면 안 된다.
pub fn publisher_of_with(path: &Path, reader: &dyn Fn(&Path) -> Result<String, String>) -> String {
    reader(path).unwrap_or_else(|e| {
        log::debug!("publisher 를 읽지 못했다: {e}");
        String::new()
    })
}

pub fn publisher_of(path: &Path) -> String {
    publisher_of_with(path, &|p| Ok(read_epub_publisher(p)))
}

/// publisher 보강 옵션. `log` 는 진행 메시지를 받는다(없으면 `log` crate INFO).
pub struct AttachOptions<'a> {
    pub workers: usize,
    pub progress_every: usize,
    pub log: Option<&'a dyn Fn(&str)>,
}

impl Default for AttachOptions<'_> {
    fn default() -> Self {
        Self { workers: PUBLISHER_WORKERS, progress_every: 5000, log: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachStats {
    pub read: usize,
    pub cached: usize,
    pub with_publisher: usize,
}

static PUBLISHER_JSONL_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn load_cache(path: &Path) -> Map<String, Value> {
    let read = || -> Result<Map<String, Value>, String> {
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > MAX_CACHE_BYTES {
            return Err("일반 파일이 아니거나 너무 크다".into());
        }
        let mut buf = Vec::with_capacity(meta.len() as usize);
        file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        match serde_json::from_slice::<Value>(&buf).map_err(|e| e.to_string())? {
            Value::Object(m) => Ok(m),
            _ => Err("JSON 객체가 아니다".into()),
        }
    };
    if !path.exists() {
        return Map::new();
    }
    read().unwrap_or_else(|e| {
        log::warn!("publisher 캐시를 못 읽어 새로 만든다: {e}");
        Map::new()
    })
}

/// 상대 경로가 심볼릭 링크로 라이브러리 밖을 가리키면 거짓. 존재하지 않으면 판단할 수 없어 참이다.
fn stays_in_root(path: &Path, root: &Path) -> bool {
    match (path.canonicalize(), root.canonicalize()) {
        (Ok(p), Ok(r)) => p.starts_with(r),
        _ => true,
    }
}

/// ES 에 publisher 가 없는 EPUB 만 파일에서 읽어 메운다. 캐시에 있는 것은 다시 안 읽는다.
fn attach_publishers_batch(docs: &mut [Value], cache: &mut Map<String, Value>, library_root: &Path, opts: AttachOptions<'_>, publisher_of: &(dyn Fn(&Path) -> String + Sync)) -> AttachStats {
    let root = library_root.to_string_lossy().into_owned();
    let id_of = |d: &Value| d.get("id").and_then(Value::as_str).unwrap_or("").to_string();

    let mut todo: Vec<(usize, PathBuf)> = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        let is_epub = d.get("type").and_then(Value::as_str) == Some("epub");
        let missing = d.get("publisher").and_then(Value::as_str).is_none_or(str::is_empty);
        if !is_epub || !missing || cache.contains_key(&id_of(d)) {
            continue;
        }
        let Some(p) = absolute_path(d, &root) else { continue };
        let relative = !Path::new(d["path"].as_str().unwrap_or("")).is_absolute();
        if relative && !stays_in_root(&p, library_root) {
            log::debug!("라이브러리 밖을 가리키는 경로라 건너뛴다");
            continue;
        }
        todo.push((i, p));
    }
    let report = |m: &str| match opts.log {
        Some(f) => f(m),
        None => log::info!("{m}"),
    };
    report(&format!("파일에서 publisher 를 읽을 EPUB {}개 (ES 에 이미 있거나 캐시 적중 {}개)", todo.len(), docs.len() - todo.len()));

    if !todo.is_empty() {
        let results: Vec<String> = run_workers(&todo, opts.workers.clamp(1, 16), publisher_of);
        for (n, ((i, _), publisher)) in todo.iter().zip(results).enumerate() {
            cache.insert(id_of(&docs[*i]), Value::String(publisher));
            let done = n + 1;
            if opts.progress_every > 0 && done % opts.progress_every == 0 {
                report(&format!("  publisher {done}/{}", todo.len()));
            }
        }
    }

    let mut found = 0;
    for d in docs.iter_mut() {
        let empty = d.get("publisher").and_then(Value::as_str).is_none_or(str::is_empty);
        if empty {
            let value = cache.get(&id_of(d)).and_then(Value::as_str).unwrap_or("").to_string();
            d["publisher"] = Value::String(value);
        }
        if d["publisher"].as_str().is_some_and(|s| !s.is_empty()) {
            found += 1;
        }
    }
    AttachStats { read: todo.len(), cached: cache.len(), with_publisher: found }
}

fn save_publisher_cache(cache: &Map<String, Value>, cache_path: &Path) {
    if let Some(parent) = cache_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(error) = fs::write(cache_path, pyjson::dumps(&Value::Object(cache.clone()), false)) {
        log::warn!("publisher 캐시를 쓰지 못했다: {error}");
    }
}

pub fn attach_publishers_with(docs: &mut [Value], cache_path: &Path, library_root: &Path, opts: AttachOptions<'_>, publisher_of: &(dyn Fn(&Path) -> String + Sync)) -> AttachStats {
    let mut cache = load_cache(cache_path);
    let stats = attach_publishers_batch(docs, &mut cache, library_root, opts, publisher_of);
    if stats.read > 0 {
        save_publisher_cache(&cache, cache_path);
    }
    stats
}

/// JSONL publisher 보강. 문서 본문은 작은 batch만 메모리에 두고 성공 시 원본 파일을 교체한다.
pub fn attach_publishers_jsonl_with(path: &Path, cache_path: &Path, library_root: &Path, workers: usize, publisher_of: &(dyn Fn(&Path) -> String + Sync)) -> io::Result<Value> {
    const BATCH_SIZE: usize = 256;
    let input = fs::File::open(path)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_JSONL_BYTES {
        return Err(invalid(format!("JSONL 이 일반 파일이 아니거나 너무 크다 ({}바이트)", metadata.len())));
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let (temporary_path, temporary_file) = (0..100)
        .find_map(|_| {
            let sequence = PUBLISHER_JSONL_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(".{name}.publisher-{}-{sequence}.tmp", std::process::id()));
            match fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
                Ok(file) => Some(Ok((candidate, file))),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .unwrap_or_else(|| Err(io::Error::new(io::ErrorKind::AlreadyExists, "publisher JSONL 임시 파일 이름을 확보하지 못했다")))?;

    let result = (|| {
        let mut reader = BufReader::new(input);
        let mut writer = io::BufWriter::new(temporary_file);
        let mut cache = load_cache(cache_path);
        let mut batch = Vec::with_capacity(BATCH_SIZE);
        let mut buffer = Vec::new();
        let mut bytes_read = 0u64;
        let mut total_read = 0usize;
        let mut total_with_publisher = 0usize;
        let silent_log = |_message: &str| {};
        let flush = |batch: &mut Vec<Value>, writer: &mut io::BufWriter<fs::File>, cache: &mut Map<String, Value>, total_read: &mut usize, total_with_publisher: &mut usize| -> io::Result<()> {
            if batch.is_empty() {
                return Ok(());
            }
            let stats = attach_publishers_batch(batch, cache, library_root, AttachOptions { workers, progress_every: 0, log: Some(&silent_log) }, publisher_of);
            *total_read += stats.read;
            *total_with_publisher += stats.with_publisher;
            for document in batch.iter() {
                writer.write_all(pyjson::dumps(document, false).as_bytes())?;
                writer.write_all(b"\n")?;
            }
            batch.clear();
            Ok(())
        };
        loop {
            buffer.clear();
            let read = (&mut reader).take(MAX_JSONL_LINE_BYTES + 1).read_until(b'\n', &mut buffer)?;
            if read == 0 {
                break;
            }
            bytes_read = bytes_read.saturating_add(read as u64);
            if bytes_read > MAX_JSONL_BYTES {
                return Err(invalid(format!("JSONL 이 너무 크다 ({bytes_read}바이트 > {MAX_JSONL_BYTES})")));
            }
            if buffer.len() as u64 > MAX_JSONL_LINE_BYTES {
                return Err(invalid("JSONL 줄이 너무 길다".into()));
            }
            if buffer.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            batch.push(serde_json::from_slice(&buffer).map_err(|error| invalid(format!("JSONL 해석 실패: {error}")))?);
            if batch.len() == BATCH_SIZE {
                flush(&mut batch, &mut writer, &mut cache, &mut total_read, &mut total_with_publisher)?;
            }
        }
        flush(&mut batch, &mut writer, &mut cache, &mut total_read, &mut total_with_publisher)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        if total_read > 0 {
            save_publisher_cache(&cache, cache_path);
        }
        fs::rename(&temporary_path, path)?;
        Ok(json!({"read":total_read,"cached":cache.len(),"with_publisher":total_with_publisher}))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn run_workers(todo: &[(usize, PathBuf)], workers: usize, publisher_of: &(dyn Fn(&Path) -> String + Sync)) -> Vec<String> {
    if workers <= 1 {
        return todo.iter().map(|(_, p)| publisher_of(p)).collect();
    }
    let slots: Vec<Mutex<String>> = todo.iter().map(|_| Mutex::new(String::new())).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((_, p)) = todo.get(i) else { break };
                    *slots[i].lock().unwrap() = publisher_of(p);
                }
            });
        }
    });
    slots.into_iter().map(|m| m.into_inner().unwrap()).collect()
}

/// `attach_publishers_with` 의 실제 EPUB 판. 요약은 `{"read", "cached", "with_publisher"}` 객체다.
pub fn attach_publishers(docs: &mut [Value], cache_path: &Path, library_root: &Path, workers: usize) -> Value {
    let stats = attach_publishers_with(docs, cache_path, library_root, AttachOptions { workers, ..Default::default() }, &publisher_of);
    json!({"read": stats.read, "cached": stats.cached, "with_publisher": stats.with_publisher})
}

/// JSONL 파일의 실제 EPUB publisher 를 보강한다.
pub fn attach_publishers_jsonl(path: &Path, cache_path: &Path, library_root: &Path, workers: usize) -> io::Result<Value> {
    attach_publishers_jsonl_with(path, cache_path, library_root, workers, &publisher_of)
}
