//! 분류 제안과 적용. Python `propose_category_changes`, `apply_category_changes`, `_move_classified_file`,
//! `_match_category_by_keywords`, `_propose_category_for_file`에 대응한다.
//!
//! 분류 판정은 세 갈래다. 우선순위는 위에서 아래다.
//!
//! 1. `BookManagerDeps::with_classifier_factory` 로 꽂은 `Classify` 모델 단독 대역(`ModelOnlyClassifier`). 서점 득표는 없다.
//! 2. `BookManagerDeps::with_classifier_service_factory` 로 꽂거나, 안 꽂으면 `TM_CLASSIFIER_MODEL_DIR` 의 모델로 만든
//!    `BookClassifierService`(`SharedBookClassifier`). 서점 득표, 정책 경계, 보정 곡선을 모두 쓴다. 매니저가 하나를 `Arc` 로 공유하고
//!    제안 하나가 끝날 때 캐시를 쓴다.
//! 3. 모델 디렉토리가 없거나 읽히지 않으면 분류기 없이 키워드만 쓴다. 이유는 경고 로그로 한 번만 남긴다.

use std::collections::HashSet;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::util::relative_to;
use super::{BookManager, ProgressFn};
use crate::book_classifier::{BookClassifierService, SharedBookClassifier};
use crate::classifier::model::{CategoryModelApi, get_default_model, model_path};
use crate::classifier::{BookCategoryClassifier, BookClassifierApi, ClassifierOptions, ModelOnlyClassifier};
use crate::es_manager::Docs;

/// 제안 시작 때 이름만 담은 행을 한 번에 보내는 개수.
const PROPOSAL_PLACEHOLDER_CHUNK: usize = 500;
/// 모델 단독 판정을 '확실'로 볼 최소 확신도. `BookClassifierService.MODEL_TRUST_MIN_CONFIDENCE`와 같다.
pub const MODEL_CERTAIN_MIN_CONFIDENCE: f64 = 0.10;

/// `propose_category_changes` 옵션. `None`은 Python 기본값(`use_bookstore=True`, `use_content_meta=True`)이다.
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

/// JSON 객체 `{카테고리: [키워드...]}`를 입력 순서를 지킨 목록으로 바꾼다. 키워드 배열이 아니면 빈 목록이다.
fn mappings_from_value(mappings: &Value) -> Vec<(Value, Vec<Value>)> {
    mappings.as_object().into_iter().flatten().map(|(category, keywords)| (json!(category), keywords.as_array().cloned().unwrap_or_default())).collect()
}

fn normalize_text(value: &str) -> String {
    let folded = pyfmt::py_casefold(value);
    folded.split(crate::classifier::tokenize::is_py_space).filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ")
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
fn next_numbered_path(target: &Path, max_index: usize) -> Option<PathBuf> {
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = target.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..=max_index).map(|n| target.with_file_name(format!("{stem} ({n}){suffix}"))).find(|candidate| !candidate.exists())
}

impl BookManager {
    pub const GRADE_CERTAIN: &'static str = "certain";
    pub const GRADE_UNSURE: &'static str = "unsure";
    pub const GRADE_UNKNOWN: &'static str = "unknown";

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

