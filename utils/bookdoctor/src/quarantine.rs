//! `quarantine`·`unquarantine`: 인코딩이 손상된(`MOJIBAKE`) TXT·EPUB을 격리 디렉터리로 옮기고 되돌린다.
//! 옛 `utils/quarantine_broken`을 옮겨 왔다. 리포트 파일을 읽지 않고 경로를 직접 검사한다.
//!
//! 판정에 오탐이 섞일 수 있으므로 이동은 항상 되돌릴 수 있어야 한다. 옮긴 내역을 매니페스트에 누적하고 `unquarantine`으로 원위치시킨다.
//! 원래 카테고리를 하위 디렉터리로 유지한다. `0_broken/3_판타지/...` 형태라 어느 카테고리에서 왔는지 보이고 되돌리기도 쉽다.
//! PDF의 `MOJIBAKE`는 격리하지 않는다. 텍스트층 판정만으로 파일을 옮기지 않는다.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::display::thousands;
use serde_json::{json, Map, Value};

use crate::cli::{Options, BROKEN_DIR_NAME};
use crate::scan::scan_file;

/// 인코딩 손상을 뜻하는 지표 코드. 다른 코드(NOT_UTF8, FORMAT_MISMATCH 등)는 격리하지 않는다.
pub const MOJIBAKE_CODE: &str = "MOJIBAKE";
/// 격리 대상 항목에 붙이는 판정 값. 옛 매니페스트와 같은 문자열이다.
pub const VERDICT_CORRUPTED: &str = "corrupted";

/// 이동 기록. 알 수 없는 키도 그대로 보존하려고 JSON 맵을 쓴다.
pub type Rec = Map<String, Value>;

/// 로그 한 줄을 받는 쪽. 메시지 앞에 `INFO`/`WARNING` 수준이 붙어 있다.
pub type Log<'a> = &'a dyn Fn(&str);

/// 파일 한 건을 옮기는 함수. 테스트가 OS 오류를 주입하는 데 쓴다.
pub type Mover<'a> = &'a dyn Fn(&Path, &Path) -> io::Result<()>;

/// 터미널에 찍을 문자열에서 제어문자와 방향·서식 문자를 지운다. 경로는 파일을 열거나 옮길 때만 원문을 쓴다.
fn show(text: &str) -> String {
    crate::display::sanitize_display(text, usize::MAX, None)
}

fn name_of(path: &str, max_chars: usize) -> String {
    Path::new(path).file_name().map(|n| crate::display::sanitize_display(&n.to_string_lossy(), max_chars, None)).unwrap_or_default()
}

