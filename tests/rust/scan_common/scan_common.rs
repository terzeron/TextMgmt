//! scan_common 테스트: 포맷과 무관한 공용 코드를 직접 확인한다.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use scan_common::args::{long_option, number};
use scan_common::files::{collect_files, has_extension};
use scan_common::text::sanitize;
use scan_common::{DEFAULT_WORKERS, Report, Severity, Summary, limited, pool, scan_streaming, workers_within_memory};

// ---- severity ----

#[test]
fn severity_orders_from_low_to_critical_and_has_upper_case_labels() {
    assert!(Severity::Low < Severity::Medium && Severity::Medium < Severity::High && Severity::High < Severity::Critical);
    let labels: Vec<_> = [Severity::Low, Severity::Medium, Severity::High, Severity::Critical].iter().map(|s| s.label()).collect();
    assert_eq!(labels, ["LOW", "MEDIUM", "HIGH", "CRITICAL"]);
}

#[cfg(feature = "serde")]
#[test]
fn severity_serializes_as_its_upper_case_label_with_the_serde_feature() {
    assert_eq!(serde_json::to_string(&Severity::Critical).unwrap(), "\"CRITICAL\"");
    assert_eq!(serde_json::to_string(&Severity::Low).unwrap(), "\"LOW\"");
}

// ---- text ----

#[test]
fn sanitize_replaces_c0_and_c1_controls_and_cuts_at_the_limit() {
    assert_eq!(sanitize("a\u{1b}[2Jb\u{85}c\u{7f}", 100, None), "a.[2Jb.c.");
    assert_eq!(sanitize("가나다라마", 3, None), "가나다");
    assert_eq!(sanitize("가나다라마", 3, Some('…')), "가나다…");
    // 딱 맞으면 자르지 않았으므로 말줄임표도 없다.
    assert_eq!(sanitize("가나다", 3, Some('…')), "가나다");
    assert_eq!(sanitize("", 3, Some('…')), "");
}

// ---- args ----

#[test]
fn number_parses_or_reports_the_argparse_style_error() {
    assert_eq!(number::<usize>("workers", "4"), Ok(4));
    assert_eq!(number::<u64>("limit", "x"), Err("argument --limit: invalid int value: 'x'".to_string()));
    assert!(number::<usize>("limit", "-1").is_err());
}

#[test]
fn long_option_reads_inline_and_separate_values_and_rejects_unknown_names() {
    let allowed = ["workers", "limit"];
    let none: Vec<String> = Vec::new();
    // 옵션이 아니면 위치 인자다. 다음 인자를 먹지 않는다.
    let rest = ["x".to_string()];
    let mut it = rest.iter();
    assert_eq!(long_option("a.pdf", &allowed, &mut it), Ok(None));
    assert_eq!(long_option("-r", &allowed, &mut it), Ok(None));
    assert_eq!(it.next().map(String::as_str), Some("x"));
    // `--이름=값`은 다음 인자를 먹지 않는다. `=` 뒤가 비어도 값이다.
    let rest = ["unused".to_string()];
    let mut it = rest.iter();
    assert_eq!(long_option("--workers=3", &allowed, &mut it), Ok(Some(("workers".into(), "3".into()))));
    assert_eq!(long_option("--limit=", &allowed, &mut it), Ok(Some(("limit".into(), String::new()))));
    assert_eq!(it.next().map(String::as_str), Some("unused"));
    // `--이름 값`은 다음 인자를 쓴다.
    let rest = ["7".to_string()];
    let mut it = rest.iter();
    assert_eq!(long_option("--limit", &allowed, &mut it), Ok(Some(("limit".into(), "7".into()))));
    // 모르는 이름, 값이 없는 경우.
    assert_eq!(long_option("--bogus=1", &allowed, &mut none.iter()), Err("unrecognized arguments: --bogus=1".to_string()));
    assert_eq!(long_option("--workers", &allowed, &mut none.iter()), Err("argument --workers: expected one argument".to_string()));
}

// ---- workers ----

#[test]
fn workers_within_memory_never_goes_below_one_and_never_above_the_request() {
    assert_eq!(workers_within_memory(2, 20, 2048), 2);
    assert_eq!(workers_within_memory(64, 440, 2048), 4);
    assert_eq!(workers_within_memory(64, 1, 2048), 64);
    assert_eq!(workers_within_memory(8, 100_000, 2048), 1);
    assert_eq!(workers_within_memory(0, 20, 2048), 1);
    assert_eq!(workers_within_memory(3, 0, 2048), 3);
}

