//! 손상 문서 격리 도구 테스트 (tests/test_quarantine_broken.py 이식).
//! 이동은 항상 되돌릴 수 있어야 한다. 판정에 오탐이 섞일 수 있기 때문이다.

use std::cell::Cell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use quarantine_broken::*;
use serde_json::{Map, Value, json};

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
fn plan_skips_files_already_quarantined_and_outside_the_library() {
    let lib = library();
    let q = lib.root.join(BROKEN_DIR_NAME).join("3_판타지");
    fs::create_dir_all(&q).unwrap();
    fs::write(q.join("z.txt"), "x").unwrap();
    assert!(plan(&lib, &[&format!("{BROKEN_DIR_NAME}/3_판타지/z.txt")]).is_empty());

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
fn plan_gives_distinct_destinations_for_same_name_in_one_run() {
    let lib = library();
    fs::write(lib.root.join("9_격언명언/a.txt"), "다른 카테고리 동명 파일").unwrap();
    let moves = plan(&lib, &["3_판타지/a.txt", "9_격언명언/a.txt"]);
    let dsts: std::collections::HashSet<String> = moves.iter().map(|m| s(m, "dst")).collect();
    assert_eq!(dsts.len(), 2);
}

#[test]
fn plan_gives_distinct_destinations_for_the_same_source_listed_twice() {
    // taken 집합이 같은 실행 안의 충돌을 막는다. 디스크에는 아직 아무것도 없다.
    let lib = library();
    let moves = plan(&lib, &["3_판타지/a.txt", "3_판타지/a.txt"]);
    assert_ne!(s(&moves[0], "dst"), s(&moves[1], "dst"));
    assert!(s(&moves[1], "dst").ends_with("a__1.txt"));
}

// ---- 이동과 복구 ----

#[test]
fn move_relocates_file_and_writes_manifest() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    let result = apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &manifest, false, &quiet);
    assert!(!lib.root.join("3_판타지/a.txt").exists());
    assert_eq!(fs::read_to_string(lib.root.join(BROKEN_DIR_NAME).join("3_판타지/a.txt")).unwrap(), "3_판타지/a.txt 내용");
    assert_eq!(result.moved.len(), 1);
    assert!(manifest.exists());
}

#[test]
fn move_keeps_inode_and_mtime_on_the_same_filesystem() {
    // book_id가 inode라 격리와 복구가 inode를 바꾸면 안 된다.
    use std::os::unix::fs::MetadataExt;
    let lib = library();
    let src = lib.root.join("3_판타지/a.txt");
    let before = fs::metadata(&src).unwrap();
    let moves = plan(&lib, &["3_판타지/a.txt"]);
    apply_moves(&moves, &lib.tmp.join("m.json"), false, &quiet);
    let after = fs::metadata(s(&moves[0], "dst")).unwrap();
    assert_eq!((before.ino(), before.mtime()), (after.ino(), after.mtime()));
}

#[test]
fn dry_run_moves_nothing_and_writes_no_manifest() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    let result = apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &manifest, true, &quiet);
    assert!(lib.root.join("3_판타지/a.txt").exists());
    assert!(!manifest.exists());
    assert_eq!(result.moved.len(), 1);
}

#[test]
fn missing_source_is_reported_not_raised() {
    let lib = library();
    let mv = json!({"src": lib.root.join("3_판타지/없음.txt").to_string_lossy(), "dst": lib.root.join(BROKEN_DIR_NAME).join("x.txt").to_string_lossy(), "size": 0}).as_object().unwrap().clone();
    let result = apply_moves(&[mv], &lib.tmp.join("m.json"), false, &quiet);
    assert!(result.moved.is_empty());
    assert_eq!(result.failed[0]["error"], "원본 없음");
}

#[test]
fn undo_restores_every_moved_file() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    apply_moves(&plan(&lib, &["3_판타지/a.txt", "9_격언명언/c.txt"]), &manifest, false, &quiet);
    assert!(!lib.root.join("3_판타지/a.txt").exists());
    let res = undo_moves(&manifest, false).unwrap();
    assert_eq!(res.restored.len(), 2);
    assert_eq!(fs::read_to_string(lib.root.join("3_판타지/a.txt")).unwrap(), "3_판타지/a.txt 내용");
    assert!(lib.root.join("9_격언명언/c.txt").exists());
}