fn field(rec: &Rec, key: &str) -> String {
    rec.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// 검사 대상: TXT와 EPUB.
fn quarantinable(p: &Path) -> bool {
    p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt") || e.eq_ignore_ascii_case("epub"))
}

/// 경로를 검사해 `MOJIBAKE`가 나온 파일을 기록으로 만든다(path, likeness, verdict).
pub fn find_corrupted(opts: &Options) -> Vec<Rec> {
    let mut found = Vec::new();
    for path in crate::cli::collect(opts, quarantinable) {
        let report = scan_file(&path, opts);
        if !report.findings.iter().any(|f| f.code == MOJIBAKE_CODE) {
            continue;
        }
        let mut rec = Rec::new();
        rec.insert("path".into(), json!(path.to_string_lossy()));
        if let Some(likeness) = report.likeness {
            rec.insert("likeness".into(), json!(likeness));
        }
        rec.insert("verdict".into(), json!(VERDICT_CORRUPTED));
        found.push(rec);
    }
    found
}

fn split_name(p: &Path) -> (String, String) {
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (stem, suffix)
}

/// 원본 경로마다 격리 경로를 정한다. 원래 카테고리를 하위 디렉터리로 유지한다.
pub fn plan_moves(records: &[Rec], library_root: &Path, log: Log) -> Vec<Rec> {
    let broken_root = library_root.join(BROKEN_DIR_NAME);
    let mut moves = Vec::new();
    let mut taken: HashSet<PathBuf> = HashSet::new();
    for rec in records {
        let Some(src) = rec.get("path").and_then(Value::as_str).map(PathBuf::from) else { continue };
        let Ok(rel) = src.strip_prefix(library_root) else {
            log(&format!("WARNING 라이브러리 밖 경로는 건너뛴다: {}", show(&src.to_string_lossy())));
            continue;
        };
        if rel.as_os_str().is_empty() || crate::cli::in_reserved_dir(&src, library_root) {
            continue;
        }
        let dst = broken_root.join(rel);

        // 이름이 겹치면 덮어쓰지 않고 접미사를 붙인다.
        let (stem, suffix) = split_name(&dst);
        let mut candidate = dst.clone();
        let mut n = 1;
        while taken.contains(&candidate) || candidate.exists() {
            candidate = dst.with_file_name(format!("{stem}__{n}{suffix}"));
            n += 1;
        }
        taken.insert(candidate.clone());

        let mut mv = Rec::new();
        mv.insert("src".into(), json!(src.to_string_lossy()));
        mv.insert("dst".into(), json!(candidate.to_string_lossy()));
        mv.insert("likeness".into(), rec.get("likeness").cloned().unwrap_or(Value::Null));
        let size = match rec.get("size") {
            Some(v) if v.is_number() => v.clone(),
            _ => json!(fs::metadata(&src).map(|m| m.len()).unwrap_or(0)),
        };
        mv.insert("size".into(), size);
        moves.push(mv);
    }
    moves
}

/// 교차 파일시스템 이동: 복사하고 mtime을 맞춘 뒤 원본을 지운다.
pub fn copy_then_remove(src: &Path, dst: &Path) -> io::Result<()> {
    let mtime = fs::metadata(src)?.modified()?;
    let copied = fs::copy(src, dst).and_then(|_| File::options().write(true).open(dst)?.set_modified(mtime));
    if let Err(e) = copied {
        let _ = fs::remove_file(dst);
        return Err(e);
    }
    fs::remove_file(src)
}

/// 파일 한 건을 옮긴다. 같은 파일시스템이면 rename이라 inode와 mtime이 그대로다(book_id가 inode다).
pub fn move_file(src: &Path, dst: &Path) -> io::Result<()> {
    match fs::rename(src, dst) {
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => copy_then_remove(src, dst),
        other => other,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Manifest {
    pub created_at: String,
    pub moved: Vec<Rec>,
    pub failed: Vec<Rec>,
}

fn with_error(mv: &Rec, error: impl Into<String>) -> Rec {
    let mut r = mv.clone();
    r.insert("error".into(), Value::String(error.into()));
    r
}

fn endpoints(mv: &Rec) -> (PathBuf, PathBuf) {
    let get = |k: &str| PathBuf::from(mv.get(k).and_then(Value::as_str).unwrap_or(""));
    (get("src"), get("dst"))
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    fs::write(path, serde_json::to_string_pretty(value).map_err(io::Error::other)?)
}

pub fn apply_moves(moves: &[Rec], manifest_path: &Path, log: Log) -> Manifest {
    apply_moves_with(moves, manifest_path, log, &move_file)
}

/// 이동을 실행하고 매니페스트를 남긴다. 한 건이라도 옮겼으면 매니페스트를 쓴다.
pub fn apply_moves_with(moves: &[Rec], manifest_path: &Path, log: Log, mover: Mover) -> Manifest {
    let mut done: Vec<Rec> = Vec::new();
    let mut failed: Vec<Rec> = Vec::new();
    for mv in moves {
        let (src, dst) = endpoints(mv);
        if !src.exists() {
            failed.push(with_error(mv, "원본 없음"));
            continue;
        }
        // 부모가 없는 경로는 create_dir_all("")이 그냥 성공한다.
        let moved = fs::create_dir_all(dst.parent().unwrap_or(Path::new(""))).and_then(|()| mover(&src, &dst));
        match moved {
            Ok(()) => done.push(mv.clone()),
            Err(e) => {
                log(&format!("WARNING 이동 실패: {} -> {}: {}", show(&src.to_string_lossy()), show(&dst.to_string_lossy()), show(&e.to_string())));
                failed.push(with_error(mv, e.to_string()));
            }
        }
    }

    let mut manifest = Manifest { created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string(), moved: done.clone(), failed };
    if !done.is_empty() {
        // 기존 매니페스트가 있으면 합쳐서 누적한다. 나눠 옮겨도 unquarantine이 전부를 되돌린다.
        if manifest_path.exists() {
            match read_previous_moved(manifest_path) {
                Ok(mut prev) => {
                    prev.extend(done);
                    manifest.moved = prev;
                }
                Err(e) => log(&format!("WARNING 기존 매니페스트를 읽지 못해 새로 쓴다: {}", show(&e.to_string()))),
            }
        }
        if let Some(dir) = manifest_path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(e) = write_json(manifest_path, &manifest) {
            log(&format!("ERROR 매니페스트를 쓰지 못했다: {}", show(&e.to_string())));
        }
    }
    manifest
}

fn read_previous_moved(path: &Path) -> io::Result<Vec<Rec>> {
    let prev: Value = serde_json::from_str(&fs::read_to_string(path)?).map_err(io::Error::other)?;
    let moved = prev.get("moved").cloned().unwrap_or_else(|| json!([]));
    let Value::Array(items) = moved else { return Err(io::Error::new(io::ErrorKind::InvalidData, "moved가 목록이 아니다")) };
    Ok(items.into_iter().filter_map(|v| if let Value::Object(o) = v { Some(o) } else { None }).collect())
}

#[derive(Debug, Clone)]
pub struct UndoResult {
    pub restored: Vec<Rec>,
    pub failed: Vec<Rec>,
}

pub fn undo_moves(manifest_path: &Path) -> io::Result<UndoResult> {
    undo_moves_with(manifest_path, &move_file)
}

/// 매니페스트에 적힌 이동을 되돌린다. 되돌린 항목은 매니페스트에서 지운다.
pub fn undo_moves_with(manifest_path: &Path, mover: Mover) -> io::Result<UndoResult> {
    if !manifest_path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("매니페스트가 없다: {}", show(&manifest_path.to_string_lossy()))));
    }
    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(manifest_path)?).map_err(io::Error::other)?;
    let moved = read_previous_moved(manifest_path)?;

    let mut restored: Vec<Rec> = Vec::new();
    let mut failed: Vec<Rec> = Vec::new();
    for mv in &moved {
        let (src, dst) = endpoints(mv);
        if !dst.exists() {
            failed.push(with_error(mv, "격리본 없음"));
            continue;
        }
        if src.exists() {
            failed.push(with_error(mv, "원위치에 파일이 이미 있다"));
            continue;
        }
        let back = fs::create_dir_all(src.parent().unwrap_or(Path::new(""))).and_then(|()| mover(&dst, &src));
        match back {
            Ok(()) => restored.push(mv.clone()),
            Err(e) => failed.push(with_error(mv, e.to_string())),
        }
    }

    if !restored.is_empty() {
        let remaining: Vec<Value> = moved.iter().filter(|m| !restored.contains(m)).cloned().map(Value::Object).collect();
        manifest["moved"] = Value::Array(remaining);
        write_json(manifest_path, &manifest)?;
    }
    Ok(UndoResult { restored, failed })
}

