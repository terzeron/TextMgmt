use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use normalize_comic_directories::cache::{BookstoreSearchCache, StoreRow};
use normalize_comic_directories::cli::{CacheOnly, LiveSearch, Options, parse_options, run, run_with_searcher, run_with_searcher_and_defaults};
use normalize_comic_directories::plan::*;

fn row(title: &str, author: &str) -> StoreRow {
    StoreRow { title: title.into(), author: author.into(), category: "만화".into(), url: "http://x/1".into(), isbn: "".into() }
}

/// 서점별로 준비한 답을 주고, 부른 기록을 남긴다.
struct Fake {
    live: bool,
    answers: HashMap<String, Result<Vec<StoreRow>, String>>,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn new(answers: Vec<(&str, Result<Vec<StoreRow>, String>)>) -> Self {
        Fake { live: true, answers: answers.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(), calls: RefCell::new(Vec::new()) }
    }
}

impl Searcher for Fake {
    fn is_live(&self) -> bool {
        self.live
    }
    fn search(&self, store: &str, title: &str, author: &str) -> Result<Vec<StoreRow>, String> {
        self.calls.borrow_mut().push(format!("{store}:{title}:{author}"));
        self.answers.get(store).cloned().unwrap_or(Ok(Vec::new()))
    }
}

fn cache_in(dir: &Path) -> BookstoreSearchCache {
    BookstoreSearchCache::new(&dir.join("cache.jsonl"), false, 7 * 24 * 3600)
}

const STORES: [&str; 3] = ["yes24", "aladin", "kyobo"];

#[test]
fn canonical_title_takes_the_first_store_with_a_korean_author_and_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cache = cache_in(tmp.path());
    let fake = Fake::new(vec![("yes24", Ok(vec![row("원피스 1권", "오다 에이이치로 만화"), row("딴책", "x")])), ("aladin", Ok(vec![row("원피스", "다른 저자")]))]);
    let got = canonical_book_title("원피스", None, &STORES, 0.0, "q", &mut cache, &fake);
    assert_eq!(got.title.as_deref(), Some("원피스"));
    assert_eq!(got.author.as_deref(), Some("오다 에이이치로"), "역할어(만화)는 떼고 한글 저자로 쓴다");
    assert_eq!(fake.calls.borrow().len(), 1);
    assert_eq!(got.store_status, "-..");
}

#[test]
fn canonical_title_without_a_korean_author_keeps_asking_the_next_stores() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cache = cache_in(tmp.path());
    let fake = Fake::new(vec![("yes24", Ok(vec![row("원피스 1권", "Oda")])), ("aladin", Ok(vec![row("원피스 (1-3권)", "오다 에이이치로")]))]);
    let got = canonical_book_title("원피스", Some("오다"), &STORES, 0.0, "q", &mut cache, &fake);
    assert_eq!((got.title.as_deref(), got.author.as_deref()), (Some("원피스"), Some("오다 에이이치로")));
    assert!(got.evidence.contains("yes24: 원피스") && got.evidence.contains("aladin: 원피스"), "{}", got.evidence);
    assert_eq!(got.store_status, "--.");
    assert_eq!(fake.calls.borrow()[0], "yes24:원피스:오다");
}

#[test]
fn rejected_candidates_become_the_evidence_when_nothing_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cache = cache_in(tmp.path());
    let fake = Fake::new(vec![("yes24", Ok(vec![row("전혀 다른 책 1", "a"), row("또 다른 책", "b"), row("셋째", "c"), row("넷째", "d")]))]);
    let got = canonical_book_title("원피스", None, &STORES, 0.0, "q", &mut cache, &fake);
    assert_eq!(got.title, None);
    assert_eq!(got.evidence, "yes24 candidates: 전혀 다른 책 1, 또 다른 책, 셋째");
    let empty = Fake::new(vec![]);
    let mut cache2 = cache_in(&tmp.path().join("other"));
    let got = canonical_book_title("원피스", None, &STORES, 0.0, "q2", &mut cache2, &empty);
    assert_eq!((got.title, got.evidence.as_str()), (None, "no bookstore result"));
}

