//! 원본: tests/test_cover_browse.py
//!
//! 커버 목록 묶음 규칙(backend/cover_browse.py)을 ES 없이 확인한다. 순수 함수만 쓴다.
//!
//! 가정한 API (아직 구현 전이다):
//! - `backend::cover_browse::MISSING_TIME: f64`
//! - `type CoverStats = BTreeMap<String, (u64, Option<f64>, Option<f64>)>` (카테고리 -> (문서 수, 수정 시각, 생성 시각))
//! - `type CoverHit = (i64, serde_json::Value, Vec<serde_json::Value>)` (book_id, _source, ES sort 값 `[수정, 생성, 경로]`)
//! - `struct Rank` (`PartialEq + PartialOrd + Debug`), `fn latest_rank(updated: Option<f64>, created: Option<f64>, path: &str) -> Rank`
//! - `fn file_rank(sort_values: &[Value]) -> Rank`
//! - `struct CoverGroup { pub name: String, pub key: String, pub updated: Option<f64>, pub created: Option<f64> }`
//!   - `CoverGroup::new(name: &str, key: &str, updated: Option<f64>, created: Option<f64>) -> CoverGroup`
//!   - `fn rank(&self) -> Rank`, `fn cursor_values(&self) -> Vec<Value>` (`[수정|null, 생성|null, key]`)
//! - `fn plan_cover_scope(stats: &CoverStats, scope: &str, hidden: &[String]) -> (Vec<String>, Vec<CoverGroup>)`
//! - `fn pick_representative(hits: &[(i64, Value)]) -> Option<(i64, Value)>`
//! - `enum CoverEntry { Group(CoverGroup), File(CoverHit) }`, `fn merge_cover_page(groups: &[CoverGroup], files: &[CoverHit], size: usize) -> (Vec<CoverEntry>, usize, usize)`

use std::collections::{BTreeMap, BTreeSet};

use backend::cover_browse::{CoverEntry, CoverGroup, CoverHit, CoverStats, MISSING_TIME, file_rank, latest_rank, merge_cover_page, pick_representative, plan_cover_scope};
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

    /// Python `latest_rank(*group.cursor_values)`: 커서 값 `[수정, 생성, 경로]`을 그대로 풀어 넘긴다.
    fn rank_of_cursor(values: &[Value]) -> backend::cover_browse::Rank {
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

    /// Python `(kind, value.name if kind == "group" else value[1]["title"])`.
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
