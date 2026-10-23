//! 라이브러리 전체에서 인코딩이 손상된 문서를 찾는다. 원본: `utils/detect_mojibake.py`.
//!
//! 손상 유형은 원본 텍스트를 잘못된 코드페이지로 읽고 다시 UTF-8로 저장한 것이다.
//! 파일 자체는 유효한 UTF-8이라 디코딩으로는 걸러지지 않고 내용만 깨진다.
//!
//! ```text
//!     정상: 소림사 장로가 강호에서 내공을 운기조식하며
//!     손상: 轝듿닶쓽 蹂댁옣씠 솗蹂대릺吏 븘땲븯怨
//! ```
//!
//! 판별은 `korean_likeness` 하나로 한다. 한글 음절 중 한국어에서 자주 쓰이는 음절의 비율이다.
//! 손상 텍스트의 한글은 희귀 음절이라 이 값이 0에 수렴한다.
//!
//! ```text
//!     손상 확실       0.000 ~ 0.006
//!     정상 한국어      0.271 ~ 0.511
//!     한문 원전        None (한글 50자 미만이라 판정 대상 아님)
//! ```
//!
//! 한자가 많은 것 자체는 손상 근거가 아니다. 한문 원전은 한글이 애초에 없어 판정 대상에서
//! 빠지고, 국한문 혼용은 한글이 정상이라 likeness가 높게 나온다.

pub mod cli;
pub mod reader;
pub mod util;

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use indexmap::IndexMap;
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub use reader::{MIN_SYLLABLES_FOR_LIKENESS, korean_likeness};

pub const DEFAULT_LIBRARY_ROOT: &str = "/mnt/data/text";
const TEXT_EXTS: [&str; 2] = ["txt", "epub"];

/// 손상 0.006 이하, 정상 0.271 이상. 사이가 비어 있어 경계를 넉넉히 잡는다.
pub const MOJIBAKE_LIKENESS_THRESHOLD: f64 = 0.10;

/// 한글이 전체의 이 비율에 못 미치면 판정하지 않는다.
/// 영문 OCR 문서에 한글 잡음이 60자쯤 섞이면 음절 수 하한은 넘지만 문서의 0.2%를 보고 전체를
/// 손상이라 부르는 셈이 된다. 실측에서 이 유형이 오탐 1,971건을 만들었다.
pub const MIN_HANGUL_RATIO: f64 = 0.10;

/// 표본 크기. 앞부분만 보면 표지와 판권지에 속아 본문 손상을 놓친다.
pub const DEFAULT_HEAD_CHARS: usize = reader::TEXT_HEAD_CHARS;
pub const DEFAULT_TAIL_CHARS: usize = reader::TEXT_TAIL_CHARS;

/// /mnt/data는 회전 디스크라 워커를 늘려도 I/O로 막힌다. 2개가 실측상 가장 빨랐다.
pub const DEFAULT_WORKERS: usize = 2;

pub const VERDICT_CORRUPTED: &str = "corrupted";
pub const VERDICT_CLEAN: &str = "clean";
pub const VERDICT_UNDETERMINED: &str = "undetermined";
pub const VERDICT_UNREADABLE: &str = "unreadable";

/// 로그 한 줄을 받는 쪽. 메시지 앞에 `INFO`/`ERROR` 수준이 붙어 있다.
pub type Log<'a> = &'a (dyn Fn(&str) + Sync);

/// 본문 표본으로 손상 여부를 판정한다.
pub fn classify_text(text: &str) -> (&'static str, Option<f64>) {
    let len = text.chars().count();
    if len < 200 {
        return (VERDICT_UNREADABLE, None);
    }
    let hangul = text.chars().filter(|&c| reader::is_hangul_syllable(c)).count();
    if (hangul as f64) / (len as f64) < MIN_HANGUL_RATIO {
        // 한국어 문서가 아니다(한문 원전, 중국어, 일본어, 영문, 악보). 판정 대상이 아니다.
        return (VERDICT_UNDETERMINED, None);
    }
    match korean_likeness(text) {
        None => (VERDICT_UNDETERMINED, None),
        Some(l) if l < MOJIBAKE_LIKENESS_THRESHOLD => (VERDICT_CORRUPTED, Some(l)),
        Some(l) => (VERDICT_CLEAN, Some(l)),
    }
}

