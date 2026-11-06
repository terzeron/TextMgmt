//! 분류 제안과 적용(분류된 파일 옮기기 포함).
//! `_match_category_by_keywords`, `_propose_category_for_file`에 대응한다.
//!
//! 분류 판정은 두 갈래다.
//!
//! 1. `BookManagerDeps::with_classifier_factory` 로 꽂은 `Classify` 모델(`ModelOnlyClassifier`).
//! 2. 꽂지 않으면 분류기 없이 카테고리 매핑 키워드만 쓴다. `classifier` crate는 trait만 남은 껍데기라 기본 구현이 없다.

use std::collections::HashSet;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{BookManager, ProgressFn};
use crate::path_validator::relative_to;
use classifier::{BookClassifierApi, ModelOnlyClassifier};
use search_manager::Docs;

/// 제안 시작 때 이름만 담은 행을 한 번에 보내는 개수.
const PROPOSAL_PLACEHOLDER_CHUNK: usize = 500;
/// 모델 단독 판정을 '확실'로 볼 최소 확신도. `BookClassifierService.MODEL_TRUST_MIN_CONFIDENCE`와 같다.
pub const MODEL_CERTAIN_MIN_CONFIDENCE: f64 = 0.10;

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

/// 분류 제안 등급. 확실한 제안만 미리 체크한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalGrade {
    Certain,
    Unsure,
    Unknown,
}

/// 목적지 후보 하나. `source`는 근거의 종류(keyword·model·bookstore), `detail`은 사람이 읽는 설명이다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CategoryCandidate {
    pub category: String,
    pub source: String,
    pub detail: String,
}

/// 파일 하나의 분류 제안. 파일을 옮기지 않는다. 기본값은 아직 분류하지 않은 자리다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Proposal {
    pub target_category: Option<String>,
    pub grade: Option<ProposalGrade>,
    pub confidence: Option<f64>,
    /// 확신도를 보정한 예상 정답률.
    pub expected_accuracy: Option<f64>,
    pub source: Option<String>,
    pub matched_keywords: Vec<String>,
    /// 목적지와 다른 곳을 가리킨 모델 판정.
    pub model_category: Option<String>,
    pub reason: String,
    pub candidates: Vec<CategoryCandidate>,
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

/// 분류 판정 한 건(모델 단독).
#[derive(Default)]
struct Classification {
    category: Option<String>,
    method: String,
    reason: String,
    model_category: Option<String>,
    confidence: Option<f64>,
    bookstore_candidates: Vec<(String, i64)>,
    model_candidates: Vec<(String, f64)>,
}

/// JSON 객체 `{카테고리: [키워드...]}`를 입력 순서를 지킨 목록으로 바꾼다. 키워드 배열이 아니면 빈 목록이다.
fn mappings_from_value(mappings: &Value) -> Vec<(Value, Vec<Value>)> {
    mappings.as_object().into_iter().flatten().map(|(category, keywords)| (json!(category), keywords.as_array().cloned().unwrap_or_default())).collect()
}

fn normalize_text(value: &str) -> String {
    let folded = value.to_lowercase();
    folded.split(char::is_whitespace).filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ")
}

fn candidate(category: &str, source: &str, detail: String) -> CategoryCandidate {
    CategoryCandidate { category: category.to_string(), source: source.to_string(), detail }
}

fn keyword_candidate(category: &str, keywords: &[String]) -> CategoryCandidate {
    let quoted = keywords.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
    candidate(category, "keyword", if quoted.is_empty() { "키워드 일치".to_string() } else { format!("키워드 {quoted} 일치") })
}

fn bookstore_candidate(category: &str, count: i64) -> CategoryCandidate {
    candidate(category, "bookstore", format!("서점 {count}곳 일치"))
}

fn model_candidate(category: &str) -> CategoryCandidate {
    candidate(category, "model", "모델 판정".to_string())
}

fn model_alternative_candidate(category: &str, score: f64) -> CategoryCandidate {
    candidate(category, "model", format!("모델 대안 {score:.3}"))
}

