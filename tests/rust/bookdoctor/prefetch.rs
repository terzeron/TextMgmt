//! 선읽기 테스트: 다음 파일을 미리 읽되 소비자보다 정해진 만큼만 앞서가고, 멈추면 스레드가 끝난다.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bookdoctor::cli::{Options, parse, print_usage, run};
use bookdoctor::prefetch::Prefetcher;

const MIB: usize = 1024 * 1024;

fn files(dir: &Path, sizes: &[usize]) -> Vec<PathBuf> {
    sizes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            // `.bin`은 실행 파일 확장자라 선읽기에서 빠지므로 일반 확장자를 쓴다.
            let p = dir.join(format!("f{i}.dat"));
            fs::write(&p, vec![b'x'; *n]).unwrap();
            p
        })
        .collect()
}

/// `prefetched()`가 `want`가 될 때까지 최대 5초 기다린다. 도달하지 못하면 마지막 값을 돌려준다.
fn wait_for(p: &Prefetcher, want: usize) -> usize {
    let start = Instant::now();
    while p.prefetched() < want && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    p.prefetched()
}

/// 값이 더 늘지 않는지 잠깐 지켜본다.
fn settle(p: &Prefetcher) -> usize {
    std::thread::sleep(Duration::from_millis(150));
    p.prefetched()
}

#[test]
fn prefetcher_reads_ahead_but_not_beyond_the_file_limit() {
    let d = tempfile::tempdir().unwrap();
    let paths = files(d.path(), &[MIB; 10]);
    let p = Prefetcher::start(paths, 3, 1 << 40, 1 << 40);
    p.advance(0);
    assert_eq!(wait_for(&p, 3), 3);
    assert_eq!(settle(&p), 3, "소비자가 움직이지 않으면 3개에서 멈춘다");
    // 소비자가 5개를 처리하면 읽는 위치도 5번으로 건너뛴다: 5·6·7번을 읽어 모두 6개가 된다(지나간 3·4번은 다시 읽지 않는다).
    p.advance(5);
    assert_eq!(wait_for(&p, 6), 6);
    assert_eq!(settle(&p), 6);
}

#[test]
fn prefetcher_respects_the_byte_limit_but_always_makes_progress() {
    let d = tempfile::tempdir().unwrap();
    let paths = files(d.path(), &[MIB; 6]);
    let p = Prefetcher::start(paths, 100, (2 * MIB + 10) as u64, 1 << 40);
    assert_eq!(wait_for(&p, 2), 2);
    assert_eq!(settle(&p), 2, "2MiB+10 한도에서는 1MiB 파일 둘까지다");

    // 파일 하나가 바이트 한도보다 커도 아무것도 안 읽고 있다면 그 파일은 읽는다(진행 보장).
    let d2 = tempfile::tempdir().unwrap();
    let big = files(d2.path(), &[3 * MIB, MIB]);
    let q = Prefetcher::start(big, 100, MIB as u64, 1 << 40);
    assert_eq!(wait_for(&q, 1), 1);
    assert_eq!(settle(&q), 1, "한도를 넘기는 두 번째 파일은 첫 파일이 소비될 때까지 기다린다");
    q.advance(1);
    assert_eq!(wait_for(&q, 2), 2);
}

#[test]
fn prefetcher_skips_files_over_the_size_cap_without_counting_them() {
    let d = tempfile::tempdir().unwrap();
    let paths = files(d.path(), &[MIB, 5 * MIB, MIB]);
    let p = Prefetcher::start(paths, 10, 1 << 40, (2 * MIB) as u64);
    assert_eq!(wait_for(&p, 2), 2);
    assert_eq!(settle(&p), 2, "5MiB 파일은 건너뛰고 앞뒤 파일만 읽는다");
}

