//! 실행 파일 이중 확인. 확장자가 실행 파일이고 매직바이트도 실행 파일(ELF·Mach-O·PE)이어야 "확정"이다.
//! 확장자만 맞고 내용이 다르면 확정하지 않고 낮은 심각도의 결함으로 남긴다.

use scan_common::sniff::Sniffed;
use scan_common::{Category, FileReport, Severity};

/// 실행 파일로 보는 확장자. 디렉토리를 훑을 때 이 확장자도 모은다.
pub const EXEC_EXTENSIONS: &[&str] = &["exe", "dll", "sys", "scr", "ocx", "cpl", "drv", "efi", "so", "elf", "ko", "dylib", "bundle", "bin", "out"];

/// 실행 파일 확장자인가(대소문자 무시).
pub fn is_exec_extension(ext: &str) -> bool {
    EXEC_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

fn kind(sniffed: Sniffed) -> Option<&'static str> {
    match sniffed {
        Sniffed::Elf => Some("ELF"),
        Sniffed::MachO => Some("Mach-O"),
        Sniffed::Pe => Some("PE"),
        _ => None,
    }
}

/// 실행 파일 확장자인 파일을 검사한다. `sniffed`는 `common::inspect_with_sniff`가 정한 매직바이트 판별이다.
pub fn scan_exe_file(sniffed: Sniffed, report: &mut FileReport) {
    match kind(sniffed) {
        Some(k) => {
            report.confirmed_executable = true;
            report.push(Category::Security, Severity::High, "EXECUTABLE_FILE", "", &format!("실행 파일 확정: {k}"));
        }
        None => report.push(Category::Defect, Severity::Low, "EXECUTABLE_EXTENSION_NO_MAGIC", "", &format!("확장자는 실행 파일인데 매직바이트가 실행 파일이 아니다({sniffed:?})")),
    }
}
