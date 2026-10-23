//! 디렉토리별 이름 변경 계획을 세우고, 리포트로 쓰고, 실제로 바꾼다.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cache::{BookstoreSearchCache, StoreRow};
use crate::names::*;
use unicode_normalization::UnicodeNormalization;

/// 조회 순서대로 서점 이름.
pub const BOOKSTORES: [&str; 3] = ["yes24", "aladin", "kyobo"];

/// 서점 검색. 실제 조회는 이 trait 뒤에 둔다.
pub trait Searcher {
    /// 실제로 서점에 물어볼 수 있는가. `false`면 캐시에 없는 질의는 건너뛴다.
    fn is_live(&self) -> bool;
    /// 서점 한 곳에서 `title`(과 `author`)로 검색한다. 오류는 빈 결과로 캐시에 남는다.
    fn search(&self, store: &str, title: &str, author: &str) -> Result<Vec<StoreRow>, String>;
}

/// 디렉토리 하나의 처리 계획.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub source: PathBuf,
    pub target: Option<PathBuf>,
    pub confidence: f64,
    pub sample_count: usize,
    pub status: String,
    pub evidence: String,
}

fn skip(source: &Path, confidence: f64, count: usize, status: &str, evidence: &str) -> Proposal {
    Proposal { source: source.to_owned(), target: None, confidence, sample_count: count, status: status.to_owned(), evidence: evidence.to_owned() }
}

/// 서점 투표로 정한 제목과 저자.
#[derive(Debug, Clone, PartialEq)]
pub struct Canonical {
    pub title: Option<String>,
    pub author: Option<String>,
    pub evidence: String,
    /// 서점별 한 글자. `+` 캐시, `-` 요청, `.` 건너뜀, `!` 캐시에 없고 조회도 못 함
    pub store_status: String,
    /// 캐시에 없는 서점이 있어 결과가 불완전하다
    pub unavailable: bool,
}

fn normalized_base(row_title: &str) -> String {
    row_title.nfkc().collect::<String>().trim().to_owned()
}

/// 서점들의 검색 결과에서 제목·저자를 다수결로 정한다. 제목과 한글 저자가 나오면 남은 서점은 묻지 않는다.
pub fn canonical_book_title(title: &str, author: Option<&str>, stores: &[&str], delay: f64, query: &str, cache: &mut BookstoreSearchCache, searcher: &dyn Searcher) -> Canonical {
    // (서점, 기본 제목, 저자)
    let mut results: Vec<(String, String, String)> = Vec::new();
    let mut rejected: Vec<String> = Vec::new();
    let cached_by_store: Vec<Option<Vec<StoreRow>>> = stores.iter().map(|s| cache.get(query, s)).collect();
    let mut status: Vec<char> = cached_by_store.iter().map(|c| if c.is_some() { '+' } else { '.' }).collect();

    for (index, store) in stores.iter().enumerate() {
        let rows = match &cached_by_store[index] {
            Some(rows) => rows.clone(),
            None if !searcher.is_live() => {
                status[index] = '!';
                return Canonical { title: None, author: None, evidence: "bookstore search unavailable; query not in cache".into(), store_status: status.iter().collect(), unavailable: true };
            }
            None => {
                status[index] = '-';
                cache.searches += 1;
                cache.wait_for_request_slot(delay);
                let started = Instant::now();
                let books = match searcher.search(store, title, author.unwrap_or("")) {
                    Ok(rows) => rows.into_iter().map(|r| StoreRow { title: r.title.trim().to_owned(), author: r.author.trim().to_owned(), ..r }).collect(),
                    Err(e) => {
                        eprintln!("{store} 검색 실패: {e}");
                        Vec::new()
                    }
                };
                cache.search_seconds += started.elapsed().as_secs_f64();
                cache.last_request_completed_at = Some(Instant::now());
                cache.put(query, store, books.clone());
                books
            }
        };

        if let Some(row) = rows.iter().find(|r| bookstore_title_matches(&r.title, title)) {
            results.push((store.to_string(), bookstore_title_base(&row.title), row.author.trim().to_owned()));
        } else if !rows.is_empty() {
            let candidates: Vec<&str> = rows.iter().take(3).map(|r| r.title.as_str()).collect();
            rejected.push(format!("{store} candidates: {}", candidates.join(", ")));
        }

        if most_common(results.iter().map(|r| normalized_base(&r.1))).is_some() && results.iter().any(|r| korean_author(Some(&r.2)).is_some()) {
            break;
        }
    }

    let store_status: String = status.iter().collect();
    let Some((canonical_title, _)) = most_common(results.iter().map(|r| normalized_base(&r.1))) else {
        let evidence = rejected.join("; ");
        return Canonical { title: None, author: None, evidence: if evidence.is_empty() { "no bookstore result".into() } else { evidence }, store_status, unavailable: false };
    };
    let matching: Vec<&(String, String, String)> = results.iter().filter(|r| normalized_base(&r.1) == canonical_title).collect();
    let canonical_author = most_common(matching.iter().filter_map(|r| korean_author(Some(&r.2)))).map(|(a, _)| a);
    let evidence = matching.iter().map(|r| format!("{}: {}", r.0, r.1)).collect::<Vec<_>>().join("; ");
    Canonical { title: Some(canonical_title), author: canonical_author, evidence, store_status, unavailable: false }
}

