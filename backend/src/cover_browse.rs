//! 책·만화 탭 커버 목록의 묶음 규칙. Python `backend/cover_browse.py`에 대응한다.
//!
//! 한 디렉토리에 권별 파일이 모여 있는 경우(만화 등)에 파일마다 커버를 그리면 같은 작품이 줄줄이
//! 이어진다. 그래서 기준 디렉토리(scope)마다 같은 규칙을 쓴다: scope에 직접 속한 파일은 한 권씩,
//! scope 바로 아래 디렉토리는 대표 한 권으로 묶는다. 전체 보기의 scope는 라이브러리 루트다.
//!
//! 정렬은 최신순이다(최신 탭과 같다). 파일은 수정 시각, 생성 시각이 큰 것부터, 같으면 경로순이다.
//! 묶음은 안에 든 문서 중 가장 최근 시각을 자기 시각으로 쓰고, 같으면 디렉토리 경로순이다.
//! 시각 값은 ES 정렬값과 같은 epoch 밀리초이고, 값이 없으면 가장 뒤로 보낸다.
//!
//! ES와 무관한 순수 로직만 둔다.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use corpus_layout::IGNORED_DIR_NAMES;
use regex::Regex;
use serde_json::Value;

/// 최신 만화 탭의 FIRST_INSTALLMENT_PATTERN과 같다. "1권", "01화"는 1권으로 본다.
/// Rust `regex`는 lookahead가 없어서 뒤쪽 경계를 소비하는 그룹으로 바꿨다(일치 여부만 쓰므로 결과가 같다).
static FIRST_INSTALLMENT_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^\d])0*1\s*(?:권|화)(?:$|[^\d])").expect("정규식"));

pub const ROOT_CATEGORY: &str = "_root";
/// ES가 desc 정렬에서 값 없는 문서에 주는 정렬값(missing: _last)과 같다.
pub const MISSING_TIME: f64 = -9_223_372_036_854_775_808.0;

/// 카테고리 -> (문서 수, 가장 최근 수정 시각, 가장 최근 생성 시각). 시각이 없으면 None.
pub type CoverStats = BTreeMap<String, (u64, Option<f64>, Option<f64>)>;
/// (book_id, _source, ES sort 값 `[수정, 생성, 경로]`)
pub type CoverHit = (i64, Value, Vec<Value>);

/// 최신순 비교 키. 오름차순으로 비교하면 가장 최근 것이 먼저 온다.
/// `-(-2^63)`이 i64를 넘으므로 i128로 담는다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank(i128, i128, String);

fn time_or_missing(value: Option<f64>) -> i128 {
    value.unwrap_or(MISSING_TIME) as i128
}

pub fn latest_rank(updated: Option<f64>, created: Option<f64>, path: &str) -> Rank {
    Rank(-time_or_missing(updated), -time_or_missing(created), path.to_string())
}

/// ES 최신순 정렬값 `[수정 시각, 생성 시각, 파일 경로]`를 비교 키로 바꾼다.
pub fn file_rank(sort_values: &[Value]) -> Rank {
    let path = match &sort_values[2] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    latest_rank(sort_values[0].as_f64(), sort_values[1].as_f64(), &path)
}

/// 작품 디렉토리 하나. `key`는 디렉토리 전체 경로, `name`은 화면에 보일 마지막 조각이다.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverGroup {
    pub name: String,
    pub key: String,
    pub updated: Option<f64>,
    pub created: Option<f64>,
}

impl CoverGroup {
    pub fn new(name: &str, key: &str, updated: Option<f64>, created: Option<f64>) -> CoverGroup {
        CoverGroup { name: name.to_string(), key: key.to_string(), updated, created }
    }

    pub fn rank(&self) -> Rank {
        latest_rank(self.updated, self.created, &self.key)
    }

    /// 다음 페이지 커서에 담을 값. `rank`와 같은 순서를 되살릴 수 있어야 한다.
    pub fn cursor_values(&self) -> Vec<Value> {
        vec![self.updated.map_or(Value::Null, Value::from), self.created.map_or(Value::Null, Value::from), Value::from(self.key.clone())]
    }
}

pub fn category_matches(category: &str, hidden_categories: &[String]) -> bool {
    hidden_categories.iter().any(|hidden| category == hidden || category.strip_prefix(hidden.as_str()).is_some_and(|rest| rest.starts_with('/')))
}

