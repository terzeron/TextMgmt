//! OLE 복합 파일 공용 검사: 한도 안에서 열기와 구조 이상 보고. DOC와 HWP 검사기가 함께 쓴다.

use common::text_sanitizer::sanitize_display;
use container_parser::ole_parser::Anomaly;
use container_parser::ole_parser::Limits as OleLimits;
use container_parser::ole_parser::OleError;
use container_parser::ole_parser::OleFile;
use std::path::Path;

use crate::{Category, CheckOptions, FileReport, Severity};

/// 한 파일에서 보고하는 구조 이상 수의 상한.
const MAX_ANOMALY_FINDINGS: usize = 50;

/// OLE 파일을 한도 안에서 연다. 열지 못하면 사유를 미완료로 남기고 `None`을 돌려준다.
pub(crate) fn open_ole(path: &Path, opts: &CheckOptions, report: &mut FileReport) -> Option<OleFile> {
    let limits = OleLimits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..OleLimits::default() };
    match OleFile::open(path, &limits) {
        Ok(file) => Some(file),
        Err(OleError::Limit(message)) => {
            report.mark_incomplete(&format!("skipped: {message}"));
            None
        }
        Err(e) => {
            report.mark_incomplete(&format!("OLE container could not be read: {e:?}"));
            None
        }
    }
}

fn severity_of(a: &Anomaly) -> Severity {
    match a {
        Anomaly::FatChainCycle { .. }
        | Anomaly::SectorOutOfRange { .. }
        | Anomaly::FatSectorCountMismatch { .. }
        | Anomaly::DifatTruncated
        | Anomaly::DirectoryTruncated
        | Anomaly::DirectoryCycle
        | Anomaly::TooManyEntries(_)
        | Anomaly::StreamSizeExceedsFile { .. }
        | Anomaly::CrossLinkedSectors { .. } => Severity::High,
        _ => Severity::Medium,
    }
}

/// 파서를 속여 무한 루프·큰 할당·범위 밖 읽기를 유도하는 데 쓰는 이상인가.
fn abusable(a: &Anomaly) -> bool {
    matches!(
        a,
        Anomaly::FatChainCycle { .. }
            | Anomaly::SectorOutOfRange { .. }
            | Anomaly::DirectoryCycle
            | Anomaly::TooManyEntries(_)
            | Anomaly::StreamSizeExceedsFile { .. }
            | Anomaly::CrossLinkedSectors { .. }
    )
}

