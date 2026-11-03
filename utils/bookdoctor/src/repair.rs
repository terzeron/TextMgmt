//! `repair`: 뷰어가 열지 못하는 결함을 고친다.
//!
//! 1. EPUB 본문 XHTML(`xhtml_fix`).
//! 2. UTF-16LE로 읽혀 UTF-8로 저장된 바이너리(HWP·PDF·PNG가 `.txt`로 남은 것)를 원래 바이트와 확장자로 되돌린다.
//!    옛 `quarantine_broken fix-ext`를 옮겨 왔다. 원본은 휴지통에 두고, 같은 inode에 덮어쓴 뒤 이름만 바꾼다(book_id가 inode다).

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::display::thousands;

use crate::cli::Options;
use crate::sanitize::apply;

pub mod xhtml_fix;

// 되돌리면 알려진 바이너리가 되는 파일의 매직 바이트.
// 바이너리를 UTF-16LE로 읽고 UTF-8로 저장하면 이런 상태가 된다.
//   OLE 매직 D0 CF 11 E0 A1 B1 1A E1
//   -> UTF-16LE로 읽으면 U+CFD0 U+E011 U+B1A1 U+E11A
//   -> UTF-8로 저장하면 ec bf 90 ee 80 91 eb 86 a1 ee 84 9a
// 이 변환은 문자가 전부 BMP 안에 있으면 무손실이라 원본을 그대로 복원할 수 있다.
const BINARY_MAGIC: [(&[u8], &str); 4] = [(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1], ".hwp"), (b"HWP Document File", ".hwp"), (b"%PDF", ".pdf"), (b"\x89PNG\r\n\x1a\n", ".png")];

/// 바이너리 여부를 가리려고 먼저 읽는 앞부분 크기. 가장 긴 매직(17바이트)이 UTF-8로 늘어나도 들어간다.
const HEAD_BYTES: usize = 64;

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
    Restored { path: String, restored_to: String, before: usize, after: usize, trash: String },
    /// 무손실로 되돌릴 수 없어 손대지 않았다.
    Skipped { path: String, reason: String },
    Error { path: String, error: String },
}

pub fn restore_binary(path: &Path, trash_dir: &Path) -> Option<FixResult> {
    restore_binary_with(path, trash_dir, &unmangle_utf16)
}

/// 확장자가 틀린 바이너리를 원래 바이트로 되돌리고 올바른 확장자로 바꾼다. 대상이 아니면 None.
///
/// 무손실 왕복을 확인한 뒤에만 쓴다. 확인에 실패하면 손대지 않는다.
pub fn restore_binary_with(path: &Path, trash_dir: &Path, unmangle: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Option<FixResult> {
    let shown = path.to_string_lossy().into_owned();
    let error = |e: String| Some(FixResult::Error { path: shown.clone(), error: e });
    let mut head = Vec::with_capacity(HEAD_BYTES);
    if let Err(e) = File::open(path).and_then(|f| f.take(HEAD_BYTES as u64).read_to_end(&mut head)) {
        return error(e.to_string());
    }
    // 앞부분만으로 걸러 낸다. 대부분의 TXT는 여기서 끝난다.
    true_extension_with(&head, unmangle)?;
    let raw = match fs::read(path) {
        Ok(r) => r,
        Err(e) => return error(e.to_string()),
    };
    let ext = true_extension_with(&raw, unmangle)?;
    if path.extension().is_some_and(|e| format!(".{}", e.to_string_lossy().to_lowercase()) == ext) {
        return None;
    }

    let restored = unmangle(&raw)?;
    let roundtrip = restored.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>();
    let lossless = restored.len() % 2 == 0 && String::from_utf16(&roundtrip).is_ok_and(|s| s.into_bytes() == raw);
    if !lossless {
        return Some(FixResult::Skipped { path: shown, reason: "무손실 왕복 실패".into() });
    }

    let bare = ext.trim_start_matches('.');
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut target: PathBuf = path.with_extension(bare);
    let mut n = 1;
    while target.exists() {
        target = path.with_file_name(format!("{stem}__{n}{ext}"));
        n += 1;
    }
    // 원본을 휴지통에 두고 같은 inode에 덮어쓴 뒤 이름만 바꾼다. rename은 inode와 mtime을 바꾸지 않는다.
    let saved = match apply(path, &restored, trash_dir).and_then(|saved| fs::rename(path, &target).map(|()| saved)) {
        Ok(saved) => saved,
        Err(e) => return error(e.to_string()),
    };
    Some(FixResult::Restored { path: shown, restored_to: target.to_string_lossy().into_owned(), before: raw.len(), after: restored.len(), trash: saved.to_string_lossy().into_owned() })
}

fn show(text: &str) -> String {
    crate::display::sanitize_display(text, usize::MAX, None)
}

fn name_of(path: &str) -> String {
    Path::new(path).file_name().map(|n| crate::display::sanitize_display(&n.to_string_lossy(), 60, None)).unwrap_or_default()
}

/// TXT 파일에서 잘못 저장된 바이너리를 찾아 되돌린다. 쓰기 실패가 있으면 1이다.
fn restore_binaries(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let paths = crate::cli::collect(opts, |p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt")));
    let trash = opts.trash_dir();
    let (mut restored, mut skipped, mut failed) = (0usize, 0usize, 0usize);
    for path in &paths {
        match restore_binary(path, &trash) {
            None => {}
            Some(FixResult::Restored { path, restored_to, before, after, trash }) => {
                restored += 1;
                writeln!(out, "RESTORED {}: -> {} ({} -> {} 바이트)", show(&path), name_of(&restored_to), thousands(before), thousands(after))?;
                writeln!(out, "  - 원본 보관 {}", show(&trash))?;
            }
            Some(FixResult::Skipped { path, reason }) => {
                skipped += 1;
                writeln!(out, "SKIP {}: {}", show(&path), show(&reason))?;
            }
            Some(FixResult::Error { path, error }) => {
                failed += 1;
                writeln!(out, "FAIL {}: {}", show(&path), show(&error))?;
            }
        }
    }
    writeln!(out, "TXT 검사 {}건, 바이너리 복원 {restored}건, 건너뜀 {skipped}건, 실패 {failed}건", paths.len())?;
    Ok(i32::from(failed > 0))
}

/// `repair` 진입점. EPUB 본문 XHTML을 고친 뒤 TXT로 남은 바이너리를 되돌린다.
pub fn run(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let epub = xhtml_fix::run(opts, out)?;
    let binaries = restore_binaries(opts, out)?;
    Ok(epub.max(binaries))
}
