//! EPUB 검사. 이후 task가 하위 모듈을 더한다.

pub mod render;
pub mod security;
pub mod structure;

use std::collections::HashSet;
use std::path::Path;

use scan_common::FileReport;

use crate::cli::Options;

/// EPUB 파일 하나를 검사해 `report`에 더한다.
pub fn scan_epub_file(path: &Path, opts: &Options, report: &mut FileReport) {
    let limits = security::Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..security::Limits::default() };
    // 중앙 디렉터리가 한도를 넘으면 어느 단계도 ZIP을 열지 않는다. 구조·렌더링·손상 검사는 한도를 따로 보지 않는다.
    if security::directory_limit_note(path).is_some() {
        security::scan(path, &limits, report);
        return;
    }
    structure::inspect(path, report);
    let structure_len = report.findings.len();
    let structure_codes: HashSet<String> = report.findings.iter().map(|f| f.code.clone()).collect();
    security::scan(path, &limits, report);
    // 같은 코드(옛 BAD_MIMETYPE처럼 두 검사에 모두 있는 것)는 구조 검사 쪽 하나만 남긴다.
    let mut index = 0;
    report.findings.retain(|f| {
        index += 1;
        index <= structure_len || !structure_codes.contains(&f.code)
    });
    // ZIP으로 열렸을 때만 렌더링·손상 검사를 한다. NOT_ZIP이면 읽을 멤버가 없다.
    if !report.has("NOT_ZIP") {
        render::inspect_with(path, report, &limits);
        // 옛 인코딩 검사 도구와 같은 표본(`read_epub_text`, 확장자와 무관)으로 인코딩 손상을 판정한다.
        // 보안 검사가 압축 폭탄이나 예산 소진을 이미 보고했다면 같은 멤버를 다시 풀지 않는다.
        if bomb_reported(report) {
            report.mark_incomplete("mojibake sampling skipped: decompression bomb or budget exhaustion already reported");
        } else {
            crate::mojibake::check_file_limited(path, report, crate::mojibake::TextSource::Epub, limits.max_total, crate::mojibake::SAMPLE_MAX_MEMBERS);
        }
    }
}

/// 보안 검사가 압축 폭탄(`HIGH_COMPRESSION_RATIO`)이나 압축 해제 예산 소진을 보고했는가.
fn bomb_reported(report: &FileReport) -> bool {
    report.has("HIGH_COMPRESSION_RATIO") || report.reasons.iter().any(|r| r.contains("decompression budget") || r.contains("decompressed-byte scan limit"))
}