#[test]
fn results_are_cached_and_a_failed_search_caches_an_empty_result() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("cache.jsonl");
    {
        let mut cache = BookstoreSearchCache::new(&path, false, 7 * 24 * 3600);
        let fake = Fake::new(vec![("yes24", Err("boom".into())), ("aladin", Ok(vec![row("원피스", "오다")])), ("kyobo", Ok(vec![]))]);
        let got = canonical_book_title("원피스", None, &STORES, 0.0, "q", &mut cache, &fake);
        assert_eq!(got.title.as_deref(), Some("원피스"));
        assert_eq!(cache.searches, 2);
    }
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2);
    let mut reloaded = BookstoreSearchCache::new(&path, false, 7 * 24 * 3600);
    let fake = Fake::new(vec![]);
    let got = canonical_book_title("원피스", None, &STORES, 0.0, "q", &mut reloaded, &fake);
    assert_eq!(got.title.as_deref(), Some("원피스"));
    assert!(fake.calls.borrow().is_empty(), "캐시에서 읽는다");
    assert_eq!(reloaded.hits, 2);
    assert_eq!(got.store_status, "++.");
}

#[test]
fn a_cache_miss_without_live_search_is_reported_as_unavailable_and_never_cached() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("cache.jsonl");
    let mut cache = BookstoreSearchCache::new(&path, false, 100);
    let mut fake = Fake::new(vec![]);
    fake.live = false;
    let got = canonical_book_title("원피스", None, &STORES, 0.0, "q", &mut cache, &fake);
    assert!(got.unavailable && got.title.is_none());
    assert_eq!(got.store_status, "!..");
    assert!(!path.exists(), "실패를 음성 캐시로 남기면 안 된다");
    assert_eq!(cache.searches, 0);
}

#[test]
fn cache_loading_skips_bad_lines_and_expires_only_empty_results() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("cache.jsonl");
    let old = 1.0;
    let lines = [
        "not json".to_owned(),
        r#"{"query": "", "store": "yes24", "results": []}"#.to_owned(),
        r#"{"query": "bad-row", "store": "yes24", "results": [{"title": "x"}]}"#.to_owned(),
        format!(r#"{{"query": "empty-old", "store": "yes24", "cached_at": {old}, "results": []}}"#),
        format!(r#"{{"query": "full-old", "store": "yes24", "cached_at": {old}, "results": [{{"title": "t", "author": "a", "category": "c", "url": "u", "isbn": "i"}}]}}"#),
        r#"{"query": "iso", "store": "aladin", "cached_at": "2020-01-01T00:00:00+00:00", "results": []}"#.to_owned(),
        r#"{"query": "nots", "store": "aladin", "cached_at": "garbage", "results": []}"#.to_owned(),
    ];
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    let mut cache = BookstoreSearchCache::new(&path, false, 3600);
    assert!(cache.get("bad-row", "yes24").is_none());
    assert!(cache.contains("empty-old", "yes24"));
    assert!(cache.get("empty-old", "yes24").is_none(), "오래된 빈 결과는 만료된다");
    assert!(!cache.contains("empty-old", "yes24"));
    assert_eq!(cache.get("full-old", "yes24").unwrap()[0].title, "t");
    assert!(cache.get("iso", "aladin").is_none() && cache.get("nots", "aladin").is_none());
    assert_eq!(cache.hits, 1);

    let mut forever = BookstoreSearchCache::new(&path, false, 0);
    assert!(forever.get("empty-old", "yes24").is_none(), "TTL 0이면 빈 결과는 바로 만료된다");
}

#[test]
fn cache_loading_rejects_invalid_utf8_like_python() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("cache.jsonl");
    fs::write(&path, [b'{', b'\n', 0xff, b'\n']).unwrap();
    let error = match BookstoreSearchCache::try_new(&path, false, 3600) {
        Ok(_) => panic!("invalid UTF-8 must fail cache loading"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn refresh_truncates_the_cache_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sub/cache.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "junk\n").unwrap();
    let cache = BookstoreSearchCache::new(&path, true, 100);
    assert_eq!(fs::read_to_string(&path).unwrap(), "");
    assert!(!cache.contains("a", "b"));
}

fn comic_root() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("만화");
    for (dir, files) in [
        ("3_미정리/원피스 모음", vec!["원피스 1권.pdf", "원피스 2권.pdf", "원피스 3권.pdf"]),
        ("3_미정리/[홍길동] 나루토", vec!["나루토 1.pdf", "나루토 2.pdf"]),
        ("4_대기/한 권", vec!["a.pdf"]),
        ("4_대기/이름없음", vec!["!!!.pdf", "???.pdf"]),
        ("9_temp/블리치 1-3권", vec!["블리치 1권.pdf", "블리치 2권.pdf", "블리치 3권.pdf"]),
        ("9_temp/제목", vec!["제목.pdf", "제목.epub"]),
    ] {
        fs::create_dir_all(root.join(dir)).unwrap();
        for f in files {
            fs::write(root.join(dir).join(f), b"x").unwrap();
        }
    }
    (tmp, root)
}

