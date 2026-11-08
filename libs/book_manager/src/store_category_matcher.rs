//! 서점 카테고리 문자열을 내부 카테고리에 대응시킨다.
//!
//! 프런트엔드 `Actions.jsx`(`calculateCategoryScores` 등)와 `Bookstore.jsx`(`getTwoLevelCategory`,
//! `extractMultiPathCategories`)의 로직을 동작 그대로 옮겼다. 새 규칙을 넣지 않는다.
//! JS와 같게 문자열 길이·부분문자열·n-gram은 UTF-16 code unit으로 센다. 순서가 결과를 바꾸는 곳(동점 정렬,
//! 객체 키 순서)은 JS의 삽입 순서와 안정 정렬을 따른다.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use serde::ser::SerializeMap;

type Utf16 = Vec<u16>;

fn utf16(s: &str) -> Utf16 {
    s.encode_utf16().collect()
}

fn contains(hay: &[u16], needle: &[u16]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

fn contains_korean(s: &[u16]) -> bool {
    s.iter().any(|c| (0xAC00..=0xD7AF).contains(c))
}

fn ngrams(s: &[u16], n: usize) -> HashSet<&[u16]> {
    if s.len() < n { HashSet::new() } else { s.windows(n).collect() }
}

/// `ngramSimilarity`: n-gram Jaccard 유사도(0~1).
fn ngram_similarity(a: &[u16], b: &[u16], n: usize) -> f64 {
    if a.len() < n || b.len() < n {
        return if a == b { 1.0 } else { 0.0 };
    }
    let (x, y) = (ngrams(a, n), ngrams(b, n));
    let intersection = x.iter().filter(|g| y.contains(*g)).count();
    let union = x.len() + y.len() - intersection;
    if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
}

/// `calculateSimilarity`: 완전 일치 1.0 > 포함 관계 0.7~0.9 > n-gram 0.3~0.6 > 0.
pub fn calculate_similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (s1, s2) = (utf16(&a.to_lowercase()), utf16(&b.to_lowercase()));
    if s1 == s2 {
        return 1.0;
    }
    let shorter = if s1.len() <= s2.len() { &s1 } else { &s2 };
    let longer = if s1.len() > s2.len() { &s1 } else { &s2 };
    let min_length = if contains_korean(shorter) { 1 } else { 2 };
    if contains(longer, shorter) && shorter.len() >= min_length {
        let ratio = shorter.len() as f64 / longer.len() as f64;
        let accidental = contains_korean(shorter) && ((shorter.len() == 1 && longer.len() > 2) || (shorter.len() >= 2 && ratio <= 0.5));
        if !accidental {
            return 0.7 + ratio * 0.2;
        }
    }
    let ngram_score = ngram_similarity(&s1, &s2, 2);
    let min_ngram_score = if contains_korean(&s1) || contains_korean(&s2) { 0.15 } else { 0.2 };
    if ngram_score >= min_ngram_score { 0.3 + ngram_score * 0.3 } else { 0.0 }
}

/// `splitBySpecialChars`: `/`, 괄호, 공백으로 나눈다.
fn split_by_special_chars(s: &str) -> Vec<String> {
    s.split(|c: char| c == '/' || c == '(' || c == ')' || c.is_whitespace()).map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

/// `filterSubstringKeywords`: 다른 키워드에 포함되는 짧은 키워드를 뺀다.
fn filter_substring_keywords(keywords: Vec<String>) -> Vec<String> {
    if keywords.len() <= 1 {
        return keywords;
    }
    let lowered: Vec<Utf16> = keywords.iter().map(|k| utf16(&k.to_lowercase())).collect();
    keywords.iter().enumerate().filter(|(i, kw)| !keywords.iter().enumerate().any(|(j, other)| other != *kw && contains(&lowered[j], &lowered[*i]))).map(|(_, kw)| kw.clone()).collect()
}

/// `stripNoiseWords`: `일반`을 지운다.
fn strip_noise_words(keywords: Vec<String>) -> Vec<String> {
    keywords.into_iter().map(|k| k.replace("일반", "").trim().to_string()).filter(|k| !k.is_empty()).collect()
}

/// `keywordCountFactor`: 키워드가 n개면 1개 100%, 2개 90%, ... 11개 이상 0%.
fn keyword_count_factor(n: usize) -> f64 {
    (1.0 - 0.1 * (n as f64 - 1.0)).max(0.0)
}

/// 서점 키워드 하나와 가장 잘 맞은 디렉토리 키워드.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeywordMatch {
    #[serde(rename = "bookstoreKeyword")]
    pub bookstore_keyword: String,
    #[serde(rename = "dirKeyword")]
    pub dir_keyword: String,
    pub similarity: f64,
    #[serde(rename = "isContained")]
    pub is_contained: bool,
    #[serde(rename = "matchType")]
    pub match_type: &'static str,
}

