//! 인코딩이 손상된 문서를 `0_broken` 디렉토리로 격리한다. 원본: `utils/quarantine_broken.py`.
//!
//! 되돌릴 수 있게 만든다. 옮긴 내역을 매니페스트에 남기고 `undo`로 원위치시킨다.
//! 판정에 오탐이 섞일 수 있으므로 이동은 항상 복구 가능해야 한다.
//!
//! 원래 카테고리를 하위 디렉토리로 유지한다. `0_broken/3_판타지/...` 형태라 어느 카테고리에서
//! 왔는지 눈으로 확인할 수 있고 되돌리기도 쉽다.

pub mod cli;

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

pub use corpus_common::DEFAULT_LIBRARY_ROOT;
// 천 단위 쉼표 출력은 corpus_common에 있다. 같은 코드를 복사해 두지 않는다.
pub(crate) use corpus_common::thousands;

/// 격리 대상으로 고른 항목에 붙이는 판정 값. 옛 detect_mojibake의 `VERDICT_CORRUPTED`와 같은 문자열이다.
pub const VERDICT_CORRUPTED: &str = "corrupted";
/// bookdoctor 리포트에서 인코딩 손상을 뜻하는 지표 코드. 다른 코드(NOT_UTF8, FORMAT_MISMATCH 등)는 격리하지 않는다.
pub const MOJIBAKE_CODE: &str = "MOJIBAKE";

pub const BROKEN_DIR_NAME: &str = "0_broken";

/// 이동 기록과 리포트 항목. 알 수 없는 키도 그대로 보존하려고 JSON 맵을 쓴다.
pub type Rec = Map<String, Value>;

/// 로그 한 줄을 받는 쪽. 메시지 앞에 `INFO`/`WARNING` 수준이 붙어 있다.
pub type Log<'a> = &'a dyn Fn(&str);

/// 파일 한 건을 옮기는 함수. 테스트가 OS 오류를 주입하는 데 쓴다.
pub type Mover<'a> = &'a dyn Fn(&Path, &Path) -> io::Result<()>;

// 되돌리면 알려진 바이너리가 되는 파일의 매직 바이트.
// 바이너리를 UTF-16LE로 읽고 UTF-8로 저장하면 이런 상태가 된다.
//   OLE 매직 D0 CF 11 E0 A1 B1 1A E1
//   -> UTF-16LE로 읽으면 U+CFD0 U+E011 U+B1A1 U+E11A
//   -> UTF-8로 저장하면 ec bf 90 ee 80 91 eb 86 a1 ee 84 9a
// 이 변환은 문자가 전부 BMP 안에 있으면 무손실이라 원본을 그대로 복원할 수 있다.
const BINARY_MAGIC: [(&[u8], &str); 4] = [(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1], ".hwp"), (b"HWP Document File", ".hwp"), (b"%PDF", ".pdf"), (b"\x89PNG\r\n\x1a\n", ".png")];

/// UTF-16LE로 읽혀 UTF-8로 저장된 바이너리를 원래 바이트로 되돌린다.
/// 되돌린 결과가 알려진 바이너리 매직으로 시작할 때만 성공으로 본다.
///
/// 표본을 바이트로 자르면 마지막 문자가 잘리므로 꼬리를 다듬어 가며 시도한다.
pub fn unmangle_utf16(raw: &[u8]) -> Option<Vec<u8>> {
    for trim in 0..4 {
        let Some(chunk) = raw.len().checked_sub(trim).map(|n| &raw[..n]) else { continue };
        let Ok(text) = std::str::from_utf8(chunk) else { continue };
        let restored: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        return BINARY_MAGIC.iter().any(|(magic, _)| restored.starts_with(magic)).then_some(restored);
    }
    None
}

/// 되돌린 바이트가 가리키는 실제 확장자. 해당 없으면 None.
pub fn true_extension(raw: &[u8]) -> Option<&'static str> {
    true_extension_with(raw, &unmangle_utf16)
}

pub fn true_extension_with(raw: &[u8], unmangle: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Option<&'static str> {
    let restored = unmangle(raw)?;
    BINARY_MAGIC.iter().find(|(magic, _)| restored.starts_with(magic)).map(|(_, ext)| *ext)
}

#[derive(Debug, Clone, PartialEq)]
pub enum FixResult {
    Restored { path: String, restored_to: String, before: usize, after: usize },
    Error { path: String, error: String },
}

/// 확장자가 틀린 바이너리를 원래 바이트로 되돌리고 올바른 확장자로 바꾼다. 대상이 아니면 None.
///
/// 무손실 왕복을 확인한 뒤에만 쓴다. 확인에 실패하면 손대지 않는다.
pub fn fix_extension(path: &Path, dry_run: bool) -> Option<FixResult> {
    fix_extension_with(path, dry_run, &unmangle_utf16)
}

