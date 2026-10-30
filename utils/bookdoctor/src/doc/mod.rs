//! 구형 DOC(OLE 복합 파일) 검사. `ole_reader`로 구조를 읽고 손상, 암호화, VBA 매크로, 내장 OLE 객체, DDE·필드를 본다.
//! 파일을 실행하거나 렌더링하지 않는다. 매크로는 텍스트로만 읽는다.

pub mod rtf;

use std::io::Read;
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
/// RTF를 읽는 최대 바이트(`OleFile`의 스트림 상한과 같다).
const RTF_READ_MAX: u64 = 64 * 1024 * 1024;
/// 한 파일에서 보고하는 구조 이상 수의 상한.
const MAX_ANOMALY_FINDINGS: usize = 50;

/// `.doc`·`.dot` 하나를 검사한다. 내용이 OLE가 아니면 열지 않는다(확장자 위장은 `common`이 알린다).
pub fn scan_doc_file(path: &Path, opts: &Options, sniffed: Sniffed, report: &mut FileReport) {
    if sniffed == Sniffed::Rtf {
        scan_rtf_file(path, report);
        return;
    }
    match sniffed {
        Sniffed::Ole => {}
        // 실행 파일은 `common`이 이미 Critical로 알렸다.
        Sniffed::Elf | Sniffed::MachO | Sniffed::Pe => return,
        // 이름만 Word 문서인 텍스트·HTML 등은 여기서 분석하지 않는다. 이름 불일치는 `common`이 알린다.
        other => {
            report.mark_incomplete(&format!("content is not an OLE document ({other:?}); not analyzed"));
            return;
        }
    }
    if let Some(mut file) = open_ole(path, opts, report) {
        inspect(&mut file, report);
    }
}

/// `.docx` 계열인데 내용이 OLE인 파일을 검사한다. 암호화 OOXML 패키지거나 구형 `.doc`를 이름만 바꾼 것이다.
/// 둘 다 아니면 열 수 없는 파일로 미완료를 남긴다(확장자 불일치는 `common`이 알린다).
pub fn scan_ole_named_docx(path: &Path, opts: &Options, report: &mut FileReport) {
    let Some(mut file) = open_ole(path, opts, report) else { return };
    if file.find("EncryptedPackage").is_some() || file.find("WordDocument").is_some() {
        inspect(&mut file, report);
    } else {
        report.mark_incomplete("OLE container cannot be inspected: .docx extension with OLE content that is neither an encrypted package nor a Word document");
    }
}