/// `scoreStoreKeywords`: 키워드별 최선 매칭 합에 키워드 수 감점을 곱한다.
fn score_store_keywords(keywords: &[String], dir_keywords: &[String]) -> (f64, Vec<KeywordMatch>) {
    let mut details = Vec::new();
    let mut sum = 0.0;
    for deep in keywords {
        let mut best = 0.0;
        let mut best_match = None;
        for dir in dir_keywords {
            let similarity = calculate_similarity(deep, dir);
            if similarity > best {
                best = similarity;
                let (dk, dr) = (utf16(&deep.to_lowercase()), utf16(&dir.to_lowercase()));
                let is_contained = contains(&dr, &dk) || contains(&dk, &dr);
                let match_type = if dk == dr {
                    "exact"
                } else if is_contained {
                    "contains"
                } else {
                    "ngram"
                };
                best_match = Some(KeywordMatch { bookstore_keyword: deep.clone(), dir_keyword: dir.clone(), similarity, is_contained, match_type });
            }
        }
        if let Some(m) = best_match {
            details.push(m);
        }
        sum += best;
    }
    (sum * keyword_count_factor(keywords.len()), details)
}

fn store_keywords(store_category: &str) -> Vec<String> {
    strip_noise_words(filter_substring_keywords(split_by_special_chars(store_category)))
}

/// 디렉토리명에서 숫자 접두를 뗀 이름. 예: `4_심리학뇌과학` → `심리학뇌과학`.
fn category_name(category: &str) -> String {
    match category.split_once('_') {
        Some((_, rest)) => rest.to_string(),
        None => category.to_string(),
    }
}

fn dir_keywords(category: &str, mappings: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut keywords = mappings.get(category).cloned().unwrap_or_default();
    keywords.push(category_name(category));
    strip_noise_words(keywords)
}

/// `getTwoLevelCategory`: `>` 경로의 마지막 두 단계. 예: `"소설/시/희곡 > SF > 한국SF"` → `"SF 한국SF"`.
pub fn two_level_category(category: &str) -> String {
    let parts: Vec<&str> = category.split('>').map(str::trim).filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_string(),
        n => format!("{} {}", parts[n - 2], parts[n - 1]),
    }
}

/// `extractMultiPathCategories`: `||`로 나뉜 경로마다 마지막 두 단계.
pub fn multi_path_categories(category: &str) -> Vec<String> {
    if category.is_empty() {
        return Vec::new();
    }
    category.split("||").map(|p| two_level_category(p.trim())).filter(|c| !c.is_empty()).collect()
}

/// `calculateCategoryScores`: 서점 카테고리 하나와 각 디렉토리의 점수. 0보다 큰 것만, `categories` 순서로 돌려준다.
pub fn category_scores(store_category: &str, categories: &[String], mappings: &BTreeMap<String, Vec<String>>) -> Vec<(String, f64)> {
    if store_category.is_empty() || categories.is_empty() {
        return Vec::new();
    }
    let keywords = store_keywords(store_category);
    if keywords.is_empty() {
        return Vec::new();
    }
    categories
        .iter()
        .filter_map(|category| {
            let (score, _) = score_store_keywords(&keywords, &dir_keywords(category, mappings));
            (score > 0.0).then(|| (category.clone(), score))
        })
        .collect()
}