/// 2순위가 빈 채로 나가지 않도록 모델이 매긴 다음 후보를 붙인다. `min_score` 미만은 붙이지 않는다.
fn fill_second_with_model(mut candidates: Vec<CategoryCandidate>, model_candidates: &[(String, f64)], min_score: f64) -> Vec<CategoryCandidate> {
    if candidates.len() >= 2 {
        return candidates;
    }
    let taken: HashSet<String> = candidates.iter().map(|c| c.category.clone()).collect();
    for (category, score) in model_candidates {
        if taken.contains(category) {
            continue;
        }
        if *score < min_score {
            break;
        }
        candidates.push(model_alternative_candidate(category, *score));
        break;
    }
    candidates
}

/// 2순위를 서점 득표로 채운다. 이미 나온 카테고리는 건너뛴다.
fn fill_second_with_bookstore(mut candidates: Vec<CategoryCandidate>, bookstore_candidates: &[(String, i64)]) -> Vec<CategoryCandidate> {
    if candidates.len() >= 2 {
        return candidates;
    }
    let taken: HashSet<String> = candidates.iter().map(|c| c.category.clone()).collect();
    if let Some((category, count)) = bookstore_candidates.iter().find(|(c, _)| !taken.contains(c)) {
        candidates.push(bookstore_candidate(category, *count));
    }
    candidates
}

pub fn vote_count_for(category: &str, bookstore_candidates: &[(String, i64)]) -> i64 {
    bookstore_candidates.iter().find(|(c, _)| c == category).map_or(1, |(_, n)| *n)
}

/// 다수결 1위 후보. 득표수를 모르면 숫자를 지어내지 않는다.
fn bookstore_majority_candidate(target: &str, bookstore_candidates: &[(String, i64)]) -> CategoryCandidate {
    match bookstore_candidates.iter().find(|(c, _)| c == target) {
        Some((_, count)) => bookstore_candidate(target, *count),
        None => candidate(target, "bookstore", "서점 다수결로 일치".to_string()),
    }
}

