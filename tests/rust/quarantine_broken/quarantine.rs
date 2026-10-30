//! 손상 문서 격리 도구 테스트 (tests/test_quarantine_broken.py 이식).
//! 이동은 항상 되돌릴 수 있어야 한다. 판정에 오탐이 섞일 수 있기 때문이다.

use std::cell::{Cell, RefCell};
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

/// bookdoctor 리포트의 `files[]` 항목 하나. 지표 코드 목록을 그대로 받는다.
fn bd_file(path: &str, codes: &[&str], likeness: Option<f64>) -> Value {
    let findings: Vec<Value> = codes.iter().map(|c| json!({"category": "defect", "severity": "HIGH", "code": c, "location": "", "detail": "", "viewer_blocked": "n/a"})).collect();
    json!({"path": path, "format": "txt", "findings": findings, "incomplete": false, "reasons": [], "likeness": likeness})
}

/// bookdoctor `--report`가 쓰는 형태의 리포트. `(경로, likeness)`는 MOJIBAKE 항목, `decoys`는 그 밖의 항목이다.
fn bd_report(mojibake: &[(&str, Option<f64>)], decoys: Vec<Value>) -> String {
    let mut files: Vec<Value> = mojibake.iter().map(|(p, l)| bd_file(p, &["MOJIBAKE"], *l)).collect();
    files.extend(decoys);
    json!({"scanned_at": "2026-10-29 21:55:56", "root": "/lib", "summary": {"scanned": files.len(), "by_severity": {}, "by_category": {}, "incomplete": 0}, "files": files}).to_string()
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
fn load_corrupted_reads_single_and_mixed_bookdoctor_reports() {
    // 옛 테스트는 scan 리포트와 recheck 목록 두 형태를 읽었다. 이제는 bookdoctor 리포트 한 형태이고,
    // 항목 하나짜리 리포트와 지표가 섞인 여러 항목 리포트가 둘 다 읽혀야 한다.
    let dir = tempfile::tempdir().unwrap();
    let scan = dir.path().join("report.json");
    fs::write(&scan, bd_report(&[("/a.txt", Some(0.002))], vec![])).unwrap();
    let recs = load_corrupted(&scan).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(s(&recs[0], "verdict"), VERDICT_CORRUPTED);
    assert_eq!(recs[0]["likeness"], json!(0.002));

    let mixed = dir.path().join("mixed.json");
    let decoys = vec![bd_file("/b.txt", &[], None), bd_file("/c.txt", &["NOT_UTF8"], None)];
    fs::write(&mixed, bd_report(&[("/a.txt", Some(0.004)), ("/d.txt", None)], decoys)).unwrap();
    let paths: Vec<String> = load_corrupted(&mixed).unwrap().iter().map(|r| s(r, "path")).collect();
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
    let entries: Vec<(String, Option<f64>)> = rels.iter().map(|r| (lib.root.join(r).to_string_lossy().into_owned(), Some(0.004))).collect();
    let refs: Vec<(&str, Option<f64>)> = entries.iter().map(|(p, l)| (p.as_str(), *l)).collect();
    fs::write(&p, bd_report(&refs, vec![])).unwrap();
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
    fs::write(&report, bd_report(&[(p(&target), Some(0.002))], vec![])).unwrap();
    let (code, out) = run_cli(&lib, &["fix-ext", "--report", p(&report)]);
    assert_eq!(code, 0);
    assert!(out.contains("교정 완료 1건"));
    assert_eq!(fs::read(lib.tmp.join("망가진문서.hwp")).unwrap(), orig);
    assert!(!target.exists());

    fs::write(&target, &mangled).unwrap();
    let plain = lib.tmp.join("정상문서.txt");
    fs::write(&plain, "보통 한국어 문장입니다").unwrap();
    let missing = lib.tmp.join("없는파일.txt");
    fs::write(&report, bd_report(&[(p(&target), None), (p(&plain), None), (p(&missing), None)], vec![])).unwrap();
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
    assert_eq!(run_cli(&lib, &["plan", "--limit", "x"]).0, 2);
}

#[test]
fn rec_is_a_json_map() {
    let _: Rec = Map::new();
}

// ---- 사용법 ----

#[test]
fn no_arguments_and_help_print_purpose_and_usage() {
    let lib = library();
    for argv in [&[][..], &["--help"], &["-h"]] {
        let (code, out) = run_cli(&lib, argv);
        assert_eq!(code, 0, "{argv:?}");
        for needle in ["quarantine_broken", "격리", "사용법:", "plan", "move", "undo", "fix-ext", "--report", "--library-root", "--manifest", "--limit", "--exclude", "--dry-run"] {
            assert!(out.contains(needle), "{needle} 없음: {out}");
        }
    }
    assert!(lib.root.join("3_판타지/a.txt").exists());
}

// ---- 분기 보강 ----

#[test]
fn cli_parses_inline_values_and_rejects_unknown_options_and_extra_positionals() {
    let lib = library();
    let report = report_for(&lib, &["3_판타지/a.txt", "3_판타지/b.txt"]);
    let inline = [format!("--report={}", report.display()), format!("--library-root={}", lib.root.display()), "--limit=1".to_string()];
    let refs: Vec<&str> = std::iter::once("plan").chain(inline.iter().map(String::as_str)).collect();
    let (code, out) = run_cli(&lib, &refs);
    assert_eq!(code, 0);
    assert!(out.contains("대상 1건"));
    assert_eq!(run_cli(&lib, &["plan", "--bogus"]).0, 2);
    assert_eq!(run_cli(&lib, &["plan", "extra", "more"]).0, 2);
}

#[test]
fn plan_prints_none_for_a_missing_likeness() {
    let lib = library();
    let report = lib.tmp.join("r.json");
    fs::write(&report, bd_report(&[(p(&lib.root.join("3_판타지/a.txt")), None)], vec![])).unwrap();
    let (_, out) = run_cli(&lib, &["plan", "--report", p(&report), "--library-root", p(&lib.root)]);
    assert!(out.contains("likeness=None"), "{out}");
}

#[test]
fn fix_extension_leaves_files_that_already_have_the_right_extension() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mangled) = binary_pair();
    let f = dir.path().join("a.hwp");
    fs::write(&f, &mangled).unwrap();
    assert!(fix_extension(&f, false).is_none());
    assert!(f.exists());
}

