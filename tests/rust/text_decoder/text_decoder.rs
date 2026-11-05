//! `text_decoder` 단위 테스트.

#[test]
fn korean_slow_path_keeps_decoding_long_text_after_an_invalid_byte() {
    use text_decoder::decode_korean_ignore;
    // 한글은 2바이트가 3바이트 UTF-8이 되므로, 버퍼를 입력 길이로만 잡으면 중간에서 OutputFull이 난다.
    let text = "안녕하세요 ".repeat(200);
    let (korean, _, _) = encoding_rs::EUC_KR.encode(&text);
    let mut raw = korean.into_owned();
    raw.push(0xff);
    assert_eq!(decode_korean_ignore(&raw, false), "안녕하세요 ".repeat(200));
}