/// 파일 한 건의 판정.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileResult {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub verdict: String,
    pub likeness: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Python `round(x, ndigits)`와 같은 결과를 낸다. 곱셈 뒤 반올림은 `0.35` 같은 값에서 어긋나므로,
/// 이진 값을 정확히 십진 전개해 짝수 쪽으로 맞추는 Rust 포맷터를 거쳐 되읽는다.
pub fn py_round(x: f64, ndigits: usize) -> f64 {
    format!("{x:.ndigits$}").parse().unwrap_or(x)
}

fn round4(x: f64) -> f64 {
    py_round(x, 4)
}

pub fn round1(x: f64) -> f64 {
    py_round(x, 1)
}

/// 파일 한 건을 읽어 판정한다.
pub fn inspect_file(path_str: &str) -> FileResult {
    let path = Path::new(path_str);
    let text = reader::read_document_text(path, DEFAULT_HEAD_CHARS, DEFAULT_TAIL_CHARS);
    let (verdict, likeness) = classify_text(&text);
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let reason = if verdict == VERDICT_UNREADABLE {
        // 같은 '판독 불가'라도 원인이 다르다. 빈 파일과 본문 추출 실패를 구분해 둔다.
        if size == 0 {
            "missing_or_empty"
        } else if text.is_empty() {
            "no_text_extracted"
        } else {
            "too_short"
        }
    } else {
        ""
    };
    FileResult {
        path: path_str.into(),
        category: Some(util::category_of(path)),
        verdict: verdict.into(),
        likeness: likeness.map(round4),
        size: Some(size),
        sample_len: Some(text.chars().count()),
        reason: Some(reason.into()),
    }
}

static SLUG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w.-]+").unwrap());

/// 대상 디렉토리마다 다른 리포트 경로. 다른 디렉토리를 스캔해도 이전 리포트를 덮어쓰지 않는다.
///
/// 라이브러리 하위는 상대 경로(2_소설외국, 하위는 `__`로 잇는다), 라이브러리 루트는 all,
/// 라이브러리 밖은 디렉토리 이름을 쓴다.
pub fn default_report_path(kind: &str, root: &Path, repo_root: &Path) -> PathBuf {
    let abs = if root.is_absolute() { root.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(root) };
    let root = PathBuf::from(util::posix_normpath(&abs.to_string_lossy()));
    let parts: Vec<String> = match root.strip_prefix(DEFAULT_LIBRARY_ROOT) {
        Ok(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()],
    };
    let slug = parts.iter().map(|p| SLUG_RE.replace_all(p, "_").into_owned()).collect::<Vec<_>>().join("__");
    let slug = if slug.is_empty() { "all".to_owned() } else { slug };
    repo_root.join("tmp").join(format!("{kind}_report_{slug}.json"))
}

/// 라이브러리 전체의 txt/epub 목록. 도구 캐시 디렉토리는 뺀다.
pub fn list_library_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_link = entry.file_type().is_ok_and(|t| t.is_symlink());
            // os.walk처럼 심볼릭 링크가 디렉토리를 가리키면 디렉토리로 보되 따라 들어가지 않는다.
            let is_dir = if is_link { std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) } else { entry.file_type().is_ok_and(|t| t.is_dir()) };
            if is_dir {
                if !is_link && !name.starts_with('.') && name != "__pycache__" {
                    walk(&path, out);
                }
            } else if !name.starts_with('.') && path.extension().is_some_and(|e| TEXT_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str())) {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort();
    files
}

#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    pub workers: usize,
    /// 0이면 전체.
    pub limit: usize,
    pub progress_every: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { workers: DEFAULT_WORKERS, limit: 0, progress_every: 2000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub scanned_at: String,
    pub root: String,
    pub threshold: f64,
    pub total: usize,
    pub counts: IndexMap<String, usize>,
    pub elapsed_sec: f64,
    pub per_category: IndexMap<String, IndexMap<String, usize>>,
    /// clean을 뺀 전부를 남긴다. 목록이 없으면 나중에 원인을 되짚을 수 없다.
    pub corrupted: Vec<FileResult>,
    pub unreadable: Vec<FileResult>,
    pub undetermined: Vec<FileResult>,
}

pub fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