pub(crate) fn report_anomalies(file: &OleFile, report: &mut FileReport) {
    let anomalies = file.anomalies();
    for a in anomalies.iter().take(MAX_ANOMALY_FINDINGS) {
        report.push(Category::Defect, severity_of(a), a.code(), "", &sanitize_display(&a.detail(), 200, Some('…')));
    }
    if anomalies.len() > MAX_ANOMALY_FINDINGS {
        report.mark_incomplete(&format!("{} structure anomalies; only the first {MAX_ANOMALY_FINDINGS} are listed", anomalies.len()));
    }
    if anomalies.iter().any(abusable) {
        report.push_once(
            Category::Security,
            Severity::High,
            "OLE_MALFORMED_STRUCTURE",
            "",
            "OLE structure is malformed in ways that trigger parser bugs (loops, out-of-range sectors, shared sectors, size lies)",
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const SIGNATURE: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

    fn file_with(dir: &tempfile::TempDir, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join("a.doc");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn header_only() -> Vec<u8> {
        let mut bytes = vec![0u8; 512];
        bytes[..8].copy_from_slice(&SIGNATURE);
        bytes
    }

    /// 헤더, FAT 섹터, 루트 항목 하나뿐인 디렉터리 섹터로 이뤄진 가장 작은 정상 CFB.
    fn minimal_cfb() -> Vec<u8> {
        let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let mut b = vec![0u8; 512 * 3];
        b[..8].copy_from_slice(&SIGNATURE);
        b[26..28].copy_from_slice(&3u16.to_le_bytes());
        b[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
        b[30..32].copy_from_slice(&9u16.to_le_bytes());
        b[32..34].copy_from_slice(&6u16.to_le_bytes());
        put32(&mut b, 44, 1); // FAT 섹터 수
        put32(&mut b, 48, 1); // 첫 디렉터리 섹터
        put32(&mut b, 56, 4096);
        put32(&mut b, 60, 0xFFFF_FFFE); // 미니 FAT 없음
        put32(&mut b, 68, 0xFFFF_FFFE);
        for i in 0..109 {
            put32(&mut b, 76 + i * 4, if i == 0 { 0 } else { 0xFFFF_FFFF });
        }
        // 섹터 0: FAT
        for i in 0..128 {
            put32(&mut b, 512 + i * 4, 0xFFFF_FFFF);
        }
        put32(&mut b, 512, 0xFFFF_FFFD); // FAT 섹터 자신
        put32(&mut b, 512 + 4, 0xFFFF_FFFE); // 디렉터리 섹터 하나
        // 섹터 1: 디렉터리, 루트 항목
        let at = 1024;
        for (i, unit) in "Root Entry".encode_utf16().enumerate() {
            b[at + i * 2..at + i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        b[at + 64..at + 66].copy_from_slice(&22u16.to_le_bytes());
        b[at + 66] = 5;
        for off in [68, 72, 76] {
            put32(&mut b, at + off, 0xFFFF_FFFF);
        }
        put32(&mut b, at + 116, 0xFFFF_FFFE);
        b
    }

    fn open(path: &Path, opts: &CheckOptions) -> (Option<OleFile>, FileReport) {
        let mut report = FileReport::new(path, None);
        (open_ole(path, opts, &mut report), report)
    }

    #[test]
    fn a_missing_file_is_noted_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let (file, report) = open(&dir.path().join("none.doc"), &CheckOptions::default());
        assert!(file.is_none());
        assert!(report.incomplete);
        assert!(report.reasons.iter().any(|r| r.starts_with("OLE container could not be read")), "{:?}", report.reasons);
    }

    #[test]
    fn a_non_ole_file_and_a_truncated_header_are_noted_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        for bytes in [b"not an ole file at all".to_vec(), SIGNATURE.to_vec()] {
            let (file, report) = open(&file_with(&dir, &bytes), &CheckOptions::default());
            assert!(file.is_none());
            assert!(report.reasons.iter().any(|r| r.starts_with("OLE container could not be read")), "{:?}", report.reasons);
        }
    }

    #[test]
    fn a_file_over_the_size_limit_is_skipped_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let opts = CheckOptions { max_total_mb: 0, ..CheckOptions::default() };
        let (file, report) = open(&file_with(&dir, &header_only()), &opts);
        assert!(file.is_none());
        assert!(report.incomplete);
        assert!(report.reasons.iter().any(|r| r.starts_with("skipped: file is 512 bytes")), "{:?}", report.reasons);
    }

    /// 디렉터리 항목이 `streams`개이고 모두 파일 전체보다 크다고 선언하는 CFB. 항목마다 구조 이상이 하나씩 나온다.
    fn cfb_with_oversized_streams(streams: usize) -> Vec<u8> {
        let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let dir_sectors = (streams + 1).div_ceil(4);
        let mut b = vec![0u8; 512 * (2 + dir_sectors)];
        b[..8].copy_from_slice(&SIGNATURE);
        b[26..28].copy_from_slice(&3u16.to_le_bytes());
        b[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
        b[30..32].copy_from_slice(&9u16.to_le_bytes());
        b[32..34].copy_from_slice(&6u16.to_le_bytes());
        put32(&mut b, 44, 1);
        put32(&mut b, 48, 1);
        put32(&mut b, 56, 4096);
        put32(&mut b, 60, 0xFFFF_FFFE);
        put32(&mut b, 68, 0xFFFF_FFFE);
        for i in 0..109 {
            put32(&mut b, 76 + i * 4, if i == 0 { 0 } else { 0xFFFF_FFFF });
        }
        for i in 0..128 {
            put32(&mut b, 512 + i * 4, 0xFFFF_FFFF);
        }
        put32(&mut b, 512, 0xFFFF_FFFD);
        for s in 1..=dir_sectors {
            put32(&mut b, 512 + s * 4, if s == dir_sectors { 0xFFFF_FFFE } else { s as u32 + 1 });
        }
        for entry in 0..=streams {
            let at = 512 * 2 + entry * 128;
            let name: Vec<u8> = format!("s{entry}").encode_utf16().flat_map(u16::to_le_bytes).collect();
            b[at..at + name.len()].copy_from_slice(&name);
            b[at + 64..at + 66].copy_from_slice(&((name.len() + 2) as u16).to_le_bytes());
            b[at + 66] = if entry == 0 { 5 } else { 2 };
            for off in [68, 72, 76] {
                put32(&mut b, at + off, 0xFFFF_FFFF);
            }
            // 루트의 자식은 항목 1이고, 스트림은 오른쪽 형제로 이어진다. 트리에 닿아야 파서가 검사한다.
            if entry == 0 {
                put32(&mut b, at + 76, 1);
            } else if entry < streams {
                put32(&mut b, at + 72, entry as u32 + 1);
            }
            put32(&mut b, at + 116, 0xFFFF_FFFE);
            if entry > 0 {
                b[at + 120..at + 128].copy_from_slice(&(1u64 << 30).to_le_bytes());
            }
        }
        b
    }

    fn report_for(bytes: &[u8]) -> FileReport {
        let dir = tempfile::tempdir().unwrap();
        let path = file_with(&dir, bytes);
        let (file, _) = open(&path, &CheckOptions::default());
        let file = file.expect("최소 CFB는 열린다");
        let mut report = FileReport::new(&path, None);
        report_anomalies(&file, &mut report);
        report
    }

    #[test]
    fn a_well_formed_file_reports_nothing() {
        let report = report_for(&minimal_cfb());
        assert!(report.findings.is_empty() && !report.incomplete, "{:?}", report.findings);
    }

    #[test]
    fn a_header_anomaly_is_a_medium_defect_without_a_location() {
        let mut bytes = minimal_cfb();
        bytes[28..30].copy_from_slice(&[0, 0]); // 바이트 순서 표지를 깬다
        let report = report_for(&bytes);
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        let f = &report.findings[0];
        assert!(f.category == Category::Defect && f.severity == Severity::Medium && f.location.is_empty());
        assert_eq!(f.code, "OLE_BAD_BYTE_ORDER");
    }

    #[test]
    fn a_parser_abuse_anomaly_adds_one_high_security_finding() {
        let mut bytes = minimal_cfb();
        bytes[512 + 4..512 + 8].copy_from_slice(&1u32.to_le_bytes()); // 디렉터리 사슬이 자기 자신으로 돌아온다
        let report = report_for(&bytes);
        let abuse: Vec<_> = report.findings.iter().filter(|f| f.code == "OLE_MALFORMED_STRUCTURE").collect();
        assert_eq!(abuse.len(), 1, "{:?}", report.findings);
        assert!(abuse[0].category == Category::Security && abuse[0].severity == Severity::High);
        assert!(report.findings.iter().any(|f| f.category == Category::Defect && f.severity == Severity::High), "{:?}", report.findings);
    }

    #[test]
    fn structural_anomalies_that_break_walkers_are_high_and_the_rest_medium() {
        assert_eq!(severity_of(&Anomaly::FatChainCycle { start: 0 }), Severity::High);
        assert_eq!(severity_of(&Anomaly::DirectoryTruncated), Severity::High);
        assert_eq!(severity_of(&Anomaly::DifatTruncated), Severity::High);
        assert_eq!(severity_of(&Anomaly::BadByteOrder), Severity::Medium);
        assert_eq!(severity_of(&Anomaly::TrailingGarbage(1)), Severity::Medium);
    }

    #[test]
    fn only_parser_abuse_anomalies_are_abusable() {
        for a in [
            Anomaly::FatChainCycle { start: 1 },
            Anomaly::SectorOutOfRange { sector: 1 },
            Anomaly::DirectoryCycle,
            Anomaly::TooManyEntries(1),
            Anomaly::StreamSizeExceedsFile { id: 1, size: 1 },
            Anomaly::CrossLinkedSectors { sector: 1 },
        ] {
            assert!(abusable(&a), "{a:?}");
        }
        for a in [Anomaly::BadByteOrder, Anomaly::DirectoryTruncated, Anomaly::DifatTruncated, Anomaly::TrailingGarbage(1), Anomaly::MiniStreamMissing] {
            assert!(!abusable(&a), "{a:?}");
        }
    }

    #[test]
    fn only_the_first_anomalies_are_listed_and_the_file_is_marked_incomplete() {
        let report = report_for(&cfb_with_oversized_streams(MAX_ANOMALY_FINDINGS + 10));
        let defects = report.findings.iter().filter(|f| f.category == Category::Defect).count();
        assert_eq!(defects, MAX_ANOMALY_FINDINGS);
        assert!(report.incomplete && report.reasons.iter().any(|r| r.contains("structure anomalies; only the first 50 are listed")), "{:?}", report.reasons);
    }
}
