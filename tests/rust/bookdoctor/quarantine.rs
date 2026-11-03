//! `quarantine`·`unquarantine`: 인코딩이 손상된 문서를 격리하고 되돌린다(옛 quarantine_broken 테스트 이식).
//! 이동은 항상 되돌릴 수 있어야 한다. 판정에 오탐이 섞일 수 있기 때문이다.

mod common;

use std::cell::Cell;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bookdoctor::cli::{execute, parse, Options, BROKEN_DIR_NAME, TRASH_DIR_NAME};
use bookdoctor::quarantine::*;
use serde_json::{json, Value};

fn quiet(_: &str) {}

struct Lib {
    _dir: tempfile::TempDir,
    root: PathBuf,
    tmp: PathBuf,
}

fn library() -> Lib {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path().to_path_buf();
    let root = tmp.join("text");
    for (cat, names) in [("3_판타지", vec!["a.txt", "b.txt"]), ("9_격언명언", vec!["c.txt"])] {
        fs::create_dir_all(root.join(cat)).unwrap();
        for n in names {
            fs::write(root.join(cat).join(n), format!("{cat}/{n} 내용")).unwrap();
        }
    }
    Lib { _dir: dir, root, tmp }
}

fn records_for(lib: &Lib, rels: &[&str]) -> Vec<Rec> {
    rels.iter().map(|r| json!({"path": lib.root.join(r).to_string_lossy(), "verdict": "corrupted", "likeness": 0.004, "size": 100}).as_object().unwrap().clone()).collect()
}

fn s(rec: &Rec, key: &str) -> String {
    rec[key].as_str().unwrap().to_owned()
}

fn plan(lib: &Lib, rels: &[&str]) -> Vec<Rec> {
    plan_moves(&records_for(lib, rels), &lib.root, &quiet)
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

// ---- 계획 ----

#[test]
fn plan_keeps_original_category_as_subdirectory() {
    let lib = library();
    let moves = plan(&lib, &["3_판타지/a.txt"]);
    assert_eq!(s(&moves[0], "dst"), lib.root.join(BROKEN_DIR_NAME).join("3_판타지").join("a.txt").to_string_lossy());
}

#[test]
fn plan_skips_quarantined_trashed_and_outside_files() {
    let lib = library();
    for dir in [BROKEN_DIR_NAME, TRASH_DIR_NAME] {
        let q = lib.root.join(dir).join("3_판타지");
        fs::create_dir_all(&q).unwrap();
        fs::write(q.join("z.txt"), "x").unwrap();
        assert!(plan(&lib, &[&format!("{dir}/3_판타지/z.txt")]).is_empty(), "{dir}");
    }
    let outside = lib.tmp.join("바깥.txt");
    fs::write(&outside, "x").unwrap();
    let rec = json!({"path": outside.to_string_lossy(), "verdict": "corrupted"}).as_object().unwrap().clone();
    let warned = Cell::new(false);
    let log = |m: &str| warned.set(warned.get() || m.contains("라이브러리 밖"));
    assert!(plan_moves(&[rec], &lib.root, &log).is_empty());
    assert!(warned.get());
}

#[test]
fn plan_avoids_overwriting_on_name_collision() {
    let lib = library();
    let dst = lib.root.join(BROKEN_DIR_NAME).join("3_판타지");
    fs::create_dir_all(&dst).unwrap();
    fs::write(dst.join("a.txt"), "먼저 있던 것").unwrap();
    assert!(s(&plan(&lib, &["3_판타지/a.txt"])[0], "dst").ends_with("a__1.txt"));
}

#[test]
fn plan_gives_distinct_destinations_within_one_run() {
    let lib = library();
    let moves = plan(&lib, &["3_판타지/a.txt", "3_판타지/a.txt"]);
    assert_ne!(s(&moves[0], "dst"), s(&moves[1], "dst"));
    assert!(s(&moves[1], "dst").ends_with("a__1.txt"));
}

#[test]
fn plan_stats_the_source_when_the_record_has_no_size() {
    let lib = library();
    let rec = json!({"path": lib.root.join("3_판타지/a.txt").to_string_lossy()}).as_object().unwrap().clone();
    let moves = plan_moves(&[rec], &lib.root, &quiet);
    assert_eq!(moves[0]["size"], json!("3_판타지/a.txt 내용".len()));
    assert_eq!(moves[0]["likeness"], Value::Null);
}

// ---- 이동과 복구 ----

#[test]
fn move_relocates_file_keeps_inode_and_writes_manifest() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    let src = lib.root.join("3_판타지/a.txt");
    let before = fs::metadata(&src).unwrap();
    let moves = plan(&lib, &["3_판타지/a.txt"]);
    let result = apply_moves(&moves, &manifest, &quiet);
    assert!(!src.exists());
    let after = fs::metadata(s(&moves[0], "dst")).unwrap();
    assert_eq!((before.ino(), before.mtime()), (after.ino(), after.mtime()), "book_id가 inode다");
    assert_eq!(result.moved.len(), 1);
    assert!(manifest.exists());
}