    /// 파일 이름, 확장자 없는 이름, 책 루트 기준 상위 디렉터리 이름을 이어 정규화한 문자열. 책 루트 밖이면 전체 상위 경로를 쓴다.
    pub fn _classification_haystack(&self, file_path: &Path) -> String {
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
    /// 반환: (목적지, 맞은 키워드, 동점 사유, 상위 2개 순위). 카테고리나 키워드가 문자열이 아니면 건너뛴다.
    pub fn _match_category_by_keywords(&self, file_path: &Path, source_category: &str, mappings: &[(Value, Vec<Value>)]) -> (Option<String>, Vec<String>, Option<String>, Ranked) {
        let m = self.match_category_by_keywords(file_path, source_category, mappings);
        (m.category, m.keywords, m.tie_reason, m.ranked)
    }

    fn match_category_by_keywords(&self, file_path: &Path, source_category: &str, mappings: &[(Value, Vec<Value>)]) -> KeywordMatch {
        let haystack = self._classification_haystack(file_path);
        let mut scored: Vec<(i64, usize, String, Vec<String>)> = Vec::new();
        for (raw_category, raw_keywords) in mappings {
            let Some(raw_category) = raw_category.as_str() else { continue };
            let target = raw_category.trim();
            if target == source_category || !Self::is_top_level_target_category(target) || !self.is_safe_category_name(target) {
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
    pub fn _is_high_confidence(&self, classifier: &dyn BookClassifierApi, confidence: f64) -> bool {
        if let Some(f) = &self.is_high_confidence_override {
            return f(classifier.as_any(), confidence);
        }
        if confidence < MODEL_CERTAIN_MIN_CONFIDENCE {
            return false;
        }
        classifier.override_below() > 0.0 && !classifier.prefers_bookstore(confidence)
    }

    /// 모델이 "이 점수 미만이면 판정하지 않는다"고 정한 값. 읽지 못하면 0이다.
    pub fn _model_decision_floor(classifier: &dyn BookClassifierApi) -> f64 {
        classifier.min_confidence().filter(|v| v.is_finite()).unwrap_or(0.0)
    }

    /// 확신도를 예상 정답률로 옮긴다. 보정이 없으면 `None`이다.
    pub fn _expected_accuracy(classifier: &dyn BookClassifierApi, confidence: f64) -> Option<f64> {
        Self::expected_accuracy_of(classifier, Some(confidence))
    }

    fn expected_accuracy_of(classifier: &dyn BookClassifierApi, confidence: Option<f64>) -> Option<f64> {
        classifier.expected_accuracy(confidence).flatten()
    }

    /// 2순위가 빈 채로 나가지 않도록 모델이 매긴 다음 후보를 붙인다(하한 없음).
    pub fn _fill_second_with_model(&self, candidates: Vec<Value>, model_candidates: &[(String, f64)]) -> Vec<Value> {
        fill_second_with_model(candidates, model_candidates, 0.0)
    }

    pub fn _fill_second_with_bookstore(&self, candidates: Vec<Value>, bookstore_candidates: &[(String, i64)]) -> Vec<Value> {
        fill_second_with_bookstore(candidates, bookstore_candidates)
    }

    pub fn _vote_count_for(category: &str, bookstore_candidates: &[(String, i64)]) -> i64 {
        vote_count_for(category, bookstore_candidates)
    }

    /// 키워드가 정하지 못했을 때 `candidates`를 만든다.
    pub fn _build_candidates(&self, target: Option<&str>, method: &str, model_category: Option<&str>, bookstore_candidates: &[(String, i64)]) -> Vec<Value> {
        build_candidates(target, method, model_category, bookstore_candidates, &[], 0.0)
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

    /// 한 파일의 제안 목적지와 등급을 만든다. 파일을 옮기지 않는다. `classifier`가 없으면 키워드만 쓴다.
    pub fn _propose_category_for_file(
        &self,
        file_path: &Path,
        source_category: &str,
        mappings: &[(Value, Vec<Value>)],
        classifier: &dyn BookClassifierApi,
        _use_bookstore: bool,
        _use_content_meta: bool,
    ) -> Value {
        self.propose_category_for_file(file_path, source_category, mappings, Some(classifier))
    }

    fn propose_category_for_file(&self, file_path: &Path, source_category: &str, mappings: &[(Value, Vec<Value>)], classifier: Option<&dyn BookClassifierApi>) -> Value {
        let keyword = self.match_category_by_keywords(file_path, source_category, mappings);
        let tie_reason = keyword.tie_reason.clone();
        let mut classification = Classification { method: "not_found".to_string(), ..Classification::default() };
        let mut reason = tie_reason.clone().unwrap_or_default();
        if let Some(classifier) = classifier {
            classification = Self::classification_of(classifier, file_path);
            if reason.is_empty() {
                reason = classification.reason.clone();
            }
        }
        let confidence = classification.confidence;
        let high = classifier.is_some_and(|c| confidence.is_some_and(|conf| self._is_high_confidence(c, conf)));
        // 모델이 스스로 버린 점수를 추천 칸에 올리지 않기 위한 하한.
        let min_score = classifier.map_or(0.0, Self::_model_decision_floor);
        // 확신도는 클래스가 많은 softmax라 점수 차이를 화면에서 읽을 수 없다. 보정한 예상 정답률을 함께 보낸다.
        let accuracy = classifier.and_then(|c| Self::expected_accuracy_of(c, confidence));
        let model_category = classification.model_category.clone();

        if let Some(keyword_category) = &keyword.category {
            // 키워드가 목적지를 정했어도 모델이 다른 곳을 자신 있게 가리키면 등급을 '불확실'로 낮춘다.
            let grade = if high && model_category.as_deref() == Some(keyword_category.as_str()) { Self::GRADE_CERTAIN } else { Self::GRADE_UNSURE };
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
                "target_category": Value::Null, "grade": Self::GRADE_UNKNOWN, "confidence": confidence, "expected_accuracy": accuracy, "source": "keyword",
                "matched_keywords": keyword.keywords, "model_category": model_category, "reason": tie_reason, "candidates": candidates,
            });
        }

        let classified = classification.category.clone();
        let method = classification.method.as_str();
        let (grade, target) = match (method, classified) {
            ("model", Some(category)) if high => (Self::GRADE_CERTAIN, Some(category)),
            ("bookstore_majority", Some(category)) => (Self::GRADE_CERTAIN, Some(category)),
            ("bookstore_single", Some(category)) => (Self::GRADE_UNSURE, Some(category)),
            // 점수가 낮은 모델 답은 목적지로 쓰지 않는다. 근거에만 남긴다.
            _ => (Self::GRADE_UNKNOWN, None),
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

        let content_type = options.content_type.unwrap_or("book");
        let (use_bookstore, use_content_meta) = (options.use_bookstore.unwrap_or(true), options.use_content_meta.unwrap_or(true));
        let empty = json!({});
        let mappings = mappings.unwrap_or(&empty);
        // Python: `use_bookstore or use_content_meta` 일 때만 서비스를 만든다. 대역 훅(`with_classifier_factory`)이 있으면 모델 단독이다.
        let model_only: Option<ModelOnlyClassifier> = match &self.classifier_factory {
            Some(factory) if use_content_meta => Some(ModelOnlyClassifier { model: factory() }),
            _ => None,
        };
        let service = if self.classifier_factory.is_none() && (use_bookstore || use_content_meta) { self.classifier_service() } else { None };
        if let Some(service) = &service {
            service.begin_request();
        }
        let service_call = service.as_ref().map(|s| s.call_with_delay(&self.category_dir(category), use_bookstore, use_content_meta, options.delay));
        let classifier: Option<&dyn BookClassifierApi> = match (&model_only, &service_call) {
            (Some(m), _) => Some(m),
            (None, Some(c)) => Some(c),
            _ => None,
        };
        let mappings = mappings_from_value(mappings);

        let propose_hook = self.seams().propose_hook;
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
            let proposal = match &propose_hook {
                Some(hook) => hook(file_path),
                None => Ok(self.propose_category_for_file(file_path, category, &mappings, classifier)),
            };
            let Ok(proposal) = proposal else {
                log::error!("분류 제안 실패: {rel_path}");
                failures.push(json!({"file_path": rel_path, "error": "분류 제안에 실패했습니다"}));
                continue;
            };
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

        // Python은 제안 경로에서 분류 캐시를 저장하지 않는다(저장은 직접 분류 CLI뿐이다). 저장하면 서점 응답이 비었던 항목이
        // 캐시에 남아 같은 파일을 다시 제안해도 서점을 재조회하지 않는다.
        (json!({"content_type": content_type, "source_category": category, "total_count": total_count, "processed_count": processed_count, "items": items, "failures": failures}), None)
    }

    /// 분류 서비스. 처음 쓸 때 한 번 만든다. 꽂은 factory 가 없으면 `TM_CLASSIFIER_MODEL_DIR` 의 모델로 만들고, 못 읽으면 `None`(분류기 없음)이다.
    fn classifier_service(&self) -> Option<Arc<SharedBookClassifier>> {
        self.classifier_service
            .get_or_init(|| match &self.classifier_service_factory {
                Some(factory) => factory(),
                None => default_classifier_service(&self.path_prefix),
            })
            .clone()
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
        let move_hook = self.seams().move_hook;
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

            // 목적지 이름 검사는 한 번만 한다(검사 대역이 호출 횟수를 셀 수 있다). 최상위 이름일 때만 검사한다.
            let safety = target_category.filter(|t| allowed_file_paths.contains(file_path_value.unwrap_or("")) && Self::is_top_level_target_category(t)).map(|t| self.try_is_safe_category_name(t));
            match (file_path_value, target_category) {
                (Some(file_path), _) if !allowed_file_paths.contains(file_path) => apply_error = Some("제안 목록에 없는 파일입니다".to_string()),
                (None, _) => apply_error = Some("제안 목록에 없는 파일입니다".to_string()),
                (Some(_), Some(target)) if !Self::is_top_level_target_category(target) => apply_error = Some("옮길 수 없는 카테고리입니다".to_string()),
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
                        notify(json!({"file_path": file_path, "apply_status": "moving", "apply_error": Value::Null}));
                        let (file_result, error) = match &move_hook {
                            Some(hook) => hook(&absolute_path, target),
                            None => self.move_classified_file(&absolute_path, target, content_type, false, options.clean_existing, Some(source_category)).await,
                        };
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
    pub async fn _move_classified_file(&self, file_path: &Path, target_category: &str, content_type: &str, opts: MoveOpts) -> (Option<Value>, Option<String>) {
        self.move_classified_file(file_path, target_category, content_type, opts.dry_run, opts.clean_existing, opts.source_category.as_deref()).await
    }

    /// 같은 책의 사본인지 판정한다. 크기가 다르면 즉시 다른 파일이다. 읽기에 실패하면 중복이 아니라고 보수적으로 판정한다.
    pub fn _is_duplicate_content(source: &Path, target: &Path) -> bool {
        same_content(source, target)
    }

    /// `name.ext`가 이미 있을 때 `name (1).ext`, `name (2).ext` 순으로 빈 자리를 찾는다. `max_index`까지 다 차면 `None`이다.
    pub fn _next_numbered_path_with(target: &Path, max_index: usize) -> Option<PathBuf> {
        next_numbered_path(target, max_index)
    }

    pub fn _next_numbered_path(target: &Path) -> Option<PathBuf> {
        next_numbered_path(target, super::DUPLICATE_NAME_MAX_INDEX)
    }

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
                let Some(numbered) = next_numbered_path(&target_path, self.duplicate_name_max_index) else {
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

/// 환경변수 `TM_CLASSIFIER_MODEL_DIR` 의 모델로 기본 서비스를 만든다. 모델이 없거나 읽히지 않으면 경고를 한 번 남기고 `None` 이다.
/// 로그에는 모델 경로만 남긴다. 서비스의 캐시 파일은 Python 과 같이 라이브러리 루트 아래 `classification_cache.json` 이다.
fn default_classifier_service(library_root: &Path) -> Option<Arc<SharedBookClassifier>> {
    let Some(model) = get_default_model() else {
        log::warn!("분류 모델을 읽지 못해 분류기 없이 동작한다 (TM_CLASSIFIER_MODEL_DIR={})", model_path().display());
        return None;
    };
    let classifier = BookCategoryClassifier::new(Some(model as Arc<dyn CategoryModelApi>), ClassifierOptions { library_root: Some(library_root.to_path_buf()), ..Default::default() });
    Some(Arc::new(SharedBookClassifier::new(BookClassifierService::new(library_root, None, Some(Arc::new(classifier))))))
}