/// OLE 파일을 한도 안에서 연다. 열지 못하면 사유를 미완료로 남기고 `None`을 돌려준다.
pub(crate) fn open_ole(path: &Path, opts: &Options, report: &mut FileReport) -> Option<OleFile> {
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

/// RTF 파일을 한도까지 읽어 `rtf::inspect`에 넘긴다. 한도를 넘으면 앞부분만 보고 미완료로 남긴다.
fn scan_rtf_file(path: &Path, report: &mut FileReport) {
    let Ok(file) = std::fs::File::open(path) else {
        report.mark_incomplete("RTF file could not be opened");
        return;
    };
    let mut bytes = Vec::new();
    if file.take(RTF_READ_MAX + 1).read_to_end(&mut bytes).is_err() {
        report.mark_incomplete("RTF file could not be read");
        return;
    }
    if bytes.len() as u64 > RTF_READ_MAX {
        bytes.truncate(RTF_READ_MAX as usize);
        report.mark_incomplete("skipped: RTF larger than the read limit; only the first part was inspected");
    }
    rtf::inspect(&bytes, report);
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

pub(crate) fn report_anomalies(file: &OleFile, report: &mut FileReport) {
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
        // 암호화 OOXML은 OLE 안에 `EncryptedPackage`로 들어 있다. 본문을 읽을 수 없고 Word 문서 구조도 없다.
        if file.find("EncryptedPackage").is_some() {
            report.mark_incomplete("encrypted: OOXML encrypted package (EncryptedPackage)");
            return true;
        }
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

/// `pos`부터 널로 끝나는 문자열을 읽는다(바이트를 그대로 글자로 본다). 길이를 자르지 않는다: 확장자는 이름의 끝에 있다.
/// 문자열, 널 다음 위치, 널을 찾았는지를 돌려준다.
fn c_string(bytes: &[u8], pos: usize) -> (String, usize, bool) {
    let rest = bytes.get(pos..).unwrap_or(&[]);
    let len = rest.iter().position(|&b| b == 0);
    let end = len.unwrap_or(rest.len());
    (rest[..end].iter().map(|&b| b as char).collect(), pos + end + 1, len.is_some())
}

fn check_objects(file: &mut OleFile, report: &mut FileReport) {
    if file.entries().iter().any(|e| e.kind == EntryKind::Storage && e.name.eq_ignore_ascii_case("ObjectPool")) {
        push_once(report, Category::Security, Severity::High, "DOC_OLE_OBJECT", "ObjectPool", "Embedded OLE objects");
    }
    let natives: Vec<_> = file.entries().iter().filter(|e| e.kind == EntryKind::Stream && e.name.trim_start_matches('\u{1}').eq_ignore_ascii_case("Ole10Native")).cloned().collect();
    for entry in natives.into_iter().take(64) {
        let prefix = match file.read_stream_prefix(&entry, NATIVE_PREFIX) {
            Ok(p) => p,
            Err(e) => {
                report.mark_incomplete(&format!("embedded object stream could not be read: {e:?}"));
                continue;
            }
        };
        // 내장 파일: 4바이트 크기와 2바이트 형식 뒤에 레이블, 원본 경로, (4바이트 미상, 임시 경로 길이 4바이트) 임시 경로가 온다.
        // Word는 어느 이름으로든 파일을 풀 수 있으므로 세 이름 모두 확장자를 본다.
        let (label, next, label_end) = c_string(&prefix, 6);
        let (source, next, source_end) = c_string(&prefix, next);
        let temp_len = prefix.get(next + 4..next + 8).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        let temp = prefix.get(next + 8..).map_or_else(String::new, |rest| c_string(&rest[..temp_len.min(rest.len())], 0).0);
        // 이름이 읽은 앞부분 안에서 끝나지 않으면 확장자를 볼 수 없다.
        if !label_end || !source_end {
            report.mark_incomplete("embedded object file name is longer than the scanned prefix; its extension was not checked");
        }
        let shown = sanitize_display(&label, 120, Some('…'));
        for name in [&label, &source, &temp] {
            let ext = name.trim_end_matches([' ', '.']).rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
            if ZIP_EXEC_EXTS.contains(&ext.as_str()) {
                push_once(report, Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &sanitize_display(name, 120, Some('…')), &format!("Embedded file type .{ext}"));
                break;
            }
        }
        // 이름이 무엇이든 내용이 실행 파일인 것을 찾는다(앞부분에서 `MZ`·`\x7fELF`가 나오는 자리마다 헤더를 확인한다).
        let hit = prefix.windows(2).enumerate().any(|(i, w)| (w == b"MZ" || w == b"\x7f\x45") && matches!(sniff(&prefix[i..]), Sniffed::Pe | Sniffed::Elf));
        if hit {
            push_once(report, Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &shown, "Embedded executable content (PE/ELF magic)");
        }
    }
}

/// 필드 중첩 깊이의 상한. 더 깊은 필드는 분석하지 않고 미완료로 남긴다.
const MAX_FIELD_DEPTH: usize = 64;
/// 분류하는 지시문 수의 상한.
const MAX_INSTRUCTIONS: usize = 20_000;

/// 필드 문자(0x13 시작, 0x14 구분, 0x15 끝)를 따라 지시문을 모아 `emit`에 넘긴다. 지시문은 시작과 구분 사이 글자다.
/// 안쪽 필드가 지시문 중간에 끼어도 바깥 지시문을 이어 붙이도록 스택으로 중첩을 추적한다.
/// 상한에 걸리면 `true`를 돌려준다.
fn scan_units(units: impl Iterator<Item = u16>, emit: &mut impl FnMut(&str)) -> bool {
    // 각 칸은 지시문 글자와 "이미 내보냈는가(결과 부분인가)"다.
    let mut stack: Vec<(String, bool)> = Vec::new();
    let mut overflow = 0usize;
    let mut emitted = 0usize;
    let mut limited = false;
    let mut send = |text: &str, limited: &mut bool| {
        if text.trim().is_empty() {
            return;
        }
        if emitted >= MAX_INSTRUCTIONS {
            *limited = true;
            return;
        }
        emitted += 1;
        emit(text);
    };
    for u in units {
        match u {
            0x13 => {
                if stack.len() < MAX_FIELD_DEPTH {
                    stack.push((String::new(), false));
                } else {
                    overflow += 1;
                    limited = true;
                }
            }
            0x14 if overflow == 0 => {
                if let Some((text, done)) = stack.last_mut()
                    && !*done
                {
                    *done = true;
                    send(text, &mut limited);
                }
            }
            0x15 => {
                if overflow > 0 {
                    overflow -= 1;
                } else if let Some((text, done)) = stack.pop()
                    && !done
                {
                    send(&text, &mut limited);
                }
            }
            _ if overflow == 0 => {
                if let Some((text, false)) = stack.last_mut()
                    && text.len() < MAX_FIELD_BYTES
                    && (u >= 0x20 || matches!(u, 0x09 | 0x0D | 0x0A))
                {
                    text.push(char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}'));
                }
            }
            _ => {}
        }
    }
    limited
}

/// `WordDocument`의 필드를 훑는다. 본문이 8비트인 문서와 UTF-16인 문서를 모두 본다.
fn scan_fields(file: &mut OleFile, report: &mut FileReport) {
    let Some(entry) = file.find("WordDocument").cloned() else { return };
    let data = match file.read_stream_prefix(&entry, FIELD_SCAN_MAX) {
        Ok(d) => d,
        Err(e) => {
            report.mark_incomplete(&format!("WordDocument stream could not be read for field analysis: {e:?}"));
            return;
        }
    };
    if entry.size > FIELD_SCAN_MAX as u64 {
        report.mark_incomplete(&format!("WordDocument stream ({} bytes) exceeds the field scan limit; only the first {} MiB were scanned", entry.size, FIELD_SCAN_MAX / (1024 * 1024)));
    }
    // 분류 규칙은 OOXML 필드와 같다. 코드 이름만 DOC_로 바꿔 낸다.
    let mut tmp = FileReport::new(Path::new(""), None);
    let mut emit = |instr: &str| classify_field(instr, "WordDocument", &mut tmp);
    let mut limited = scan_units(data.iter().map(|&b| u16::from(b)), &mut emit);
    // UTF-16LE 본문: 시작 위치가 짝수일 수도 홀수일 수도 있어 두 정렬을 모두 본다.
    for start in 0..2usize {
        limited |= scan_units(data[start.min(data.len())..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])), &mut emit);
    }
    if limited {
        report.mark_incomplete(&format!("field analysis limits reached (depth {MAX_FIELD_DEPTH}, {MAX_INSTRUCTIONS} instructions); some fields were not analyzed"));
    }
    for f in tmp.findings {
        let code = f.code.replacen("OOXML_", "DOC_", 1);
        push_once(report, Category::Security, f.severity, &code, &f.location, &f.detail);
    }
}