#[test]
fn undo_empties_the_manifest_so_it_is_not_repeated() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &manifest, false, &quiet);
    undo_moves(&manifest, false).unwrap();
    assert_eq!(read_json(&manifest)["moved"], json!([]));
    assert!(undo_moves(&manifest, false).unwrap().restored.is_empty());
}

#[test]
fn undo_does_not_overwrite_a_file_back_at_the_origin() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &manifest, false, &quiet);
    fs::write(lib.root.join("3_판타지/a.txt"), "나중에 생긴 다른 파일").unwrap();
    let res = undo_moves(&manifest, false).unwrap();
    assert!(res.restored.is_empty());
    assert_eq!(res.failed[0]["error"], "원위치에 파일이 이미 있다");
    assert_eq!(fs::read_to_string(lib.root.join("3_판타지/a.txt")).unwrap(), "나중에 생긴 다른 파일");
}

#[test]
fn manifest_accumulates_across_separate_runs() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &manifest, false, &quiet);
    apply_moves(&plan(&lib, &["9_격언명언/c.txt"]), &manifest, false, &quiet);
    assert_eq!(read_json(&manifest)["moved"].as_array().unwrap().len(), 2);
    assert_eq!(undo_moves(&manifest, false).unwrap().restored.len(), 2);
}

#[test]
fn undo_without_manifest_is_not_found() {
    let lib = library();
    let err = undo_moves(&lib.tmp.join("없음.json"), false).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[test]
fn move_rewrites_manifest_when_previous_one_is_broken() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    fs::write(&manifest, "이건 JSON 이 아니다").unwrap();
    let result = apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &manifest, false, &quiet);
    assert_eq!(result.moved.len(), 1);
    assert_eq!(read_json(&manifest)["moved"].as_array().unwrap().len(), 1);
}

#[test]
fn move_reports_os_error_and_undo_reports_os_error() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    let moves = plan(&lib, &["3_판타지/a.txt"]);
    let boom = |_: &Path, _: &Path| -> io::Result<()> { Err(io::Error::other("디스크 가득 참")) };
    let result = apply_moves_with(&moves, &manifest, false, &quiet, &boom);
    assert!(result.moved.is_empty());
    assert!(result.failed[0]["error"].as_str().unwrap().contains("디스크 가득 참"));

    apply_moves(&moves, &manifest, false, &quiet);
    let boom = |_: &Path, _: &Path| -> io::Result<()> { Err(io::Error::other("권한 없음")) };
    let res = undo_moves_with(&manifest, false, &boom).unwrap();
    assert!(res.restored.is_empty());
    assert!(res.failed[0]["error"].as_str().unwrap().contains("권한 없음"));
}

#[test]
fn undo_reports_missing_quarantined_file_and_dry_run_moves_nothing() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    let moves = plan(&lib, &["3_판타지/a.txt", "9_격언명언/c.txt"]);
    apply_moves(&moves, &manifest, false, &quiet);

    let dry = undo_moves(&manifest, true).unwrap();
    assert_eq!(dry.restored.len(), 2);
    assert!(Path::new(&s(&moves[0], "dst")).exists());
    assert_eq!(read_json(&manifest)["moved"].as_array().unwrap().len(), 2);

    fs::remove_file(s(&moves[0], "dst")).unwrap();
    let res = undo_moves(&manifest, false).unwrap();
    assert_eq!(res.restored.len(), 1);
    assert_eq!(res.failed[0]["error"], "격리본 없음");
}

#[test]
fn move_across_filesystems_falls_back_to_copy_and_keeps_mtime() {
    // 교차 파일시스템 이동은 rename이 EXDEV로 실패한다. 복사 뒤 원본을 지우고 mtime을 보존한다.
    let lib = library();
    let src = lib.root.join("3_판타지/a.txt");
    let dst = lib.tmp.join("elsewhere/a.txt");
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    let mtime = fs::metadata(&src).unwrap().modified().unwrap() - std::time::Duration::from_secs(86_400);
    fs::File::options().write(true).open(&src).unwrap().set_modified(mtime).unwrap();
    copy_then_remove(&src, &dst).unwrap();
    assert!(!src.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "3_판타지/a.txt 내용");
    assert_eq!(fs::metadata(&dst).unwrap().modified().unwrap(), mtime);
}

