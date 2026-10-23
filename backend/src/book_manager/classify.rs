//! 분류 제안과 적용. Python `propose_category_changes`, `apply_category_changes`, `_move_classified_file`,
//! `_match_category_by_keywords`, `_propose_category_for_file`에 대응한다.
//!
//! TODO(classifier): 분류기 서비스(`BookClassifierService`: 서점 조회, 정책 경계, 보정 곡선)가 아직 없다.
//! 지금은 `BookManagerDeps::with_classifier_factory`로 꽂은 `Classify` 모델 판정만 쓴다. 서점 득표(`use_bookstore`)는
//! 무시하고, 정책 경계(`prefers_bookstore`)가 없어 `is_high_confidence`는 기본이 `false`다(Python의 "정책 파일 없음"과 같은 보수적 판정).
//! 분류기 에이전트가 서비스를 만들면 `classify_with_model`과 `is_high_confidence`를 그 trait로 바꾼다.

use std::any::Any;
use std::collections::HashSet;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::util::relative_to;
use super::{BookManager, DUPLICATE_NAME_MAX_INDEX, ProgressFn};
use crate::classifier::Classify;
use crate::es_manager::Docs;

/// 제안 시작 때 이름만 담은 행을 한 번에 보내는 개수.
const PROPOSAL_PLACEHOLDER_CHUNK: usize = 500;
/// 모델 단독 판정을 '확실'로 볼 최소 확신도. `BookClassifierService.MODEL_TRUST_MIN_CONFIDENCE`와 같다.
pub const MODEL_CERTAIN_MIN_CONFIDENCE: f64 = 0.10;

const GRADE_CERTAIN: &str = "certain";
const GRADE_UNSURE: &str = "unsure";
const GRADE_UNKNOWN: &str = "unknown";

/// `propose_category_changes` 옵션. `None`은 Python 기본값(`use_bookstore=True`, `use_content_meta=True`)이다.
#[derive(Default)]
pub struct ProposeOptions<'a> {
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

type Ranked = Vec<(String, Vec<String>)>;