#[test]
fn missing_files_and_directories_do_not_stop_the_prefetcher() {
    let d = tempfile::tempdir().unwrap();
    let mut paths = files(d.path(), &[MIB]);
    paths.push(d.path().join("missing.bin"));
    paths.push(d.path().to_path_buf()); // 디렉터리
    paths.extend(files(&d.path().join(".."), &[]));
    let tail = d.path().join("tail.dat");
    fs::write(&tail, vec![b'y'; MIB]).unwrap();
    paths.push(tail);
    let p = Prefetcher::start(paths, 10, 1 << 40, 1 << 40);
    assert_eq!(wait_for(&p, 2), 2, "없는 파일과 디렉터리를 지나 마지막 파일까지 읽는다");
}

#[test]
fn dropping_the_prefetcher_stops_the_thread_promptly() {
    let d = tempfile::tempdir().unwrap();
    let paths = files(d.path(), &[MIB; 20]);
    let p = Prefetcher::start(paths, 2, 1 << 40, 1 << 40);
    wait_for(&p, 2);
    let start = Instant::now();
    drop(p); // 스레드가 소비자를 기다리는 중이어도 깨워서 끝낸다.
    assert!(start.elapsed() < Duration::from_secs(2), "drop이 스레드를 기다리며 멈췄다: {:?}", start.elapsed());
}

#[test]
fn empty_path_list_is_fine() {
    let p = Prefetcher::start(Vec::new(), 4, 1 << 20, 1 << 20);
    p.advance(0);
    assert_eq!(settle(&p), 0);
}

// --- CLI ---

fn args(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

#[test]
fn no_prefetch_option_is_parsed_and_documented() {
    assert!(parse(&args("a")).unwrap().prefetch);
    assert!(Options::default().prefetch);
    assert!(!parse(&args("--no-prefetch a")).unwrap().prefetch);
    let mut out = Vec::new();
    print_usage(&mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("--no-prefetch"));
}

#[test]
fn output_is_the_same_with_and_without_prefetch() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..30 {
        let body = if i % 3 == 0 { String::new() } else { "소림사 장로가 강호에서 내공을 운기조식했다.\n".repeat(10 + i) };
        fs::write(d.path().join(format!("t{i:02}.txt")), body).unwrap();
    }
    let run_with = |extra: &[&str]| {
        let mut argv: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
        argv.extend(["-r".into(), "--json".into(), "--min-severity".into(), "low".into(), d.path().to_string_lossy().into_owned()]);
        let mut out = Vec::new();
        let code = run(&argv, &mut out);
        (code, String::from_utf8(out).unwrap())
    };
    let (c1, o1) = run_with(&[]);
    let (c2, o2) = run_with(&["--no-prefetch"]);
    assert_eq!((c1, &o1), (c2, &o2));
    assert!(o1.contains("EMPTY_FILE"));
}

#[test]
fn prefetcher_jumps_to_the_consumer_instead_of_rereading_files_already_passed() {
    let d = tempfile::tempdir().unwrap();
    let paths = files(d.path(), &[MIB; 10]);
    let p = Prefetcher::start(paths, 3, 1 << 40, 1 << 40);
    assert_eq!(wait_for(&p, 3), 3);
    assert_eq!(settle(&p), 3);
    p.advance(8);
    assert_eq!(wait_for(&p, 5), 5, "8번과 9번만 읽는다");
    assert_eq!(settle(&p), 5, "소비자가 지나간 3~7번은 다시 읽지 않는다");
}

#[test]
fn prefetcher_skips_executable_extensions_because_those_scanners_read_only_the_head() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("big.exe");
    fs::write(&a, vec![b'x'; MIB]).unwrap();
    let b = d.path().join("doc.txt");
    fs::write(&b, vec![b'y'; MIB]).unwrap();
    let c = d.path().join("lib.so");
    fs::write(&c, vec![b'z'; MIB]).unwrap();
    let p = Prefetcher::start(vec![a, b, c], 10, 1 << 40, 1 << 40);
    assert_eq!(wait_for(&p, 1), 1);
    assert_eq!(settle(&p), 1, "txt 하나만 읽는다");
}