#[test]
fn build_proposals_assigns_statuses() {
    let (tmp, root) = comic_root();
    // "원피스 모음"은 파일 제목과 달라 건너뛴다. 나머지는 서점 답에 따라 갈린다.
    let fake = Fake::new(vec![("yes24", Ok(vec![row("나루토 1", "기시모토 마사시")]))]);
    let mut cache = cache_in(tmp.path());
    let proposals = build_proposals(&root, 0.0, 0, &mut cache, &fake);
    let by_source: HashMap<String, &Proposal> = proposals.iter().map(|p| (p.source.file_name().unwrap().to_string_lossy().into_owned(), p)).collect();

    assert_eq!(by_source["원피스 모음"].status, "SKIP_TITLE_MISMATCH");
    assert_eq!(by_source["한 권"].status, "SKIP_INSUFFICIENT_FILES");
    assert_eq!(by_source["이름없음"].status, "SKIP_UNPARSEABLE_FILENAMES");
    assert_eq!(by_source["제목"].status, "SKIP_NO_RANGE");
    let naruto = by_source["[홍길동] 나루토"];
    assert_eq!(naruto.status, "RENAME");
    assert_eq!(naruto.target.as_deref(), Some(root.join("3_미정리/[홍길동] 나루토 (1-2권)").as_path()));
    assert_eq!(by_source["블리치 1-3권"].status, "SKIP_NO_BOOKSTORE_MATCH");
    assert_eq!(proposals.len(), 6);
}

#[test]
fn build_proposals_marks_cache_misses_without_live_search() {
    let (tmp, root) = comic_root();
    let mut fake = Fake::new(vec![]);
    fake.live = false;
    let mut cache = cache_in(tmp.path());
    let proposals = build_proposals(&root, 0.0, 0, &mut cache, &fake);
    let bleach = proposals.iter().find(|p| p.source.ends_with("블리치 1-3권")).unwrap();
    assert_eq!(bleach.status, "SKIP_NOT_CACHED");
}

#[test]
fn max_searches_limits_new_queries_but_not_cached_ones() {
    let (tmp, root) = comic_root();
    let fake = Fake::new(vec![("yes24", Ok(vec![row("나루토", "기시모토 마사시")]))]);
    let mut cache = cache_in(tmp.path());
    let proposals = build_proposals(&root, 0.0, 1, &mut cache, &fake);
    let statuses: Vec<_> = proposals.iter().filter(|p| p.status.starts_with("SKIP_SEARCH") || p.status == "RENAME").map(|p| p.status.as_str()).collect();
    assert_eq!(statuses.iter().filter(|s| **s == "SKIP_SEARCH_LIMIT").count(), 1, "{statuses:?}");
}

