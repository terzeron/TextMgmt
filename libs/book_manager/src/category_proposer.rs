//! 분류 제안과 적용(분류된 파일 옮기기 포함).
//! `_match_category_by_keywords`, `_propose_category_for_file`에 대응한다.
//!
//! 분류 제안은 근거별 점수의 가중치 합산이다(`evidence_scorer`). 근거는 같은 ISBN 사본의 위치, 카테고리 매핑 키워드,
//! `BookManagerDeps::with_classifier_factory`로 꽂은 `Classify` 모델이다. 모델이 없으면 앞의 둘만 쓴다.

use std::collections::HashSet;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{BookManager, ProgressFn};
use crate::bookstore_evidence::{BookstoreLookup, StoreVote, bookstore_scores, store_vote};
use crate::bookstore_query_builder::query_from_file;
use crate::evidence_scorer::{Evidence, combine, detail_of, isbn_scores, keyword_scores};
use crate::path_validator::relative_to;
use classifier::Classify;
use classifier::feature_hasher::BookFields;
use search_manager::{Docs, ISBN_MATCH_LIMIT};

/// 제안 시작 때 이름만 담은 행을 한 번에 보내는 개수.
const PROPOSAL_PLACEHOLDER_CHUNK: usize = 500;

/// `propose_category_changes` 옵션. `None`은 기본값(`use_bookstore=true`, `use_content_meta=true`)이다.
#[derive(Default)]
pub struct ProposeOptions<'a> {
    pub content_type: Option<&'a str>,
    pub delay: Option<f64>,
    pub use_bookstore: Option<bool>,
    pub use_content_meta: Option<bool>,
    pub on_progress: Option<ProgressFn<'a>>,
}

/// `apply_category_changes` 옵션.
#[derive(Default)]
pub struct ApplyOptions<'a> {
    /// `None`이면 `"book"`.
    pub content_type: Option<&'a str>,
    pub clean_existing: bool,
    pub on_progress: Option<ProgressFn<'a>>,
    /// 한 건 처리 직전·직후에 부른다. 호출자가 그 한 건만 저장할 수 있다.
    pub on_item_done: Option<ProgressFn<'a>>,
    /// `false`를 돌려주면 즉시 멈춘다(다른 작업이 이 작업을 대체했다).
    pub should_continue: Option<&'a (dyn Fn() -> bool + Send + Sync)>,
}

/// `_move_classified_file` 옵션.
#[derive(Debug, Clone, Default)]
pub struct MoveOpts {
    /// 옮기지 않고 무엇을 할지만 돌려준다.
    pub dry_run: bool,
    /// 대상에 같은 내용의 파일이 이미 있으면 원본을 지우고 ES 문서를 정리한다.
    pub clean_existing: bool,
    /// 빈 디렉터리를 정리할 때 기준이 되는 원래 카테고리.
    pub source_category: Option<String>,
}

/// 목적지 후보 하나. `source`는 합산에 가장 크게 기여한 근거(isbn·keyword·model), `detail`은 근거별 원래 점수다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CategoryCandidate {
    pub category: String,
    pub source: String,
    pub detail: String,
    /// 가중치 합산 점수(0~1).
    pub score: f64,
}

/// 파일 하나의 분류 제안. 파일을 옮기지 않는다. 기본값은 아직 분류하지 않은 자리다(`classified: false`).
/// 예전 payload의 `grade`·`confidence`·`expected_accuracy`·`source`는 읽을 때 무시한다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Proposal {
    /// 합산 점수 1등 카테고리. 근거가 하나도 없으면 `None`.
    pub target_category: Option<String>,
    /// 1등의 합산 점수.
    pub score: Option<f64>,
    /// 분류를 마쳤는가. 이름만 깔아 둔 자리는 `false`다.
    pub classified: bool,
    pub matched_keywords: Vec<String>,
    /// 모델 단독 1등.
    pub model_category: Option<String>,
    pub reason: String,
    pub candidates: Vec<CategoryCandidate>,
    /// 서점별 표. 서점 근거를 쓰지 않았으면 비어 있다.
    pub bookstore_votes: Vec<StoreVote>,
}

/// 제안 항목의 적용 상태.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyStatus {
    #[default]
    Pending,
    /// 옮기기 시작했다. 도중에 죽으면 이 상태로 남는다.
    Moving,
    Moved,
    Failed,
}

/// 제안 목록의 한 행.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProposalItem {
    pub file_path: String,
    pub book_id: Option<i64>,
    pub title: String,
    pub current_category: String,
    #[serde(flatten)]
    pub proposal: Proposal,
    pub apply_status: ApplyStatus,
    pub apply_error: Option<String>,
}

/// 처리하지 못한 파일.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemFailure {
    pub file_path: String,
    pub error: String,
}