/// `_match_category_by_keywords`의 결과.
struct KeywordMatch {
    category: Option<String>,
    keywords: Vec<String>,
    tie_reason: Option<String>,
    ranked: Ranked,
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

fn normalize_text(value: &str) -> String {
    value.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn candidate(category: &str, source: &str, detail: String) -> Value {
    json!({"category": category, "source": source, "detail": detail})
}

fn keyword_candidate(category: &str, keywords: &[String]) -> Value {
    let quoted = keywords.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
    candidate(category, "keyword", if quoted.is_empty() { "키워드 일치".to_string() } else { format!("키워드 {quoted} 일치") })
}

fn bookstore_candidate(category: &str, count: i64) -> Value {
    candidate(category, "bookstore", format!("서점 {count}곳 일치"))
}

fn model_candidate(category: &str) -> Value {
    candidate(category, "model", "모델 판정".to_string())
}

fn model_alternative_candidate(category: &str, score: f64) -> Value {
    candidate(category, "model", format!("모델 대안 {score:.3}"))
}

fn category_of(c: &Value) -> &str {
    c.get("category").and_then(Value::as_str).unwrap_or("")
}

/// 2순위가 빈 채로 나가지 않도록 모델이 매긴 다음 후보를 붙인다. `min_score` 미만은 붙이지 않는다.
fn fill_second_with_model(mut candidates: Vec<Value>, model_candidates: &[(String, f64)], min_score: f64) -> Vec<Value> {
    if candidates.len() >= 2 {
        return candidates;
    }
    let taken: HashSet<String> = candidates.iter().map(|c| category_of(c).to_string()).collect();
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
fn fill_second_with_bookstore(mut candidates: Vec<Value>, bookstore_candidates: &[(String, i64)]) -> Vec<Value> {
    if candidates.len() >= 2 {
        return candidates;
    }
    let taken: HashSet<String> = candidates.iter().map(|c| category_of(c).to_string()).collect();
    if let Some((category, count)) = bookstore_candidates.iter().find(|(c, _)| !taken.contains(c)) {
        candidates.push(bookstore_candidate(category, *count));
    }
    candidates
}

fn vote_count_for(category: &str, bookstore_candidates: &[(String, i64)]) -> i64 {
    bookstore_candidates.iter().find(|(c, _)| c == category).map_or(1, |(_, n)| *n)
}

/// 다수결 1위 후보. 득표수를 모르면 숫자를 지어내지 않는다.
fn bookstore_majority_candidate(target: &str, bookstore_candidates: &[(String, i64)]) -> Value {
    match bookstore_candidates.iter().find(|(c, _)| c == target) {
        Some((_, count)) => bookstore_candidate(target, *count),
        None => candidate(target, "bookstore", "서점 다수결로 일치".to_string()),
    }
}

/// 키워드가 정하지 못했을 때 `candidates`를 만든다. 1위는 이미 정해진 `target`을 그대로 앵커링한다.
fn build_candidates(target: Option<&str>, method: &str, model_category: Option<&str>, bookstore_candidates: &[(String, i64)], model_candidates: &[(String, f64)], min_score: f64) -> Vec<Value> {
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
fn next_numbered_path(target: &Path) -> Option<PathBuf> {
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = target.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..=DUPLICATE_NAME_MAX_INDEX).map(|n| target.with_file_name(format!("{stem} ({n}){suffix}"))).find(|candidate| !candidate.exists())
}

impl BookManager {
    /// 편집 화면으로 보낼 book_id. 이 저장소에서 book_id는 파일의 inode 번호다. 파일이 사라졌으면 `None`.
    pub fn _book_id_for_path(file_path: &Path) -> Option<i64> {
        std::fs::metadata(file_path).ok().map(|m| m.ino() as i64)
    }

    /// book_id가 없는 제안 항목에 inode를 채운다. 이미 있는 항목은 stat 하지 않는다.
    pub fn fill_missing_book_ids(&self, mut items: Vec<Value>, path_prefix: &Path) -> Vec<Value> {
        for item in &mut items {
            if item.get("book_id").is_some_and(|v| !v.is_null()) {
                continue;
            }
            let Some(rel_path) = item.get("file_path").and_then(Value::as_str).filter(|p| !p.is_empty()) else { continue };
            item["book_id"] = json!(Self::_book_id_for_path(&path_prefix.join(rel_path)));
        }
        items
    }

    /// 카테고리 직하위(또는 하위 전체)의 색인 대상 파일을 정렬해서 돌려준다.
    pub(crate) fn iter_category_indexable_files(&self, category: &str, recursive: bool) -> Vec<PathBuf> {
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
                if Self::is_dir_nofollow(&entry) {
                    if recursive {
                        stack.push(entry);
                    }
                    continue;
                }
                if !Self::is_regular_file_nofollow(&entry) {
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

    fn classification_haystack(&self, file_path: &Path) -> String {
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

    /// 등록된 키워드로 목적지를 고른다. 동점이면 고르지 않고 사유를 돌려준다.
    fn match_category_by_keywords(&self, file_path: &Path, source_category: &str, mappings: &Value) -> KeywordMatch {
        let haystack = self.classification_haystack(file_path);
        let mut scored: Vec<(i64, usize, String, Vec<String>)> = Vec::new();
        for (raw_category, raw_keywords) in mappings.as_object().into_iter().flatten() {
            let target = raw_category.trim();
            if target == source_category || !Self::is_top_level_target_category(target) || !self.is_safe_category_name(target) {
                continue;
            }
            let mut matched: Vec<String> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for raw_keyword in raw_keywords.as_array().into_iter().flatten().filter_map(Value::as_str) {
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

    /// 모델 점수가 '확실'이라고 부를 만큼 높은가. 정책(서점 경계)을 읽을 수 없으면 낮음으로 본다.
    fn is_high_confidence(&self, classifier: &Arc<dyn Classify>, confidence: Option<f64>) -> bool {
        let Some(confidence) = confidence else { return false };
        match &self.is_high_confidence_override {
            Some(f) => f(classifier as &dyn Any, confidence),
            None => false,
        }
    }

    /// 모델 한 번 판정. `Classify`만 있으므로 서점 득표는 비어 있다.
    fn classify_with_model(classifier: &Arc<dyn Classify>, file_path: &Path, use_content_meta: bool) -> Classification {
        let mut out = Classification { method: "not_found".to_string(), ..Classification::default() };
        if !use_content_meta || !classifier.is_available() {
            return out;
        }
        match classifier.classify_path(file_path, None) {
            Ok(prediction) => {
                out.method = if prediction.category.is_some() { "model" } else { "not_found" }.to_string();
                out.model_category = prediction.category.clone().or_else(|| prediction.ranked.first().map(|(c, _)| c.clone()));
                out.category = prediction.category;
                out.confidence = Some(prediction.confidence);
                out.model_candidates = prediction.ranked;
                out.reason = prediction.reason;
            }
            Err(e) => out.reason = e,
        }
        out
    }

    /// 한 파일의 제안 목적지와 등급을 만든다. 파일을 옮기지 않는다.
    fn propose_category_for_file(&self, file_path: &Path, source_category: &str, mappings: &Value, classifier: Option<&Arc<dyn Classify>>, use_content_meta: bool) -> Value {
        let keyword = self.match_category_by_keywords(file_path, source_category, mappings);
        let tie_reason = keyword.tie_reason.clone();
        let mut classification = Classification { method: "not_found".to_string(), ..Classification::default() };
        let mut reason = tie_reason.clone().unwrap_or_default();
        if let Some(classifier) = classifier {
            classification = Self::classify_with_model(classifier, file_path, use_content_meta);
            if reason.is_empty() {
                reason = classification.reason.clone();
            }
        }
        let confidence = classification.confidence;
        let high = classifier.is_some_and(|c| self.is_high_confidence(c, confidence));
        // TODO(classifier): 모델이 스스로 버린 점수 하한(`classifier.min_confidence`)과 보정 곡선(`expected_accuracy`)은 서비스가 생기면 채운다.
        let min_score = 0.0;
        let accuracy: Option<f64> = None;
        let model_category = classification.model_category.clone();

        if let Some(keyword_category) = &keyword.category {
            // 키워드가 목적지를 정했어도 모델이 다른 곳을 자신 있게 가리키면 등급을 '불확실'로 낮춘다.
            let grade = if high && model_category.as_deref() == Some(keyword_category.as_str()) { GRADE_CERTAIN } else { GRADE_UNSURE };
            let mut candidates = vec![keyword_candidate(keyword_category, &keyword.keywords)];
            if let Some((category, keywords)) = keyword.ranked.iter().find(|(c, _)| c != keyword_category) {
                candidates.push(keyword_candidate(category, keywords));
            }
            let candidates = fill_second_with_model(candidates, &classification.model_candidates, min_score);
            return json!({
                "target_category": keyword_category, "grade": grade, "confidence": confidence, "expected_accuracy": accuracy, "source": "keyword",
                "matched_keywords": keyword.keywords, "model_category": if model_category.as_deref() != Some(keyword_category.as_str()) { json!(model_category) } else { Value::Null },
                "reason": reason, "candidates": candidates,
            });
        }

        if let Some(tie_reason) = tie_reason {
            // 동점이면 등급은 unknown 그대로다. 사람이 골라야 체크박스가 켜진다.
            let candidates: Vec<Value> = keyword.ranked.iter().take(2).map(|(c, k)| keyword_candidate(c, k)).collect();
            let candidates = fill_second_with_model(candidates, &classification.model_candidates, min_score);
            return json!({
                "target_category": Value::Null, "grade": GRADE_UNKNOWN, "confidence": confidence, "expected_accuracy": accuracy, "source": "keyword",
                "matched_keywords": keyword.keywords, "model_category": model_category, "reason": tie_reason, "candidates": candidates,
            });
        }

        let classified = classification.category.clone();
        let method = classification.method.as_str();
        let (grade, target) = match (method, classified) {
            ("model", Some(category)) if high => (GRADE_CERTAIN, Some(category)),
            ("bookstore_majority", Some(category)) => (GRADE_CERTAIN, Some(category)),
            ("bookstore_single", Some(category)) => (GRADE_UNSURE, Some(category)),
            // 점수가 낮은 모델 답은 목적지로 쓰지 않는다. 근거에만 남긴다.
            _ => (GRADE_UNKNOWN, None),
        };
        let candidates = build_candidates(target.as_deref(), method, model_category.as_deref(), &classification.bookstore_candidates, &classification.model_candidates, min_score);
        json!({
            "target_category": target, "grade": grade, "confidence": confidence, "expected_accuracy": accuracy, "source": method,
            "matched_keywords": keyword.keywords, "model_category": model_category, "reason": reason, "candidates": candidates,
        })
    }

    /// 선택 카테고리 직하위 파일의 분류 제안을 만든다. 파일을 옮기지 않는다.
    pub async fn propose_category_changes(&self, category: &str, mappings: Option<&Value>, options: ProposeOptions<'_>) -> (Value, Option<String>) {
        if category.is_empty() {
            return (json!({}), Some("카테고리 이름이 비어있습니다".to_string()));
        }
        if !self.is_safe_category_name(category) {
            return (json!({}), Some("잘못된 카테고리 경로입니다".to_string()));
        }
        if !self.category_dir(category).is_dir() {
            return (json!({}), Some(format!("디렉토리를 찾을 수 없습니다: {category}")));
        }

        let content_type = "book";
        let (use_bookstore, use_content_meta) = (options.use_bookstore.unwrap_or(true), options.use_content_meta.unwrap_or(true));
        let empty = json!({});
        let mappings = mappings.unwrap_or(&empty);
        let classifier: Option<Arc<dyn Classify>> = if use_bookstore || use_content_meta { self.classifier_factory.as_ref().map(|factory| factory()) } else { None };

        let file_paths = self.iter_category_indexable_files(category, false);
        let total_count = file_paths.len();
        let mut processed_count = 0usize;
        let mut items: Vec<Value> = Vec::new();
        let mut failures: Vec<Value> = Vec::new();

        let report = |processed: usize, payload: Value| {
            let Some(callback) = options.on_progress else { return };
            let mut merged = Map::new();
            merged.insert("total_count".into(), json!(total_count));
            merged.insert("processed_count".into(), json!(processed));
            if let Some(extra) = payload.as_object() {
                merged.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            callback(Value::Object(merged));
        };

        // 분류를 시작하기 전에 대상 파일의 이름만 담은 행을 먼저 깔아 둔다.
        let mut placeholders: Vec<Value> = Vec::new();
        for file_path in &file_paths {
            let Some(rel_path) = relative_to(file_path, &self.path_prefix) else { continue };
            placeholders.push(json!({
                "file_path": rel_path, "book_id": Self::_book_id_for_path(file_path), "title": file_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                "current_category": category, "target_category": Value::Null, "grade": Value::Null, "confidence": Value::Null, "expected_accuracy": Value::Null,
                "source": Value::Null, "matched_keywords": [], "model_category": Value::Null, "reason": "", "candidates": [], "apply_status": "pending", "apply_error": Value::Null,
            }));
        }
        for chunk in placeholders.chunks(PROPOSAL_PLACEHOLDER_CHUNK) {
            report(processed_count, json!({"new_items": chunk}));
        }

        for file_path in &file_paths {
            processed_count += 1;
            let Some(rel_path) = relative_to(file_path, &self.path_prefix) else {
                failures.push(json!({"file_path": file_path.to_string_lossy(), "error": "잘못된 파일 경로입니다"}));
                continue;
            };
            let proposal = self.propose_category_for_file(file_path, category, mappings, classifier.as_ref(), use_content_meta);
            let mut new_item = json!({
                "file_path": rel_path, "book_id": Self::_book_id_for_path(file_path), "title": file_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                "current_category": category, "apply_status": "pending", "apply_error": Value::Null,
            });
            if let (Some(target), Some(extra)) = (new_item.as_object_mut(), proposal.as_object()) {
                target.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            items.push(new_item.clone());
            // 진행 보고에는 이번 틱에서 분류가 끝난 항목만 싣는다. 누적 목록은 호출자에게 돌려준다.
            report(processed_count, json!({"updated_items": [new_item]}));
        }

        (json!({"content_type": content_type, "source_category": category, "total_count": total_count, "processed_count": processed_count, "items": items, "failures": failures}), None)
    }

    /// 승인된 항목만 실제로 옮긴다.
    pub async fn apply_category_changes(&self, items: &[Value], allowed_file_paths: &HashSet<String>, on_progress: Option<ProgressFn<'_>>) -> (Value, Option<String>) {
        self.apply_category_changes_with(items, allowed_file_paths, ApplyOptions { on_progress, ..ApplyOptions::default() }).await
    }

    /// `apply_category_changes`의 전체 옵션 판. 목적지(`target_category`)는 브라우저가 준 값이라 `allowed_file_paths`와 안전한 이름인지 다시 검증한다.
    pub async fn apply_category_changes_with(&self, items: &[Value], allowed_file_paths: &HashSet<String>, options: ApplyOptions<'_>) -> (Value, Option<String>) {
        let content_type = options.content_type.unwrap_or("book");
        let mut results: Vec<Value> = Vec::new();
        let (mut applied_count, mut failed_count) = (0i64, 0i64);
        let root = self.resolved_root().unwrap_or_else(|_| self.path_prefix.clone());
        let notify = |payload: Value| {
            if let Some(callback) = options.on_item_done {
                callback(payload);
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
            let mut apply_status = "failed";
            let mut apply_error: Option<String> = None;

            match (file_path_value, target_category) {
                (Some(file_path), _) if !allowed_file_paths.contains(file_path) => apply_error = Some("제안 목록에 없는 파일입니다".to_string()),
                (None, _) => apply_error = Some("제안 목록에 없는 파일입니다".to_string()),
                (Some(_), Some(target)) if !Self::is_top_level_target_category(target) || !self.is_safe_category_name(target) => apply_error = Some("옮길 수 없는 카테고리입니다".to_string()),
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
                        notify(json!({"file_path": file_path, "apply_status": "moving", "apply_error": Value::Null}));
                        let (file_result, error) = self.move_classified_file(&absolute_path, target, content_type, false, options.clean_existing, Some(source_category)).await;
                        match (file_result, error) {
                            (Some(_), None) => apply_status = "moved",
                            (_, error) => apply_error = Some(error.unwrap_or_else(|| "분류 적용에 실패했습니다".to_string())),
                        }
                    }
                }
            }

            if apply_status == "moved" {
                applied_count += 1;
            } else {
                failed_count += 1;
            }
            results.push(json!({"file_path": file_path_value, "target_category": target_category, "apply_status": apply_status, "apply_error": apply_error}));
            notify(json!({"file_path": file_path_value, "apply_status": apply_status, "apply_error": apply_error}));
            if let Some(callback) = options.on_progress {
                // 제안 단계의 total_count와 섞이지 않게 apply 전용 키를 쓴다.
                callback(json!({"apply_total_count": items.len(), "applied_count": applied_count, "failed_count": failed_count}));
            }
        }

        (json!({"content_type": content_type, "total_count": items.len(), "applied_count": applied_count, "failed_count": failed_count, "results": results}), None)
    }

    /// 파일 하나를 목적지 카테고리로 옮기고 ES를 맞춘다. 실패하면 파일과 ES 문서를 되돌린다.
    pub(crate) async fn move_classified_file(
        &self,
        file_path: &Path,
        target_category: &str,
        content_type: &str,
        dry_run: bool,
        clean_existing: bool,
        source_category: Option<&str>,
    ) -> (Option<Value>, Option<String>) {
        let root = self.resolved_root().unwrap_or_else(|_| self.path_prefix.clone());
        let Some(old_rel_path) = relative_to(file_path, &self.path_prefix) else { return (None, Some("잘못된 파일 경로입니다".to_string())) };

        let mut target_path = self.path_prefix.join(target_category).join(Self::file_name_of(file_path));
        if !self.resolve_path(&target_path).is_ok_and(|p| p.starts_with(&root)) {
            return (None, Some("잘못된 대상 경로입니다".to_string()));
        }
        let mut target_rel_path = relative_to(&target_path, &self.path_prefix).unwrap_or_default();
        if self.path_exists(&target_path) {
            let is_same_file = self.fs.same_file(file_path, &target_path).unwrap_or(false);
            if is_same_file && old_rel_path == target_rel_path {
                // 이미 목적지에 있다. 옮길 것이 없다고 답하고 끝낸다.
                let status = if dry_run { "dry_run" } else { "moved" };
                return (Some(json!({"status": status, "action": "noop", "from": old_rel_path, "to": target_rel_path, "target_category": target_category, "deleted_count": 0})), None);
            }
            if !is_same_file && !same_content(file_path, &target_path) {
                // 이름만 겹치고 내용이 다르면 서로 다른 책이다. 번호를 붙여 둘 다 남긴다.
                let Some(numbered) = next_numbered_path(&target_path) else {
                    return (None, Some(format!("대상 경로에 같은 이름의 파일이 너무 많습니다: {target_rel_path}")));
                };
                target_path = numbered;
                target_rel_path = relative_to(&target_path, &self.path_prefix).unwrap_or_default();
            } else {
                // 내용이 같은 별개 파일이거나, 경로는 다른데 같은 실체인 하드링크다. 둘 다 "대상에 이미 있다"로 본다.
                if clean_existing {
                    if dry_run {
                        return (
                            Some(json!({"status": "dry_run", "action": "duplicate_clean", "from": old_rel_path, "to": target_rel_path, "target_category": target_category, "deleted_count": 0})),
                            None,
                        );
                    }
                    if let Err(e) = self.fs.remove_file(file_path) {
                        return (None, Some(format!("중복 파일 정리 실패: {e}")));
                    }
                    if let Some(source) = source_category {
                        self.clean_empty_parent_dirs(file_path.parent().unwrap_or(Path::new("")), &self.category_dir(source));
                    }
                    return match self.es.delete_by_file_paths(std::slice::from_ref(&old_rel_path), None) {
                        Ok(deleted_count) => {
                            (Some(json!({"status": "duplicate_cleaned", "from": old_rel_path, "to": target_rel_path, "target_category": target_category, "deleted_count": deleted_count})), None)
                        }
                        Err(e) => (None, Some(format!("중복 파일 정리 실패: {e}"))),
                    };
                }
                return (None, Some(format!("대상 경로에 파일이 이미 존재합니다: {target_rel_path}")));
            }
        }

        if dry_run {
            return (Some(json!({"status": "dry_run", "action": "move", "from": old_rel_path, "to": target_rel_path, "target_category": target_category, "deleted_count": 0})), None);
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
            return (None, Some(format!("파일 이동 실패: {e}")));
        }
        if let Some(source) = source_category {
            self.clean_empty_parent_dirs(&old_parent, &self.category_dir(source));
        }

        let outcome: Result<Value, String> = (|| {
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
            let (inserted, add_error) = self.add_book_raw(&Docs::from([(new_book_id, new_doc)]))?;
            if inserted != Some(new_book_id) || add_error.is_some() {
                return Err(add_error.unwrap_or_else(|| "ES 적재 실패".to_string()));
            }
            Ok(json!({"status": "moved", "from": old_rel_path, "to": target_rel_path, "book_id": new_book_id, "target_category": target_category, "deleted_count": deleted_count}))
        })();

        match outcome {
            Ok(result) => (Some(result), None),
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
                        Ok((restored, restore_error)) if restored != Some(id) || restore_error.is_some() => {
                            rollback_messages.push(format!("기존 ES 문서 복구 실패: {}", restore_error.unwrap_or_else(|| "None".to_string())))
                        }
                        Ok(_) => {}
                        Err(e) => rollback_messages.push(format!("기존 ES 문서 복구 실패: {e}")),
                    }
                }
                let message = if rollback_messages.is_empty() { error } else { format!("{error}; {}", rollback_messages.join("; ")) };
                (None, Some(message))
            }
        }
    }
}
