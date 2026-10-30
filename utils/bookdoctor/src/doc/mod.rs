//! 구형 DOC(OLE 복합 파일) 검사. `ole_reader`로 구조를 읽고 손상, 암호화, VBA 매크로, 내장 OLE 객체, DDE·필드를 본다.
//! 파일을 실행하거나 렌더링하지 않는다. 매크로는 텍스트로만 읽는다.

use std::path::Path;

use ole_reader::{Anomaly, EntryKind, Limits as OleLimits, OleError, OleFile, vba};
use scan_common::sniff::{Sniffed, sniff};
use scan_common::text::sanitize_display;
use scan_common::{Category, FileReport, Severity};

use crate::cli::Options;
use crate::zipcheck::fast::ZIP_EXEC_EXTS;
use crate::zipcheck::ooxml::classify_field;

/// 자동 실행 진입점. 문서를 열거나 닫을 때 Word가 부른다.
const AUTOEXEC: &[&str] = &["autoopen", "autoexec", "autoclose", "auto_open", "document_open", "document_close", "documentopen", "workbook_open"];
/// 실행·내려받기·환경 접근을 뜻하는 키워드. 자동 실행과 함께 있으면 악성 매크로의 전형이다.
const SUSPICIOUS: &[&str] = &["shell", "createobject", "wscript.shell", "powershell", "cmd.exe", "urldownloadtofile", "xmlhttp", "adodb.stream", "environ"];
/// 본문 스트림에서 필드를 찾는 최대 바이트.
const FIELD_SCAN_MAX: usize = 16 * 1024 * 1024;
/// 필드 지시문 하나로 모으는 최대 바이트(끝나지 않는 필드가 메모리를 쓰지 못하게 한다).
const MAX_FIELD_BYTES: usize = 8192;
/// 매크로 소스를 푸는 총 바이트 상한.
const VBA_BUDGET: u64 = 16 * 1024 * 1024;
/// 내장 객체(`Ole10Native`)에서 읽는 앞부분.
const NATIVE_PREFIX: usize = 64 * 1024;
/// 한 파일에서 보고하는 구조 이상 수의 상한.
const MAX_ANOMALY_FINDINGS: usize = 50;

/// `.doc`·`.dot` 하나를 검사한다. 내용이 OLE가 아니면 열지 않는다(확장자 위장은 `common`이 알린다).
pub fn scan_doc_file(path: &Path, opts: &Options, sniffed: Sniffed, report: &mut FileReport) {
    if sniffed != Sniffed::Ole {
        return;
    }
    let limits = OleLimits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..OleLimits::default() };
    match OleFile::open(path, &limits) {
        Ok(mut file) => inspect(&mut file, report),
        Err(OleError::Limit(message)) => report.mark_incomplete(&format!("skipped: {message}")),
        Err(e) => report.mark_incomplete(&format!("OLE container could not be read: {e:?}")),
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
    matches!(a, Anomaly::FatChainCycle { .. } | Anomaly::SectorOutOfRange { .. } | Anomaly::DirectoryCycle | Anomaly::TooManyEntries(_) | Anomaly::StreamSizeExceedsFile { .. } | Anomaly::CrossLinkedSectors { .. })
}

fn push_once(report: &mut FileReport, category: Category, severity: Severity, code: &str, location: &str, detail: &str) {
    if !report.findings.iter().any(|f| f.code == code && f.location == location) {
        report.push(category, severity, code, location, detail);
    }
}

/// 열린 OLE 파일을 분석해 `report`에 더한다.
pub fn inspect(file: &mut OleFile, report: &mut FileReport) {
    report_anomalies(file, report);
    let encrypted = check_word_stream(file, report);
    check_vba(file, report);
    check_objects(file, report);
    if !encrypted {
        scan_fields(file, report);
    }
}