#[test]
fn conflicts_are_detected_for_occupied_and_duplicate_targets() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("만화");
    for (dir, name) in [("3_미정리/a", "나루토"), ("3_미정리/b", "나루토")] {
        fs::create_dir_all(root.join(dir)).unwrap();
        for i in 1..=2 {
            fs::write(root.join(dir).join(format!("{name} {i}권.pdf")), b"x").unwrap();
        }
    }
    // 두 디렉토리 모두 파일 제목은 '나루토'인데 디렉토리 제목이 달라 건너뛰므로 이름을 맞춘다.
    fs::rename(root.join("3_미정리/a"), root.join("3_미정리/나루토")).unwrap();
    fs::rename(root.join("3_미정리/b"), root.join("3_미정리/나루토 (완결)")).unwrap();
    let fake = Fake::new(vec![("yes24", Ok(vec![row("나루토", "기시모토 마사시")]))]);
    let mut cache = cache_in(tmp.path());
    let proposals = build_proposals(&root, 0.0, 0, &mut cache, &fake);
    assert!(proposals.iter().all(|p| p.status == "SKIP_CONFLICT"), "{:?}", proposals.iter().map(|p| &p.status).collect::<Vec<_>>());

    // 대상이 이미 있는 경우
    let (tmp2, root2) = comic_root();
    fs::create_dir_all(root2.join("3_미정리/[기시모토 마사시] 나루토 (1-2권)")).unwrap();
    let mut cache2 = cache_in(tmp2.path());
    let fake2 = Fake::new(vec![("yes24", Ok(vec![row("나루토", "기시모토 마사시")]))]);
    let proposals = build_proposals(&root2, 0.0, 0, &mut cache2, &fake2);
    let naruto = proposals.iter().find(|p| p.source.ends_with("[홍길동] 나루토")).unwrap();
    assert_eq!(naruto.status, "RENAME", "원래 저자 태그를 쓰므로 대상이 다르다");
}

#[test]
fn report_is_a_bom_prefixed_crlf_tsv_with_minimal_quoting() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("out/report.tsv");
    let proposals = vec![
        Proposal { source: "/a/b".into(), target: Some("/a/c".into()), confidence: 0.6666666, sample_count: 3, status: "RENAME".into(), evidence: "plain".into() },
        Proposal { source: "/a/d".into(), target: None, confidence: 1.0, sample_count: 2, status: "SKIP_X".into(), evidence: "has\ttab and \"quote\"".into() },
    ];
    write_report(&proposals, &path).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..3], b"\xef\xbb\xbf");
    let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
    assert_eq!(text, "status\tconfidence\tbook_files\tsource\ttarget\tevidence\r\nRENAME\t0.667\t3\t/a/b\t/a/c\tplain\r\nSKIP_X\t1.000\t2\t/a/d\t\t\"has\ttab and \"\"quote\"\"\"\r\n");
}

#[test]
fn apply_renames_moves_directories_and_rolls_back_on_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for n in ["a", "b", "c"] {
        fs::create_dir(root.join(n)).unwrap();
        fs::write(root.join(n).join("f"), n).unwrap();
    }
    let rename = |s: &str, t: &str| Proposal { source: root.join(s), target: Some(root.join(t)), confidence: 1.0, sample_count: 2, status: "RENAME".into(), evidence: String::new() };
    let skip = Proposal { source: root.join("c"), target: None, confidence: 1.0, sample_count: 2, status: "SKIP_NO_RANGE".into(), evidence: String::new() };

    // 서로 이름을 맞바꾸는 순환도 처리한다.
    let swap = vec![rename("a", "b"), rename("b", "a"), skip];
    assert_eq!(apply_renames(&swap).unwrap(), 2);
    assert_eq!(fs::read_to_string(root.join("a/f")).unwrap(), "b");
    assert_eq!(fs::read_to_string(root.join("b/f")).unwrap(), "a");
    assert!(fs::read_dir(root).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".normalize-")));

    // 점검 뒤에 대상이 생기면 아무것도 옮기지 않는다.
    let blocked = vec![rename("a", "c")];
    let err = apply_renames(&blocked).unwrap_err();
    assert!(err.contains("destination appeared"), "{err}");
    assert_eq!(fs::read_to_string(root.join("a/f")).unwrap(), "b");
}