pub fn fix_extension_with(path: &Path, dry_run: bool, unmangle: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Option<FixResult> {
    let shown = path.to_string_lossy().into_owned();
    let raw = match fs::read(path) {
        Ok(r) => r,
        Err(e) => return Some(FixResult::Error { path: shown, error: e.to_string() }),
    };

    let ext = true_extension_with(&raw, unmangle)?;
    if path.extension().is_some_and(|e| format!(".{}", e.to_string_lossy().to_lowercase()) == ext) {
        return None;
    }

    let restored = unmangle(&raw)?;
    let roundtrip = restored.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>();
    let lossless = restored.len() % 2 == 0 && String::from_utf16(&roundtrip).is_ok_and(|s| s.into_bytes() == raw);
    if !lossless {
        // 무손실이 아니면 건드리지 않는다.
        return Some(FixResult::Error { path: shown, error: "무손실 왕복 실패".into() });
    }

    let bare = ext.trim_start_matches('.');
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut target = path.with_extension(bare);
    let mut n = 1;
    while target.exists() {
        target = path.with_file_name(format!("{stem}__{n}{ext}"));
        n += 1;
    }

    let result = FixResult::Restored { path: shown, restored_to: target.to_string_lossy().into_owned(), before: raw.len(), after: restored.len() };
    if !dry_run && let Err(e) = fs::write(&target, &restored).and_then(|()| fs::remove_file(path)) {
        return Some(FixResult::Error { path: path.to_string_lossy().into_owned(), error: e.to_string() });
    }
    Some(result)
}

/// bookdoctor `--report` 리포트에서 인코딩 손상(`MOJIBAKE`) 항목만 뽑는다.
///
/// `files[]` 중 `findings`에 `code == "MOJIBAKE"`가 있는 항목만 `Rec`(path, likeness, verdict)로 바꾼다.
/// likeness가 없거나 null이면 키를 넣지 않는다(옛 코드도 없는 키는 그대로 두었다).
/// 옛 detect_mojibake 형식(`{"corrupted": [...]}`)은 조용히 빈 결과가 되지 않도록 오류로 돌려준다.
pub fn load_corrupted(report_path: &Path) -> io::Result<Vec<Rec>> {
    let data: Value = serde_json::from_str(&fs::read_to_string(report_path)?).map_err(io::Error::other)?;
    let invalid = |msg: &str| io::Error::new(io::ErrorKind::InvalidData, msg.to_owned());
    let Value::Object(mut o) = data else {
        return Err(invalid("리포트가 객체가 아니다. `bookdoctor --report`로 다시 만들어라"));
    };
    let Some(files) = o.remove("files") else {
        let hint = if o.contains_key("corrupted") { "옛 detect_mojibake 리포트 형식이다" } else { "리포트에 files 목록이 없다" };
        return Err(invalid(&format!("{hint}. `bookdoctor --report`로 리포트를 다시 만들어라")));
    };
    let Value::Array(items) = files else {
        return Err(invalid("리포트의 files가 배열이 아니다. `bookdoctor --report`로 다시 만들어라"));
    };
    let mut found = Vec::new();
    for item in items {
        let Value::Object(file) = item else { return Err(invalid("files 항목이 객체가 아니다")) };
        let is_mojibake = file.get("findings").and_then(Value::as_array).is_some_and(|fs| fs.iter().any(|f| f.get("code").and_then(Value::as_str) == Some(MOJIBAKE_CODE)));
        if !is_mojibake {
            continue;
        }
        let mut rec = Rec::new();
        if let Some(path) = file.get("path") {
            rec.insert("path".into(), path.clone());
        }
        if let Some(likeness) = file.get("likeness").filter(|l| l.is_number()) {
            rec.insert("likeness".into(), likeness.clone());
        }
        rec.insert("verdict".into(), json!(VERDICT_CORRUPTED));
        found.push(rec);
    }
    Ok(found)
}

fn split_name(p: &Path) -> (String, String) {
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (stem, suffix)
}

/// 원본 경로마다 격리 경로를 정한다. 원래 카테고리를 하위 디렉토리로 유지한다.
pub fn plan_moves(records: &[Rec], library_root: &Path, log: Log) -> Vec<Rec> {
    let broken_root = library_root.join(BROKEN_DIR_NAME);
    let mut moves = Vec::new();
    let mut taken: HashSet<PathBuf> = HashSet::new();
    for rec in records {
        let Some(src) = rec.get("path").and_then(Value::as_str).map(PathBuf::from) else { continue };
        let Ok(rel) = src.strip_prefix(library_root) else {
            log(&format!("WARNING 라이브러리 밖 경로는 건너뛴다: {}", src.display()));
            continue;
        };
        match rel.components().next() {
            Some(first) if first.as_os_str() == BROKEN_DIR_NAME => continue,
            None => continue,
            _ => {}
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
        mv.insert("size".into(), rec.get("size").cloned().unwrap_or(json!(0)));
        moves.push(mv);
    }
    moves
}

/// 교차 파일시스템 이동: 복사하고 mtime을 맞춘 뒤 원본을 지운다(`shutil.move`의 폴백과 같다).
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
    pub dry_run: bool,
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

pub fn apply_moves(moves: &[Rec], manifest_path: &Path, dry_run: bool, log: Log) -> Manifest {
    apply_moves_with(moves, manifest_path, dry_run, log, &move_file)
}

/// 이동을 실행하고 매니페스트를 남긴다. 한 건이라도 옮겼으면 매니페스트를 쓴다.
pub fn apply_moves_with(moves: &[Rec], manifest_path: &Path, dry_run: bool, log: Log, mover: Mover) -> Manifest {
    let mut done: Vec<Rec> = Vec::new();
    let mut failed: Vec<Rec> = Vec::new();

    for mv in moves {
        let (src, dst) = endpoints(mv);
        if !src.exists() {
            failed.push(with_error(mv, "원본 없음"));
            continue;
        }
        if dry_run {
            done.push(mv.clone());
            continue;
        }
        // 부모가 없는 경로는 create_dir_all("")이 그냥 성공한다.
        let moved = fs::create_dir_all(dst.parent().unwrap_or(Path::new(""))).and_then(|()| mover(&src, &dst));
        match moved {
            Ok(()) => done.push(mv.clone()),
            Err(e) => {
                log(&format!("WARNING 이동 실패: {} -> {}: {e}", src.display(), dst.display()));
                failed.push(with_error(mv, e.to_string()));
            }
        }
    }

    let mut manifest = Manifest { created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string(), dry_run, moved: done.clone(), failed };
    if !done.is_empty() && !dry_run {
        // 기존 매니페스트가 있으면 합쳐서 누적한다. 나눠 옮겨도 undo가 전부를 되돌린다.
        if manifest_path.exists() {
            match read_previous_moved(manifest_path) {
                Ok(mut prev) => {
                    prev.extend(done);
                    manifest.moved = prev;
                }
                Err(e) => log(&format!("WARNING 기존 매니페스트를 읽지 못해 새로 쓴다: {e}")),
            }
        }
        if let Some(dir) = manifest_path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(e) = write_json(manifest_path, &manifest) {
            log(&format!("ERROR 매니페스트를 쓰지 못했다: {e}"));
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

pub fn undo_moves(manifest_path: &Path, dry_run: bool) -> io::Result<UndoResult> {
    undo_moves_with(manifest_path, dry_run, &move_file)
}

/// 매니페스트에 적힌 이동을 되돌린다.
pub fn undo_moves_with(manifest_path: &Path, dry_run: bool, mover: Mover) -> io::Result<UndoResult> {
    if !manifest_path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("매니페스트가 없다: {}", manifest_path.display())));
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
        if dry_run {
            restored.push(mv.clone());
            continue;
        }
        let back = fs::create_dir_all(src.parent().unwrap_or(Path::new(""))).and_then(|()| mover(&dst, &src));
        match back {
            Ok(()) => restored.push(mv.clone()),
            Err(e) => failed.push(with_error(mv, e.to_string())),
        }
    }

    if !restored.is_empty() && !dry_run {
        let remaining: Vec<Value> = moved.iter().filter(|m| !restored.contains(m)).cloned().map(Value::Object).collect();
        manifest["moved"] = Value::Array(remaining);
        write_json(manifest_path, &manifest)?;
    }
    Ok(UndoResult { restored, failed })
}

pub fn summarize(moves: &[Rec], out: &mut dyn Write) -> io::Result<()> {
    // Counter.most_common처럼 건수 내림차순, 동률은 먼저 나온 순서.
    let mut by_cat: IndexMap<String, usize> = IndexMap::new();
    let mut total_size = 0.0;
    for mv in moves {
        let dst = PathBuf::from(mv.get("dst").and_then(Value::as_str).unwrap_or(""));
        let parts: Vec<String> = dst.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let cat = parts.iter().position(|p| p == BROKEN_DIR_NAME).and_then(|i| parts.get(i + 1)).cloned().unwrap_or_else(|| "(최상위)".to_owned());
        *by_cat.entry(cat).or_default() += 1;
        total_size += mv.get("size").and_then(Value::as_f64).unwrap_or(0.0);
    }
    writeln!(out, "\n대상 {}건, {:.1}MB", thousands(moves.len()), total_size / 1024.0 / 1024.0)?;
    writeln!(out, "  {:<26}{:>6}", "원래 카테고리", "건수")?;
    let mut rows: Vec<(&String, &usize)> = by_cat.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    for (cat, n) in rows {
        writeln!(out, "  {cat:<26}{n:>6}")?;
    }
    Ok(())
}