#[test]
fn limited_keeps_the_head_and_zero_means_all() {
    let files: Vec<PathBuf> = (0..5).map(|i| PathBuf::from(format!("{i}"))).collect();
    assert_eq!(limited(files.clone(), 0).len(), 5);
    assert_eq!(limited(files.clone(), 2), vec![PathBuf::from("0"), PathBuf::from("1")]);
    assert_eq!(limited(files, 99).len(), 5);
}

#[test]
fn pool_has_at_least_one_thread_and_the_default_is_two() {
    assert_eq!(DEFAULT_WORKERS, 2);
    assert_eq!(pool(0).current_num_threads(), 1);
    assert_eq!(pool(3).current_num_threads(), 3);
}

// ---- files ----

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"x").unwrap();
}

fn pdf(p: &Path) -> bool {
    has_extension(p, "pdf")
}

#[test]
fn has_extension_ignores_case_and_requires_a_regular_file() {
    let d = tempfile::tempdir().unwrap();
    touch(&d.path().join("a.PDF"));
    touch(&d.path().join("b.txt"));
    fs::create_dir(d.path().join("dir.pdf")).unwrap();
    assert!(has_extension(&d.path().join("a.PDF"), "pdf"));
    assert!(!has_extension(&d.path().join("b.txt"), "pdf"));
    assert!(!has_extension(&d.path().join("dir.pdf"), "pdf"), "디렉토리는 파일이 아니다");
    assert!(!has_extension(&d.path().join("none.pdf"), "pdf"));
}