/// 삽입 순서를 지키며 같은 키의 값을 더한다.
pub(crate) fn add_in_order(acc: &mut Vec<(String, f64)>, key: &str, value: f64) {
    match acc.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => *v += value,
        None => acc.push((key.to_string(), value)),
    }
}

/// `findTopSimilarCategories`: 여러 서점 카테고리 점수를 합산한 상위 `top_n`개. 동점은 처음 나온 순서다.
pub fn top_similar_categories(suggested: &[(String, String)], categories: &[String], mappings: &BTreeMap<String, Vec<String>>, top_n: usize) -> Vec<String> {
    if suggested.is_empty() || categories.is_empty() {
        return Vec::new();
    }
    let mut combined: Vec<(String, f64)> = Vec::new();
    for (_, store_category) in suggested {
        if store_category.is_empty() {
            continue;
        }
        for (category, score) in category_scores(store_category, categories, mappings) {
            add_in_order(&mut combined, &category, score);
        }
    }
    combined.sort_by(|a, b| b.1.total_cmp(&a.1));
    combined.into_iter().take(top_n).map(|(c, _)| c).collect()
}

/// 서점 하나에서 뽑은 키워드.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoreKeywords {
    pub original: String,
    pub keywords: Vec<String>,
}

/// 매칭 하나에 어느 서점 키워드였는지를 붙인 것.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoreKeywordMatch {
    pub store: String,
    #[serde(flatten)]
    pub matched: KeywordMatch,
}

/// 디렉토리 하나의 점수 계산 과정.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CategoryDetail {
    pub category: String,
    #[serde(rename = "categoryName")]
    pub category_name: String,
    #[serde(rename = "dirKeywords")]
    pub dir_keywords: Vec<String>,
    #[serde(rename = "totalScore")]
    pub total_score: f64,
    #[serde(rename = "matchDetails")]
    pub match_details: Vec<StoreKeywordMatch>,
}

/// `getSimilarityDebugInfo`의 결과. 키 이름과 순서가 화면이 읽는 JS 결과와 같다.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SimilarityDebug {
    #[serde(rename = "bookstoreKeywords", serialize_with = "ordered_map")]
    pub bookstore_keywords: Vec<(String, StoreKeywords)>,
    #[serde(rename = "categoryDetails")]
    pub category_details: Vec<CategoryDetail>,
}

fn ordered_map<S: serde::Serializer>(entries: &[(String, StoreKeywords)], serializer: S) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(Some(entries.len()))?;
    for (k, v) in entries {
        map.serialize_entry(k, v)?;
    }
    map.end()
}

/// `getSimilarityDebugInfo`: 상위 `top_n`개 디렉토리의 점수 계산 과정. 카테고리 목록이 비면 `None`이다.
pub fn similarity_debug(suggested: &[(String, String)], categories: &[String], mappings: &BTreeMap<String, Vec<String>>, top_n: usize) -> Option<SimilarityDebug> {
    if categories.is_empty() {
        return None;
    }
    let bookstore_keywords: Vec<(String, StoreKeywords)> =
        suggested.iter().filter(|(_, c)| !c.is_empty()).map(|(store, c)| (store.clone(), StoreKeywords { original: c.clone(), keywords: store_keywords(c) })).collect();
    let mut details: Vec<CategoryDetail> = categories
        .iter()
        .filter_map(|category| {
            let dir = dir_keywords(category, mappings);
            let mut match_details = Vec::new();
            let mut total_score = 0.0;
            for (store, info) in &bookstore_keywords {
                let (score, matches) = score_store_keywords(&info.keywords, &dir);
                match_details.extend(matches.into_iter().map(|matched| StoreKeywordMatch { store: store.clone(), matched }));
                total_score += score;
            }
            (total_score > 0.0).then(|| CategoryDetail { category: category.clone(), category_name: category_name(category), dir_keywords: dir, total_score, match_details })
        })
        .collect();
    details.sort_by(|a, b| b.total_score.total_cmp(&a.total_score));
    details.truncate(top_n);
    Some(SimilarityDebug { bookstore_keywords, category_details: details })
}