fn report_anomalies(file: &OleFile, report: &mut FileReport) {
    let anomalies = file.anomalies();
    for a in anomalies.iter().take(MAX_ANOMALY_FINDINGS) {
        report.push(Category::Defect, severity_of(a), a.code(), "", &sanitize_display(&a.detail(), 200, Some('…')));
    }
    if anomalies.len() > MAX_ANOMALY_FINDINGS {
        report.mark_incomplete(&format!("{} structure anomalies; only the first {MAX_ANOMALY_FINDINGS} are listed", anomalies.len()));
    }
    if anomalies.iter().any(abusable) {
        push_once(report, Category::Security, Severity::High, "OLE_MALFORMED_STRUCTURE", "", "OLE structure is malformed in ways that trigger parser bugs (loops, out-of-range sectors, shared sectors, size lies)");
    }
}

/// `WordDocument` 스트림을 확인한다. 암호화되어 있으면 `true`다(본문을 읽을 수 없다).
fn check_word_stream(file: &mut OleFile, report: &mut FileReport) -> bool {
    let Some(entry) = file.find("WordDocument").cloned() else {
        report.push(Category::Defect, Severity::Medium, "FORMAT_MISMATCH", "WordDocument", "OLE 파일이지만 Word 문서가 아니다(WordDocument 스트림이 없다)");
        return false;
    };
    let Ok(head) = file.read_stream_prefix(&entry, 32) else { return false };
    // FIB 기본부: flags1은 오프셋 10의 16비트이고 fEncrypted는 0x0100이다(MS-DOC FibBase).
    let flags1 = head.get(10..12).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
    if flags1 & 0x0100 != 0 {
        report.mark_incomplete("encrypted: Word binary document (FIB.fEncrypted)");
        return true;
    }
    false
}

fn check_vba(file: &mut OleFile, report: &mut FileReport) {
    let roots: Vec<String> = file.entries().iter().filter(|e| e.kind == EntryKind::Storage && e.name.eq_ignore_ascii_case("VBA")).map(|e| file.path_of(e)).take(4).collect();
    for root in roots {
        let shown = sanitize_display(&root, 120, Some('…'));
        push_once(report, Category::Security, Severity::High, "DOC_VBA_MACRO", &shown, "VBA macro project");
        match vba::read_project(file, &root, VBA_BUDGET) {
            Err(e) => report.mark_incomplete(&format!("VBA project {shown} could not be read: {e:?}")),
            Ok(project) => {
                if let Some(first) = project.errors.first() {
                    report.mark_incomplete(&format!("VBA project {shown} could not be fully read: {}", sanitize_display(first, 160, Some('…'))));
                }
                let mut auto: Option<(String, &str)> = None;
                let mut suspicious: Option<&str> = None;
                for m in &project.modules {
                    let lower = m.source.to_lowercase();
                    if auto.is_none()
                        && let Some(name) = AUTOEXEC.iter().find(|n| lower.contains(*n))
                    {
                        auto = Some((m.name.clone(), name));
                    }
                    if suspicious.is_none() {
                        suspicious = SUSPICIOUS.iter().find(|k| lower.contains(*k)).copied();
                    }
                }
                if let Some((module, name)) = auto {
                    let loc = sanitize_display(&format!("{root}/{module}"), 160, Some('…'));
                    push_once(report, Category::Security, Severity::High, "DOC_VBA_AUTOEXEC", &loc, &format!("auto-run entry point {name}"));
                    if let Some(keyword) = suspicious {
                        push_once(report, Category::Security, Severity::Critical, "DOC_VBA_SUSPICIOUS", &loc, &format!("auto-run macro that uses {keyword}"));
                    }
                }
            }
        }
    }
}