/// 원래 카테고리별 건수와 크기 합계. 건수 내림차순, 동률은 먼저 나온 순서다.
pub fn summarize(moves: &[Rec], out: &mut dyn Write) -> io::Result<()> {
    let mut by_cat: Vec<(String, usize)> = Vec::new();
    let mut total_size = 0.0;
    for mv in moves {
        let dst = PathBuf::from(mv.get("dst").and_then(Value::as_str).unwrap_or(""));
        let parts: Vec<String> = dst.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let cat = parts.iter().position(|p| p == BROKEN_DIR_NAME).and_then(|i| parts.get(i + 1)).cloned().unwrap_or_else(|| "(최상위)".to_owned());
        match by_cat.iter_mut().find(|(c, _)| *c == cat) {
            Some((_, n)) => *n += 1,
            None => by_cat.push((cat, 1)),
        }
        total_size += mv.get("size").and_then(Value::as_f64).unwrap_or(0.0);
    }
    writeln!(out, "\n대상 {}건, {:.1}MB", thousands(moves.len()), total_size / 1024.0 / 1024.0)?;
    writeln!(out, "  {:<26}{:>6}", "원래 카테고리", "건수")?;
    by_cat.sort_by(|a, b| b.1.cmp(&a.1));
    for (cat, n) in by_cat {
        writeln!(out, "  {:<26}{n:>6}", show(&cat))?;
    }
    Ok(())
}

fn log_to_stderr(line: &str) {
    eprintln!("{}", show(line));
}

/// `quarantine` 진입점. 경로를 검사해 손상 판정 파일을 옮긴다. 이동 실패가 있으면 1이다.
pub fn run(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let records = find_corrupted(opts);
    let moves = plan_moves(&records, &opts.library_root, &log_to_stderr);
    summarize(&moves, out)?;
    let manifest = opts.manifest_path();
    let result = apply_moves(&moves, &manifest, &log_to_stderr);
    // 매니페스트의 moved는 이전 실행분까지 누적한 목록이다. 이번에 옮긴 수는 계획에서 실패를 뺀 값이다.
    let moved_now = moves.len() - result.failed.len();
    writeln!(out, "\n이동 완료 {}건, 실패 {}건", thousands(moved_now), thousands(result.failed.len()))?;
    for f in result.failed.iter().take(10) {
        writeln!(out, "  실패: {} - {}", name_of(&field(f, "src"), 50), show(&field(f, "error")))?;
    }
    if moved_now > 0 {
        writeln!(out, "매니페스트: {}", show(&manifest.to_string_lossy()))?;
        writeln!(out, "되돌리려면: bookdoctor unquarantine")?;
    }
    Ok(i32::from(!result.failed.is_empty()))
}

/// `unquarantine` 진입점. 매니페스트의 이동을 모두 되돌린다. 되돌리지 못한 항목이 있으면 1이다.
pub fn undo(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let res = undo_moves(&opts.manifest_path())?;
    writeln!(out, "원위치 {}건, 실패 {}건", thousands(res.restored.len()), thousands(res.failed.len()))?;
    for f in res.failed.iter().take(10) {
        writeln!(out, "  실패: {} - {}", name_of(&field(f, "dst"), 50), show(&field(f, "error")))?;
    }
    Ok(i32::from(!res.failed.is_empty()))
}
