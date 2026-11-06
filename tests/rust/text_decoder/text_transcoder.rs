//! `text_decoder::text_transcoder` 단위 테스트.

use text_decoder::text_transcoder::{Enc, decode};

#[test]
fn decode_follows_strict_and_replace_codec_semantics() {
    assert_eq!(decode(Enc::Utf16, b"\xff\xfea\0", false).as_deref(), Some("a"));
    assert_eq!(decode(Enc::Utf16, b"\xfe\xff\0a", false).as_deref(), Some("a"));
    assert_eq!(decode(Enc::Utf16, b"a\0", false).as_deref(), Some("a"), "BOM이 없으면 리틀엔디언");
    assert_eq!(decode(Enc::Utf16Le, b"\xff\xfea\0", false).as_deref(), Some("\u{feff}a"), "utf-16-le는 BOM을 지우지 않는다");
    assert_eq!(decode(Enc::Utf16Le, b"a", false), None, "홀수 바이트는 오류");
    assert_eq!(decode(Enc::Utf8, b"\xef\xbb\xbfa", false).as_deref(), Some("\u{feff}a"));
    assert_eq!(decode(Enc::Utf8Sig, b"\xef\xbb\xbfa", false).as_deref(), Some("a"));
    assert_eq!(decode(Enc::Utf8, b"a\xff", false), None);
    assert_eq!(decode(Enc::Utf8, b"a\xff", true).as_deref(), Some("a\u{fffd}"));
    assert_eq!(decode(Enc::Cp1252, b"caf\xe9", false).as_deref(), Some("café"));
    assert_eq!(Enc::Cp949.name(), "cp949");
}