// ---- 리포트 읽기 ----

#[test]
fn load_corrupted_reads_both_report_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let scan = dir.path().join("report.json");
    fs::write(&scan, json!({"corrupted": [{"path": "/a.txt", "verdict": "corrupted"}]}).to_string()).unwrap();
    assert_eq!(load_corrupted(&scan).unwrap().len(), 1);

    let recheck = dir.path().join("recheck.json");
    fs::write(&recheck, json!([{"path": "/a.txt", "verdict": "corrupted"}, {"path": "/b.txt", "verdict": "clean"}, {"path": "/c.txt", "verdict": "undetermined"}, {"path": "/d.txt"}]).to_string())
        .unwrap();
    let paths: Vec<String> = load_corrupted(&recheck).unwrap().iter().map(|r| s(r, "path")).collect();
    assert_eq!(paths, ["/a.txt", "/d.txt"]);
}

// ---- 확장자 교정 ----

const OLE_AS_UTF16_CHARS: &str = "\u{cfd0}\u{e011}\u{b1a1}\u{e11a}";
const OLE_MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

/// (원래 바이너리, 망가진 상태) 쌍. 문자열에서 만들어야 UTF-16LE 왕복이 보장된다.
fn binary_pair() -> (Vec<u8>, Vec<u8>) {
    let text = format!("{OLE_AS_UTF16_CHARS}{}", "FileHeader 본문 내용 가나다라마바사".repeat(20));
    (text.encode_utf16().flat_map(u16::to_le_bytes).collect(), text.into_bytes())
}

#[test]
fn unmangle_restores_original_bytes() {
    let (orig, mangled) = binary_pair();
    assert!(orig.starts_with(&OLE_MAGIC));
    assert_eq!(unmangle_utf16(&mangled), Some(orig));
}

#[test]
fn unmangle_none_for_plain_text_tolerates_truncation_and_gives_up() {
    assert_eq!(unmangle_utf16("평범한 한국어 텍스트입니다. ".repeat(20).as_bytes()), None);
    let (_, mangled) = binary_pair();
    assert!(unmangle_utf16(&mangled[..mangled.len() - 1]).is_some());
    assert_eq!(unmangle_utf16(&[0xff, 0xfe, 0xfd, 0xfc, 0xfb]), None);
}

#[test]
fn true_extension_identifies_binaries() {
    let (_, mangled) = binary_pair();
    assert_eq!(true_extension(&mangled), Some(".hwp"));
    assert_eq!(true_extension("무림맹의 장문인. ".repeat(40).as_bytes()), None);
    assert_eq!(true_extension_with(b"whatever", &|_| Some(b"ZZZZ not a known magic".to_vec())), None);
}

#[test]
fn fix_extension_renames_and_restores() {
    let dir = tempfile::tempdir().unwrap();
    let (orig, mangled) = binary_pair();
    let p = dir.path().join("경혼기.txt");
    fs::write(&p, &mangled).unwrap();
    let res = fix_extension(&p, false).unwrap();
    assert!(matches!(res, FixResult::Restored { .. }));
    assert!(!p.exists());
    assert_eq!(fs::read(dir.path().join("경혼기.hwp")).unwrap(), orig);
}

#[test]
fn fix_extension_dry_run_changes_nothing_and_ignores_normal_text() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mangled) = binary_pair();
    let p = dir.path().join("책.txt");
    fs::write(&p, &mangled).unwrap();
    assert!(fix_extension(&p, true).is_some());
    assert!(p.exists());
    assert!(!dir.path().join("책.hwp").exists());

    let plain = dir.path().join("소설.txt");
    fs::write(&plain, "무림맹의 장문인은 제자들을 이끌었다. ".repeat(30)).unwrap();
    assert!(fix_extension(&plain, false).is_none());
    assert!(plain.exists());
}

