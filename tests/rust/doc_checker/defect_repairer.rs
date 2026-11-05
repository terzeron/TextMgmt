use doc_checker::defect_repairer::*;

const OLE_AS_UTF16_CHARS: &str = "\u{cfd0}\u{e011}\u{b1a1}\u{e11a}";
const OLE_MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

/// (원래 바이너리, 망가진 상태) 쌍. 문자열에서 만들어야 UTF-16LE 왕복이 보장된다.
fn binary_pair() -> (Vec<u8>, Vec<u8>) {
    let text = format!("{OLE_AS_UTF16_CHARS}{}", "FileHeader 본문 내용 가나다라마바사".repeat(20));
    (text.encode_utf16().flat_map(u16::to_le_bytes).collect(), text.into_bytes())
}

#[test]
fn unmangle_restores_original_bytes() {
    let (orig, mangled) = binary_pair();
    assert!(orig.starts_with(&OLE_MAGIC));
    assert_eq!(unmangle_utf16(&mangled), Some(orig));
}

#[test]
fn unmangle_none_for_plain_text_tolerates_truncation_and_gives_up() {
    assert_eq!(unmangle_utf16("평범한 한국어 텍스트입니다. ".repeat(20).as_bytes()), None);
    let (_, mangled) = binary_pair();
    assert!(unmangle_utf16(&mangled[..mangled.len() - 1]).is_some());
    assert_eq!(unmangle_utf16(&[0xff, 0xfe, 0xfd, 0xfc, 0xfb]), None);
}

#[test]
fn true_extension_identifies_binaries() {
    let (_, mangled) = binary_pair();
    assert_eq!(true_extension(&mangled), Some(".hwp"));
    assert_eq!(true_extension("무림맹의 장문인. ".repeat(40).as_bytes()), None);
    assert_eq!(true_extension_with(b"whatever", &|_| Some(b"ZZZZ not a known magic".to_vec())), None);
}