fn check_objects(file: &mut OleFile, report: &mut FileReport) {
    if file.entries().iter().any(|e| e.kind == EntryKind::Storage && e.name.eq_ignore_ascii_case("ObjectPool")) {
        push_once(report, Category::Security, Severity::High, "DOC_OLE_OBJECT", "ObjectPool", "Embedded OLE objects");
    }
    let natives: Vec<_> = file.entries().iter().filter(|e| e.kind == EntryKind::Stream && e.name.trim_start_matches('\u{1}').eq_ignore_ascii_case("Ole10Native")).cloned().collect();
    for entry in natives.into_iter().take(64) {
        let Ok(prefix) = file.read_stream_prefix(&entry, NATIVE_PREFIX) else { continue };
        // 내장 파일 이름: 4바이트 크기와 2바이트 플래그 뒤에 널로 끝나는 레이블이 온다.
        let label: String = prefix.get(6..).unwrap_or(&[]).iter().take_while(|&&b| b != 0).take(260).map(|&b| b as char).collect();
        let shown = sanitize_display(&label, 120, Some('…'));
        let ext = label.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        if ZIP_EXEC_EXTS.contains(&ext.as_str()) {
            push_once(report, Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &shown, &format!("Embedded file type .{ext}"));
        }
        // 이름이 무엇이든 내용이 실행 파일인 것을 찾는다(앞부분에서 `MZ`·`\x7fELF`가 나오는 자리마다 헤더를 확인한다).
        let hit = prefix.windows(2).enumerate().any(|(i, w)| (w == b"MZ" || w == b"\x7f\x45") && matches!(sniff(&prefix[i..]), Sniffed::Pe | Sniffed::Elf));
        if hit {
            push_once(report, Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &shown, "Embedded executable content (PE/ELF magic)");
        }
    }
}

/// `0x13` 필드 시작부터 `0x14`(구분)나 `0x15`(끝)까지를 지시문으로 모은다. 본문이 8비트인 문서와 UTF-16인 문서를 모두 본다.
fn scan_fields(file: &mut OleFile, report: &mut FileReport) {
    let Some(entry) = file.find("WordDocument").cloned() else { return };
    let Ok(data) = file.read_stream_prefix(&entry, FIELD_SCAN_MAX) else { return };
    let mut instructions: Vec<String> = Vec::new();
    // 8비트 본문.
    let mut cap: Option<Vec<u8>> = None;
    for &b in &data {
        match b {
            0x13 => cap = Some(Vec::new()),
            0x14 | 0x15 => {
                if let Some(c) = cap.take() {
                    instructions.push(c.iter().map(|&x| x as char).collect());
                }
            }
            _ => {
                if let Some(c) = cap.as_mut()
                    && c.len() < MAX_FIELD_BYTES
                    && (b >= 0x20 || matches!(b, b'\t' | b'\r' | b'\n'))
                {
                    c.push(b);
                }
            }
        }
    }
    // UTF-16LE 본문: 시작 위치가 짝수일 수도 홀수일 수도 있어 두 정렬을 모두 본다.
    for start in 0..2usize {
        let mut cap: Option<Vec<u16>> = None;
        let mut i = start;
        while i + 1 < data.len() {
            let u = u16::from_le_bytes([data[i], data[i + 1]]);
            match u {
                0x13 => cap = Some(Vec::new()),
                0x14 | 0x15 => {
                    if let Some(c) = cap.take() {
                        instructions.push(String::from_utf16_lossy(&c));
                    }
                }
                _ => {
                    if let Some(c) = cap.as_mut()
                        && c.len() < MAX_FIELD_BYTES
                        && (u >= 0x20 || matches!(u, 0x09 | 0x0D | 0x0A))
                    {
                        c.push(u);
                    }
                }
            }
            i += 2;
        }
    }
    for instr in instructions {
        // 분류 규칙은 OOXML 필드와 같다. 코드 이름만 DOC_로 바꿔 낸다.
        let mut tmp = FileReport::new(Path::new(""), None);
        classify_field(&instr, "WordDocument", &mut tmp);
        for f in tmp.findings {
            let code = f.code.replacen("OOXML_", "DOC_", 1);
            push_once(report, Category::Security, f.severity, &code, &f.location, &f.detail);
        }
    }
}