/// 카테고리 하나의 분류 제안 결과.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProposeResult {
    pub content_type: String,
    pub source_category: String,
    pub total_count: usize,
    pub processed_count: usize,
    pub items: Vec<ProposalItem>,
    pub failures: Vec<ItemFailure>,
}

/// 승인 항목 하나의 적용 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppliedItem {
    pub file_path: Option<String>,
    pub target_category: Option<String>,
    pub apply_status: ApplyStatus,
    pub apply_error: Option<String>,
}

/// 분류 적용 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApplyResult {
    pub content_type: String,
    pub total_count: usize,
    pub applied_count: i64,
    pub failed_count: i64,
    pub results: Vec<AppliedItem>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveStatus {
    /// 옮기지 않고 할 일만 알렸다.
    DryRun,
    #[default]
    Moved,
    /// 대상에 같은 내용이 있어 원본을 지웠다.
    DuplicateCleaned,
}

/// 미리보기에서 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveAction {
    /// 이미 목적지에 있다.
    Noop,
    DuplicateClean,
    Move,
}

/// 분류된 파일 하나의 이동 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MoveOutcome {
    pub status: MoveStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<MoveAction>,
    pub from: String,
    pub to: String,
    /// 새로 색인한 문서 ID. 실제로 옮겼을 때만 있다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_id: Option<i64>,
    pub target_category: String,
    pub deleted_count: i64,
}

/// 키워드 점수 순위: (카테고리, 맞은 키워드들).
pub type Ranked = Vec<(String, Vec<String>)>;

/// `_match_category_by_keywords`의 결과.
pub struct KeywordMatch {
    /// 고른 카테고리. 동점이거나 맞은 키워드가 없으면 `None`.
    pub category: Option<String>,
    /// 고른 카테고리에서 맞은 키워드.
    pub keywords: Vec<String>,
    /// 동점이라 고르지 못한 이유.
    pub tie_reason: Option<String>,
    pub ranked: Ranked,
}

/// JSON 객체 `{카테고리: [키워드...]}`를 입력 순서를 지킨 목록으로 바꾼다. 키워드 배열이 아니면 빈 목록이다.
fn mappings_from_value(mappings: &Value) -> Vec<(Value, Vec<Value>)> {
    mappings.as_object().into_iter().flatten().map(|(category, keywords)| (json!(category), keywords.as_array().cloned().unwrap_or_default())).collect()
}

fn normalize_text(value: &str) -> String {
    let folded = value.to_lowercase();
    folded.split(char::is_whitespace).filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ")
}

