//! 분류 제안의 근거별 카테고리 점수와 가중치 합산. 파일·ES를 모르는 순수 계산이다.
//!
//! `score(c) = Σ w · p(c) / Σ w`. 합은 그 파일에 "있음"인 근거만 더한다. 근거는 ISBN·서점·키워드·모델 넷이다.

use std::collections::BTreeMap;

use search_manager::ISBN_MATCH_LIMIT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    Isbn,
    Bookstore,
    Keyword,
    Model,
}

impl Evidence {
    /// 후보 `source`에 싣는 이름.
    pub fn name(self) -> &'static str {
        match self {
            Evidence::Isbn => "isbn",
            Evidence::Bookstore => "bookstore",
            Evidence::Keyword => "keyword",
            Evidence::Model => "model",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Evidence::Isbn => "ISBN",
            Evidence::Bookstore => "서점",
            Evidence::Keyword => "키워드",
            Evidence::Model => "모델",
        }
    }

    pub fn weight(self) -> f64 {
        EVIDENCE_WEIGHTS.iter().find(|(e, _)| *e == self).map_or(0.0, |(_, w)| *w)
    }
}

/// 초기 가중치. 순위는 ISBN > 서점 > 키워드 > 모델이다. 튜닝하지 않은 값이다.
pub const EVIDENCE_WEIGHTS: [(Evidence, f64); 4] = [(Evidence::Isbn, 0.45), (Evidence::Bookstore, 0.30), (Evidence::Keyword, 0.15), (Evidence::Model, 0.10)];

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredCategory {
    pub category: String,
    pub score: f64,
    /// 이 카테고리에 점수를 준 근거와 그 근거의 원래 점수 `p`.
    pub contributions: Vec<(Evidence, f64)>,
}

impl ScoredCategory {
    /// 합산 점수에 가장 크게 기여한(`w · p`) 근거.
    pub fn dominant(&self) -> Evidence {
        self.contributions.iter().max_by(|a, b| (a.0.weight() * a.1).total_cmp(&(b.0.weight() * b.1))).map_or(Evidence::Model, |(e, _)| *e)
    }
}

fn shares(counts: BTreeMap<String, usize>) -> Option<Vec<(String, f64)>> {
    let total: usize = counts.values().sum();
    if total == 0 {
        return None;
    }
    Some(counts.into_iter().map(|(c, n)| (c, n as f64 / total as f64)).collect())
}

/// 같은 ISBN 문서들이 있는 최상위 카테고리(경로 첫 조각) 비율. 승인은 최상위 카테고리만 목적지로 받으므로 하위 디렉토리로 나누지 않는다.
/// 루트 낱개 파일은 세지 않는다. 일치가 없거나 상한 이상이면(잘못 추출된 흔한 값) 근거가 없다.
pub fn isbn_scores(matched_paths: &[String]) -> Option<Vec<(String, f64)>> {
    if matched_paths.len() >= ISBN_MATCH_LIMIT {
        return None;
    }
    let mut counts = BTreeMap::new();
    for path in matched_paths {
        if let Some((category, rest)) = path.split_once('/')
            && !category.is_empty()
            && !rest.is_empty()
        {
            *counts.entry(category.to_string()).or_insert(0) += 1;
        }
    }
    shares(counts)
}

/// 카테고리별 일치 키워드 수의 비율.
pub fn keyword_scores(ranked: &[(String, Vec<String>)]) -> Option<Vec<(String, f64)>> {
    shares(ranked.iter().map(|(c, k)| (c.clone(), k.len())).filter(|(_, n)| *n > 0).collect())
}

/// 합산 점수 내림차순. 동점은 카테고리 이름 오름차순.
pub fn combine(parts: &[(Evidence, Vec<(String, f64)>)]) -> Vec<ScoredCategory> {
    let weight_total: f64 = parts.iter().map(|(e, _)| e.weight()).sum();
    if weight_total <= 0.0 {
        return Vec::new();
    }
    let mut by_category: BTreeMap<String, ScoredCategory> = BTreeMap::new();
    for (evidence, scores) in parts {
        for (category, p) in scores {
            let entry = by_category.entry(category.clone()).or_insert_with(|| ScoredCategory { category: category.clone(), score: 0.0, contributions: Vec::new() });
            entry.score += evidence.weight() * p / weight_total;
            entry.contributions.push((*evidence, *p));
        }
    }
    let mut ranked: Vec<ScoredCategory> = by_category.into_values().collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.category.cmp(&b.category)));
    ranked
}

/// 사람이 읽는 근거 요약. 예: `"ISBN 0.50 · 모델 0.41"`.
pub fn detail_of(c: &ScoredCategory) -> String {
    c.contributions.iter().map(|(e, p)| format!("{} {p:.2}", e.label())).collect::<Vec<_>>().join(" · ")
}
