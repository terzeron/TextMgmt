//! `--isolate` 테스트: 파일마다 하위 프로세스에서 메모리·시간 상한을 걸고 검사한다.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use bookdoctor::cli::{Options, parse};
use bookdoctor::isolate::scan_isolated;
use bookdoctor::scan::scan_file;

const REAL: &str = env!("CARGO_BIN_EXE_bookdoctor");

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn sample(dir: &Path) -> PathBuf {
    let p = dir.join("a.docx");
    common::docx_with(&p, &[], &[("word/vbaProject.bin", b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 macro")]);
    p
}

fn opts_with(f: impl FnOnce(&mut Options)) -> Options {
    let mut o = Options::default();
    f(&mut o);
    o
}

#[test]
fn isolated_scan_matches_the_in_process_report() {
    let d = tempfile::tempdir().unwrap();
    let docx = sample(d.path());
    let clean = d.path().join("clean.txt");
    fs::write(&clean, "깨끗한 한글 텍스트입니다.\n".repeat(30)).unwrap();
    let opts = Options::default();
    for p in [&docx, &clean] {
        let direct = scan_file(p, &opts);
        let isolated = scan_isolated(Path::new(REAL), p, &opts);
        assert_eq!(direct, isolated, "{}", p.display());
    }
    // 깨끗한 파일도 "깨끗함"으로 돌아온다(출력이 없어서 실패로 오해하지 않는다).
    assert!(!scan_isolated(Path::new(REAL), &clean, &opts).incomplete);
}

#[test]
fn a_child_that_aborts_is_reported_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let exe = script(d.path(), "crash.sh", "kill -ABRT $$");
    let r = scan_isolated(&exe, &sample(d.path()), &Options::default());
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("isolated scan failed") && m.contains("signal 6")), "{:?}", r.reasons);
    assert!(r.findings.is_empty());
}

#[test]
fn a_nonzero_exit_and_garbage_output_are_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let p = sample(d.path());
    for (name, body, needle) in [("fail.sh", "echo oops >&2; exit 3", "exit status 3"), ("garbage.sh", "echo not json", "unreadable"), ("empty.sh", "exit 0", "unreadable"), ("two.sh", "echo '{}'; echo '{}'", "unreadable")] {
        let exe = script(d.path(), name, body);
        let r = scan_isolated(&exe, &p, &Options::default());
        assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("isolated scan failed") && m.contains(needle)), "{name}: {:?}", r.reasons);
    }
    // 사유에 자식의 stderr 끝부분이 들어간다.
    let exe = script(d.path(), "fail2.sh", "echo 'boom reason' >&2; exit 3");
    assert!(scan_isolated(&exe, &p, &Options::default()).reasons.iter().any(|m| m.contains("boom reason")));
}

#[test]
fn a_child_over_the_timeout_is_killed_with_its_whole_group() {
    let d = tempfile::tempdir().unwrap();
    let pidfile = d.path().join("grandchild.pid");
    let exe = script(d.path(), "hang.sh", &format!("sleep 300 &\necho $! > '{}'\nwait", pidfile.display()));
    let opts = opts_with(|o| o.isolate_timeout_s = 1);
    let started = Instant::now();
    let r = scan_isolated(&exe, &sample(d.path()), &opts);
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("isolated scan failed") && m.contains("timeout")), "{:?}", r.reasons);
    // 손자(sleep)까지 죽었다.
    let pid: i32 = fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    let mut alive = true;
    for _ in 0..40 {
        alive = Path::new(&format!("/proc/{pid}")).exists() && fs::read_to_string(format!("/proc/{pid}/stat")).map(|s| !s.contains(") Z ")).unwrap_or(false);
        if !alive {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!alive, "grandchild {pid} survived the timeout");
}