/// 같은 크기의 두 파일이 바이트까지 같은가. 읽기에 실패하면 보수적으로 `false`다.
fn same_content(a: &Path, b: &Path) -> bool {
    let compare = || -> std::io::Result<bool> {
        if std::fs::metadata(a)?.len() != std::fs::metadata(b)?.len() {
            return Ok(false);
        }
        let (mut fa, mut fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
        let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
        loop {
            let na = read_full(&mut fa, &mut ba)?;
            let nb = read_full(&mut fb, &mut bb)?;
            if na != nb || ba[..na] != bb[..nb] {
                return Ok(false);
            }
            if na == 0 {
                return Ok(true);
            }
        }
    };
    compare().unwrap_or_else(|e| {
        log::warn!("_is_duplicate_content: 비교 실패 ({} vs {}): {e}", a.display(), b.display());
        false
    })
}

fn read_full(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

/// `name.ext`가 이미 있을 때 `name (1).ext`, `name (2).ext` 순으로 빈 자리를 찾는다.
pub fn next_numbered_path_with(target: &Path, max_index: usize) -> Option<PathBuf> {
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = target.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..=max_index).map(|n| target.with_file_name(format!("{stem} ({n}){suffix}"))).find(|candidate| !candidate.exists())
}

impl BookManager {
    /// book_id가 없는 제안 항목에 inode를 채운다. 이미 있는 항목은 stat 하지 않는다.
    pub fn fill_missing_book_ids(&self, mut items: Vec<Value>, path_prefix: &Path) -> Vec<Value> {
        for item in &mut items {
            if item.get("book_id").is_some_and(|v| !v.is_null()) {
                continue;
            }
            let Some(rel_path) = item.get("file_path").and_then(Value::as_str).filter(|p| !p.is_empty()) else { continue };
            item["book_id"] = json!(book_id_for_path(&path_prefix.join(rel_path)));
        }
        items
    }

    /// 카테고리 직하위(또는 하위 전체)의 색인 대상 파일을 정렬해서 돌려준다.
    pub fn iter_category_indexable_files(&self, category: &str, recursive: bool) -> Vec<PathBuf> {
        if let Some(f) = &self.iter_category_indexable_files_override {
            return f(category, recursive);
        }
        let category_dir = self.category_dir(category);
        let Ok(root) = self.resolved_root() else { return Vec::new() };
        let mut files: Vec<PathBuf> = Vec::new();
        let mut stack = vec![category_dir.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = self.fs.read_dir(&dir) else { continue };
            for entry in entries {
                let Ok(relative) = entry.strip_prefix(&category_dir) else { continue };
                if relative.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
                    continue;
                }
                if crate::path_validator::is_dir_nofollow(&entry) {
                    if recursive {
                        stack.push(entry);
                    }
                    continue;
                }
                if !crate::path_validator::is_regular_file_nofollow(&entry) {
                    continue;
                }
                match self.resolve_path(&entry) {
                    Ok(resolved) if resolved.starts_with(&root) => {}
                    _ => continue,
                }
                if self.is_indexable_file_path(&entry) {
                    files.push(entry);
                }
            }
        }
        files.sort();
        files
    }

    /// 파일 이름, 확장자 없는 이름, 책 루트 기준 상위 디렉터리 이름을 이어 정규화한 문자열. 책 루트 밖이면 전체 상위 경로를 쓴다.
    pub fn classification_haystack(&self, file_path: &Path) -> String {
        let name = file_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = file_path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = file_path.parent().unwrap_or(Path::new(""));
        let parent_parts: Vec<String> = match parent.strip_prefix(&self.path_prefix) {
            Ok(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect(),
            Err(_) => parent.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect(),
        };
        let mut parts = vec![name, stem];
        parts.extend(parent_parts);
        normalize_text(&parts.join(" "))
    }

    pub fn match_category_by_keywords(&self, file_path: &Path, source_category: &str, mappings: &[(Value, Vec<Value>)]) -> KeywordMatch {
        let haystack = self.classification_haystack(file_path);
        let mut scored: Vec<(i64, usize, String, Vec<String>)> = Vec::new();
        for (raw_category, raw_keywords) in mappings {
            let Some(raw_category) = raw_category.as_str() else { continue };
            let target = raw_category.trim();
            if target == source_category || !crate::path_validator::is_top_level_target_category(target) || !is_recommendable_category(target) || !self.is_safe_category_name(target) {
                continue;
            }
            let mut matched: Vec<String> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for raw_keyword in raw_keywords.iter().filter_map(Value::as_str) {
                let keyword = raw_keyword.trim();
                let normalized = normalize_text(keyword);
                if normalized.is_empty() || !seen.insert(normalized.clone()) {
                    continue;
                }
                if haystack.contains(&normalized) {
                    matched.push(keyword.to_string());
                }
            }
            if !matched.is_empty() {
                let score: i64 = matched.iter().map(|k| normalize_text(k).chars().count() as i64).sum();
                scored.push((score, matched.len(), target.to_string(), matched));
            }
        }
        if scored.is_empty() {
            return KeywordMatch { category: None, keywords: Vec::new(), tie_reason: None, ranked: Vec::new() };
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        let ranked: Ranked = scored.iter().take(2).map(|(_, _, c, k)| (c.clone(), k.clone())).collect();
        let (best_score, best_count) = (scored[0].0, scored[0].1);
        let tied: Vec<&str> = scored.iter().filter(|s| s.0 == best_score && s.1 == best_count).map(|s| s.2.as_str()).collect();
        let best_keywords = scored[0].3.clone();
        if tied.len() > 1 {
            let names = tied.iter().take(3).copied().collect::<Vec<_>>().join(", ");
            return KeywordMatch { category: None, keywords: best_keywords, tie_reason: Some(format!("여러 카테고리가 동일 점수로 일치합니다: {names}")), ranked };
        }
        KeywordMatch { category: Some(scored[0].2.clone()), keywords: best_keywords, tie_reason: None, ranked }
    }

    /// 한 파일의 제안을 만든다. 파일을 옮기지 않는다.
    ///
    /// 근거는 ISBN(같은 ISBN 사본이 있는 디렉토리), 키워드, 모델 셋이다. 있는 근거만 가중치로 합산해 점수 내림차순 상위 2개를 후보로 낸다.
    /// ES 문서는 inode로 한 번만 읽는다. 읽지 못하면 ISBN·모델 근거 없이 키워드로만 판정하고 `reason`에 남긴다.
    pub fn propose_category_for_file(&self, file_path: &Path, source_category: &str, mappings: &[(Value, Vec<Value>)], model: Option<&dyn Classify>) -> Proposal {
        self.propose_category_for_file_with(file_path, source_category, mappings, model, None, &[]).0
    }

    /// `propose_category_for_file`에 서점 근거를 더한 판. `categories`는 서점 표를 대응시킬 카테고리 목록이다.
    /// 반환의 두 번째 값은 서점을 실제로 불렀는지다(캐시로 끝나면 거짓).
    pub fn propose_category_for_file_with(
        &self,
        file_path: &Path,
        source_category: &str,
        mappings: &[(Value, Vec<Value>)],
        model: Option<&dyn Classify>,
        bookstore: Option<&dyn BookstoreLookup>,
        categories: &[String],
    ) -> (Proposal, bool) {
        let keyword = self.match_category_by_keywords(file_path, source_category, mappings);
        let own_id = book_id_for_path(file_path);
        let mut notes: Vec<String> = Vec::new();
        let doc = match own_id.map(|id| self.es.search_by_id(id)) {
            Some(Ok(doc)) => doc.filter(|d| d.as_object().is_some_and(|o| !o.is_empty())),
            Some(Err(error)) => {
                notes.push(format!("ES 조회 실패: {error}"));
                None
            }
            None => None,
        };

        let mut parts: Vec<(Evidence, Vec<(String, f64)>)> = Vec::new();
        if let Some(isbn) = doc.as_ref().and_then(|d| d.get("isbn")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
            match self.es.search_by_isbn(isbn, ISBN_MATCH_LIMIT) {
                // 상한은 자기 문서를 빼기 전 건수로 본다. 빼고 세면 상한에 걸린 흔한 값이 49건으로 빠져나간다.
                Ok(rows) if rows.len() < ISBN_MATCH_LIMIT => {
                    let paths: Vec<String> = rows.into_iter().filter(|(id, path)| Some(*id) != own_id && is_recommendable_category(path)).map(|(_, path)| path).collect();
                    parts.extend(isbn_scores(&paths).map(|scores| (Evidence::Isbn, scores)));
                }
                Ok(_) => {}
                Err(error) => notes.push(format!("ISBN 조회 실패: {error}")),
            }
        }
        parts.extend(keyword_scores(&keyword.ranked).map(|scores| (Evidence::Keyword, scores)));

        let mut bookstore_votes = Vec::new();
        let mut network_used = false;
        if let (Some(lookup), Some(id)) = (bookstore, own_id) {
            let file_stem = file_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let isbn = doc.as_ref().and_then(|d| d.get("isbn")).and_then(Value::as_str);
            let outcome = lookup.lookup(id, &query_from_file(&file_stem, isbn));
            network_used = outcome.network_used;
            let keyword_map = keyword_map(mappings);
            let allowed = |c: &str| is_recommendable_category(c) && crate::path_validator::is_top_level_target_category(c) && self.is_safe_category_name(c) && self.category_dir(c).is_dir();
            bookstore_votes = outcome.results.iter().map(|r| store_vote(r, categories, &keyword_map, &allowed)).collect();
            parts.extend(bookstore_scores(&bookstore_votes).map(|scores| (Evidence::Bookstore, scores)));
        }

        let mut model_category = None;
        if let (Some(model), Some(doc)) = (model, &doc)
            && !BookFields::from_es_source(doc).is_empty()
        {
            match model.classify_document(doc) {
                Ok(prediction) => {
                    // 디스크에 디렉토리가 없는 카테고리는 옮길 수 없다. 남은 것끼리 합 1로 다시 맞춘다.
                    let kept: Vec<(String, f64)> =
                        prediction.ranked.into_iter().filter(|(c, p)| *p > 0.0 && is_recommendable_category(c) && self.is_safe_category_name(c) && self.category_dir(c).is_dir()).collect();
                    let total: f64 = kept.iter().map(|(_, p)| p).sum();
                    if total > 0.0 {
                        model_category = kept.first().map(|(c, _)| c.clone());
                        parts.push((Evidence::Model, kept.into_iter().map(|(c, p)| (c, p / total)).collect()));
                    }
                }
                Err(error) => notes.push(format!("모델 판정 실패: {error}")),
            }
        }

        let ranked = combine(&parts);
        if ranked.is_empty() {
            notes.push("근거가 없습니다".to_string());
        }
        let candidates = ranked.iter().take(2).map(|c| CategoryCandidate { category: c.category.clone(), source: c.dominant().name().to_string(), detail: detail_of(c), score: c.score }).collect();
        let proposal = Proposal {
            target_category: ranked.first().map(|c| c.category.clone()),
            score: ranked.first().map(|c| c.score),
            classified: true,
            matched_keywords: keyword.keywords,
            model_category,
            reason: notes.join("; "),
            candidates,
            bookstore_votes,
        };
        (proposal, network_used)
    }

    /// 책 루트 바로 아래의 카테고리 디렉토리 이름(숨김 제외, 이름순). 서점 표를 대응시킬 목록이다.
    fn top_level_categories(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.path_prefix)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|e| crate::path_validator::is_dir_nofollow(&e.path()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|name| !name.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// 선택 카테고리 직하위 파일의 분류 제안을 만든다. 파일을 옮기지 않는다.
    pub async fn propose_category_changes(&self, category: &str, mappings: Option<&Value>, options: ProposeOptions<'_>) -> Result<ProposeResult, String> {
        if category.is_empty() {
            return Err("카테고리 이름이 비어있습니다".to_string());
        }
        if !self.is_safe_category_name(category) {
            return Err("잘못된 카테고리 경로입니다".to_string());
        }
        if !self.category_dir(category).is_dir() {
            return Err(format!("디렉토리를 찾을 수 없습니다: {category}"));
        }

        let content_type = options.content_type.unwrap_or("book");
        // 서점 근거는 책에만 쓴다. 만화는 웹툰 서점 규칙이 따로 있다.
        let bookstore = match &self.bookstore_lookup_factory {
            Some(factory) if options.use_bookstore.unwrap_or(true) && content_type != "comic" => factory(),
            _ => None,
        };
        let bookstore_categories = if bookstore.is_some() { self.top_level_categories() } else { Vec::new() };
        let delay = std::time::Duration::from_secs_f64(options.delay.unwrap_or(0.0).max(0.0));
        let use_content_meta = options.use_content_meta.unwrap_or(true);
        let empty = json!({});
        let mappings = mappings.unwrap_or(&empty);
        // 꽂은 모델이 있고 본문·메타데이터 판정을 켰을 때만 모델을 쓴다. 팩토리는 제안 한 번에 한 번 부른다.
        let model: Option<Arc<dyn Classify>> = match &self.classifier_factory {
            Some(factory) if use_content_meta => factory(),
            _ => None,
        };
        let mappings = mappings_from_value(mappings);

        let propose_hook = self.seams().propose_hook;
        let file_paths = self.iter_category_indexable_files(category, false);
        let total_count = file_paths.len();
        let mut processed_count = 0usize;
        let mut items: Vec<ProposalItem> = Vec::new();
        let mut failures: Vec<ItemFailure> = Vec::new();

        // 진행 보고는 호출자가 그대로 저장하는 JSON 객체다. 항목 목록 키 하나와 건수를 싣는다.
        let report = |processed: usize, key: &str, list: &[ProposalItem]| {
            if let Some(callback) = options.on_progress {
                callback(json!({"total_count": total_count, "processed_count": processed, key: list}));
            }
        };
        let item_for = |file_path: &Path, rel_path: String, proposal: Proposal| ProposalItem {
            file_path: rel_path,
            book_id: book_id_for_path(file_path),
            title: file_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            current_category: category.to_string(),
            proposal,
            apply_status: ApplyStatus::Pending,
            apply_error: None,
        };

        // 분류를 시작하기 전에 대상 파일의 이름만 담은 행을 먼저 깔아 둔다.
        let placeholders: Vec<ProposalItem> = file_paths.iter().filter_map(|file_path| Some(item_for(file_path, relative_to(file_path, &self.path_prefix)?, Proposal::default()))).collect();
        for chunk in placeholders.chunks(PROPOSAL_PLACEHOLDER_CHUNK) {
            report(processed_count, "new_items", chunk);
        }

        for file_path in &file_paths {
            processed_count += 1;
            let Some(rel_path) = relative_to(file_path, &self.path_prefix) else {
                failures.push(ItemFailure { file_path: file_path.to_string_lossy().into_owned(), error: "잘못된 파일 경로입니다".to_string() });
                continue;
            };
            let proposal = match &propose_hook {
                Some(hook) => hook(file_path),
                None => {
                    let (proposal, network_used) = self.propose_category_for_file_with(file_path, category, &mappings, model.as_deref(), bookstore.as_deref(), &bookstore_categories);
                    // 서점을 실제로 부른 파일 뒤에만 쉰다. 캐시로 끝난 파일은 서점에 부담을 주지 않는다.
                    if network_used && !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    Ok(proposal)
                }
            };
            let Ok(proposal) = proposal else {
                log::error!("분류 제안 실패: {rel_path}");
                failures.push(ItemFailure { file_path: rel_path, error: "분류 제안에 실패했습니다".to_string() });
                continue;
            };
            let new_item = item_for(file_path, rel_path, proposal);
            // 진행 보고에는 이번 틱에서 분류가 끝난 항목만 싣는다. 누적 목록은 호출자에게 돌려준다.
            report(processed_count, "updated_items", std::slice::from_ref(&new_item));
            items.push(new_item);
        }

        Ok(ProposeResult { content_type: content_type.to_string(), source_category: category.to_string(), total_count, processed_count, items, failures })
    }

    /// 승인된 항목만 실제로 옮긴다.
    pub async fn apply_category_changes(&self, items: &[Value], allowed_file_paths: &HashSet<String>, on_progress: Option<ProgressFn<'_>>) -> Result<ApplyResult, String> {
        self.apply_category_changes_with(items, allowed_file_paths, ApplyOptions { on_progress, ..ApplyOptions::default() }).await
    }

    /// `apply_category_changes`의 전체 옵션 판. 목적지(`target_category`)는 브라우저가 준 값이라 `allowed_file_paths`와 안전한 이름인지 다시 검증한다.
    pub async fn apply_category_changes_with(&self, items: &[Value], allowed_file_paths: &HashSet<String>, options: ApplyOptions<'_>) -> Result<ApplyResult, String> {
        let content_type = options.content_type.unwrap_or("book");
        let mut results: Vec<AppliedItem> = Vec::new();
        let (mut applied_count, mut failed_count) = (0i64, 0i64);
        let move_hook = self.seams().move_hook;
        let root = self.resolved_root().unwrap_or_else(|_| self.path_prefix.clone());
        // 한 건의 상태 변화. 호출자가 그 한 건만 저장한다.
        let notify = |file_path: Option<&str>, status: ApplyStatus, error: Option<&str>| {
            if let Some(callback) = options.on_item_done {
                callback(json!({"file_path": file_path, "apply_status": status, "apply_error": error}));
            }
        };

        for item in items {
            // 다른 승인 작업이 이 작업을 대체했으면 여기서 즉시 멈춘다.
            if options.should_continue.is_some_and(|f| !f()) {
                log::warn!("분류 적용 중단: 다른 작업이 이 작업을 대체했다");
                break;
            }
            let file_path_value = item.get("file_path").and_then(Value::as_str);
            let target_category = item.get("target_category").and_then(Value::as_str);
            let mut apply_status = ApplyStatus::Failed;
            let mut apply_error: Option<String> = None;

            // 목적지 이름 검사는 한 번만 한다(검사 대역이 호출 횟수를 셀 수 있다). 최상위 이름일 때만 검사한다.
            let safety = target_category
                .filter(|t| allowed_file_paths.contains(file_path_value.unwrap_or("")) && crate::path_validator::is_top_level_target_category(t))
                .map(|t| self.try_is_safe_category_name(t));
            match (file_path_value, target_category) {
                (Some(file_path), _) if !allowed_file_paths.contains(file_path) => apply_error = Some("제안 목록에 없는 파일입니다".to_string()),
                (None, _) => apply_error = Some("제안 목록에 없는 파일입니다".to_string()),
                (Some(_), Some(target)) if !crate::path_validator::is_top_level_target_category(target) => apply_error = Some("옮길 수 없는 카테고리입니다".to_string()),
                // 검사 자체가 파일시스템 오류로 실패해도 그 항목만 실패로 남기고 배치는 계속한다.
                (Some(_), Some(_)) if matches!(safety, Some(Err(_))) => apply_error = Some("분류 적용 중 오류가 발생했습니다".to_string()),
                (Some(_), Some(_)) if !matches!(safety, Some(Ok(true))) => apply_error = Some("옮길 수 없는 카테고리입니다".to_string()),
                (Some(_), None) => apply_error = Some("옮길 수 없는 카테고리입니다".to_string()),
                (Some(file_path), Some(target)) => {
                    let absolute_path = self.path_prefix.join(file_path);
                    // 제안과 승인 사이에 경로가 corpus 밖을 가리키는 링크로 바뀔 수 있어, 링크 여부와 실제 위치를 다시 확인한다.
                    if std::fs::symlink_metadata(&absolute_path).is_ok_and(|m| m.file_type().is_symlink()) {
                        apply_error = Some("심볼릭 링크는 옮길 수 없습니다".to_string());
                    } else if !self.fs.is_file(&absolute_path).unwrap_or(false) {
                        apply_error = Some("파일을 찾을 수 없습니다".to_string());
                    } else if !self.resolve_path(&absolute_path).is_ok_and(|p| p.starts_with(&root)) {
                        apply_error = Some("파일 경로가 corpus 밖입니다".to_string());
                    } else {
                        // 제안 시점의 카테고리를 source_category로 써서 빈 디렉터리 정리 대상을 맞춘다.
                        let source_category = file_path.rsplit_once('/').map_or(Self::ROOT_CATEGORY, |(dir, _)| dir);
                        // 실제 이동 직전에 "moving"을 기록한다. 도중에 죽으면 pending이 아니라 moving으로 남아 "시도는 했다"를 알린다.
                        notify(Some(file_path), ApplyStatus::Moving, None);
                        let moved = match &move_hook {
                            Some(hook) => hook(&absolute_path, target),
                            None => {
                                self.move_classified_file(
                                    &absolute_path,
                                    target,
                                    content_type,
                                    MoveOpts { dry_run: false, clean_existing: options.clean_existing, source_category: Some(source_category.to_owned()) },
                                )
                                .await
                            }
                        };
                        match moved {
                            Ok(_) => apply_status = ApplyStatus::Moved,
                            Err(error) => apply_error = Some(error),
                        }
                    }
                }
            }

            if apply_status == ApplyStatus::Moved {
                applied_count += 1;
            } else {
                failed_count += 1;
            }
            notify(file_path_value, apply_status, apply_error.as_deref());
            results.push(AppliedItem { file_path: file_path_value.map(str::to_string), target_category: target_category.map(str::to_string), apply_status, apply_error });
            if let Some(callback) = options.on_progress {
                // 제안 단계의 total_count와 섞이지 않게 apply 전용 키를 쓴다.
                callback(json!({"apply_total_count": items.len(), "applied_count": applied_count, "failed_count": failed_count}));
            }
        }

        Ok(ApplyResult { content_type: content_type.to_string(), total_count: items.len(), applied_count, failed_count, results })
    }

    /// 분류된 파일을 대상 카테고리로 옮긴다. `opts`로 미리보기·중복 정리·빈 디렉터리 기준을 정한다.
    pub async fn move_classified_file(&self, file_path: &Path, target_category: &str, content_type: &str, opts: MoveOpts) -> Result<MoveOutcome, String> {
        let MoveOpts { dry_run, clean_existing, source_category } = opts;
        let source_category = source_category.as_deref();
        let root = self.resolved_root().unwrap_or_else(|_| self.path_prefix.clone());
        let Some(old_rel_path) = relative_to(file_path, &self.path_prefix) else { return Err("잘못된 파일 경로입니다".to_string()) };

        let mut target_path = self.path_prefix.join(target_category).join(crate::path_validator::file_name_of(file_path));
        if !self.resolve_path(&target_path).is_ok_and(|p| p.starts_with(&root)) {
            return Err("잘못된 대상 경로입니다".to_string());
        }
        let mut target_rel_path = relative_to(&target_path, &self.path_prefix).unwrap_or_default();
        let outcome = |status: MoveStatus, action: Option<MoveAction>, to: &str, book_id: Option<i64>, deleted_count: i64| MoveOutcome {
            status,
            action,
            from: old_rel_path.clone(),
            to: to.to_string(),
            book_id,
            target_category: target_category.to_string(),
            deleted_count,
        };
        if self.path_exists(&target_path) {
            let is_same_file = self.fs.same_file(file_path, &target_path).unwrap_or(false);
            if is_same_file && old_rel_path == target_rel_path {
                // 이미 목적지에 있다. 옮길 것이 없다고 답하고 끝낸다.
                let status = if dry_run { MoveStatus::DryRun } else { MoveStatus::Moved };
                return Ok(outcome(status, Some(MoveAction::Noop), &target_rel_path, None, 0));
            }
            if !is_same_file && !same_content(file_path, &target_path) {
                // 이름만 겹치고 내용이 다르면 서로 다른 책이다. 번호를 붙여 둘 다 남긴다.
                let Some(numbered) = next_numbered_path_with(&target_path, self.duplicate_name_max_index) else {
                    return Err(format!("대상 경로에 같은 이름의 파일이 너무 많습니다: {target_rel_path}"));
                };
                target_path = numbered;
                target_rel_path = relative_to(&target_path, &self.path_prefix).unwrap_or_default();
            } else {
                // 내용이 같은 별개 파일이거나, 경로는 다른데 같은 실체인 하드링크다. 둘 다 "대상에 이미 있다"로 본다.
                if clean_existing {
                    if dry_run {
                        return Ok(outcome(MoveStatus::DryRun, Some(MoveAction::DuplicateClean), &target_rel_path, None, 0));
                    }
                    if let Err(e) = self.fs.remove_file(file_path) {
                        return Err(format!("중복 파일 정리 실패: {e}"));
                    }
                    if let Some(source) = source_category {
                        self.clean_empty_parent_dirs(file_path.parent().unwrap_or(Path::new("")), &self.category_dir(source));
                    }
                    return match self.es.delete_by_file_paths(std::slice::from_ref(&old_rel_path), None) {
                        Ok(deleted_count) => Ok(outcome(MoveStatus::DuplicateCleaned, None, &target_rel_path, None, deleted_count)),
                        Err(e) => Err(format!("중복 파일 정리 실패: {e}")),
                    };
                }
                return Err(format!("대상 경로에 파일이 이미 존재합니다: {target_rel_path}"));
            }
        }

        if dry_run {
            return Ok(outcome(MoveStatus::DryRun, Some(MoveAction::Move), &target_rel_path, None, 0));
        }

        let (old_book_id, old_doc): (Option<i64>, Option<Value>) = match self.fs.metadata(file_path) {
            Ok(meta) => {
                let id = meta.ino() as i64;
                match self.find_doc(id) {
                    Ok(doc) => (Some(id), doc),
                    Err(e) => {
                        log::warn!("_move_classified_file: 기존 ES 문서 조회 실패 ({old_rel_path}): {e}");
                        (Some(id), None)
                    }
                }
            }
            Err(e) => {
                log::warn!("_move_classified_file: 기존 ES 문서 조회 실패 ({old_rel_path}): {e}");
                (None, None)
            }
        };

        let old_parent = file_path.parent().unwrap_or(Path::new("")).to_path_buf();
        let moved = match target_path.parent() {
            Some(parent) => std::fs::create_dir_all(parent),
            None => Ok(()),
        }
        .and_then(|()| self.move_file(file_path, &target_path));
        if let Err(e) = moved {
            return Err(format!("파일 이동 실패: {e}"));
        }
        if let Some(source) = source_category {
            self.clean_empty_parent_dirs(&old_parent, &self.category_dir(source));
        }

        let moved: Result<MoveOutcome, String> = (|| {
            let mut deleted_count = self.es.delete_by_file_paths(std::slice::from_ref(&old_rel_path), None).map_err(|e| e.to_string())?;
            let (new_book_id, mut new_doc) = match &old_doc {
                Some(doc) => (std::fs::metadata(&target_path).map_err(|e| e.to_string())?.ino() as i64, doc.clone()),
                None => {
                    let data = self.reader().read_file_with_options(&target_path, Some(&self.path_prefix), content_type == "comic");
                    if data.len() != 1 {
                        return Err(format!("지원하지 않는 파일 형식입니다: {target_rel_path}"));
                    }
                    data.into_iter().next().ok_or_else(|| "지원하지 않는 파일 형식입니다".to_string())?
                }
            };
            new_doc["category"] = json!(target_category);
            new_doc["file_path"] = json!(target_rel_path);
            deleted_count += self.es.delete_by_file_paths(std::slice::from_ref(&target_rel_path), Some(&[new_book_id])).map_err(|e| e.to_string())?;
            match self.add_book_raw(&Docs::from([(new_book_id, new_doc)])) {
                Ok(inserted) if inserted == new_book_id => {}
                Ok(_) => return Err("ES 적재 실패".to_string()),
                Err(error) => return Err(error.to_string()),
            }
            Ok(outcome(MoveStatus::Moved, None, &target_rel_path, Some(new_book_id), deleted_count))
        })();

        match moved {
            Ok(result) => Ok(result),
            Err(error) => {
                let mut rollback_messages: Vec<String> = Vec::new();
                if self.path_exists(&target_path)
                    && !self.path_exists(file_path)
                    && let Err(e) = std::fs::create_dir_all(file_path.parent().unwrap_or(Path::new(""))).and_then(|()| self.move_file(&target_path, file_path))
                {
                    rollback_messages.push(format!("파일 롤백 실패: {e}"));
                }
                if let (Some(id), Some(doc)) = (old_book_id, &old_doc) {
                    match self.add_book_raw(&Docs::from([(id, doc.clone())])) {
                        Ok(restored) if restored == id => {}
                        Ok(_) => rollback_messages.push("기존 ES 문서 복구 실패: None".to_string()),
                        Err(e) => rollback_messages.push(format!("기존 ES 문서 복구 실패: {e}")),
                    }
                }
                let message = if rollback_messages.is_empty() { error } else { format!("{error}; {}", rollback_messages.join("; ")) };
                Err(message)
            }
        }
    }
}

/// 키워드 매핑 목록을 서점 대응용 `{카테고리: [키워드]}`로 바꾼다. 문자열이 아닌 항목은 건너뛴다.
fn keyword_map(mappings: &[(Value, Vec<Value>)]) -> std::collections::BTreeMap<String, Vec<String>> {
    mappings.iter().filter_map(|(c, ks)| Some((c.as_str()?.trim().to_string(), ks.iter().filter_map(Value::as_str).map(str::to_string).collect()))).collect()
}

/// `0_` 카테고리는 보관·작업용이라 자동 추천 후보로 내지 않는다. 직접 선택으로는 옮길 수 있다.
/// 책 루트 기준 상대 경로를 받아도 첫 조각으로 판정된다.
pub fn is_recommendable_category(category_or_path: &str) -> bool {
    !category_or_path.starts_with("0_")
}

/// 편집 화면으로 보낼 book_id. 이 저장소에서 book_id는 파일의 inode 번호다. 파일이 사라졌으면 `None`.
pub fn book_id_for_path(file_path: &Path) -> Option<i64> {
    std::fs::metadata(file_path).ok().map(|m| m.ino() as i64)
}

/// 같은 책의 사본인지 판정한다. 크기가 다르면 즉시 다른 파일이다. 읽기에 실패하면 중복이 아니라고 보수적으로 판정한다.
pub fn is_duplicate_content(source: &Path, target: &Path) -> bool {
    same_content(source, target)
}

pub fn next_numbered_path(target: &Path) -> Option<PathBuf> {
    next_numbered_path_with(target, super::DUPLICATE_NAME_MAX_INDEX)
}