fn sorted_subdirs(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(parent) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
    dirs.sort();
    dirs
}

/// 대상 카테고리의 모든 디렉토리에 계획을 세운다. 이름 충돌은 마지막에 `SKIP_CONFLICT`로 바꾼다.
pub fn build_proposals(root: &Path, delay: f64, max_searches: usize, cache: &mut BookstoreSearchCache, searcher: &dyn Searcher) -> Vec<Proposal> {
    let started_at = Instant::now();
    let mut proposals: Vec<Proposal> = Vec::new();
    let author_references = local_author_references(root);
    let mut query_cache: HashMap<(String, String), Canonical> = HashMap::new();
    let mut queries_processed = 0usize;
    eprintln!("Store status [yes24/aladin/kyobo]: +=cache, -=request, .=skipped, !=not cached and no live search");

    for category in TARGET_CATEGORIES {
        for directory in sorted_subdirs(&root.join(category)) {
            let identity = infer_identity(&directory);
            let (confidence, count) = (identity.confidence, identity.file_count);
            let Some(title) = identity.title.clone() else {
                proposals.push(skip(&directory, confidence, count, identity.failure_status.as_deref().unwrap_or("SKIP_UNPARSEABLE_FILENAMES"), &identity.failure_reason));
                continue;
            };
            let dir_name = directory.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let original_author = directory_author(&dir_name);
            let Some(metadata) = normalized_directory_metadata(&directory) else {
                proposals.push(skip(&directory, confidence, count, "SKIP_NO_RANGE", ""));
                continue;
            };
            let query_author = original_author.clone().or_else(|| identity.author.clone());
            let query_key = (title_key(&title), title_key(query_author.as_deref().unwrap_or("")));
            let cache_key = format!("{}|{}", query_key.0, query_key.1);
            if !query_cache.contains_key(&query_key) {
                let has_any_cached_store = BOOKSTORES.iter().any(|s| cache.contains(&cache_key, s));
                if max_searches > 0 && queries_processed >= max_searches && !has_any_cached_store {
                    proposals.push(skip(&directory, confidence, count, "SKIP_SEARCH_LIMIT", ""));
                    continue;
                }
                let canonical = canonical_book_title(&title, query_author.as_deref(), &BOOKSTORES, delay, &cache_key, cache, searcher);
                queries_processed += 1;
                eprint!("{} ", canonical.store_status);
                query_cache.insert(query_key.clone(), canonical);
                if queries_processed.is_multiple_of(25) {
                    eprintln!("\nTitle queries: {queries_processed}; store requests: {}; store cache hits: {}; bookstore time: {:.1}s", cache.searches, cache.hits, cache.search_seconds);
                }
            }
            let canonical = &query_cache[&query_key];
            let Some(book_title) = &canonical.title else {
                let status = if canonical.unavailable { "SKIP_NOT_CACHED" } else { "SKIP_NO_BOOKSTORE_MATCH" };
                proposals.push(skip(&directory, confidence, count, status, &canonical.evidence));
                continue;
            };
            let resolved_author = korean_author(original_author.as_deref())
                .or_else(|| korean_author(identity.author.as_deref()))
                .or_else(|| matching_local_author(book_title, &author_references))
                .or_else(|| korean_author(canonical.author.as_deref()));
            let Some(resolved_author) = resolved_author else {
                proposals.push(skip(&directory, confidence, count, "SKIP_NO_AUTHOR", &canonical.evidence));
                continue;
            };
            let Some(target_name) = filesystem_safe_name(&format!("[{resolved_author}] {book_title} {metadata}")) else {
                proposals.push(skip(&directory, confidence, count, "SKIP_INVALID_NAME", &canonical.evidence));
                continue;
            };
            let target = directory.with_file_name(target_name);
            let status = if target == directory { "UNCHANGED" } else { "RENAME" };
            proposals.push(Proposal { source: directory, target: Some(target), confidence, sample_count: count, status: status.into(), evidence: canonical.evidence.clone() });
        }
    }

    eprintln!(
        "\nProposal scan: {:.1}s; directories: {}; title queries: {queries_processed}; store requests: {}; store cache hits: {}; bookstore time: {:.1}s",
        started_at.elapsed().as_secs_f64(),
        proposals.len(),
        cache.searches,
        cache.hits,
        cache.search_seconds
    );

    let planned_sources: HashSet<PathBuf> = proposals.iter().filter(|p| p.status == "RENAME").map(|p| p.source.clone()).collect();
    let mut target_counts: HashMap<PathBuf, usize> = HashMap::new();
    for p in proposals.iter().filter(|p| p.status == "RENAME") {
        if let Some(t) = &p.target {
            *target_counts.entry(t.clone()).or_insert(0) += 1;
        }
    }
    for p in proposals.iter_mut().filter(|p| p.status == "RENAME") {
        let Some(target) = &p.target else { continue };
        let occupied = target.exists() && !planned_sources.contains(target);
        let duplicate = target_counts[target] > 1;
        if occupied || duplicate {
            p.status = "SKIP_CONFLICT".into();
        }
    }
    proposals
}