#[test]
fn apply_renames_restores_sources_when_a_later_step_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("a")).unwrap();
    fs::create_dir(root.join("b")).unwrap();
    let proposals = vec![
        Proposal { source: root.join("a"), target: Some(root.join("x")), confidence: 1.0, sample_count: 2, status: "RENAME".into(), evidence: String::new() },
        // 부모가 없는 대상이라 두 번째 이름 바꾸기가 실패한다.
        Proposal { source: root.join("b"), target: Some(root.join("missing/parent/y")), confidence: 1.0, sample_count: 2, status: "RENAME".into(), evidence: String::new() },
    ];
    assert!(apply_renames(&proposals).is_err());
    assert!(root.join("a").is_dir() && root.join("b").is_dir(), "실패하면 원래 이름으로 되돌린다");
    assert!(!root.join("x").exists());
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_options_reads_flags_and_rejects_bad_input() {
    let o = parse_options(&args(&["--root", "/r", "--report=/t.tsv", "--delay", "0.5", "--max-searches", "7", "--cache", "/c.jsonl", "--negative-cache-seconds", "9", "--apply"])).unwrap();
    assert_eq!(
        (o.root.as_path(), o.report.as_path(), o.delay, o.max_searches, o.cache.as_path(), o.negative_cache_seconds, o.apply),
        (Path::new("/r"), Path::new("/t.tsv"), 0.5, 7, Path::new("/c.jsonl"), 9, true)
    );
    let d = parse_options(&[]).unwrap();
    assert_eq!(d.root, Path::new("/mnt/data/만화"));
    assert_eq!((d.max_searches, d.negative_cache_seconds, d.apply), (0, 7 * 24 * 3600, false));
    let f = parse_options(&args(&["--offline", "--refresh-cache"])).unwrap();
    assert!(f.offline && f.refresh_cache);
    assert!(!d.offline && !d.refresh_cache);
    assert!(parse_options(&args(&["--delay", "x"])).unwrap_err().contains("--delay"));
    assert_eq!(parse_options(&args(&["--max-searches", "-1"])).unwrap().max_searches, 0, "음수는 0으로 맞춘다");
    assert!(parse_options(&args(&["--max-searches", "abc"])).unwrap_err().contains("--max-searches"));
    assert!(parse_options(&args(&["--root"])).unwrap_err().contains("expected one argument"));
    assert!(parse_options(&args(&["--bogus"])).unwrap_err().contains("unrecognized"));
}

fn run_args(root: &Path, extra: &[&str]) -> (i32, String, tempfile::TempDir) {
    let work = tempfile::tempdir().unwrap();
    let mut a = args(&["run", "--root", root.to_str().unwrap(), "--report", work.path().join("r.tsv").to_str().unwrap(), "--cache", work.path().join("c.jsonl").to_str().unwrap()]);
    a.extend(extra.iter().map(|s| s.to_string()));
    let mut out = Vec::new();
    let code = run("ncd", &a, &mut out);
    (code, String::from_utf8(out).unwrap(), work)
}

#[test]
fn no_arguments_runs_with_the_provided_default_options() {
    let (_tmp, root) = comic_root();
    let work = tempfile::tempdir().unwrap();
    let options = Options { root: root.clone(), report: work.path().join("default.tsv"), cache: work.path().join("cache.jsonl"), offline: true, ..Options::default() };
    let mut out = Vec::new();
    assert_eq!(run_with_searcher_and_defaults("ncd", &[], &mut out, &CacheOnly, options), 0);
    assert!(String::from_utf8(out).unwrap().contains("Directories scanned: 6"));
    assert!(work.path().join("default.tsv").exists());
    assert!(root.join("3_미정리/[홍길동] 나루토").exists());
}

#[test]
fn help_prints_usage_and_bad_commands_exit_two() {
    let mut out = Vec::new();
    assert_eq!(run("ncd", &args(&["--help"]), &mut out), 0);
    assert!(String::from_utf8(out).unwrap().contains("사용법"));
    assert_eq!(run("ncd", &args(&["-h"]), &mut Vec::new()), 0);
    assert_eq!(run("ncd", &args(&["bogus"]), &mut Vec::new()), 2);
    assert_eq!(run("ncd", &args(&["run", "--bogus"]), &mut Vec::new()), 2);
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(run("ncd", &args(&["run", "--root", tmp.path().join("none").to_str().unwrap()]), &mut Vec::new()), 2);
}

#[test]
fn run_scans_with_the_cache_only_and_writes_the_report_without_renaming() {
    let (_tmp, root) = comic_root();
    let (code, text, work) = run_args(&root, &["--offline"]);
    assert_eq!(code, 0);
    assert!(text.contains("Directories scanned: 6"), "{text}");
    assert!(text.contains("Dry run only. Pass --apply to rename directories."));
    assert!(text.contains("Rename proposals: 0; unchanged: 0; skipped: 6"), "{text}");
    assert!(work.path().join("r.tsv").exists());
    assert!(root.join("3_미정리/[홍길동] 나루토").exists());
    let _ = Options::default;
}

