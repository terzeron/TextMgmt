//! 앞부분 바이트로 파일 종류를 가른다. 확장자는 믿지 않는다.

/// 매직바이트로 판별한 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Pdf,
    Zip,
    Elf,
    MachO,
    Pe,
    Html,
    Unknown,
}

const PDF_WINDOW: usize = 1024;
const MACHO_MAGICS: [[u8; 4]; 5] = [[0xFE, 0xED, 0xFA, 0xCE], [0xFE, 0xED, 0xFA, 0xCF], [0xCE, 0xFA, 0xED, 0xFE], [0xCF, 0xFA, 0xED, 0xFE], [0xCA, 0xFE, 0xBA, 0xBE]];

/// 파일 앞부분(`head`)의 바이트로 종류를 판별한다. `Pdf`는 앞 1024바이트 안 어디든 `%PDF-`가 있으면 맞다.
pub fn sniff(head: &[u8]) -> Sniffed {
    if head[..head.len().min(PDF_WINDOW)].windows(5).any(|w| w == b"%PDF-") {
        return Sniffed::Pdf;
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Sniffed::Zip;
    }
    if head.starts_with(b"\x7fELF") {
        return Sniffed::Elf;
    }
    if MACHO_MAGICS.iter().any(|m| head.starts_with(m)) {
        return Sniffed::MachO;
    }
    if head.starts_with(b"MZ") {
        return Sniffed::Pe;
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