#[test]
fn collect_files_sorts_and_only_descends_when_recursive() {
    let d = tempfile::tempdir().unwrap();
    for n in ["b.pdf", "a.pdf", "notes.txt", "sub/c.pdf", "sub/deep/d.pdf"] {
        touch(&d.path().join(n));
    }
    let names = |v: Vec<PathBuf>| v.iter().map(|p| p.strip_prefix(d.path()).unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
    assert_eq!(names(collect_files(&[d.path().to_path_buf()], false, pdf)), ["a.pdf", "b.pdf"]);
    assert_eq!(names(collect_files(&[d.path().to_path_buf()], true, pdf)), ["a.pdf", "b.pdf", "sub/c.pdf", "sub/deep/d.pdf"]);
}

#[test]
fn collect_files_keeps_missing_and_explicit_paths_so_they_can_be_reported_incomplete() {
    let d = tempfile::tempdir().unwrap();
    touch(&d.path().join("ok.pdf"));
    touch(&d.path().join("plain.txt"));
    let missing = d.path().join("missing.pdf");
    // 명시한 파일은 확장자와 무관하게 남고, 없는 경로도 남는다. 중복은 처음 것만 남는다.
    let got = collect_files(&[d.path().join("plain.txt"), missing.clone(), d.path().join("ok.pdf"), d.path().join("ok.pdf"), missing.clone()], false, pdf);
    assert_eq!(got, vec![d.path().join("plain.txt"), missing, d.path().join("ok.pdf")]);
}

#[test]
fn collect_files_does_not_follow_symlinked_directories_inside_the_tree_but_follows_the_top_one() {
    let d = tempfile::tempdir().unwrap();
    touch(&d.path().join("real/a.pdf"));
    std::os::unix::fs::symlink(d.path().join("real"), d.path().join("link")).unwrap();
    let got = collect_files(&[d.path().to_path_buf()], true, pdf);
    assert_eq!(got, vec![d.path().join("real/a.pdf")], "트리 안의 링크 디렉토리는 따라가지 않는다");
    // 맨 위 입력이 링크여도 그 안은 훑는다.
    let top = collect_files(&[d.path().join("link")], false, pdf);
    assert_eq!(top, vec![d.path().join("link/a.pdf")]);
}

#[test]
fn collect_files_keeps_an_unreadable_directory_instead_of_skipping_it() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let locked = d.path().join("locked");
    fs::create_dir(&locked).unwrap();
    touch(&d.path().join("ok.pdf"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let got = collect_files(&[d.path().to_path_buf()], true, pdf);
    let readable_anyway = fs::read_dir(&locked).is_ok();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if readable_anyway {
        return; // root로 실행되면 권한이 막지 못한다.
    }
    assert!(got.contains(&locked), "읽을 수 없는 디렉토리는 남겨서 INCOMPLETE로 보고되어야 한다: {got:?}");
    assert!(got.contains(&d.path().join("ok.pdf")));
}

// ---- stream ----

struct Fake {
    name: String,
    severities: Vec<Severity>,
    incomplete: bool,
}

impl Report for Fake {
    fn severities(&self) -> impl Iterator<Item = Severity> + '_ {
        self.severities.iter().copied()
    }
    fn is_incomplete(&self) -> bool {
        self.incomplete
    }
}

fn paths(n: usize) -> Vec<PathBuf> {
    (0..n).map(|i| PathBuf::from(format!("{i:03}"))).collect()
}

fn fake(path: &Path) -> Fake {
    Fake { name: path.to_string_lossy().into_owned(), severities: Vec::new(), incomplete: false }
}

#[test]
fn scan_streaming_emits_every_report_in_input_order_across_windows() {
    let pool = pool(3);
    let input = paths(23);
    let mut order = Vec::new();
    let summary = scan_streaming(&input, &pool, 5, fake, |r: &Fake| {
        order.push(r.name.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(order, input.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>());
    assert_eq!(summary.scanned, 23);
}

#[test]
fn scan_streaming_holds_at_most_one_window_of_reports() {
    // 한 창이 끝나기 전에는 다음 창을 시작하지 않는다. 보고서가 창 크기보다 많이 살아 있으면 안 된다.
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    struct Counted(Fake);
    impl Counted {
        fn new(p: &Path) -> Self {
            let now = LIVE.fetch_add(1, Ordering::SeqCst) + 1;
            PEAK.fetch_max(now, Ordering::SeqCst);
            Counted(fake(p))
        }
    }
    impl Drop for Counted {
        fn drop(&mut self) {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl Report for Counted {
        fn severities(&self) -> impl Iterator<Item = Severity> + '_ {
            self.0.severities.iter().copied()
        }
        fn is_incomplete(&self) -> bool {
            false
        }
    }
    let pool = pool(2);
    scan_streaming(&paths(40), &pool, 4, Counted::new, |_| Ok(())).unwrap();
    assert_eq!(LIVE.load(Ordering::SeqCst), 0);
    assert!(PEAK.load(Ordering::SeqCst) <= 4, "동시에 살아 있던 보고서가 창(4)보다 많다: {}", PEAK.load(Ordering::SeqCst));
}

#[test]
fn summary_counts_severities_and_decides_the_exit_code() {
    let pool = pool(2);
    let run = |reports: Vec<(Vec<Severity>, bool)>| -> Summary {
        let input = paths(reports.len());
        let by_name: std::collections::HashMap<String, (Vec<Severity>, bool)> = input.iter().map(|p| p.to_string_lossy().into_owned()).zip(reports).collect();
        scan_streaming(
            &input,
            &pool,
            2,
            |p| {
                let (severities, incomplete) = by_name[&p.to_string_lossy().into_owned()].clone();
                Fake { name: String::new(), severities, incomplete }
            },
            |_| Ok(()),
        )
        .unwrap()
    };
    let clean = run(vec![(vec![], false), (vec![], false)]);
    assert_eq!((clean.exit_code(), clean.scanned, clean.by_severity.len()), (0, 2, 0));
    let incomplete = run(vec![(vec![], false), (vec![], true)]);
    assert_eq!(incomplete.exit_code(), 2);
    // 지표가 있으면 뒤창의 INCOMPLETE에 가려지지 않는다.
    let both = run(vec![(vec![Severity::High, Severity::High, Severity::Critical], false), (vec![], true), (vec![Severity::Low], false)]);
    assert_eq!(both.exit_code(), 1);
    assert_eq!(both.by_severity.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(), [("CRITICAL", 1), ("HIGH", 2), ("LOW", 1)]);
}

#[test]
fn scan_streaming_stops_at_the_first_emit_error() {
    let pool = pool(1);
    let calls = AtomicUsize::new(0);
    let err = scan_streaming(
        &paths(10),
        &pool,
        2,
        |p| {
            calls.fetch_add(1, Ordering::SeqCst);
            fake(p)
        },
        |_| Err(io::Error::other("closed")),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), "closed");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "첫 창만 검사하고 멈춘다");
}

#[test]
fn scan_streaming_with_no_paths_returns_an_empty_summary() {
    let s = scan_streaming(&[], &pool(1), 4, fake, |_| Ok(())).unwrap();
    assert_eq!((s.scanned, s.exit_code()), (0, 0));
    // window가 0이어도 멈추지 않는다.
    let s = scan_streaming(&paths(3), &pool(1), 0, fake, |_| Ok(())).unwrap();
    assert_eq!(s.scanned, 3);
}
