//! 포맷과 무관한 공통 파일 검사. 포맷 불일치 지표는 여기서만 낸다.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use scan_common::names::windows_reserved;
use scan_common::sniff::{Sniffed, sniff};
use scan_common::{Category, FileReport, Severity};

use crate::cli::Options;

/// 앞부분으로 돌려주는 바이트 수.
const HEAD_BYTES: u64 = 64 * 1024;
/// 희소 파일과 끝 0 검사의 기준.
const SPARSE_MIN_LEN: u64 = 1024 * 1024;
const TAIL_BYTES: u64 = 4096;
const NAME_MAX_BYTES: usize = 255;

/// `sniff`와 같되 ZIP 매직이 맨 앞에 있으면 ZIP으로 본다. `sniff`는 앞 1KiB에 `%PDF-`가 있으면 먼저 PDF로 판정하므로,
/// PDF를 멤버로 담은 EPUB이 PDF로 오판되는 것을 막는다.
pub fn sniff_head(head: &[u8]) -> Sniffed {
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Sniffed::Zip;
    }
    sniff(head)
}

/// 머리(`head`)만으로 `sniff_head`가 정하지 못한 PE를 파일에서 확인한다. e_lfanew가 머리 밖에 있어도 유효한 PE일 수 있으므로,
/// `MZ`로 시작하고 e_lfanew가 머리 끝 이후·파일 안이면 그 자리의 4바이트가 `PE\0\0`인지 읽는다.
fn sniff_file(head: &[u8], file: &mut File, len: u64) -> std::io::Result<Sniffed> {
    let sniffed = sniff_head(head);
    // `%PDF-`가 머리에 있어 Pdf로 판정된 경우에도 먼 PE 헤더를 확인한다(PE가 DOS stub에 `%PDF-`를 넣어 위장할 수 있다).
    if !matches!(sniffed, Sniffed::Unknown | Sniffed::Pdf) || !head.starts_with(b"MZ") || head.len() < 0x40 {
        return Ok(sniffed);
    }
    let off = u64::from(u32::from_le_bytes([head[0x3C], head[0x3D], head[0x3E], head[0x3F]]));
    if off < head.len() as u64 || off.checked_add(4).is_none_or(|end| end > len) {
        return Ok(sniffed);
    }
    file.seek(SeekFrom::Start(off))?;
    let mut sig = [0u8; 4];
    file.read_exact(&mut sig)?;
    Ok(if &sig == b"PE\0\0" { Sniffed::Pe } else { sniffed })
}

fn is_exe(s: Sniffed) -> bool {
    matches!(s, Sniffed::Elf | Sniffed::MachO | Sniffed::Pe)
}

fn check_name(path: &Path, report: &mut FileReport) {
    let Some(name) = path.file_name() else { return };
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(name).to_vec();
    #[cfg(not(unix))]
    let bytes = name.to_string_lossy().into_owned().into_bytes();
    if bytes.iter().any(|b| b.is_ascii_control()) {
        report.push(Category::Defect, Severity::Medium, "FILENAME_CONTROL_CHAR", "", "파일명에 제어문자가 있다");
    } else if name.to_string_lossy().chars().any(scan_common::text::is_hidden_format_char) {
        // 방향 지정·폭 없는 문자는 이름을 다르게 보이게 한다(예: `gpj.exe`를 뒤집어 `exe.jpg`처럼).
        report.push(Category::Defect, Severity::Medium, "FILENAME_CONTROL_CHAR", "", "파일명에 방향·서식 문자가 있다");
    }
    // 이름 구성요소 중 파일명(마지막)만 본다.
    if bytes.len() > NAME_MAX_BYTES {
        report.push(Category::Defect, Severity::Low, "FILENAME_TOO_LONG", "", &format!("파일명이 {}바이트다 (최대 {NAME_MAX_BYTES})", bytes.len()));
    }
    if windows_reserved(&name.to_string_lossy()) {
        report.push(Category::Defect, Severity::Low, "FILENAME_RESERVED_NAME", "", "Windows 예약 장치 이름이다");
    }
}

/// 마지막 `TAIL_BYTES`바이트가 전부 0인가. 파일 전체를 읽지 않고 끝으로 건너뛴다.
fn tail_is_zero(file: &mut File, len: u64) -> std::io::Result<bool> {
    file.seek(SeekFrom::Start(len - TAIL_BYTES))?;
    let mut buf = [0u8; TAIL_BYTES as usize];
    file.read_exact(&mut buf)?;
    Ok(buf.iter().all(|&b| b == 0))
}

#[cfg(unix)]
fn is_sparse(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.len() >= SPARSE_MIN_LEN && meta.blocks() * 512 * 100 < meta.len() * 99
}

#[cfg(not(unix))]
fn is_sparse(_meta: &fs::Metadata) -> bool {
    false
}