#[test]
fn missing_source_is_reported_not_raised() {
    let lib = library();
    let mv = json!({"src": lib.root.join("3_판타지/없음.txt").to_string_lossy(), "dst": lib.root.join(BROKEN_DIR_NAME).join("x.txt").to_string_lossy(), "size": 0}).as_object().unwrap().clone();
    let result = apply_moves(&[mv], &lib.tmp.join("m.json"), &quiet);
    assert!(result.moved.is_empty());
    assert_eq!(result.failed[0]["error"], "원본 없음");
}

#[test]
fn undo_restores_every_moved_file_and_empties_the_manifest() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    apply_moves(&plan(&lib, &["3_판타지/a.txt", "9_격언명언/c.txt"]), &manifest, &quiet);
    let res = undo_moves(&manifest).unwrap();
    assert_eq!(res.restored.len(), 2);
    assert_eq!(fs::read_to_string(lib.root.join("3_판타지/a.txt")).unwrap(), "3_판타지/a.txt 내용");
    assert_eq!(read_json(&manifest)["moved"], json!([]));
    assert!(undo_moves(&manifest).unwrap().restored.is_empty());
}

#[test]
fn undo_does_not_overwrite_a_file_back_at_the_origin() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &manifest, &quiet);
    fs::write(lib.root.join("3_판타지/a.txt"), "나중에 생긴 다른 파일").unwrap();
    let res = undo_moves(&manifest).unwrap();
    assert!(res.restored.is_empty());
    assert_eq!(res.failed[0]["error"], "원위치에 파일이 이미 있다");
}

#[test]
fn undo_reports_a_missing_quarantined_file() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    let moves = plan(&lib, &["3_판타지/a.txt", "9_격언명언/c.txt"]);
    apply_moves(&moves, &manifest, &quiet);
    fs::remove_file(s(&moves[0], "dst")).unwrap();
    let res = undo_moves(&manifest).unwrap();
    assert_eq!(res.restored.len(), 1);
    assert_eq!(res.failed[0]["error"], "격리본 없음");
}

#[test]
fn manifest_accumulates_across_runs_and_survives_a_broken_one() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    fs::write(&manifest, "이건 JSON 이 아니다").unwrap();
    apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &manifest, &quiet);
    assert_eq!(read_json(&manifest)["moved"].as_array().unwrap().len(), 1);
    apply_moves(&plan(&lib, &["9_격언명언/c.txt"]), &manifest, &quiet);
    assert_eq!(read_json(&manifest)["moved"].as_array().unwrap().len(), 2);
    assert_eq!(undo_moves(&manifest).unwrap().restored.len(), 2);
}

#[test]
fn undo_without_manifest_is_not_found() {
    let lib = library();
    assert_eq!(undo_moves(&lib.tmp.join("없음.json")).unwrap_err().kind(), io::ErrorKind::NotFound);
}

