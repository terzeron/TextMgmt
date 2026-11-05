//! `repair`: 뷰어가 열지 못하는 결함을 고친다.
//!
//! 1. EPUB 본문 XHTML(`xhtml_fix`).
//! 2. UTF-16LE로 읽혀 UTF-8로 저장된 바이너리(HWP·PDF·PNG가 `.txt`로 남은 것)를 원래 바이트와 확장자로 되돌린다.
//!    옛 `quarantine_broken fix-ext`를 옮겨 왔다. 원본은 휴지통에 두고, 같은 inode에 덮어쓴 뒤 이름만 바꾼다(book_id가 inode다).

pub mod xhtml_fix;

// 되돌리면 알려진 바이너리가 되는 파일의 매직 바이트.
// 바이너리를 UTF-16LE로 읽고 UTF-8로 저장하면 이런 상태가 된다.
//   OLE 매직 D0 CF 11 E0 A1 B1 1A E1
//   -> UTF-16LE로 읽으면 U+CFD0 U+E011 U+B1A1 U+E11A
//   -> UTF-8로 저장하면 ec bf 90 ee 80 91 eb 86 a1 ee 84 9a
// 이 변환은 문자가 전부 BMP 안에 있으면 무손실이라 원본을 그대로 복원할 수 있다.
const BINARY_MAGIC: [(&[u8], &str); 4] = [(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1], ".hwp"), (b"HWP Document File", ".hwp"), (b"%PDF", ".pdf"), (b"\x89PNG\r\n\x1a\n", ".png")];

/// UTF-16LE로 읽혀 UTF-8로 저장된 바이너리를 원래 바이트로 되돌린다.
/// 되돌린 결과가 알려진 바이너리 매직으로 시작할 때만 성공으로 본다.
///
/// 표본을 바이트로 자르면 마지막 문자가 잘리므로 꼬리를 다듬어 가며 시도한다.
pub fn unmangle_utf16(raw: &[u8]) -> Option<Vec<u8>> {
    for trim in 0..4 {
        let Some(chunk) = raw.len().checked_sub(trim).map(|n| &raw[..n]) else { continue };
        let Ok(text) = std::str::from_utf8(chunk) else { continue };
        let restored: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        return BINARY_MAGIC.iter().any(|(magic, _)| restored.starts_with(magic)).then_some(restored);
    }
    None
}

/// 되돌린 바이트가 가리키는 실제 확장자. 해당 없으면 None.
pub fn true_extension(raw: &[u8]) -> Option<&'static str> {
    true_extension_with(raw, &unmangle_utf16)
}

pub fn true_extension_with(raw: &[u8], unmangle: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Option<&'static str> {
    let restored = unmangle(raw)?;
    BINARY_MAGIC.iter().find(|(magic, _)| restored.starts_with(magic)).map(|(_, ext)| *ext)
}
