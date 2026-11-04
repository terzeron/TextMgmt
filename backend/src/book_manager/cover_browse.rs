//! 책·만화 탭 커버 목록의 묶음 규칙.
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

use file_record::ignore_rules::IGNORED_DIR_NAMES;
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
    #[cfg(test)]
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
    // 처음 본 순서를 유지한 뒤 안정 정렬한다.
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

#[cfg(test)]
mod tests {
    //! 커버 목록 묶음 규칙 테스트.
    //!
    //! 묶음 규칙을 ES 없이 확인한다. 순수 함수만 쓴다.
    //!

    use std::collections::{BTreeMap, BTreeSet};

    use super::{CoverEntry, CoverGroup, CoverHit, CoverStats, MISSING_TIME, file_rank, latest_rank, merge_cover_page, pick_representative, plan_cover_scope};
    use serde_json::{Value, json};

    /// {카테고리: 문서 수}를 ES 집계 결과 모양 (수, 수정 시각, 생성 시각)으로 바꾼다.
    fn stats(counts: &[(&str, u64)], updated: &[(&str, f64)], created: &[(&str, f64)]) -> CoverStats {
        let find = |list: &[(&str, f64)], key: &str| list.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        counts.iter().map(|(c, n)| (c.to_string(), (*n, find(updated, c), find(created, c)))).collect::<BTreeMap<_, _>>()
    }

    fn hit(book_id: i64, title: &str, updated: Option<f64>, created: Option<f64>, path: &str) -> CoverHit {
        let path = if path.is_empty() { format!("{title}.epub") } else { path.to_string() };
        (book_id, json!({"title": title, "file_path": path}), vec![json!(updated.unwrap_or(MISSING_TIME)), json!(created.unwrap_or(MISSING_TIME)), json!(path)])
    }

    fn group(name: &str, key: &str, updated: Option<f64>, created: Option<f64>) -> CoverGroup {
        CoverGroup::new(name, key, updated, created)
    }

    fn keys(groups: &[CoverGroup]) -> Vec<&str> {
        groups.iter().map(|g| g.key.as_str()).collect()
    }

    fn key_set(groups: &[CoverGroup]) -> BTreeSet<&str> {
        groups.iter().map(|g| g.key.as_str()).collect()
    }