/// csv.writer(QUOTE_MINIMAL)와 같다: 구분자·따옴표·줄바꿈이 들어 있으면 따옴표로 감싸고 `"`는 겹쳐 쓴다.
fn csv_field(value: &str) -> String {
    if value.contains(['\t', '"', '\r', '\n']) { format!("\"{}\"", value.replace('"', "\"\"")) } else { value.to_owned() }
}

/// 탭 구분 리포트(UTF-8 BOM, CRLF)를 쓴다.
pub fn write_report(proposals: &[Proposal], path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = Vec::from(*b"\xef\xbb\xbf");
    let mut row = |fields: [String; 6]| {
        let line = fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join("\t");
        let _ = write!(out, "{line}\r\n");
    };
    row(["status", "confidence", "book_files", "source", "target", "evidence"].map(String::from));
    for item in proposals {
        row([
            item.status.clone(),
            format!("{:.3}", item.confidence),
            item.sample_count.to_string(),
            item.source.display().to_string(),
            item.target.as_ref().map(|t| t.display().to_string()).unwrap_or_default(),
            item.evidence.clone(),
        ]);
    }
    fs::write(path, out)
}

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn temporary_name(source: &Path) -> PathBuf {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    source.with_file_name(format!(".normalize-{:x}{:x}{n:x}", std::process::id(), nanos))
}

/// `RENAME` 계획을 적용한다. 먼저 모두 임시 이름으로 옮긴 뒤 목표 이름으로 바꿔서, 서로 이름을 맞바꾸는 경우도 된다.
/// 중간에 실패하면 원래 이름으로 되돌린다.
pub fn apply_renames(proposals: &[Proposal]) -> Result<usize, String> {
    let renames: Vec<(&Path, &Path)> = proposals.iter().filter(|p| p.status == "RENAME").filter_map(|p| p.target.as_deref().map(|t| (p.source.as_path(), t))).collect();
    let sources: HashSet<&Path> = renames.iter().map(|(s, _)| *s).collect();
    if renames.iter().any(|(_, t)| t.exists() && !sources.contains(t)) {
        return Err("A destination appeared after preflight; no rename was started.".into());
    }

    // (원래 이름, 임시 이름, 목표 이름)
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
    let attempt = (|| -> Result<(), String> {
        for (source, target) in &renames {
            let temporary = temporary_name(source);
            fs::rename(source, &temporary).map_err(|e| format!("{}: {e}", source.display()))?;
            staged.push((source.to_path_buf(), temporary, target.to_path_buf()));
        }
        for (_, temporary, target) in &staged {
            if target.exists() {
                return Err(format!("File exists: {}", target.display()));
            }
            fs::rename(temporary, target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        Ok(())
    })();
    if let Err(e) = attempt {
        for (source, temporary, target) in staged.iter().rev() {
            let current = if target.exists() { target } else { temporary };
            if current.exists() && !source.exists() {
                let _ = fs::rename(current, source);
            }
        }
        return Err(e);
    }
    Ok(renames.len())
}
