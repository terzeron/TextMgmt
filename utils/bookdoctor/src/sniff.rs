//! 바이트와 이름만 보고 판단하는 검사: 매직바이트 판별, 섀넌 엔트로피, Windows 예약 이름. 확장자는 믿지 않는다.



// ---- 앞부분 바이트로 파일 종류를 가른다. 확장자는 믿지 않는다. ----

/// 매직바이트로 판별한 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Pdf,
    Zip,
    Elf,
    MachO,
    Pe,
    /// OLE 복합 파일(구형 DOC·HWP 5.x, 암호화 OOXML의 컨테이너).
    Ole,
    /// RTF 문서(`{\rtf`로 시작). Word는 이름과 무관하게 연다.
    Rtf,
    /// HWP 3.x 이하(`HWP Document File V` 서명, OLE가 아니다).
    Hwp3,
    Html,
    Unknown,
}

const PDF_WINDOW: usize = 1024;
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const MACHO_MAGICS: [[u8; 4]; 5] = [[0xFE, 0xED, 0xFA, 0xCE], [0xFE, 0xED, 0xFA, 0xCF], [0xCE, 0xFA, 0xED, 0xFE], [0xCF, 0xFA, 0xED, 0xFE], [0xCA, 0xFE, 0xBA, 0xBE]];

/// `MZ`로 시작하고 e_lfanew(오프셋 0x3C, u32 LE)가 `PE\0\0`를 가리키면 PE다. `MZ`만으로는 평문과 구분되지 않는다.
fn is_pe(head: &[u8]) -> bool {
    if !head.starts_with(b"MZ") || head.len() < 0x40 {
        return false;
    }
    let off = u32::from_le_bytes([head[0x3C], head[0x3D], head[0x3E], head[0x3F]]) as usize;
    off.checked_add(4).is_some_and(|end| end <= head.len()) && &head[off..off + 4] == b"PE\0\0"
}

/// 파일 앞부분(`head`)의 바이트로 종류를 판별한다. 오프셋 0의 실행 파일 매직(ELF·Mach-O·`MZ`+PE)이 먼저다.
/// 그 밖에는 앞 1024바이트 안 어디든 `%PDF-`가 있으면 `Pdf`다.
pub fn sniff(head: &[u8]) -> Sniffed {
    // 오프셋 0의 실행 파일 매직은 "앞 1KiB 안의 `%PDF-`"보다 강한 신호다. 실행 파일은 DOS stub 같은 자기 영역에 `%PDF-`를 넣어 PDF로 위장할 수 있다.
    if head.starts_with(b"\x7fELF") {
        return Sniffed::Elf;
    }
    if MACHO_MAGICS.iter().any(|m| head.starts_with(m)) {
        return Sniffed::MachO;
    }
    if is_pe(head) {
        return Sniffed::Pe;
    }
    if head.starts_with(&OLE_MAGIC) {
        return Sniffed::Ole;
    }
    if head.starts_with(b"HWP Document File V") {
        return Sniffed::Hwp3;
    }
    // Word는 `{\rt`만 맞아도 RTF로 연다. BOM과 앞 공백도 건너뛴다(HTML 판별과 같다).
    let trimmed = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let lead = trimmed.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(trimmed.len());
    if trimmed[lead..].starts_with(b"{\\rt") {
        return Sniffed::Rtf;
    }
    if head[..head.len().min(PDF_WINDOW)].windows(5).any(|w| w == b"%PDF-") {
        return Sniffed::Pdf;
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Sniffed::Zip;
    }
    let body = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let start = body.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(body.len());
    let rest = &body[start..];
    let starts_ci = |p: &[u8]| rest.len() >= p.len() && rest[..p.len()].eq_ignore_ascii_case(p);
    if starts_ci(b"<!doctype html") || starts_ci(b"<html") {
        return Sniffed::Html;
    }
    Sniffed::Unknown
}

// ---- 바이트 분포의 섀넌 엔트로피. 압축·암호화된 데이터는 8에 가깝고 텍스트는 대체로 5 안팎이다. ----

/// 텍스트로 읽히는 확장자(소문자). 이름이 이 확장자인데 내용이 높은 엔트로피면 렌더러가 읽지 못하는 숨은 데이터일 수 있다.
const TEXT_EXTENSIONS: &[&str] = &["xml", "rels", "html", "htm", "xhtml", "js", "css", "txt", "json", "svg"];

/// 섀넌 엔트로피(비트/바이트, 0.0~8.0). 빈 입력은 0.0이다.
pub fn shannon(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &b in bytes {
        counts[usize::from(b)] += 1;
    }
    let total = bytes.len() as f64;
    counts.iter().filter(|&&c| c > 0).map(|&c| {
        let p = c as f64 / total;
        -p * p.log2()
    }).sum()
}

/// 파일 이름(경로 포함 가능)이 텍스트 확장자인가. 대소문자는 가리지 않고, 줄기가 없는 `.xml`이나 확장자 없는 이름은 아니다.
pub fn is_text_name(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => TEXT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        _ => false,
    }
}

// ---- 파일 이름 규칙. ----

const WIN_RESERVED: &[&str] =
    &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];

/// 경로 성분이 Windows 예약 장치 이름인가. 확장자와 끝의 점·공백은 무시하고 대소문자를 구분하지 않는다(`com1.txt`, `NUL. `).
pub fn windows_reserved(component: &str) -> bool {
    let stem = component.trim_end_matches(['.', ' ']).split('.').next().unwrap_or("").to_lowercase();
    WIN_RESERVED.contains(&stem.as_str())
}