/// 캐시·추출 산출물 디렉토리는 라이브러리 책이 아니므로 목록에서 뺀다. 색인 순회와 같은 기준이다.
pub fn is_ignored_category(category: &str) -> bool {
    category.split('/').any(|segment| segment.starts_with('.') || IGNORED_DIR_NAMES.contains(&segment))
}

fn later(current: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    match (current, candidate) {
        (current, None) => current,
        (None, candidate) => candidate,
        (Some(a), Some(b)) => Some(a.max(b)),
    }
}

/// 보여줄 범위를 개별 파일 카테고리와 작품 묶음으로 나눈다.
///
/// scope가 비어 있으면 라이브러리 루트다. 루트 직속 파일(`_root`)은 한 권씩, 최상위 디렉토리는
/// 대표 한 권으로 묶는다. scope가 있으면 그 디렉토리에 직접 있는 파일은 한 권씩, 바로 아래
/// 디렉토리는 대표 한 권으로 묶는다. 묶음은 최신순으로 돌려준다.
pub fn plan_cover_scope(category_stats: &CoverStats, scope: &str, hidden_categories: &[String]) -> (Vec<String>, Vec<CoverGroup>) {
    let mut file_categories: Vec<String> = Vec::new();
    // Python dict처럼 처음 본 순서를 유지한 뒤 안정 정렬한다.
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, CoverGroup> = BTreeMap::new();
    for (category, &(_count, updated, created)) in category_stats {
        if is_ignored_category(category) || category_matches(category, hidden_categories) {
            continue;
        }
        let (key, name) = if scope.is_empty() {
            if category == ROOT_CATEGORY {
                file_categories.push(category.clone());
                continue;
            }
            let top = category.split('/').next().unwrap_or("").to_string();
            (top.clone(), top)
        } else {
            if category == scope {
                file_categories.push(category.clone());
                continue;
            }
            let Some(rest) = category.strip_prefix(scope).and_then(|r| r.strip_prefix('/')) else { continue };
            let first = rest.split('/').next().unwrap_or("").to_string();
            (format!("{scope}/{first}"), first)
        };
        let known = groups.get(&key);
        let merged = CoverGroup { name, key: key.clone(), updated: later(known.and_then(|g| g.updated), updated), created: later(known.and_then(|g| g.created), created) };
        if known.is_none() {
            order.push(key.clone());
        }
        groups.insert(key, merged);
    }
    let mut sorted: Vec<CoverGroup> = order.into_iter().filter_map(|key| groups.remove(&key)).collect();
    sorted.sort_by_key(CoverGroup::rank);
    (file_categories, sorted)
}

/// 파일 경로순으로 받은 후보 중 1권을 우선하고, 없으면 맨 앞을 대표로 한다.
pub fn pick_representative(hits: &[(i64, Value)]) -> Option<(i64, Value)> {
    let first = hits.first()?;
    for hit in hits {
        let source = &hit.1;
        let file_path = source.get("file_path").and_then(Value::as_str).unwrap_or("");
        let file_name = file_path.rsplit('/').next().unwrap_or("");
        let title = source.get("title").and_then(Value::as_str).unwrap_or("");
        if FIRST_INSTALLMENT_PATTERN.is_match(&format!("{title} {file_name}")) {
            return Some(hit.clone());
        }
    }
    Some(first.clone())
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoverEntry {
    Group(CoverGroup),
    File(CoverHit),
}

/// 최신순으로 정렬된 묶음과 파일을 합쳐 `size`개만 고른다.
///
/// 순서가 같으면 묶음이 먼저다. 반환: (항목 목록, 쓴 묶음 수, 쓴 파일 수).
pub fn merge_cover_page(groups: &[CoverGroup], files: &[CoverHit], size: usize) -> (Vec<CoverEntry>, usize, usize) {
    let mut entries = Vec::new();
    let (mut group_index, mut file_index) = (0, 0);
    while entries.len() < size && (group_index < groups.len() || file_index < files.len()) {
        let mut take_group = file_index >= files.len();
        if !take_group && group_index < groups.len() {
            take_group = groups[group_index].rank() <= file_rank(&files[file_index].2);
        }
        if take_group {
            entries.push(CoverEntry::Group(groups[group_index].clone()));
            group_index += 1;
        } else {
            entries.push(CoverEntry::File(files[file_index].clone()));
            file_index += 1;
        }
    }
    (entries, group_index, file_index)
}