#[test]
fn fix_extension_reports_a_write_failure_and_keeps_the_original() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (_, mangled) = binary_pair();
    let f = dir.path().join("책.txt");
    fs::write(&f, &mangled).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let res = fix_extension(&f, false);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(res, Some(FixResult::Error { .. })), "{res:?}");
    assert!(f.exists());
}

#[test]
fn load_corrupted_rejects_reports_without_a_corrupted_list() {
    let dir = tempfile::tempdir().unwrap();
    // 옛 테스트의 세 본문에 최상위 배열(옛 recheck 형태)을 더했다. 모두 files 목록이 없다.
    for body in [r#"{"total": 3}"#, r#""text""#, "not json", r#"[{"path": "/a"}]"#] {
        let f = dir.path().join("r.json");
        fs::write(&f, body).unwrap();
        assert!(load_corrupted(&f).is_err(), "{body}");
    }
    assert!(load_corrupted(&dir.path().join("none.json")).is_err());
    // 객체가 아닌 항목에서 실패한다. 조용히 결과를 줄이지 않는다.
    let f = dir.path().join("mixed.json");
    fs::write(&f, r#"{"files": [1, "x", {"path": "/a"}]}"#).unwrap();
    assert_eq!(load_corrupted(&f).unwrap_err().kind(), io::ErrorKind::InvalidData);
}

// ---- bookdoctor 리포트 형식 ----

#[test]
fn only_mojibake_findings_are_quarantined() {
    let lib = library();
    let a = lib.root.join("3_판타지/a.txt");
    let b = lib.root.join("3_판타지/b.txt");
    let c = lib.root.join("9_격언명언/c.txt");
    let decoys = vec![
        bd_file(p(&a), &["NOT_UTF8", "FORMAT_MISMATCH"], None),
        bd_file(p(&c), &["BINARY_CONTENT", "MOJIBAKE_SUSPECT"], Some(0.001)),
        bd_file(p(&b), &["NOT_UTF8", "MOJIBAKE", "FORMAT_MISMATCH"], Some(0.003)),
    ];
    let f = lib.tmp.join("r.json");
    fs::write(&f, bd_report(&[], decoys)).unwrap();
    let moves = plan_moves(&load_corrupted(&f).unwrap(), &lib.root, &quiet);
    let srcs: Vec<String> = moves.iter().map(|m| s(m, "src")).collect();
    assert_eq!(srcs, [b.to_string_lossy()]);
}

#[test]
fn rejects_old_encoding_tool_report_shape() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("old.json");
    fs::write(&f, json!({"corrupted": [{"path": "/a.txt", "verdict": "corrupted", "likeness": 0.004}]}).to_string()).unwrap();
    let err = load_corrupted(&f).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("bookdoctor --report"), "{err}");
}