#[test]
fn run_with_apply_renames_the_planned_directories() {
    let (_tmp, root) = comic_root();
    // 캐시를 미리 채워 두면 서점 없이도 이름이 바뀐다.
    let work = tempfile::tempdir().unwrap();
    let cache = work.path().join("c.jsonl");
    let line = serde_json::json!({"query": "나루토|홍길동", "store": "yes24", "cached_at": 4102444800.0, "results": [{"title": "나루토 1", "author": "기시모토 마사시", "category": "만화", "url": "u", "isbn": ""}]});
    fs::write(&cache, format!("{line}\n")).unwrap();
    let a = args(&["run", "--root", root.to_str().unwrap(), "--report", work.path().join("r.tsv").to_str().unwrap(), "--cache", cache.to_str().unwrap(), "--offline", "--apply"]);
    let mut out = Vec::new();
    assert_eq!(run("ncd", &a, &mut out), 0);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Rename proposals: 1;") && text.contains("Renamed: 1"), "{text}");
    assert!(root.join("3_미정리/[홍길동] 나루토 (1-2권)").is_dir());
}

#[test]
fn cache_writes_that_fail_do_not_enter_the_in_memory_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join("file");
    fs::write(&blocker, "x").unwrap();
    // 부모가 파일이라 디렉토리를 만들 수 없다.
    let mut cache = BookstoreSearchCache::new(&blocker.join("sub/cache.jsonl"), false, 100);
    cache.put("q", "yes24", vec![row("t", "a")]);
    assert!(!cache.contains("q", "yes24"));
    let refreshed = BookstoreSearchCache::new(&blocker.join("sub/cache.jsonl"), true, 100);
    assert!(!refreshed.contains("q", "yes24"));
}

#[test]
fn refresh_cache_failure_stops_the_run_before_writing_a_report() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("comics");
    fs::create_dir(&root).unwrap();
    let blocker = work.path().join("not-a-directory");
    fs::write(&blocker, "x").unwrap();
    let report = work.path().join("report.tsv");
    let options = Options { root, report: report.clone(), cache: blocker.join("cache.jsonl"), refresh_cache: true, offline: true, ..Options::default() };
    let mut out = Vec::new();

    assert_eq!(run_with_searcher_and_defaults("ncd", &[], &mut out, &CacheOnly, options), 1);
    assert!(!report.exists());
}

#[test]
fn cache_accepts_naive_iso_timestamps_and_non_string_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("cache.jsonl");
    let lines = [
        r#"{"query": "naive", "store": "yes24", "cached_at": "2099-01-01T00:00:00", "results": []}"#,
        r#"{"query": "naive-space", "store": "yes24", "cached_at": "2099-01-01 00:00:00.5", "results": []}"#,
        r#"{"query": 123, "store": "yes24", "cached_at": 4102444800, "results": []}"#,
        r#"{"query": "no-time", "store": "yes24", "results": []}"#,
        r#"{"query": "no-results", "store": "yes24"}"#,
    ];
    fs::write(&path, lines.join("\n")).unwrap();
    let mut cache = BookstoreSearchCache::new(&path, false, 3600);
    assert!(cache.get("naive", "yes24").is_some(), "미래 시각은 만료되지 않는다");
    assert!(cache.get("naive-space", "yes24").is_some());
    assert!(cache.get("123", "yes24").is_some());
    assert!(cache.get("no-time", "yes24").is_none(), "시각이 없으면 0이라 만료된다");
    assert!(!cache.contains("no-results", "yes24"));
}

#[test]
fn wait_for_request_slot_sleeps_only_for_the_remaining_delay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cache = cache_in(tmp.path());
    let start = std::time::Instant::now();
    cache.wait_for_request_slot(5.0);
    assert!(start.elapsed().as_secs_f64() < 1.0, "이전 요청이 없으면 기다리지 않는다");
    cache.last_request_completed_at = Some(std::time::Instant::now());
    cache.wait_for_request_slot(0.0);
    assert!(start.elapsed().as_secs_f64() < 1.0, "지연이 0이면 기다리지 않는다");
    cache.wait_for_request_slot(0.05);
    assert!(start.elapsed().as_secs_f64() >= 0.04);
}