pub fn thousands(n: usize) -> String {
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

pub fn pool(workers: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(workers.max(1)).build().expect("thread pool")
}

pub fn limited(mut files: Vec<PathBuf>, limit: usize) -> Vec<PathBuf> {
    if limit > 0 {
        files.truncate(limit);
    }
    files
}

/// 라이브러리를 훑어 판정 결과를 모은다.
pub fn scan(root: &Path, opts: &ScanOptions, log: Log) -> ScanReport {
    log(&format!("INFO 파일 목록 수집: {}", root.display()));
    let files = limited(list_library_files(root), opts.limit);
    let n = files.len();
    log(&format!("INFO 대상 {}건, 워커 {}개", thousands(n), opts.workers));

    let started = Instant::now();
    let done = AtomicUsize::new(0);
    let corrupted = AtomicUsize::new(0);
    let results: Vec<FileResult> = pool(opts.workers).install(|| {
        files
            .par_iter()
            .map(|p| {
                let r = inspect_file(&p.to_string_lossy());
                if r.verdict == VERDICT_CORRUPTED {
                    corrupted.fetch_add(1, Ordering::Relaxed);
                }
                let i = done.fetch_add(1, Ordering::Relaxed) + 1;
                if opts.progress_every > 0 && i.is_multiple_of(opts.progress_every) {
                    let secs = started.elapsed().as_secs_f64().max(0.001);
                    let rate = i as f64 / secs;
                    let eta = (n - i) as f64 / rate;
                    log(&format!(
                        "INFO   {}/{} ({:.1}%) {:.0}건/s ETA {:.0}분  손상 {}",
                        thousands(i),
                        thousands(n),
                        i as f64 / n as f64 * 100.0,
                        rate,
                        eta / 60.0,
                        thousands(corrupted.load(Ordering::Relaxed))
                    ));
                }
                r
            })
            .collect()
    });

    let mut counts: IndexMap<String, usize> = IndexMap::new();
    let mut per_category: IndexMap<String, IndexMap<String, usize>> = IndexMap::new();
    for r in &results {
        *counts.entry(r.verdict.clone()).or_default() += 1;
        let cat = r.category.as_deref().filter(|c| !c.is_empty()).unwrap_or("(기타)");
        *per_category.entry(cat.to_owned()).or_default().entry(r.verdict.clone()).or_default() += 1;
    }
    let pick = |v: &str| results.iter().filter(|r| r.verdict == v).cloned().collect::<Vec<_>>();
    ScanReport {
        scanned_at: now_iso(),
        root: root.to_string_lossy().into_owned(),
        threshold: MOJIBAKE_LIKENESS_THRESHOLD,
        total: results.len(),
        counts,
        elapsed_sec: round1(started.elapsed().as_secs_f64()),
        per_category,
        corrupted: pick(VERDICT_CORRUPTED),
        unreadable: pick(VERDICT_UNREADABLE),
        undetermined: pick(VERDICT_UNDETERMINED),
    }
}

pub fn print_summary(report: &ScanReport, out: &mut dyn Write) -> io::Result<()> {
    let total = report.total.max(1) as f64;
    writeln!(out, "\n검사 {}건 / {:.1}분  (임계값 likeness < {})", thousands(report.total), report.elapsed_sec / 60.0, report.threshold)?;
    for verdict in [VERDICT_CORRUPTED, VERDICT_CLEAN, VERDICT_UNDETERMINED, VERDICT_UNREADABLE] {
        let n = report.counts.get(verdict).copied().unwrap_or(0);
        writeln!(out, "  {verdict:<14} {:>8}  ({:.2}%)", thousands(n), n as f64 / total * 100.0)?;
    }

    // (손상, 전체, 카테고리) 내림차순. 손상이 0건인 카테고리는 눈으로 훑을 목록에서 뺀다.
    let rows: BTreeSet<(usize, usize, &String)> = report
        .per_category
        .iter()
        .filter_map(|(cat, b)| {
            let bad = b.get(VERDICT_CORRUPTED).copied().unwrap_or(0);
            (bad > 0).then(|| (bad, b.values().sum(), cat))
        })
        .collect();
    if !rows.is_empty() {
        writeln!(out, "\n손상이 있는 카테고리 {}개", rows.len())?;
        writeln!(out, "  {:<26}{:>8}{:>8}{:>8}", "카테고리", "손상", "전체", "비율")?;
        for (bad, tot, cat) in rows.iter().rev().take(40) {
            writeln!(out, "  {cat:<26}{:>8}{:>8}{:>8}", thousands(*bad), thousands(*tot), format!("{:.1}%", *bad as f64 / *tot as f64 * 100.0))?;
        }
    }
    Ok(())
}