/// 공통 검사를 하고 앞 64KiB를 돌려준다. 이후 검사를 계속할 수 없으면 `None`이다.
pub fn inspect(path: &Path, opts: &Options, report: &mut FileReport) -> Option<Vec<u8>> {
    inspect_with_sniff(path, opts, report).map(|(head, _)| head)
}

/// `inspect`와 같되 매직바이트 판별 결과도 함께 돌려준다. 머리 밖에 있는 PE 헤더까지 확인한 값이다.
pub fn inspect_with_sniff(path: &Path, opts: &Options, report: &mut FileReport) -> Option<(Vec<u8>, Sniffed)> {
    check_name(path, report);
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            report.mark_incomplete(&format!("unreadable: {e}"));
            return None;
        }
    };
    // FIFO·소켓·장치는 열기만 해도 멈출 수 있으므로 열지 않는다.
    if !meta.is_file() {
        report.mark_incomplete("unreadable: not a regular file");
        return None;
    }
    let len = meta.len();
    if len == 0 {
        report.push(Category::Defect, Severity::Medium, "EMPTY_FILE", "", "길이가 0이다");
        return None;
    }
    // 실행 파일 확장자는 앞 64KiB와 PE 헤더 4바이트만 읽는다. 파서용 크기 한도와 잘린 다운로드 검사(희소 파일, 끝 0)는 걸지 않는다.
    // 펌웨어·이미지처럼 끝이 0으로 찬 정상 데이터가 오탐이 되고, 큰 설치 파일이 한도로 확정을 피하는 것을 막는다.
    let exec_ext = path.extension().is_some_and(|e| crate::exe::is_exec_extension(&e.to_string_lossy()));
    if !exec_ext && len > opts.max_total_mb.saturating_mul(1024 * 1024) {
        report.mark_incomplete(&format!("skipped: {len} bytes exceeds --max-total-mb"));
        return None;
    }
    if !exec_ext && is_sparse(&meta) {
        report.push(Category::Defect, Severity::High, "SPARSE_FILE", "", "할당된 블록이 길이의 99% 미만이다 (잘린 다운로드)");
    }
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            report.mark_incomplete(&format!("unreadable: {e}"));
            return None;
        }
    };
    let mut head = Vec::new();
    if let Err(e) = (&mut file).take(HEAD_BYTES).read_to_end(&mut head) {
        report.mark_incomplete(&format!("unreadable: {e}"));
        return None;
    }
    if !exec_ext && len >= TAIL_BYTES {
        match tail_is_zero(&mut file, len) {
            Ok(true) => report.push(Category::Defect, Severity::High, "ZERO_TAIL", "", "마지막 4096바이트가 전부 0이다"),
            Ok(false) => {}
            Err(e) => {
                report.mark_incomplete(&format!("unreadable: {e}"));
                return None;
            }
        }
    }
    let sniffed = match sniff_file(&head, &mut file, len) {
        Ok(s) => s,
        Err(e) => {
            report.mark_incomplete(&format!("unreadable: {e}"));
            return None;
        }
    };
    check_content_vs_name(path, sniffed, report);
    Some((head, sniffed))
}

fn check_content_vs_name(path: &Path, sniffed: Sniffed, report: &mut FileReport) {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mismatch = match ext.as_str() {
        "pdf" => sniffed != Sniffed::Pdf,
        "epub" => sniffed != Sniffed::Zip,
        "txt" => matches!(sniffed, Sniffed::Pdf | Sniffed::Zip | Sniffed::Elf | Sniffed::MachO | Sniffed::Pe),
        // ZIP으로 묶인 포맷은 내용이 ZIP이어야 한다. OLE(구형 DOC·HWP, 암호화 OOXML)도 이름과 다른 내용이다.
        "zip" | "docx" | "docm" | "dotx" | "dotm" | "hwpx" => sniffed != Sniffed::Zip,
        // 구형 Word 문서는 OLE 복합 파일이어야 한다. RTF나 ZIP을 이름만 바꾼 것은 불일치다.
        "doc" | "dot" => sniffed != Sniffed::Ole,
        "rtf" => sniffed != Sniffed::Rtf,
        "hwp" => !matches!(sniffed, Sniffed::Ole | Sniffed::Hwp3),
        _ => false,
    };
    if mismatch {
        report.push(Category::Defect, Severity::Medium, "FORMAT_MISMATCH", "", &format!("확장자 .{ext}와 내용({sniffed:?})이 다르다"));
    }
    if is_exe(sniffed) && matches!(ext.as_str(), "epub" | "pdf" | "txt" | "zip" | "docx" | "docm" | "dotx" | "dotm" | "hwpx" | "doc" | "dot" | "rtf" | "hwp") {
        report.push(Category::Security, Severity::Critical, "DISGUISED_EXECUTABLE", "", &format!("확장자 .{ext}인데 실행 파일({sniffed:?})이다"));
    }
}