#[test]
fn fix_extension_avoids_overwriting_existing_target() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mangled) = binary_pair();
    let p = dir.path().join("책.txt");
    fs::write(&p, &mangled).unwrap();
    fs::write(dir.path().join("책.hwp"), "먼저 있던 것").unwrap();
    let Some(FixResult::Restored { restored_to, .. }) = fix_extension(&p, false) else { panic!("not restored") };
    assert_eq!(Path::new(&restored_to).file_name().unwrap(), "책__1.hwp");
    assert_eq!(fs::read_to_string(dir.path().join("책.hwp")).unwrap(), "먼저 있던 것");
}

#[test]
fn fix_extension_reports_unreadable_file() {
    let dir = tempfile::tempdir().unwrap();
    // 디렉토리는 읽을 수 없다.
    assert!(matches!(fix_extension(dir.path(), false), Some(FixResult::Error { .. })));
}

#[test]
fn fix_extension_refuses_lossy_roundtrip_and_skips_when_unmangle_flakes() {
    let dir = tempfile::tempdir().unwrap();
    let (orig, mangled) = binary_pair();
    let f = dir.path().join("a.txt");
    fs::write(&f, &mangled).unwrap();
    // 무손실이 아니면 손대지 않는다. 원본을 잃는 것이 오탐보다 나쁘다.
    let lossy = |_: &[u8]| Some([orig.clone(), vec![0, 0]].concat());
    let Some(FixResult::Error { error, .. }) = fix_extension_with(&f, false, &lossy) else { panic!("expected error") };
    assert_eq!(error, "무손실 왕복 실패");
    assert_eq!(fs::read(&f).unwrap(), mangled);

    // true_extension에서는 성공하고 본 검사에서는 실패하는 경계.
    let calls = Cell::new(0);
    let flaky = |_: &[u8]| {
        calls.set(calls.get() + 1);
        (calls.get() == 1).then(|| orig.clone())
    };
    assert!(fix_extension_with(&f, false, &flaky).is_none());
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
    assert!(out.contains("3_판타지") && out.contains("9_격언명언"));
    let a = out.find("3_판타지").unwrap();
    let b = out.find("9_격언명언").unwrap();
    assert!(a < b, "건수가 많은 카테고리가 먼저 나온다");
}

#[test]
fn summarize_labels_files_without_a_category() {
    assert!(summary(&[json!({"dst": format!("/text/{BROKEN_DIR_NAME}"), "size": 0})]).contains("(최상위)"));
}

// ---- CLI ----

fn report_for(lib: &Lib, rels: &[&str]) -> PathBuf {
    let p = lib.tmp.join("report.json");
    fs::write(&p, serde_json::to_string(&records_for(lib, rels)).unwrap()).unwrap();
    p
}