#[test]
fn report_without_files_array_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for body in [r#"{"summary": {}}"#, r#"{"files": null}"#, r#"{"files": {"a": 1}}"#, r#"{"files": "x"}"#] {
        let f = dir.path().join("r.json");
        fs::write(&f, body).unwrap();
        assert_eq!(load_corrupted(&f).unwrap_err().kind(), io::ErrorKind::InvalidData, "{body}");
    }
}

#[test]
fn likeness_null_or_missing_is_handled_like_the_old_code() {
    // 옛 코드는 likeness 키가 없는 항목을 그대로 받아 plan_moves가 null로, plan 출력이 `None`으로 보여 줬다.
    let lib = library();
    let a = lib.root.join("3_판타지/a.txt");
    let b = lib.root.join("3_판타지/b.txt");
    let mut no_key = bd_file(p(&b), &["MOJIBAKE"], None);
    no_key.as_object_mut().unwrap().remove("likeness");
    let f = lib.tmp.join("r.json");
    fs::write(&f, bd_report(&[(p(&a), None)], vec![no_key])).unwrap();
    let recs = load_corrupted(&f).unwrap();
    assert_eq!(recs.len(), 2);
    assert!(recs.iter().all(|r| !r.contains_key("likeness") && s(r, "verdict") == VERDICT_CORRUPTED));
    let moves = plan_moves(&recs, &lib.root, &quiet);
    assert!(moves.iter().all(|m| m["likeness"].is_null()));
    let (_, out) = run_cli(&lib, &["plan", "--report", p(&f), "--library-root", p(&lib.root)]);
    assert_eq!(out.matches("likeness=None").count(), 2, "{out}");
}

#[test]
fn reads_a_report_shaped_like_the_real_bookdoctor_output() {
    // `bookdoctor -r --report r.json DIR`를 한글 음절을 무작위로 섞은 TXT 하나에 돌려 얻은 files[0]이다
    // (경로만 /lib/a.txt로 바꿨다). quarantine_broken은 bookdoctor에 의존하지 않으므로 실제 출력을 그대로 붙여 둔다.
    let real = r#"{
      "scanned_at": "2026-10-29 21:55:56",
      "root": "/lib",
      "summary": {"scanned": 1, "by_severity": {"HIGH": 1}, "by_category": {"defect": 1}, "incomplete": 0},
      "files": [{
        "path": "/lib/a.txt",
        "format": "txt",
        "findings": [{"category": "defect", "severity": "HIGH", "code": "MOJIBAKE", "location": "", "detail": "likeness=0.002", "viewer_blocked": "n/a"}],
        "incomplete": false,
        "reasons": [],
        "likeness": 0.002
      }]
    }"#;
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("r.json");
    fs::write(&f, real).unwrap();
    let recs = load_corrupted(&f).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!((s(&recs[0], "path"), s(&recs[0], "verdict")), ("/lib/a.txt".to_owned(), "corrupted".to_owned()));
    assert_eq!(recs[0]["likeness"], json!(0.002));
}

#[test]
fn plan_skips_records_without_a_path() {
    let lib = library();
    let rec = json!({"verdict": "corrupted"}).as_object().unwrap().clone();
    assert!(plan_moves(&[rec], &lib.root, &quiet).is_empty());
}

