//! 범용 ZIP 빠른 점검. 열 수 있는지, 멤버를 풀 수 있는지를 멤버 수에 비례하는 비용으로 확인한다.
//!
//! 기본은 멤버마다 앞 `PROBE_BYTES`바이트만 풀어 본다(전체를 풀지 않는다). `deep`이면 예산 안에서 끝까지 풀어 CRC까지 확인한다.
//! 읽기는 선언 크기를 믿지 않고 상한에서 멈춘다. 중첩 ZIP과 내장 실행 파일은 표시만 하고 재귀하지 않는다.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use corpus_common::reader::open_zip;
use scan_common::sniff::{Sniffed, sniff};
use scan_common::text::sanitize_display;
use scan_common::{Category, FileReport, Severity};
use zip::CompressionMethod;

use crate::epub::security::{EXEC_EXTS, Limits, directory_limit_note, read_limited};
use crate::zipnames;

/// 멤버마다 풀어 보는 앞부분의 바이트 수.
pub const PROBE_BYTES: usize = 4096;
const RATIO_MIN_SIZE: u64 = 1024 * 1024;
const RATIO_LIMIT: u64 = 200;

fn extension(name: &str) -> String {
    name.rsplit('/').next().and_then(|base| base.rsplit_once('.')).map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

/// 읽기 오류를 CRC 불일치와 그 밖의 해제 실패로 가른다.
fn read_failure(report: &mut FileReport, shown: &str, error: &str) {
    let code = if error.to_lowercase().contains("checksum") { "ZIP_CRC_MISMATCH" } else { "ZIP_ENTRY_UNREADABLE" };
    report.push(Category::Defect, Severity::High, code, shown, error);
}

/// ZIP 하나를 점검해 `report`에 더한다. 열기 전에 중앙 디렉터리 상한부터 본다. 열어서 멤버를 훑었으면 `true`다.
pub fn check(path: &Path, limits: &Limits, deep: bool, report: &mut FileReport) -> bool {
    if let Some(note) = directory_limit_note(path) {
        report.mark_incomplete(&note);
        return false;
    }
    let Some(mut zip) = open_zip(path) else {
        report.push(Category::Defect, Severity::High, "NOT_ZIP", "", "ZIP으로 열 수 없다");
        return false;
    };
    let count = zip.len();
    if count > limits.max_files {
        report.mark_incomplete(&format!("Too many entries: {count} (max {})", limits.max_files));
    }
    let mut raw = File::open(path).ok();
    let mut spent = 0u64;
    for i in 0..count.min(limits.max_files) {
        let (name, start, raw_name, symlink, is_dir, size, csize, encrypted, method) = match zip.by_index_raw(i) {
            Ok(f) => (f.name().map(|c| c.into_owned()).unwrap_or_else(|_| "<invalid-name>".into()), f.header_start(), f.name_raw().to_vec(), f.is_symlink(), f.is_dir(), f.size(), f.compressed_size(), f.encrypted(), f.compression()),
            Err(_) => {
                report.push(Category::Security, Severity::High, "BAD_LOCAL_HEADER", &format!("#{i}"), "ZIP local header absent or invalid");
                continue;
            }
        };
        let shown = sanitize_display(&name, 200, Some('…'));
        let mut seen = Vec::new();
        for found in zipnames::check(&name) {
            if !seen.contains(&found.code) {
                seen.push(found.code);
                report.push(Category::Security, found.severity, found.code, &shown, found.detail);
            }
        }
        if let Some(raw) = raw.as_mut()
            && let Some(issue) = zipnames::local_header_issue(raw, start, &raw_name)
        {
            report.push(Category::Security, issue.severity, issue.code, &shown, &sanitize_display(&issue.detail, 200, Some('…')));
        }
        if symlink {
            report.push(Category::Security, Severity::Medium, "ZIP_SYMLINK", &shown, "ZIP member represented as symlink");
        }
        if is_dir {
            continue;
        }
        if encrypted {
            report.mark_incomplete(&format!("{shown}: ZIP-encrypted entry cannot be reliably inspected"));
            continue;
        }
        if !matches!(method, CompressionMethod::Stored | CompressionMethod::Deflated) {
            report.mark_incomplete(&format!("{shown}: Unusual or unsupported ZIP compression method"));
            continue;
        }
        if csize > 0 && size / csize > RATIO_LIMIT && size > RATIO_MIN_SIZE {
            report.push(Category::Security, Severity::Medium, "HIGH_COMPRESSION_RATIO", &shown, "Potential decompression bomb");
        }
        let mut flagged_exec = false;
        let ext = extension(&name);
        if EXEC_EXTS.contains(&ext.as_str()) {
            flagged_exec = true;
            report.push(Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &shown, &format!("Executable/script file type: .{ext}"));
        }

        // 앞부분 시험: 풀 수 있는지와 내용 종류를 본다.
        let mut probe = Vec::with_capacity(PROBE_BYTES);
        let result = match zip.by_index(i) {
            Ok(entry) => entry.take(PROBE_BYTES as u64).read_to_end(&mut probe).map(|_| ()).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match result {
            Err(e) => read_failure(report, &shown, &e),
            Ok(()) => {
                if !flagged_exec && matches!(sniff(&probe), Sniffed::Pe | Sniffed::Elf | Sniffed::MachO) {
                    report.push(Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &shown, "Executable content (PE/ELF/Mach-O magic)");
                }
                if probe.starts_with(b"PK\x03\x04") {
                    report.push(Category::Security, Severity::Medium, "NESTED_ARCHIVE", &shown, "ZIP archive inside the archive");
                }
            }
        }

        // 깊은 점검: 예산 안에서 끝까지 풀어 CRC를 확인한다.
        if deep {
            if size > limits.max_member {
                report.mark_incomplete(&format!("{shown}: Member larger than max inspection size: {size}"));
                continue;
            }
            let bound = limits.max_member.min(limits.max_total.saturating_sub(spent));
            if bound == 0 {
                report.mark_incomplete("deep scan stopped: decompression budget exhausted");
                break;
            }
            match read_limited(&mut zip, i, bound) {
                Err(e) => read_failure(report, &shown, &e),
                Ok(data) => {
                    spent += data.len() as u64;
                    if data.len() as u64 > bound {
                        report.mark_incomplete(&format!("{shown}: Member is larger than its declared size; only the first part was inspected"));
                    }
                }
            }
        }
    }
    true
}