#[test]
fn the_memory_limit_stops_an_allocating_child() {
    let d = tempfile::tempdir().unwrap();
    let exe = script(d.path(), "alloc.sh", "exec python3 -I -c 'x = bytearray(600 * 1024 * 1024)'");
    let p = sample(d.path());
    // 상한 없이(충분히 큰 값)는 할당이 된다. 그래도 JSON이 아니므로 "unreadable"이다: 실패 사유가 메모리가 아님을 확인한다.
    let loose = scan_isolated(&exe, &p, &opts_with(|o| o.isolate_mem_mb = 8192));
    assert!(loose.reasons.iter().any(|m| m.contains("unreadable")), "{:?}", loose.reasons);
    // 64MiB 상한이면 할당이 실패해 python이 비정상 종료한다.
    let tight = scan_isolated(&exe, &p, &opts_with(|o| o.isolate_mem_mb = 64));
    assert!(tight.incomplete && tight.reasons.iter().any(|m| m.contains("isolated scan failed") && m.contains("exit status")), "{:?}", tight.reasons);
}

#[test]
fn options_are_passed_to_the_child_without_isolate() {
    let d = tempfile::tempdir().unwrap();
    let argsfile = d.path().join("args.txt");
    let exe = script(d.path(), "echo.sh", &format!("printf '%s\\n' \"$@\" > '{}'\necho '{{\"path\":\"x\",\"format\":null,\"findings\":[],\"incomplete\":false,\"reasons\":[],\"likeness\":null,\"risk_score\":0}}'", argsfile.display()));
    let p = sample(d.path());
    let opts = opts_with(|o| {
        o.zip_deep = true;
        o.max_total_mb = 7;
        o.recurse = false;
        o.pdf_text = true;
        o.no_poppler = true;
        o.isolate = true;
    });
    let r = scan_isolated(&exe, &p, &opts);
    assert!(!r.incomplete, "{:?}", r.reasons);
    let args: Vec<String> = fs::read_to_string(&argsfile).unwrap().lines().map(str::to_owned).collect();
    for want in ["--child", "--zip-deep", "--no-recurse", "--pdf-text", "--no-poppler"] {
        assert!(args.iter().any(|a| a == want), "{want} missing: {args:?}");
    }
    assert!(args.windows(2).any(|w| w[0] == "--max-total-mb" && w[1] == "7"), "{args:?}");
    assert!(args.windows(2).any(|w| w[0] == "--min-severity" && w[1] == "low"), "the child must not suppress: {args:?}");
    assert!(!args.iter().any(|a| a.starts_with("--isolate")), "{args:?}");
    assert_eq!(args.last().map(String::as_str), Some(p.to_str().unwrap()));
}

#[test]
fn the_isolate_options_parse_and_validate() {
    let argv = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let o = parse(&argv(&["--isolate", "--isolate-mem-mb", "64", "--isolate-timeout-s", "5", "a.zip"])).unwrap();
    assert!(o.isolate && o.isolate_mem_mb == 64 && o.isolate_timeout_s == 5);
    let d = Options::default();
    assert!(!d.isolate && !d.child && d.isolate_mem_mb == 4096 && d.isolate_timeout_s == 120);
    assert!(parse(&argv(&["--isolate-mem-mb", "x", "a"])).is_err());
    assert!(parse(&argv(&["--isolate-mem-mb", "0", "a"])).is_err(), "a zero limit would disable the guard");
    assert!(parse(&argv(&["--child", "--json", "a"])).unwrap().child);
}

#[test]
fn end_to_end_the_parent_continues_after_children_fail() {
    // 메모리 상한이 너무 작아 자식이 모두 못 뜬다. 부모는 두 파일 모두 미완료로 보고하고 끝까지 간다.
    let d = tempfile::tempdir().unwrap();
    let a = sample(d.path());
    let b = d.path().join("b.docx");
    fs::copy(&a, &b).unwrap();
    let out = Command::new(REAL).args(["--isolate", "--isolate-mem-mb", "8", "--isolate-timeout-s", "20", "--json", "--min-severity", "low"]).arg(&a).arg(&b).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stdout.lines().filter(|l| l.contains("isolated scan failed")).count(), 2, "{stdout}\n{stderr}");
    assert!(stderr.contains("검사하지 못함 2건"), "{stderr}");
    // 정상 상한이면 지표가 나온다.
    let ok = Command::new(REAL).args(["--isolate", "--json", "--min-severity", "low"]).arg(&a).arg(&b).output().unwrap();
    let ok_out = String::from_utf8_lossy(&ok.stdout);
    assert_eq!(ok_out.lines().filter(|l| l.contains("OOXML_VBA_PROJECT")).count(), 2, "{ok_out}\n{}", String::from_utf8_lossy(&ok.stderr));
}