#[test]
fn copy_fallback_cleans_up_when_the_copy_fails() {
    let lib = library();
    let src = lib.root.join("3_판타지/a.txt");
    let err = copy_then_remove(&src, &lib.tmp.join("no/such/dir/a.txt")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert!(src.exists());
}

#[test]
fn move_file_falls_back_to_copy_across_filesystems() {
    use std::os::unix::fs::MetadataExt;
    let shm = Path::new("/dev/shm");
    let lib = library();
    if !shm.is_dir() || fs::metadata(shm).unwrap().dev() == fs::metadata(&lib.tmp).unwrap().dev() {
        return; // 다른 파일시스템이 없는 환경에서는 건너뛴다.
    }
    let src = lib.root.join("3_판타지/a.txt");
    let dst = shm.join(format!("qb-{}.txt", std::process::id()));
    move_file(&src, &dst).unwrap();
    assert!(!src.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "3_판타지/a.txt 내용");
    fs::remove_file(&dst).unwrap();
}

#[test]
fn manifest_write_failure_is_logged_but_the_move_is_reported() {
    let lib = library();
    let blocker = lib.tmp.join("file");
    fs::write(&blocker, "x").unwrap();
    let logged = RefCell::new(Vec::<String>::new());
    let log = |m: &str| logged.borrow_mut().push(m.to_string());
    let result = apply_moves(&plan(&lib, &["3_판타지/a.txt"]), &blocker.join("m.json"), false, &log);
    assert_eq!(result.moved.len(), 1);
    assert!(logged.borrow().iter().any(|m| m.starts_with("ERROR 매니페스트를 쓰지 못했다")), "{:?}", logged.borrow());
}

#[test]
fn summaries_group_thousands_with_commas() {
    let moves: Vec<Value> = (0..1234).map(|i| json!({"dst": format!("/text/{BROKEN_DIR_NAME}/3_판타지/{i}.txt"), "size": 1})).collect();
    assert!(summary(&moves).contains("대상 1,234건"));
}

// ---- 크기 보충 ----

#[test]
fn plan_stats_the_source_when_the_record_has_no_size() {
    // bookdoctor 리포트에는 size가 없다. 계획 단계에서 원본 파일 크기를 읽어 채운다.
    let lib = library();
    let big = lib.root.join("3_판타지/큰파일.txt");
    fs::write(&big, vec![b'x'; 1024 * 1024]).unwrap();
    let f = lib.tmp.join("r.json");
    fs::write(&f, bd_report(&[(p(&big), Some(0.002))], vec![])).unwrap();
    let moves = plan_moves(&load_corrupted(&f).unwrap(), &lib.root, &quiet);
    assert_eq!(moves[0]["size"], json!(1024 * 1024));
    assert!(summary(&moves.into_iter().map(Value::Object).collect::<Vec<_>>()).contains("대상 1건, 1.0MB"));
}

#[test]
fn plan_keeps_a_size_the_record_already_has_and_uses_zero_for_a_missing_file() {
    let lib = library();
    let mut recs = records_for(&lib, &["3_판타지/a.txt"]);
    recs[0].insert("size".into(), json!(777));
    assert_eq!(plan_moves(&recs, &lib.root, &quiet)[0]["size"], json!(777));

    let ghost = lib.root.join("3_판타지/없는파일.txt");
    let rec = json!({"path": ghost.to_string_lossy(), "verdict": "corrupted"}).as_object().unwrap().clone();
    assert_eq!(plan_moves(&[rec], &lib.root, &quiet)[0]["size"], json!(0));
}

#[test]
fn load_corrupted_skips_pdf_mojibake_and_logs_it() {
    // `bookdoctor --pdf-text`는 PDF에도 MOJIBAKE를 낸다. 설계서 12-1: 격리 대상은 지금처럼 txt와 epub뿐이다.
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("report.json");
    let mut pdf = bd_file("/lib/a.pdf", &["MOJIBAKE"], Some(0.001));
    pdf["format"] = json!("pdf");
    let mut epub = bd_file("/lib/b.epub", &["MOJIBAKE"], Some(0.002));
    epub["format"] = json!("epub");
    fs::write(&f, bd_report(&[("/lib/c.txt", Some(0.003))], vec![pdf, epub])).unwrap();
    let logged = std::cell::RefCell::new(Vec::<String>::new());
    let recs = load_corrupted_logged(&f, &|m| logged.borrow_mut().push(m.to_owned())).unwrap();
    let paths: Vec<String> = recs.iter().map(|r| s(r, "path")).collect();
    assert_eq!(paths, ["/lib/c.txt", "/lib/b.epub"]);
    assert_eq!(logged.borrow().len(), 1);
    assert!(logged.borrow()[0].contains("/lib/a.pdf"), "{:?}", logged.borrow());
    assert_eq!(load_corrupted(&f).unwrap().len(), 2);
}

// ---- 표시용 문자열 정리: 화면에는 ESC[31m·U+202E가 없고, 열거나 옮기는 경로는 원문이다 ----

const EVIL: &str = "ev\u{1b}[31m\u{202e}il.txt";

fn assert_clean(text: &str) {
    assert!(!text.contains('\u{1b}') && !text.contains('\u{202e}'), "{text:?}");
}

fn evil_library() -> Lib {
    let lib = library();
    fs::write(lib.root.join("3_판타지").join(EVIL), "본문").unwrap();
    lib
}

#[test]
fn cli_plan_and_move_print_clean_names_but_move_the_real_file() {
    let lib = evil_library();
    let rel = format!("3_판타지/{EVIL}");
    let report = report_for(&lib, &[rel.as_str()]);
    let manifest = lib.tmp.join("manifest.json");
    let (_, out) = run_cli(&lib, &["plan", "--report", p(&report), "--library-root", p(&lib.root)]);
    assert_clean(&out);
    assert!(out.contains("ev.[31m.il.txt"), "{out:?}");
    let (code, out) = run_cli(&lib, &["move", "--report", p(&report), "--library-root", p(&lib.root), "--manifest", p(&manifest)]);
    assert_eq!(code, 0);
    assert_clean(&out);
    assert!(!lib.root.join("3_판타지").join(EVIL).exists());
    assert!(lib.root.join(BROKEN_DIR_NAME).join("3_판타지").join(EVIL).exists());
}

#[test]
fn cli_failure_lines_and_manifest_path_are_clean() {
    let lib = library();
    let manifest = lib.tmp.join(format!("m{EVIL}.json"));
    let rel = format!("3_판타지/없는{EVIL}");
    let report = report_for(&lib, &[rel.as_str()]);
    let (_, out) = run_cli(&lib, &["move", "--report", p(&report), "--library-root", p(&lib.root), "--manifest", p(&manifest)]);
    assert_clean(&out);
    assert!(out.contains("원본 없음"), "{out:?}");
    // 실제 이동 뒤 격리본을 지워 undo 실패 줄과 매니페스트 줄도 확인한다.
    fs::write(lib.root.join("3_판타지").join(EVIL), "본문").unwrap();
    let rel = format!("3_판타지/{EVIL}");
    let report = report_for(&lib, &[rel.as_str()]);
    let (_, out) = run_cli(&lib, &["move", "--report", p(&report), "--library-root", p(&lib.root), "--manifest", p(&manifest)]);
    assert_clean(&out);
    assert!(out.contains("매니페스트: "), "{out:?}");
    let moved = read_json(&manifest)["moved"].clone();
    fs::remove_file(moved[0]["dst"].as_str().unwrap()).unwrap();
    let (_, out) = run_cli(&lib, &["undo", "--manifest", p(&manifest)]);
    assert_clean(&out);
    assert!(out.contains("격리본 없음"), "{out:?}");
}

#[test]
fn move_failure_log_is_clean_while_the_mover_gets_the_raw_paths() {
    let lib = evil_library();
    let rel = format!("3_판타지/{EVIL}");
    let moves = plan(&lib, &[rel.as_str()]);
    let seen = RefCell::new(Vec::<PathBuf>::new());
    let logged = RefCell::new(Vec::<String>::new());
    let boom = |a: &Path, _: &Path| -> io::Result<()> {
        seen.borrow_mut().push(a.to_path_buf());
        Err(io::Error::other(format!("실패 {EVIL}")))
    };
    let result = apply_moves_with(&moves, &lib.tmp.join("manifest.json"), false, &|m| logged.borrow_mut().push(m.to_owned()), &boom);
    assert_eq!(result.failed.len(), 1);
    assert_eq!(seen.borrow()[0], lib.root.join("3_판타지").join(EVIL));
    assert!(!logged.borrow().is_empty());
    for l in logged.borrow().iter() {
        assert_clean(l);
    }
}

#[test]
fn skipped_paths_and_summary_categories_are_clean() {
    let lib = library();
    let logged = RefCell::new(Vec::<String>::new());
    let outside = json!({"path": format!("/elsewhere/{EVIL}"), "verdict": "corrupted"}).as_object().unwrap().clone();
    plan_moves(&[outside], &lib.root, &|m| logged.borrow_mut().push(m.to_owned()));
    assert_eq!(logged.borrow().len(), 1);
    assert_clean(&logged.borrow()[0]);

    let mv = json!({"src": "/x", "dst": format!("{}/_broken/3_{EVIL}/a.txt", lib.root.display())}).as_object().unwrap().clone();
    let mut out = Vec::new();
    summarize(&[mv], &mut out).unwrap();
    assert_clean(&String::from_utf8(out).unwrap());
}