#[test]
fn os_errors_are_reported_for_move_and_undo() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    let moves = plan(&lib, &["3_판타지/a.txt"]);
    let boom = |_: &Path, _: &Path| -> io::Result<()> { Err(io::Error::other("디스크 가득 참")) };
    let result = apply_moves_with(&moves, &manifest, &quiet, &boom);
    assert!(result.moved.is_empty() && result.failed[0]["error"].as_str().unwrap().contains("디스크 가득 참"));
    apply_moves(&moves, &manifest, &quiet);
    let boom = |_: &Path, _: &Path| -> io::Result<()> { Err(io::Error::other("권한 없음")) };
    let res = undo_moves_with(&manifest, &boom).unwrap();
    assert!(res.restored.is_empty() && res.failed[0]["error"].as_str().unwrap().contains("권한 없음"));
}

#[test]
fn move_across_filesystems_falls_back_to_copy_and_keeps_mtime() {
    let lib = library();
    let src = lib.root.join("3_판타지/a.txt");
    let dst = lib.tmp.join("elsewhere/a.txt");
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    let mtime = fs::metadata(&src).unwrap().modified().unwrap() - std::time::Duration::from_secs(86_400);
    fs::File::options().write(true).open(&src).unwrap().set_modified(mtime).unwrap();
    copy_then_remove(&src, &dst).unwrap();
    assert!(!src.exists());
    assert_eq!(fs::metadata(&dst).unwrap().modified().unwrap(), mtime);
}

// ---- 요약 ----

fn summary(moves: &[Value]) -> String {
    let moves: Vec<Rec> = moves.iter().map(|v| v.as_object().unwrap().clone()).collect();
    let mut out = Vec::new();
    summarize(&moves, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn summarize_groups_by_original_category_and_survives_null_size() {
    let out = summary(&[
        json!({"dst": format!("/text/{BROKEN_DIR_NAME}/3_판타지/a.txt"), "size": 1024 * 1024}),
        json!({"dst": format!("/text/{BROKEN_DIR_NAME}/3_판타지/b.txt"), "size": null}),
        json!({"dst": format!("/text/{BROKEN_DIR_NAME}/9_격언명언/c.txt"), "size": 0}),
    ]);
    assert!(out.contains("대상 3건, 1.0MB"));
    assert!(out.find("3_판타지").unwrap() < out.find("9_격언명언").unwrap(), "건수가 많은 카테고리가 먼저 나온다");
    assert!(summary(&[json!({"dst": format!("/text/{BROKEN_DIR_NAME}"), "size": 0})]).contains("(최상위)"));
}

// ---- 명령: 경로를 직접 검사해 옮기고 되돌린다 ----

fn run_in(root: &Path, args: &[&str]) -> (i32, String) {
    let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let opts = Options { library_root: root.to_path_buf(), no_pdf_worker: true, ..parse(&argv).unwrap() };
    let code = execute(&opts, &mut out, false, true);
    (code, String::from_utf8_lossy(&out).into_owned())
}

#[test]
fn quarantine_scans_the_path_and_moves_only_mojibake_files() {
    let lib = library();
    let broken = lib.root.join("3_판타지/broken.txt");
    fs::write(&broken, common::corrupted()).unwrap();
    let clean = lib.root.join("3_판타지/clean.txt");
    fs::write(&clean, common::clean_korean()).unwrap();
    let (code, out) = run_in(&lib.root, &["quarantine", lib.root.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("이동 완료 1건"), "{out}");
    let moved = lib.root.join(BROKEN_DIR_NAME).join("3_판타지/broken.txt");
    assert!(moved.exists() && !broken.exists());
    assert!(clean.exists());
    let manifest = lib.root.join(BROKEN_DIR_NAME).join("manifest.json");
    let m = read_json(&manifest);
    assert_eq!(m["moved"][0]["src"], json!(broken.to_string_lossy()));
    assert!(m["moved"][0]["likeness"].is_number(), "{m}");

    // 다시 돌려도 격리 디렉터리 안의 파일은 다시 옮기지 않는다.
    let (code, out) = run_in(&lib.root, &["quarantine", lib.root.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("이동 완료 0건"), "{out}");

    let (code, out) = run_in(&lib.root, &["unquarantine"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("원위치 1건"), "{out}");
    assert!(broken.exists() && !moved.exists());
}

#[test]
fn unquarantine_without_a_manifest_exits_1() {
    let lib = library();
    let (code, _) = run_in(&lib.root, &["unquarantine"]);
    assert_eq!(code, 1);
}