    fn set(items: &[&'static str]) -> BTreeSet<&'static str> {
        items.iter().copied().collect()
    }

    mod test_rank {
        use super::*;

        #[test]
        fn test_newer_comes_first_then_created_then_path() {
            assert!(latest_rank(Some(200.0), Some(1.0), "b") < latest_rank(Some(100.0), Some(9.0), "a"));
            assert!(latest_rank(Some(100.0), Some(9.0), "b") < latest_rank(Some(100.0), Some(1.0), "a"));
            assert!(latest_rank(Some(100.0), Some(1.0), "a") < latest_rank(Some(100.0), Some(1.0), "b"));
        }

        #[test]
        fn test_missing_times_sort_last() {
            assert!(latest_rank(Some(100.0), Some(1.0), "a") < latest_rank(None, Some(1.0), "a"));
            assert!(latest_rank(Some(100.0), Some(1.0), "a") < latest_rank(Some(100.0), None, "a"));
        }

        #[test]
        fn test_file_rank_matches_es_sort_values() {
            assert_eq!(file_rank(&[json!(200), json!(5), json!("a.epub")]), latest_rank(Some(200.0), Some(5.0), "a.epub"));
            assert_eq!(file_rank(&[json!(MISSING_TIME), json!(MISSING_TIME), json!("a.epub")]), latest_rank(None, None, "a.epub"));
        }
    }

    mod test_plan_cover_scope {
        use super::*;

        const COUNTS: [(&str, u64); 7] =
            [("_root", 2), ("5_수학과학일반", 100), ("5_완결", 1), ("5_완결/[삼촌] 이런 영웅은 싫어", 303), ("5_완결/암살교실", 185), ("5_완결/암살교실/외전", 4), ("0_웹툰/9차원 소녀", 9)];

        fn plan(scope: &str, hidden: &[&str], updated: &[(&str, f64)], created: &[(&str, f64)]) -> (Vec<String>, Vec<CoverGroup>) {
            let hidden: Vec<String> = hidden.iter().map(|s| s.to_string()).collect();
            plan_cover_scope(&stats(&COUNTS, updated, created), scope, &hidden)
        }

        #[test]
        fn test_root_scope_keeps_root_files_and_groups_top_level_directories() {
            let (files, groups) = plan("", &[], &[], &[]);

            // 최상위 디렉토리(5_완결 등)의 직속 파일은 루트 화면에서 펼치지 않고 디렉토리 대표 한 권으로 묶는다.
            assert_eq!(files, ["_root"]);
            assert_eq!(key_set(&groups), set(&["5_수학과학일반", "5_완결", "0_웹툰"]));
        }

        #[test]
        fn test_top_level_directory_direct_files_do_not_leak_into_root_files() {
            let (files, _groups) = plan_cover_scope(&stats(&[("3_판타지", 12), ("3_무협", 1)], &[], &[]), "", &[]);

            assert_eq!(files, Vec::<String>::new());
        }

        #[test]
        fn test_deeper_directories_collapse_into_their_top_level_group() {
            let (_files, groups) = plan("", &[], &[], &[]);

            assert_eq!(keys(&groups).iter().filter(|k| **k == "5_완결").count(), 1);
        }

        #[test]
        fn test_groups_are_sorted_newest_first() {
            let (_files, groups) = plan("", &[], &[("0_웹툰/9차원 소녀", 300.0), ("5_완결/암살교실", 200.0), ("5_수학과학일반", 100.0)], &[]);

            assert_eq!(keys(&groups), ["0_웹툰", "5_완결", "5_수학과학일반"]);
        }

        #[test]
        fn test_group_time_is_the_newest_time_inside_it() {
            // 5_완결 안에서 가장 최근 문서가 하위 디렉토리(외전)에 있어도 5_완결 묶음의 시각이 된다.
            let (_files, groups) = plan("", &[], &[("5_완결/암살교실/외전", 500.0), ("5_완결/암살교실", 10.0), ("0_웹툰/9차원 소녀", 100.0)], &[("5_완결", 7.0)]);

            let by_key: BTreeMap<&str, &CoverGroup> = groups.iter().map(|g| (g.key.as_str(), g)).collect();
            assert_eq!(by_key["5_완결"].updated, Some(500.0));
            assert_eq!(by_key["5_완결"].created, Some(7.0));
            assert_eq!(keys(&groups)[0], "5_완결");
        }

        #[test]
        fn test_groups_without_times_go_last_and_ties_use_key() {
            let (_files, groups) = plan_cover_scope(&stats(&[("b/x", 1), ("a/x", 1), ("c/x", 1)], &[("c/x", 5.0)], &[]), "", &[]);

            assert_eq!(keys(&groups), ["c", "a", "b"]);
        }

        #[test]
        fn test_selected_directory_keeps_direct_files_and_groups_children() {
            let (files, groups) = plan("5_완결", &[], &[("5_완결/암살교실", 9.0), ("5_완결/[삼촌] 이런 영웅은 싫어", 1.0)], &[]);

            assert_eq!(files, ["5_완결"]);
            assert_eq!(keys(&groups), ["5_완결/암살교실", "5_완결/[삼촌] 이런 영웅은 싫어"]);
        }

        #[test]
        fn test_selected_work_directory_groups_its_subdirectories() {
            let (files, groups) = plan("5_완결/암살교실", &[], &[], &[]);

            assert_eq!(files, ["5_완결/암살교실"]);
            assert_eq!(groups.iter().map(|g| (g.name.as_str(), g.key.as_str())).collect::<Vec<_>>(), [("외전", "5_완결/암살교실/외전")]);
        }

        #[test]
        fn test_selected_directory_without_direct_files_has_only_groups() {
            let (files, groups) = plan_cover_scope(&stats(&[("a/b", 1), ("a/c", 1)], &[], &[]), "a", &[]);

            assert_eq!(files, Vec::<String>::new());
            assert_eq!(groups.iter().map(|g| g.name.as_str()).collect::<BTreeSet<_>>(), set(&["b", "c"]));
        }

        #[test]
        fn test_scope_does_not_match_sibling_with_same_prefix() {
            let (files, groups) = plan_cover_scope(&stats(&[("5_완결", 1), ("5_완결2", 1), ("5_완결2/x", 1)], &[], &[]), "5_완결", &[]);

            assert_eq!(files, ["5_완결"]);
            assert!(groups.is_empty());
        }

        #[test]
        fn test_hidden_categories_are_dropped_with_their_subtrees() {
            let (files, groups) = plan("", &["5_완결/암살교실", "5_수학과학일반"], &[], &[]);

            assert_eq!(files, ["_root"]);
            assert_eq!(key_set(&groups), set(&["5_완결", "0_웹툰"]));
        }

        #[test]
        fn test_hidden_top_level_directory_hides_its_group() {
            let (_files, groups) = plan("", &["5_완결"], &[], &[]);

            assert_eq!(key_set(&groups), set(&["5_수학과학일반", "0_웹툰"]));
        }

        #[test]
        fn test_cache_and_extraction_directories_are_not_listed() {
            let counts = [(".preview_cache", 48), ("소설/.hidden", 1), ("소설/OEBPS", 3), ("소설/작품", 5), ("0_trash", 52)];

            let (_files, root_groups) = plan_cover_scope(&stats(&counts, &[], &[]), "", &[]);
            let (_files, scoped) = plan_cover_scope(&stats(&counts, &[], &[]), "소설", &[]);

            assert_eq!(key_set(&root_groups), set(&["소설", "0_trash"]));
            assert_eq!(keys(&scoped), ["소설/작품"]);
        }
    }

    mod test_cover_group {
        use super::*;

        /// 커서 값 `[수정, 생성, 경로]`을 그대로 풀어 `latest_rank`에 넘긴다.
        fn rank_of_cursor(values: &[Value]) -> crate::book_manager::cover_browse::Rank {
            latest_rank(values[0].as_f64(), values[1].as_f64(), values[2].as_str().unwrap())
        }

        #[test]
        fn test_cursor_values_round_trip_into_the_same_rank() {
            let g = group("작품", "a/작품", Some(123.0), Some(45.0));

            assert_eq!(rank_of_cursor(&g.cursor_values()), g.rank());
        }

        #[test]
        fn test_cursor_values_keep_missing_times() {
            let g = group("작품", "a/작품", None, None);

            assert_eq!(g.cursor_values(), vec![Value::Null, Value::Null, json!("a/작품")]);
            assert_eq!(rank_of_cursor(&g.cursor_values()), g.rank());
        }
    }

    mod test_pick_representative {
        use super::*;

        #[test]
        fn test_prefers_first_installment() {
            let hits = [(1, json!({"title": "작품 02권", "file_path": "d/작품 02권.pdf"})), (2, json!({"title": "작품 01권", "file_path": "d/작품 01권.pdf"}))];

            assert_eq!(pick_representative(&hits).unwrap().0, 2);
        }

        #[test]
        fn test_does_not_confuse_numbers_that_merely_contain_one() {
            let hits = [(1, json!({"title": "작품 11권", "file_path": "d/작품 11권.pdf"})), (2, json!({"title": "작품 21화", "file_path": "d/작품 21화.pdf"}))];

            assert_eq!(pick_representative(&hits).unwrap().0, 1);
        }

        #[test]
        fn test_falls_back_to_first_candidate() {
            let hits = [(7, json!({"title": "가", "file_path": "d/가.pdf"})), (8, json!({"title": "나", "file_path": "d/나.pdf"}))];

            assert_eq!(pick_representative(&hits).unwrap().0, 7);
        }

        #[test]
        fn test_returns_none_without_candidates() {
            assert!(pick_representative(&[]).is_none());
        }

        #[test]
        fn test_matches_on_file_name_when_title_lacks_installment() {
            let hits = [(1, json!({"title": "작품", "file_path": "d/작품 02화.pdf"})), (2, json!({"title": "작품", "file_path": "d/작품 1화.pdf"}))];

            assert_eq!(pick_representative(&hits).unwrap().0, 2);
        }
    }

    mod test_merge_cover_page {
        use super::*;

        /// `(종류, 묶음 이름 또는 파일 제목)`.
        fn describe(entries: &[CoverEntry]) -> Vec<(&'static str, String)> {
            entries
                .iter()
                .map(|e| match e {
                    CoverEntry::Group(g) => ("group", g.name.clone()),
                    CoverEntry::File(f) => ("file", f.1["title"].as_str().unwrap().to_string()),
                })
                .collect()
        }

        fn kinds(entries: &[CoverEntry]) -> Vec<&'static str> {
            describe(entries).into_iter().map(|(k, _)| k).collect()
        }

        #[test]
        fn test_merges_groups_and_files_newest_first() {
            let groups = [group("나", "x/나", Some(400.0), None), group("마", "x/마", Some(100.0), None)];
            let files = [hit(1, "가", Some(500.0), Some(0.0), ""), hit(2, "다", Some(200.0), Some(0.0), "")];

            let (entries, groups_used, files_used) = merge_cover_page(&groups, &files, 10);

            assert_eq!(describe(&entries), [("file", "가".to_string()), ("group", "나".to_string()), ("file", "다".to_string()), ("group", "마".to_string())]);
            assert_eq!((groups_used, files_used), (2, 2));
        }

        #[test]
        fn test_group_comes_first_when_rank_ties() {
            let (entries, _g, _f) = merge_cover_page(&[group("x", "d", Some(100.0), Some(0.0))], &[hit(1, "x", Some(100.0), Some(0.0), "d")], 10);

            assert_eq!(kinds(&entries), ["group", "file"]);
        }

        #[test]
        fn test_files_without_times_go_after_groups_with_times() {
            let (entries, _g, _f) = merge_cover_page(&[group("작품", "x/작품", Some(100.0), None)], &[hit(1, "오래된 책", None, Some(0.0), "")], 10);

            assert_eq!(kinds(&entries), ["group", "file"]);
        }

        #[test]
        fn test_stops_at_size_and_reports_how_many_were_used() {
            let groups = [group("나", "x/나", Some(400.0), None), group("라", "x/라", Some(100.0), None)];
            let files = [hit(1, "가", Some(500.0), Some(0.0), ""), hit(2, "다", Some(200.0), Some(0.0), "")];

            let (entries, groups_used, files_used) = merge_cover_page(&groups, &files, 3);

            assert_eq!(entries.len(), 3);
            assert_eq!((groups_used, files_used), (1, 2));
        }

        #[test]
        fn test_handles_one_empty_side() {
            let (_, g, f) = merge_cover_page(&[], &[hit(1, "가", Some(1.0), Some(0.0), "")], 5);
            assert_eq!((g, f), (0, 1));
            let (_, g, f) = merge_cover_page(&[group("가", "x/가", Some(1.0), None)], &[], 5);
            assert_eq!((g, f), (1, 0));
            let (entries, g, f) = merge_cover_page(&[], &[], 5);
            assert!(entries.is_empty());
            assert_eq!((g, f), (0, 0));
        }
    }
}