fn run_cli(lib: &Lib, argv: &[&str]) -> (i32, String) {
    let args: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    let code = cli::run(&args, &lib.tmp, &mut out, &quiet);
    (code, String::from_utf8(out).unwrap())
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn cli_plan_prints_summary_without_moving() {
    let lib = library();
    let report = report_for(&lib, &["3_판타지/a.txt", "9_격언명언/c.txt"]);
    let (code, out) = run_cli(&lib, &["plan", "--report", p(&report), "--library-root", p(&lib.root)]);
    assert_eq!(code, 0);
    assert!(out.contains("대상 2건") && out.contains("3_판타지") && out.contains("likeness=0.004"));
    assert!(lib.root.join("3_판타지/a.txt").exists());
}

#[test]
fn cli_move_then_undo_round_trip() {
    let lib = library();
    let report = report_for(&lib, &["3_판타지/a.txt"]);
    let manifest = lib.tmp.join("manifest.json");
    let src = lib.root.join("3_판타지/a.txt");
    let (code, out) = run_cli(&lib, &["move", "--report", p(&report), "--library-root", p(&lib.root), "--manifest", p(&manifest)]);
    assert_eq!(code, 0);
    assert!(!src.exists() && out.contains("이동 완료 1건"));
    let (code, out) = run_cli(&lib, &["undo", "--manifest", p(&manifest)]);
    assert_eq!(code, 0);
    assert!(src.exists() && out.contains("원위치 1건"));
}

#[test]
fn cli_move_dry_run_keeps_files() {
    let lib = library();
    let report = report_for(&lib, &["3_판타지/a.txt"]);
    let manifest = lib.tmp.join("manifest.json");
    let (_, out) = run_cli(&lib, &["move", "--report", p(&report), "--library-root", p(&lib.root), "--manifest", p(&manifest), "--dry-run"]);
    assert!(out.contains("이동 예정 1건"));
    assert!(lib.root.join("3_판타지/a.txt").exists() && !manifest.exists());
}

#[test]
fn cli_limit_and_exclude_narrow_the_target() {
    let lib = library();
    let report = report_for(&lib, &["3_판타지/a.txt", "3_판타지/b.txt", "9_격언명언/c.txt"]);
    let (_, out) = run_cli(&lib, &["plan", "--report", p(&report), "--library-root", p(&lib.root), "--exclude", "c.txt", "--limit", "1"]);
    assert!(out.contains("대상 1건"));
}

#[test]
fn cli_move_and_undo_report_failures() {
    let lib = library();
    let manifest = lib.tmp.join("manifest.json");
    let report = report_for(&lib, &["3_판타지/없는파일.txt"]);
    let (_, out) = run_cli(&lib, &["move", "--report", p(&report), "--library-root", p(&lib.root), "--manifest", p(&manifest)]);
    assert!(out.contains("실패: 없는파일.txt - 원본 없음"));

    let report = report_for(&lib, &["3_판타지/a.txt"]);
    run_cli(&lib, &["move", "--report", p(&report), "--library-root", p(&lib.root), "--manifest", p(&manifest)]);
    let moved = read_json(&manifest)["moved"].clone();
    fs::remove_file(moved[0]["dst"].as_str().unwrap()).unwrap();
    let (code, out) = run_cli(&lib, &["undo", "--manifest", p(&manifest)]);
    assert_eq!(code, 0);
    assert!(out.contains("실패: a.txt - 격리본 없음"));
}

#[test]
fn cli_undo_without_manifest_exits_1() {
    let lib = library();
    assert_eq!(run_cli(&lib, &["undo", "--manifest", p(&lib.tmp.join("none.json"))]).0, 1);
}

#[test]
fn cli_fix_ext_restores_reports_and_dry_runs() {
    let lib = library();
    let (orig, mangled) = binary_pair();
    let target = lib.tmp.join("망가진문서.txt");
    fs::write(&target, &mangled).unwrap();
    let report = lib.tmp.join("fix.json");
    fs::write(&report, json!([{"path": target.to_string_lossy(), "verdict": "corrupted"}]).to_string()).unwrap();
    let (code, out) = run_cli(&lib, &["fix-ext", "--report", p(&report)]);
    assert_eq!(code, 0);
    assert!(out.contains("교정 완료 1건"));
    assert_eq!(fs::read(lib.tmp.join("망가진문서.hwp")).unwrap(), orig);
    assert!(!target.exists());

    fs::write(&target, &mangled).unwrap();
    let plain = lib.tmp.join("정상문서.txt");
    fs::write(&plain, "보통 한국어 문장입니다").unwrap();
    let missing = lib.tmp.join("없는파일.txt");
    fs::write(&report, json!([{"path": target.to_string_lossy()}, {"path": plain.to_string_lossy()}, {"path": missing.to_string_lossy()}]).to_string()).unwrap();
    fs::remove_file(lib.tmp.join("망가진문서.hwp")).unwrap();
    let (_, out) = run_cli(&lib, &["fix-ext", "--report", p(&report), "--dry-run"]);
    assert!(out.contains("교정 예정 1건") && out.contains("건너뜀: 없는파일.txt"));
    assert!(target.exists());
}

#[test]
fn cli_help_and_bad_arguments() {
    let lib = library();
    assert_eq!(run_cli(&lib, &["--help"]).0, 0);
    assert_eq!(run_cli(&lib, &["bogus"]).0, 2);
    assert_eq!(run_cli(&lib, &[]).0, 2);
    assert_eq!(run_cli(&lib, &["plan", "--limit", "x"]).0, 2);
}

#[test]
fn rec_is_a_json_map() {
    let _: Rec = Map::new();
}
