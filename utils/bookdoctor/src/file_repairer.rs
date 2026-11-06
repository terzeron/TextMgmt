//! `repair`: 뷰어가 열지 못하는 결함을 고친다.
//!
//! 1. EPUB 본문 XHTML(`doc_repairer::defect_repairer`의 재구성·검증).
//! 2. UTF-16LE로 읽혀 UTF-8로 저장된 바이너리(HWP·PDF·PNG가 `.txt`로 남은 것)를 원래 바이트와 확장자로 되돌린다.
//!    원본은 휴지통에 두고, 같은 inode에 덮어쓴 뒤 이름만 바꾼다(book_id가 inode다).
//!
//! 파일 순회와 적용은 여기서 하고, 판별·재구성 기법은 `doc_repairer::defect_repairer`에 있다.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use common::text_sanitizer::sanitize_display;
use doc_checker::format_dispatcher::scan_file;
use doc_repairer::defect_repairer::{self, Verdict};
use doc_repairer::defect_repairer::{true_extension_with, unmangle_utf16};

use crate::FileReport;
use crate::file_rewriter::{apply, apply_and_report, has_serious_security, rescan_bytes};
use crate::option_model::Options;
use crate::output_formatter::{show, thousands};
use doc_checker::CheckOptions;

/// 바이너리 여부를 가리려고 먼저 읽는 앞부분 크기. 가장 긴 매직(17바이트)이 UTF-8로 늘어나도 들어간다.
const HEAD_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum FixResult {
    Restored {
        path: String,
        restored_to: String,
        before: usize,
        after: usize,
        trash: String,
    },
    /// 무손실로 되돌릴 수 없어 손대지 않았다.
    Skipped {
        path: String,
        reason: String,
    },
    Error {
        path: String,
        error: String,
    },
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

fn name_of(path: &str) -> String {
    Path::new(path).file_name().map(|n| sanitize_display(&n.to_string_lossy(), 60, None)).unwrap_or_default()
}

/// TXT 파일에서 잘못 저장된 바이너리를 찾아 되돌린다. 쓰기 실패가 있으면 1이다.
fn restore_binaries(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let paths = crate::scan_runner::collect(opts, |p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt")));
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

fn has_defect(report: &FileReport) -> bool {
    report.findings.iter().any(|f| matches!(f.code.to_string().as_str(), "XHTML_PARSE_FAILED" | "HEAD_MISSING"))
}

/// 고친 결과를 임시 파일에 써서 다시 검사한다. 본문 결함이 남거나 보안 지표(HIGH 이상)가 있으면 오류다.
/// 수리는 마크업을 읽는 방식을 바꾼다. 닫히지 않은 따옴표에 가려져 있던 `<script>` 같은 것이 살아날 수 있으므로 결과를 다시 검사한다.
fn rescan_xhtml(bytes: &[u8], opts: &CheckOptions) -> Result<(), String> {
    let report = rescan_bytes(bytes, "epub", opts)?;
    if has_serious_security(&report) {
        return Err("수리한 결과에 보안 지표(HIGH 이상)가 있다. 먼저 --sanitize로 정리한다".to_owned());
    }
    if has_defect(&report) { Err("재검사에서 본문 결함이 남았다".to_owned()) } else { Ok(()) }
}

/// `--fix-xhtml` 진입점. 종료 코드를 돌려준다(건너뜀·삭제 후보가 있어도 0, 쓰기 실패가 있으면 1).
pub fn fix_xhtml(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let paths = crate::scan_runner::collect(opts, |p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("epub")));
    let scan_opts = opts.check_options();
    let (mut done, mut untouched, mut candidates, mut skipped, mut failed) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for path in &paths {
        let shown = show(&path.to_string_lossy());
        let report = scan_file(path, &scan_opts);
        if !has_defect(&report) {
            untouched += 1;
            continue;
        }
        if has_serious_security(&report) {
            writeln!(out, "SKIP {shown}: 보안 지표(HIGH 이상)가 있다. 먼저 --sanitize로 정리한다")?;
            skipped += 1;
            continue;
        }
        let planned = fs::read(path).map_err(|e| e.to_string()).and_then(|src| defect_repairer::rebuild(&src).map(|verdict| (src, verdict)));
        let (src, plan) = match planned {
            Ok((_, Verdict::Delete(why))) => {
                writeln!(out, "DELETE_CANDIDATE {shown}: {}", show(&why))?;
                candidates += 1;
                continue;
            }
            Ok((src, Verdict::Fix(plan))) => (src, plan),
            Err(e) => {
                writeln!(out, "SKIP {shown}: {}", show(&e))?;
                skipped += 1;
                continue;
            }
        };
        if plan.edited.is_empty() && plan.dropped.is_empty() {
            writeln!(out, "UNCHANGED {shown}: 고칠 멤버를 찾지 못했다")?;
            untouched += 1;
            continue;
        }
        if let Err(e) = defect_repairer::verify(&src, &plan.bytes).and_then(|()| rescan_xhtml(&plan.bytes, &scan_opts)) {
            writeln!(out, "SKIP {shown}: 검증 실패: {}", show(&e))?;
            skipped += 1;
            continue;
        }
        writeln!(out, "FIXED {shown}: 멤버 {}개 수정, {}개 제거", plan.edited.len(), plan.dropped.len())?;
        for (name, what) in &plan.edited {
            writeln!(out, "  - 수정 {} ({})", show(name), show(what))?;
        }
        for (name, why) in &plan.dropped {
            writeln!(out, "  - 제거 {} ({})", show(name), show(why))?;
        }
        if !apply_and_report(path, &plan.bytes, opts, &shown, out)? {
            failed += 1;
            continue;
        }
        done += 1;
    }
    writeln!(out, "검사 {}건, 수리 {done}건, 대상 아님·변경 없음 {untouched}건, 삭제 후보 {candidates}건, 건너뜀 {skipped}건, 실패 {failed}건", paths.len())?;
    Ok(i32::from(failed > 0))
}

/// `repair` 진입점. EPUB 본문 XHTML을 고친 뒤 TXT로 남은 바이너리를 되돌린다.
pub fn run(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let epub = fix_xhtml(opts, out)?;
    let binaries = restore_binaries(opts, out)?;
    Ok(epub.max(binaries))
}
