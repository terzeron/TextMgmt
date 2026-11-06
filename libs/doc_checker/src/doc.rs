//! 구형 DOC(OLE 복합 파일) 검사. `ole` 모듈로 구조를 읽고 손상, 암호화, VBA 매크로, 내장 OLE 객체, DDE·필드를 본다.
//! 파일을 실행하거나 렌더링하지 않는다. 매크로는 텍스트로만 읽는다.

use crate::Category;
use crate::CheckOptions;
use crate::FileReport;
use crate::Severity;
use crate::magic_sniffer::Sniffed;
use crate::magic_sniffer::sniff;
use crate::ole_parser::Anomaly;
use crate::ole_parser::EntryKind;
use crate::ole_parser::Limits as OleLimits;
use crate::ole_parser::OleError;
use crate::ole_parser::OleFile;
use crate::ole_parser::vba_decoder;
use crate::text_sanitizer::sanitize_display;
use crate::zipcheck::fast_zip_checker::ZIP_EXEC_EXTS;
use crate::zipcheck::ooxml_checker::classify_field;
use std::io::Read;
use std::path::Path;

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
pub fn scan_doc_file(path: &Path, opts: &CheckOptions, sniffed: Sniffed, report: &mut FileReport) {
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
pub fn scan_ole_named_docx(path: &Path, opts: &CheckOptions, report: &mut FileReport) {
    let Some(mut file) = open_ole(path, opts, report) else { return };
    if file.find("EncryptedPackage").is_some() || file.find("WordDocument").is_some() {
        inspect(&mut file, report);
    } else {
        report.mark_incomplete("OLE container cannot be inspected: .docx extension with OLE content that is neither an encrypted package nor a Word document");
    }
}

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

/// RTF 파일을 한도까지 읽어 `inspect_rtf`에 넘긴다. 한도를 넘으면 앞부분만 보고 미완료로 남긴다.
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
    inspect_rtf(&bytes, report);
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
        push_once(
            report,
            Category::Security,
            Severity::High,
            "OLE_MALFORMED_STRUCTURE",
            "",
            "OLE structure is malformed in ways that trigger parser bugs (loops, out-of-range sectors, shared sectors, size lies)",
        );
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
        match vba_decoder::read_project(file, &root, VBA_BUDGET) {
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
                push_once(report, Category::Security, Severity::Critical, "EMBEDDED_EXECUTABLE", &sanitize_display(name, 120, Some('…')), &format!("Embedded file type .{ext}"));
                break;
            }
        }
        // 이름이 무엇이든 내용이 실행 파일인 것을 찾는다(앞부분에서 `MZ`·`\x7fELF`가 나오는 자리마다 헤더를 확인한다).
        let hit = prefix.windows(2).enumerate().any(|(i, w)| (w == b"MZ" || w == b"\x7f\x45") && matches!(sniff(&prefix[i..]), Sniffed::Pe | Sniffed::Elf));
        if hit {
            push_once(report, Category::Security, Severity::Critical, "EMBEDDED_EXECUTABLE", &shown, "Embedded executable content (PE/ELF magic)");
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

// ---- rtf_checker ----
// RTF 검사. RTF는 텍스트 형식이라 Word가 `.doc`·`.docx`라는 이름으로도 연다. 내장 OLE 객체를 심는 공격의 통로다.
// 구조를 해석하지 않고 바이트를 선형으로 훑는다(중첩 깊이를 세지 않으므로 깊이 폭탄이 통하지 않는다).
/// 객체를 뜻하는 제어어. 하나라도 있으면 내장 객체가 있다.
/// `\datastore`는 뺀다: Word가 저장한 RTF는 모두 customXml용 `\datastore`를 가지므로 이것만으로는 신호가 못 된다.
const OBJECT_WORDS: &[&str] = &["objdata", "objclass", "objupdate", "objautlink"];
/// 제어어 이름으로 읽는 최대 글자 수. RTF 규격은 32자다.
const MAX_WORD_LEN: usize = 32;
/// `\objdata` 하나에서 해독하는 앞부분(바이트). OLE1 머리말 뒤의 본문 매직까지 닿는다.
const PAYLOAD_PREFIX: usize = 4096;
/// 페이로드를 확인하는 `\objdata` 수의 상한.
const MAX_OBJDATA: usize = 256;
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// `\'xx`로 쓴 영문자·숫자를 그 문자로 되돌린다. RTF 파서는 이 이스케이프를 합쳐 읽으므로 `\obj\'64ata`도 `\objdata`다.
/// 그 밖의 바이트(구분자나 제어 문자가 될 수 있는 값)는 건드리지 않는다.
fn normalize(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && bytes.get(i + 1) == Some(&b'\'')
            && let (Some(&h), Some(&l)) = (bytes.get(i + 2), bytes.get(i + 3))
            && let (Some(h), Some(l)) = ((h as char).to_digit(16), (l as char).to_digit(16))
        {
            let decoded = (h * 16 + l) as u8;
            if decoded.is_ascii_alphanumeric() {
                out.push(decoded);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// `pos`(`\` 다음 바이트)부터 제어어 이름을 읽는다. 이름과 이름이 끝난 위치를 돌려준다.
fn control_word(data: &[u8], pos: usize) -> (&[u8], usize) {
    let end = data[pos..].iter().take(MAX_WORD_LEN + 1).position(|b| !b.is_ascii_alphabetic()).map_or(data.len().min(pos + MAX_WORD_LEN + 1), |n| pos + n);
    (&data[pos..end], end)
}

/// 이름 뒤의 숫자 인자와 구분 공백 하나를 읽는다. 인자 값(없으면 0)과 다음 위치를 돌려준다.
fn parameter(data: &[u8], mut pos: usize) -> (i64, usize) {
    let negative = data.get(pos) == Some(&b'-');
    if negative {
        pos += 1;
    }
    let mut value: i64 = 0;
    while let Some(d) = data.get(pos).filter(|b| b.is_ascii_digit()) {
        value = value.saturating_mul(10).saturating_add(i64::from(d - b'0')).min(i64::from(u32::MAX));
        pos += 1;
    }
    if data.get(pos) == Some(&b' ') {
        pos += 1;
    }
    (if negative { -value } else { value }, pos)
}

/// `\objdata` 하나를 해독할 때 살펴보는 입력 바이트의 상한.
const DECODE_BUDGET: usize = 64 * 1024;

/// 16진 본문 해독 결과.
struct Decoded {
    bytes: Vec<u8>,
    /// 본문 그룹의 끝(`}`)이나 `PAYLOAD_PREFIX`에 닿기 전에 입력 상한을 다 썼다.
    budget_hit: bool,
    /// 본문 안에 안쪽 그룹이나 `\binN`이 있었다. Word가 이를 어떻게 읽는지 확정할 수 없다.
    ambiguous: bool,
}

/// `\objdata` 본문의 16진 앞부분을 해독한다. RTF 파서처럼 공백, 제어어(`\par` 등), 안쪽 그룹, `\binN` 원시 바이트를 건너뛰고
/// 본문 그룹을 닫는 `}`에서 멈춘다. 16진이 아닌 글자는 무시한다.
fn decode_hex_prefix(data: &[u8], start: usize) -> Decoded {
    let end = data.len().min(start.saturating_add(DECODE_BUDGET));
    let mut out = Vec::new();
    let mut pending: Option<u32> = None;
    let mut depth = 0usize;
    let mut ambiguous = false;
    let mut i = start;
    while i < end {
        let b = data[i];
        i += 1;
        match b {
            b'{' => {
                depth += 1;
                ambiguous = true;
            }
            b'}' if depth == 0 => return Decoded { bytes: out, budget_hit: false, ambiguous },
            b'}' => depth -= 1,
            b'\\' => match data.get(i) {
                Some(c) if c.is_ascii_alphabetic() => {
                    let (name, name_end) = control_word(data, i);
                    let (value, after) = parameter(data, name_end);
                    i = after;
                    if name.eq_ignore_ascii_case(b"bin") && value > 0 {
                        i = i.saturating_add(value as usize);
                        ambiguous = true;
                    }
                }
                // `\'xx`는 16진 두 글자까지 건너뛴다. 그 밖의 기호(`\\`, `\{`, `\}`)는 한 글자다.
                Some(b'\'') => i += 3,
                Some(_) => i += 1,
                None => {}
            },
            _ if depth == 0 => {
                if let Some(d) = (b as char).to_digit(16) {
                    match pending.take() {
                        Some(h) => {
                            out.push((h * 16 + d) as u8);
                            if out.len() >= PAYLOAD_PREFIX {
                                return Decoded { bytes: out, budget_hit: false, ambiguous };
                            }
                        }
                        None => pending = Some(d),
                    }
                }
            }
            _ => {}
        }
    }
    // 입력이 끝났거나 상한을 다 썼다. 파일 끝까지 읽었다면 상한 문제가 아니다.
    Decoded { bytes: out, budget_hit: i < data.len(), ambiguous }
}

/// OLE1 머리말에서 본문(NativeData)의 위치를 알 수 있는가.
enum NativeOffset {
    NotOle1,
    /// 본문 위치가 해독한 앞부분 안에 있다.
    Within,
    /// 머리말이 해독한 앞부분보다 길어 본문 위치를 알 수 없다.
    Beyond,
}

/// OLE1 객체 머리말(버전 `0x501`, 형식 2, 문자열 세 개, 본문 크기)을 읽어 본문 위치가 앞부분 안인지 가른다.
fn native_offset(d: &[u8]) -> NativeOffset {
    let u32_at = |pos: usize| d.get(pos..pos + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if d.len() < 8 || d[..4] != [0x01, 0x05, 0x00, 0x00] || u32_at(4) != Some(2) {
        return NativeOffset::NotOle1;
    }
    let mut pos = 8usize;
    for _ in 0..3 {
        let Some(len) = u32_at(pos) else { return NativeOffset::Beyond };
        pos = pos.saturating_add(4).saturating_add(len);
    }
    // 본문 크기(4바이트) 다음이 본문이다.
    let at = pos.saturating_add(4);
    if at > d.len() { NativeOffset::Beyond } else { NativeOffset::Within }
}

#[derive(PartialEq)]
enum Payload {
    None,
    Ole,
    Executable,
}

/// 해독한 앞부분에서 실행 파일이나 OLE 복합 파일을 찾는다. OLE1 머리말(`01050000`)은 클래스 이름 뒤에 본문이 오므로 앞부분 어디든 본다.
fn payload_kind(decoded: &[u8]) -> Payload {
    let mut kind = Payload::None;
    for i in 0..decoded.len() {
        let rest = &decoded[i..];
        if rest.starts_with(&OLE_MAGIC) {
            kind = Payload::Ole;
        } else if rest.starts_with(b"MZ") && sniff(rest) == Sniffed::Pe || rest.starts_with(b"\x7fELF") {
            return Payload::Executable;
        }
    }
    kind
}

fn push_rtf_once(report: &mut FileReport, severity: Severity, code: &str, location: &str, detail: &str) {
    if !report.has(code) {
        report.push(Category::Security, severity, code, location, detail);
    }
}

/// RTF 본문(`bytes`)을 검사한다. 지표는 코드마다 한 번만 낸다.
pub fn inspect_rtf(bytes: &[u8], report: &mut FileReport) {
    let data = normalize(bytes);
    let mut i = 0;
    let mut objdata_seen = 0;
    let mut incomplete_reason: Option<&str> = None;
    while let Some(off) = data[i..].iter().position(|&b| b == b'\\') {
        let start = i + off + 1;
        let (name, end) = control_word(&data, start);
        i = start.max(end);
        let Ok(name) = std::str::from_utf8(name) else { continue };
        let lower = name.to_ascii_lowercase();
        let datastore = lower == "datastore";
        if !datastore && !OBJECT_WORDS.contains(&lower.as_str()) {
            continue;
        }
        if datastore {
            push_rtf_once(report, Severity::Medium, "RTF_DATASTORE", "\\datastore", "RTF has a data store (Word writes one for customXml); payload checked");
        } else {
            push_rtf_once(report, Severity::High, "RTF_EMBEDDED_OBJECT", &sanitize_display(&format!("\\{lower}"), 40, Some('…')), "RTF embeds an OLE object");
        }
        // 16진 본문을 갖는 제어어는 앞부분에서 실행 파일·OLE 매직을 찾는다.
        if lower == "objdata" || datastore {
            if objdata_seen >= MAX_OBJDATA {
                incomplete_reason.get_or_insert("more embedded object bodies than the scan limit; the rest were not decoded");
                continue;
            }
            objdata_seen += 1;
            let (_, after) = parameter(&data, end);
            let decoded = decode_hex_prefix(&data, after);
            match payload_kind(&decoded.bytes) {
                Payload::Executable => push_rtf_once(report, Severity::Critical, "RTF_EMBEDDED_EXECUTABLE", &format!("\\{lower}"), "Embedded object payload is an executable (PE/ELF)"),
                // customXml 저장소는 OLE 복합 파일을 담는 것이 표준이라 `\datastore`의 OLE 매직은 지표가 아니다.
                Payload::Ole if !datastore => push_rtf_once(report, Severity::High, "RTF_EMBEDDED_OLE", &format!("\\{lower}"), "Embedded object payload is an OLE compound file"),
                Payload::Ole => {}
                Payload::None => {
                    // 본문을 확인하지 못한 채 지표가 없으면 깨끗하다고 하지 않는다.
                    if decoded.budget_hit {
                        incomplete_reason.get_or_insert("an embedded object body could not be decoded within the scan budget");
                    } else if decoded.ambiguous {
                        incomplete_reason.get_or_insert("an embedded object body has nested groups or \\bin data; how Word reads it is ambiguous");
                    } else if !datastore && matches!(native_offset(&decoded.bytes), NativeOffset::Beyond) && decoded.bytes.len() >= PAYLOAD_PREFIX {
                        incomplete_reason.get_or_insert("an embedded object's header is longer than the decoded prefix; its payload was not checked");
                    }
                }
            }
        }
    }
    if let Some(reason) = incomplete_reason {
        report.mark_incomplete(reason);
    }
}