#[test]
fn default_paths_are_per_user_not_shared_tmp() {
    let o = Options::default();
    assert!(!o.cache.starts_with("/tmp") && !o.report.starts_with("/tmp"), "{:?} {:?}", o.cache, o.report);
    assert!(o.cache.ends_with("tm_dev/comic-bookstore-title-cache.jsonl"));
}

#[test]
fn a_group_or_world_writable_cache_is_refused_before_anything_runs() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, root) = comic_root();
    let work = tempfile::tempdir().unwrap();
    let cache = work.path().join("c.jsonl");
    fs::write(&cache, "").unwrap();
    assert!(normalize_comic_directories::cli::check_cache_trust(&work.path().join("missing")).is_ok());
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(normalize_comic_directories::cli::check_cache_trust(&cache).is_ok());
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(normalize_comic_directories::cli::check_cache_trust(&cache).unwrap_err().contains("chmod go-w"));
    let a = args(&["run", "--root", root.to_str().unwrap(), "--report", work.path().join("r.tsv").to_str().unwrap(), "--cache", cache.to_str().unwrap()]);
    assert_eq!(run("ncd", &a, &mut Vec::new()), 1);
    assert!(!work.path().join("r.tsv").exists(), "거부하면 리포트도 쓰지 않는다");
}

#[test]
fn live_search_maps_bookstore_rows_into_cache_rows() {
    struct Pages;
    impl bookstore::Fetcher for Pages {
        fn get(&self, _store: bookstore::Store, url: &str) -> Result<String, String> {
            if url.contains("Product/Search") {
                Ok(r#"<ul id="yesSchList"><li><div class="itemUnit"><div class="item_info"><div class="info_row info_name"><a class="gd_name" href="/product/goods/1">x</a></div></div></div></li></ul>"#.into())
            } else {
                Ok(r#"<h2 class="gd_name">나루토 1</h2><span class="gd_auth"><a>기시모토 마사시</a></span> ISBN13 9788900000001"#.into())
            }
        }
    }
    let live = LiveSearch { fetcher: Pages };
    let rows = live.search("yes24", "나루토", "").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].title.as_str(), rows[0].author.as_str(), rows[0].isbn.as_str(), rows[0].url.as_str()),
        ("나루토 1", "기시모토 마사시", "9788900000001", "https://www.yes24.com/product/goods/1")
    );
    assert!(live.is_live());
    assert!(live.search("nope", "x", "").unwrap_err().contains("알 수 없는 서점"));
}

#[test]
fn run_goes_live_by_default_fills_the_cache_and_offline_ignores_the_searcher() {
    let (_tmp, root) = comic_root();
    let work = tempfile::tempdir().unwrap();
    let cache = work.path().join("c.jsonl");
    let fake = Fake::new(vec![("yes24", Ok(vec![row("나루토 1", "기시모토 마사시")]))]);
    let base = |extra: &[&str]| {
        let mut a = args(&["run", "--root", root.to_str().unwrap(), "--report", work.path().join("r.tsv").to_str().unwrap(), "--cache", cache.to_str().unwrap(), "--delay", "0"]);
        a.extend(extra.iter().map(|s| s.to_string()));
        a
    };
    let mut out = Vec::new();
    assert_eq!(run_with_searcher("ncd", &base(&[]), &mut out, &fake), 0);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Rename proposals: 1;"), "{text}");
    assert!(!fake.calls.borrow().is_empty());
    assert!(fs::read_to_string(&cache).unwrap().lines().count() >= 2, "조회 결과를 캐시에 남긴다");

    // 이번에는 캐시만으로 같은 결과가 나오고 서점은 부르지 않는다.
    let again = Fake::new(vec![]);
    let mut out = Vec::new();
    assert_eq!(run_with_searcher("ncd", &base(&["--offline"]), &mut out, &again), 0);
    assert!(String::from_utf8(out).unwrap().contains("Rename proposals: 1;"));
    assert!(again.calls.borrow().is_empty());

    // --refresh-cache는 캐시를 비우고 다시 조회한다.
    let third = Fake::new(vec![("yes24", Ok(vec![row("나루토 1", "기시모토 마사시")]))]);
    assert_eq!(run_with_searcher("ncd", &base(&["--refresh-cache"]), &mut Vec::new(), &third), 0);
    assert!(!third.calls.borrow().is_empty());
}