/// 키워드가 정하지 못했을 때 `candidates`를 만든다. 1위는 이미 정해진 `target`을 그대로 앵커링한다.
fn build_candidates(
    target: Option<&str>,
    method: &str,
    model_category: Option<&str>,
    bookstore_candidates: &[(String, i64)],
    model_candidates: &[(String, f64)],
    min_score: f64,
) -> Vec<CategoryCandidate> {
    if let Some(target) = target {
        let first = match method {
            "bookstore_majority" => bookstore_majority_candidate(target, bookstore_candidates),
            "bookstore_single" => bookstore_candidate(target, vote_count_for(target, bookstore_candidates)),
            _ => {
                let candidates = fill_second_with_bookstore(vec![model_candidate(target)], bookstore_candidates);
                return fill_second_with_model(candidates, model_candidates, min_score);
            }
        };
        let candidates = fill_second_with_bookstore(vec![first], bookstore_candidates);
        return fill_second_with_model(candidates, model_candidates, min_score);
    }
    if method == "conflict" {
        return bookstore_candidates.iter().take(2).map(|(c, n)| bookstore_candidate(c, *n)).collect();
    }
    if let Some(model_category) = model_category {
        let candidates = fill_second_with_bookstore(vec![model_candidate(model_category)], bookstore_candidates);
        return fill_second_with_model(candidates, model_candidates, min_score);
    }
    bookstore_candidates.iter().take(2).map(|(c, n)| bookstore_candidate(c, *n)).collect()
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
            if target == source_category || !crate::path_validator::is_top_level_target_category(target) || !self.is_safe_category_name(target) {
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

    /// 모델 점수가 '확실'이라고 부를 만큼 높은가. 서점 정책의 경계와 `MODEL_CERTAIN_MIN_CONFIDENCE`를 둘 다 넘어야 한다.
    /// 정책이 없으면(경계가 0 이하) 근거가 없으므로 낮음이다.
    pub fn is_high_confidence(&self, classifier: &dyn BookClassifierApi, confidence: f64) -> bool {
        if let Some(f) = &self.is_high_confidence_override {
            return f(classifier.as_any(), confidence);
        }
        if confidence < MODEL_CERTAIN_MIN_CONFIDENCE {
            return false;
        }
        classifier.override_below() > 0.0 && !classifier.prefers_bookstore(confidence)
    }

    /// 2순위가 빈 채로 나가지 않도록 모델이 매긴 다음 후보를 붙인다(하한 없음).
    pub fn fill_second_with_model(&self, candidates: Vec<CategoryCandidate>, model_candidates: &[(String, f64)]) -> Vec<CategoryCandidate> {
        fill_second_with_model(candidates, model_candidates, 0.0)
    }

    pub fn fill_second_with_bookstore(&self, candidates: Vec<CategoryCandidate>, bookstore_candidates: &[(String, i64)]) -> Vec<CategoryCandidate> {
        fill_second_with_bookstore(candidates, bookstore_candidates)
    }

    /// 키워드가 정하지 못했을 때 `candidates`를 만든다.
    pub fn build_candidates(&self, target: Option<&str>, method: &str, model_category: Option<&str>, bookstore_candidates: &[(String, i64)]) -> Vec<CategoryCandidate> {
        build_candidates(target, method, model_category, bookstore_candidates, &[], 0.0)
    }

    /// 한 파일의 제안 목적지와 등급을 만든다. 파일을 옮기지 않는다. `classifier`가 없으면 키워드만 쓴다.
    pub fn propose_category_for_file(&self, file_path: &Path, source_category: &str, mappings: &[(Value, Vec<Value>)], classifier: Option<&dyn BookClassifierApi>) -> Proposal {
        let keyword = self.match_category_by_keywords(file_path, source_category, mappings);
        let tie_reason = keyword.tie_reason.clone();
        let mut classification = Classification { method: "not_found".to_string(), ..Classification::default() };
        let mut reason = tie_reason.clone().unwrap_or_default();
        if let Some(classifier) = classifier {
            classification = classification_of(classifier, file_path);
            if reason.is_empty() {
                reason = classification.reason.clone();
            }
        }
        let confidence = classification.confidence;
        let high = classifier.is_some_and(|c| confidence.is_some_and(|conf| self.is_high_confidence(c, conf)));
        // 모델이 스스로 버린 점수를 추천 칸에 올리지 않기 위한 하한.
        let min_score = classifier.map_or(0.0, model_decision_floor);
        // 확신도는 클래스가 많은 softmax라 점수 차이를 화면에서 읽을 수 없다. 보정한 예상 정답률을 함께 보낸다.
        let accuracy = classifier.and_then(|c| expected_accuracy_of(c, confidence));
        let model_category = classification.model_category.clone();

        if let Some(keyword_category) = &keyword.category {
            // 키워드가 목적지를 정했어도 모델이 다른 곳을 자신 있게 가리키면 등급을 '불확실'로 낮춘다.
            let grade = if high && model_category.as_deref() == Some(keyword_category.as_str()) { ProposalGrade::Certain } else { ProposalGrade::Unsure };
            let mut candidates = vec![keyword_candidate(keyword_category, &keyword.keywords)];
            if let Some((category, keywords)) = keyword.ranked.iter().find(|(c, _)| c != keyword_category) {
                candidates.push(keyword_candidate(category, keywords));
            }
            let candidates = fill_second_with_model(candidates, &classification.model_candidates, min_score);
            return Proposal {
                target_category: Some(keyword_category.clone()),
                grade: Some(grade),
                confidence,
                expected_accuracy: accuracy,
                source: Some("keyword".to_string()),
                matched_keywords: keyword.keywords,
                model_category: model_category.filter(|m| m != keyword_category),
                reason,
                candidates,
            };
        }

        if let Some(tie_reason) = tie_reason {
            // 동점이면 등급은 unknown 그대로다. 사람이 골라야 체크박스가 켜진다.
            let candidates = keyword.ranked.iter().take(2).map(|(c, k)| keyword_candidate(c, k)).collect();
            let candidates = fill_second_with_model(candidates, &classification.model_candidates, min_score);
            return Proposal {
                target_category: None,
                grade: Some(ProposalGrade::Unknown),
                confidence,
                expected_accuracy: accuracy,
                source: Some("keyword".to_string()),
                matched_keywords: keyword.keywords,
                model_category,
                reason: tie_reason,
                candidates,
            };
        }

        let classified = classification.category.clone();
        let method = classification.method.as_str();
        let (grade, target) = match (method, classified) {
            ("model", Some(category)) if high => (ProposalGrade::Certain, Some(category)),
            ("bookstore_majority", Some(category)) => (ProposalGrade::Certain, Some(category)),
            ("bookstore_single", Some(category)) => (ProposalGrade::Unsure, Some(category)),
            // 점수가 낮은 모델 답은 목적지로 쓰지 않는다. 근거에만 남긴다.
            _ => (ProposalGrade::Unknown, None),
        };
        let candidates = build_candidates(target.as_deref(), method, model_category.as_deref(), &classification.bookstore_candidates, &classification.model_candidates, min_score);
        Proposal {
            target_category: target,
            grade: Some(grade),
            confidence,
            expected_accuracy: accuracy,
            source: Some(method.to_string()),
            matched_keywords: keyword.keywords,
            model_category,
            reason,
            candidates,
        }
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
        // 서점 판정은 없어졌다. `use_bookstore`는 요청 호환을 위해 받기만 한다.
        let use_content_meta = options.use_content_meta.unwrap_or(true);
        let empty = json!({});
        let mappings = mappings.unwrap_or(&empty);
        // 꽂은 모델이 있고 본문·메타데이터 판정을 켰을 때만 모델을 쓴다. 서점 판정은 없다.
        let model_only: Option<ModelOnlyClassifier> = match &self.classifier_factory {
            Some(factory) if use_content_meta => Some(ModelOnlyClassifier { model: factory() }),
            _ => None,
        };
        let classifier: Option<&dyn BookClassifierApi> = model_only.as_ref().map(|m| m as &dyn BookClassifierApi);
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
                None => Ok(self.propose_category_for_file(file_path, category, &mappings, classifier)),
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

/// 편집 화면으로 보낼 book_id. 이 저장소에서 book_id는 파일의 inode 번호다. 파일이 사라졌으면 `None`.
pub fn book_id_for_path(file_path: &Path) -> Option<i64> {
    std::fs::metadata(file_path).ok().map(|m| m.ino() as i64)
}

/// 모델이 "이 점수 미만이면 판정하지 않는다"고 정한 값. 읽지 못하면 0이다.
pub fn model_decision_floor(classifier: &dyn BookClassifierApi) -> f64 {
    classifier.min_confidence().filter(|v| v.is_finite()).unwrap_or(0.0)
}

/// 확신도를 예상 정답률로 옮긴다. 보정이 없으면 `None`이다.
pub fn expected_accuracy(classifier: &dyn BookClassifierApi, confidence: f64) -> Option<f64> {
    expected_accuracy_of(classifier, Some(confidence))
}

fn expected_accuracy_of(classifier: &dyn BookClassifierApi, confidence: Option<f64>) -> Option<f64> {
    classifier.expected_accuracy(confidence).flatten()
}

/// `classify_file`의 항목에서 분류 결과를 읽는다.
fn classification_of(classifier: &dyn BookClassifierApi, file_path: &Path) -> Classification {
    let (category, method, reason, entry) = classifier.classify_file(file_path);
    let pairs =
        |key: &str| -> Vec<(String, Value)> { entry.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.clone()))).collect() };
    Classification {
        category,
        method,
        reason,
        model_category: entry.get("model_category").and_then(Value::as_str).map(str::to_string),
        confidence: entry.get("confidence").and_then(Value::as_f64),
        bookstore_candidates: pairs("bookstore_candidates").into_iter().filter_map(|(c, n)| Some((c, n.as_i64()?))).collect(),
        model_candidates: pairs("model_candidates").into_iter().filter_map(|(c, n)| Some((c, n.as_f64()?))).collect(),
    }
}

/// 같은 책의 사본인지 판정한다. 크기가 다르면 즉시 다른 파일이다. 읽기에 실패하면 중복이 아니라고 보수적으로 판정한다.
pub fn is_duplicate_content(source: &Path, target: &Path) -> bool {
    same_content(source, target)
}

pub fn next_numbered_path(target: &Path) -> Option<PathBuf> {
    next_numbered_path_with(target, super::DUPLICATE_NAME_MAX_INDEX)
}
