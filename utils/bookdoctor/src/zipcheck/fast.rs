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

use crate::epub::security::{Limits, directory_limit_note, read_limited, zip_directory_bounds};
use crate::zipnames;

/// ZIP 안에서 이름만으로 실행 파일로 보는 확장자. EPUB 목록에서 데이터로 흔한 `bin`을 빼고(내용이 실행 파일이면 매직 검사가 잡는다),
/// EPUB은 따로 다루지만 ZIP·문서 패키지에서는 흔한 전달 수단인 `js`, 디스크 이미지, 설치·도움말 형식을 더했다.
pub(crate) const ZIP_EXEC_EXTS: &[&str] = &[
    "exe", "dll", "scr", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "jse", "wsf", "wsh", "hta", "msi", "msp", "jar", "lnk", "reg", "cpl", "sh", "bash", "zsh", "py", "pl", "rb", "php", "apk", "app", "dmg", "so", "dylib", "run", "swf", "class", "js",
    "pif", "msc", "scf", "url", "iso", "img", "vhd", "vhdx", "chm", "xll", "wsc", "one", "appx", "msix",
];

/// 멤버마다 풀어 보는 앞부분의 바이트 수.
pub const PROBE_BYTES: usize = 4096;
const RATIO_MIN_SIZE: u64 = 1024 * 1024;
const RATIO_LIMIT: u64 = 200;

fn extension(name: &str) -> String {
    name.rsplit('/').next().and_then(|base| base.rsplit_once('.')).map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

/// 읽기 오류를 CRC 불일치, 선언 크기 위조, 그 밖의 해제 실패로 가른다. zip crate는 선언 크기를 넘어 읽으면 오류를 낸다.
pub(super) fn read_failure(report: &mut FileReport, shown: &str, error: &str) {
    let lower = error.to_lowercase();
    let (category, code, detail) = if lower.contains("larger than its declared") {
        (Category::Security, "ZIP_SIZE_MISMATCH", "Declared size is smaller than the data it yields")
    } else if lower.contains("checksum") {
        (Category::Defect, "ZIP_CRC_MISMATCH", error)
    } else {
        (Category::Defect, "ZIP_ENTRY_UNREADABLE", error)
    };
    // 같은 멤버의 같은 오류는 한 번만 낸다(빠른 점검과 보안 검사가 같은 부분을 읽는다).
    if !report.findings.iter().any(|f| f.code == code && f.location == shown) {
        report.push(category, Severity::High, code, shown, detail);
    }
}

/// 디코딩한 이름이 있으면 그것을, 디코딩에 실패했으면 원본 바이트의 손실 변환을 이름으로 쓴다. 둘째 값은 디코딩 실패 여부다.
pub fn resolve_name<E>(decoded: Result<String, E>, raw: &[u8]) -> (String, bool) {
    match decoded {
        Ok(name) => (name, false),
        Err(_) => (String::from_utf8_lossy(raw).into_owned(), true),
    }
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
    // zip crate는 같은 이름의 멤버를 하나로 합쳐 앞 사본을 보여 주지 않는다. 중앙 디렉터리가 알리는 항목 수와 다르면 가려진 멤버가 있다.
    if let Some((entries, _)) = zip_directory_bounds(path)
        && entries != count as u64
    {
        report.push(Category::Security, Severity::High, "ZIP_DUPLICATE_NAME", "", &format!("The directory lists {entries} entries but the reader sees {count}: duplicate names hide members"));
        report.mark_incomplete("duplicate member names: only one copy of each name could be inspected");
    }
    let mut raw = File::open(path).ok();
    let mut spent = 0u64;
    for i in 0..count.min(limits.max_files) {
        let (decoded, start, raw_name, symlink, is_dir, size, csize, encrypted, method) = match zip.by_index_raw(i) {
            Ok(f) => (f.name().map(|c| c.into_owned()), f.header_start(), f.name_raw().to_vec(), f.is_symlink(), f.is_dir(), f.size(), f.compressed_size(), f.encrypted(), f.compression()),
            Err(_) => {
                report.push(Category::Security, Severity::High, "BAD_LOCAL_HEADER", &format!("#{i}"), "ZIP local header absent or invalid");
                continue;
            }
        };
        // 추출기마다 이름을 다르게 읽을 수 있다(UTF-8, CP437, 손실 변환). 디코딩한 이름과 원본 바이트의 손실 변환 이름을 모두 검사한다.
        // 디코딩에 실패하면 자리표시자가 아니라 원본 바이트로 검사하고, 실패 자체를 지표로 남긴다.
        let lossy = String::from_utf8_lossy(&raw_name).into_owned();
        let (name, undecodable) = resolve_name(decoded, &raw_name);
        let shown = sanitize_display(&name, 200, Some('…'));
        if undecodable {
            report.push(Category::Security, Severity::High, "ZIP_INVALID_NAME", &shown, "Entry name cannot be decoded (extractors may read it differently)");
        }
        let mut seen = Vec::new();
        let mut candidates = vec![name.as_str()];
        if lossy != name {
            candidates.push(lossy.as_str());
        }
        for candidate in candidates {
            for found in zipnames::check(candidate) {
                if !seen.contains(&found.code) {
                    seen.push(found.code);
                    report.push(Category::Security, found.severity, found.code, &shown, found.detail);
                }
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
        // 무압축 멤버는 압축 크기와 원래 크기가 같아야 한다. 다르면 헤더가 거짓이다.
        if method == CompressionMethod::Stored && csize != size {
            report.push(Category::Security, Severity::High, "ZIP_SIZE_MISMATCH", &shown, &format!("Stored member declares {size} bytes but occupies {csize}"));
        }
        let mut flagged_exec = false;
        let ext = extension(&name);
        if ZIP_EXEC_EXTS.contains(&ext.as_str()) {
            flagged_exec = true;
            report.push(Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &shown, &format!("Executable/script file type: .{ext}"));
        }

        // 앞부분 시험: 풀 수 있는지와 내용 종류를 본다.
        let mut probe = Vec::with_capacity(PROBE_BYTES);
        let result = match zip.by_index(i) {
            Ok(entry) => entry.take(PROBE_BYTES as u64).read_to_end(&mut probe).map(|_| ()).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let probe_failed = result.is_err();
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
        // 앞부분 시험이 이미 실패한 멤버는 같은 오류를 다시 내지 않도록 건너뛴다.
        if deep && !probe_failed {
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
