use std::fs;
use std::path::Path;

use move_semi_clusters::{Outcome, move_semi_clusters, run};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"x").unwrap();
}

fn write_report(root: &Path, clusters: serde_json::Value) -> std::path::PathBuf {
    let report = root.join("hybrid_clustering_report.json");
    fs::write(&report, serde_json::json!({ "clusters": clusters }).to_string()).unwrap();
    report
}

fn entry(path: &Path) -> serde_json::Value {
    serde_json::json!({ "file": path.to_string_lossy() })
}

#[test]
fn moves_each_cluster_into_its_default_category() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let a = root.join("0_telegram/a.txt");
    let b = root.join("0_telegram/sub/b.epub");
    touch(&a);
    touch(&b);
    let report = write_report(root, serde_json::json!({ "준확실_3_무협": [entry(&a)], "준확실_9_성인": [entry(&b)] }));

    let outcome = move_semi_clusters(root, &report, false).unwrap();

    assert!(root.join("3_무협/a.txt").exists() && root.join("9_성인/b.epub").exists());
    assert!(!a.exists() && !b.exists());
    assert_eq!(outcome.moved["3_무협"], 1);
    assert_eq!(outcome.moved["9_성인"], 1);
    assert!(!root.join("0_telegram/sub").exists(), "비게 된 하위 디렉토리는 청소한다");
    assert!(root.join("0_telegram").exists(), "소스 카테고리 자체는 남긴다");
}

#[test]
fn removes_the_source_when_the_destination_already_has_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let src = root.join("0_telegram/dup.txt");
    touch(&src);
    touch(&root.join("3_판타지/dup.txt"));
    let report = write_report(root, serde_json::json!({ "준확실_3_판타지": [entry(&src)] }));

    let outcome = move_semi_clusters(root, &report, false).unwrap();

    assert!(!src.exists());
    assert_eq!(outcome.cleaned["3_판타지"], 1);
    assert_eq!(outcome.moved["3_판타지"], 0);
}

#[test]
fn skips_report_entries_whose_file_is_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let report = write_report(root, serde_json::json!({ "준확실_3_여성향": [entry(&root.join("0_telegram/none.txt"))] }));

    let outcome = move_semi_clusters(root, &report, false).unwrap();

    assert_eq!(outcome.total_moved() + outcome.total_cleaned(), 0);
    assert!(!root.join("3_여성향").exists(), "옮길 파일이 없으면 대상 디렉토리도 만들지 않는다");
}

#[test]
fn greek_myth_goes_to_western_classics_even_from_another_cluster_and_sweeps_leftovers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let listed = root.join("0_telegram/이윤기의 그리스 로마 신화 1.epub");
    let leftover = root.join("0_telegram/deep/이윤기의 그리스 로마 신화 2.epub");
    touch(&listed);
    touch(&leftover);
    let report = write_report(root, serde_json::json!({ "준확실_3_무협": [entry(&listed)] }));

    let outcome = move_semi_clusters(root, &report, false).unwrap();

    assert!(root.join("1_서양고전/이윤기의 그리스 로마 신화 1.epub").exists());
    assert!(root.join("1_서양고전/이윤기의 그리스 로마 신화 2.epub").exists());
    assert_eq!(outcome.moved["1_서양고전"], 2);
    assert!(!root.join("3_무협").exists());
    assert!(!root.join("0_telegram/deep").exists());
}

#[test]
fn dry_run_changes_nothing_but_reports_the_same_counts_without_double_counting() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let listed = root.join("0_telegram/이윤기의 그리스 로마 신화 1.epub");
    let plain = root.join("0_telegram/p.txt");
    touch(&listed);
    touch(&plain);
    let report = write_report(root, serde_json::json!({ "준확실_9_BLGL": [entry(&plain), entry(&listed)] }));

    let outcome = move_semi_clusters(root, &report, true).unwrap();

    assert!(listed.exists() && plain.exists());
    assert!(!root.join("9_BLGL").exists());
    assert_eq!(outcome.moved["9_BLGL"], 1);
    assert_eq!(outcome.moved["1_서양고전"], 1);
}

#[test]
fn unknown_clusters_in_the_report_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let f = root.join("0_telegram/z.txt");
    touch(&f);
    let report = write_report(root, serde_json::json!({ "확실_3_무협": [entry(&f)] }));

    let outcome = move_semi_clusters(root, &report, false).unwrap();

    assert!(f.exists());
    assert_eq!(outcome.total_moved(), 0);
}

#[test]
fn bad_reports_are_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    assert!(move_semi_clusters(root, &root.join("missing.json"), false).unwrap_err().contains("missing.json"));
    let bad = root.join("bad.json");
    fs::write(&bad, "not json").unwrap();
    assert!(move_semi_clusters(root, &bad, false).is_err());
    fs::write(&bad, "{}").unwrap();
    assert!(move_semi_clusters(root, &bad, false).unwrap_err().contains("clusters"));
    fs::write(&bad, r#"{"clusters": {"준확실_3_무협": [{"nofile": 1}]}}"#).unwrap();
    assert!(move_semi_clusters(root, &bad, false).unwrap_err().contains("file"));
}

#[test]
fn summary_lists_only_touched_categories_in_sorted_order() {
    let mut outcome = Outcome::default();
    outcome.moved.insert("3_무협".into(), 2);
    outcome.cleaned.insert("3_무협".into(), 1);
    outcome.moved.insert("1_서양고전".into(), 0);
    outcome.moved.insert("9_성인".into(), 0);
    outcome.cleaned.insert("9_성인".into(), 4);
    let text = outcome.summary();
    assert!(text.starts_with("=== 준확실 클러스터 도서 이동 완료 ==="));
    let a = text.find("▶ 3_무협: 신규 이동 2권, 중복 정리 1권 (합계 3권)").unwrap();
    let b = text.find("▶ 9_성인: 신규 이동 0권, 중복 정리 4권 (합계 4권)").unwrap();
    assert!(a < b && !text.contains("1_서양고전"));
    assert!(text.contains("총 처리: 신규 이동 2권, 중복 정리 5권 (합계 7권)"));
}

#[test]
fn run_prints_usage_without_arguments_and_rejects_bad_ones() {
    let mut out = Vec::new();
    assert_eq!(run("msc", &[], &mut out), 0);
    assert!(String::from_utf8(out).unwrap().contains("사용법"));

    let mut out = Vec::new();
    assert_eq!(run("msc", &["-h".into()], &mut out), 0);
    assert_eq!(run("msc", &["bogus".into()], &mut Vec::new()), 2);
    assert_eq!(run("msc", &["run".into(), "--nope".into()], &mut Vec::new()), 2);
    assert_eq!(run("msc", &["run".into(), "--library-root".into()], &mut Vec::new()), 2);
}

#[test]
fn run_executes_and_prints_the_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let f = root.join("0_telegram/q.txt");
    touch(&f);
    write_report(root, serde_json::json!({ "준확실_3_판타지": [entry(&f)] }));

    let args: Vec<String> = ["run", "--library-root", root.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    assert_eq!(run("msc", &args, &mut out), 0);
    assert!(String::from_utf8(out).unwrap().contains("▶ 3_판타지: 신규 이동 1권"));
    assert!(root.join("3_판타지/q.txt").exists());

    let mut args = args;
    args.push("--report".into());
    args.push(root.join("nope.json").to_string_lossy().into_owned());
    assert_eq!(run("msc", &args, &mut Vec::new()), 1);

    let dry: Vec<String> = ["run", "--dry-run", "--library-root", root.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
    assert_eq!(run("msc", &dry, &mut Vec::new()), 0);
}
