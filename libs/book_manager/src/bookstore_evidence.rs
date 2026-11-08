//! 서점 근거. 서점 세 곳의 검색 결과를 각각 내부 카테고리 한 표로 바꾸고, 표가 몇 개 모였는지로 0~1 점수를 낸다.
//!
//! 서점 조회와 캐시는 이 crate 밖(backend)이 `BookstoreLookup`으로 구현해 꽂는다. 여기는 순수 계산이다.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::bookstore_query_builder::BookQuery;
use crate::store_category_matcher::{add_in_order, category_scores, multi_path_categories};

/// 표를 내는 서점. 순서가 화면과 payload의 순서다.
pub const STORES: [&str; 3] = ["yes24", "aladin", "kyobo"];

/// 같은 카테고리에 표를 던진 서점 수(0~3) → 서점 점수. 분모는 언제나 3이다.
pub const AGREEMENT_SCORES: [f64; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];

/// 서점 한 곳의 조회 결과 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupStatus {
    Found,
    NotFound,
    Blocked,
    Error,
}

/// 서점 검색 결과 한 행. 표에는 `category`만 쓴다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoreRow {
    pub title: String,
    pub author: String,
    pub category: String,
}

/// 서점 한 곳의 조회 결과.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreResult {
    pub store: String,
    pub status: LookupStatus,
    pub rows: Vec<StoreRow>,
}

/// 파일 하나의 서점 조회 결과. `network_used`는 서점을 실제로 불렀는지다(캐시로 끝나면 거짓).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupOutcome {
    pub results: Vec<StoreResult>,
    pub network_used: bool,
}

/// 서점 조회. backend가 캐시와 서점 검색으로 구현한다.
pub trait BookstoreLookup: Send + Sync {
    fn lookup(&self, inode: i64, query: &BookQuery) -> LookupOutcome;

    /// 제안 하나 동안의 서점별 집계. 제안이 끝날 때 진행 상태의 `bookstore_counts`로 보고한다.
    fn summary(&self) -> Option<serde_json::Value> {
        None
    }
}

/// 분류 제안 한 번에 한 번 부르는 조회 팩토리.
pub type BookstoreLookupFactory = Arc<dyn Fn() -> Option<Arc<dyn BookstoreLookup>> + Send + Sync>;

/// 서점 한 곳의 표. 표를 내지 않으면 `category`가 없고 `abstain`에 이유가 있다
/// (`not_found`/`blocked`/`error`/`no_match`/`tie`/`excluded`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoreVote {
    pub store: String,
    pub category: Option<String>,
    pub abstain: Option<String>,
}

fn abstain(store: &str, reason: &str) -> StoreVote {
    StoreVote { store: store.to_string(), category: None, abstain: Some(reason.to_string()) }
}

/// 서점 한 곳의 결과를 표 하나로 바꾼다. 행마다 `||` 경로를 두 단계로 줄인 문자열의 대응 점수를 카테고리별로 더해 1등을 고른다.
/// `allowed`가 거르는 카테고리(0_ 접두, 최상위 아님, 디렉토리 없음)가 1등이면 기권한다. 동점 1등도 기권한다.
pub fn store_vote(result: &StoreResult, categories: &[String], mappings: &BTreeMap<String, Vec<String>>, allowed: &dyn Fn(&str) -> bool) -> StoreVote {
    match result.status {
        LookupStatus::NotFound => return abstain(&result.store, "not_found"),
        LookupStatus::Blocked => return abstain(&result.store, "blocked"),
        LookupStatus::Error => return abstain(&result.store, "error"),
        LookupStatus::Found if result.rows.is_empty() => return abstain(&result.store, "not_found"),
        LookupStatus::Found => {}
    }
    let mut totals: Vec<(String, f64)> = Vec::new();
    for row in &result.rows {
        for path in multi_path_categories(&row.category) {
            for (category, score) in category_scores(&path, categories, mappings) {
                add_in_order(&mut totals, &category, score);
            }
        }
    }
    let Some(best) = totals.iter().map(|(_, s)| *s).max_by(f64::total_cmp) else {
        return abstain(&result.store, "no_match");
    };
    let top: Vec<&String> = totals.iter().filter(|(_, s)| *s == best).map(|(c, _)| c).collect();
    if top.len() > 1 {
        return abstain(&result.store, "tie");
    }
    if !allowed(top[0]) {
        return abstain(&result.store, "excluded");
    }
    StoreVote { store: result.store.clone(), category: Some(top[0].clone()), abstain: None }
}

/// 표 수에 따른 서점 점수. 표가 하나도 없으면(모두 기권) 서점 근거가 없다.
pub fn bookstore_scores(votes: &[StoreVote]) -> Option<Vec<(String, f64)>> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for category in votes.iter().filter_map(|v| v.category.as_ref()) {
        match counts.iter_mut().find(|(c, _)| c == category) {
            Some((_, n)) => *n += 1,
            None => counts.push((category.clone(), 1)),
        }
    }
    if counts.is_empty() {
        return None;
    }
    Some(counts.into_iter().map(|(c, n)| (c, AGREEMENT_SCORES[n.min(3)])).collect())
}
