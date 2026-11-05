//! 바이트를 텍스트로 푼다: 잘못된 바이트를 버리는 UTF-8·CP949 디코딩과 한글 음절 판정.
//! `doc_reader`와 bookdoctor가 쓴다. 본문 표본 추출과 인코딩 추정은 bookdoctor `text_sampler`에 있다.

pub fn is_hangul_syllable(c: char) -> bool {
    ('가'..='힣').contains(&c)
}

/// UTF-8로 읽되 잘못된 바이트는 버린다.
pub fn decode_utf8_ignore(raw: &[u8]) -> String {
    raw.utf8_chunks().map(|c| c.valid()).collect()
}

/// CP949(= WHATWG euc-kr, UHC 포함)를 엄격하게 푼다.
pub fn decode_cp949_strict(raw: &[u8]) -> Option<String> {
    encoding_rs::EUC_KR.decode_without_bom_handling_and_without_replacement(raw).map(|c| c.into_owned())
}

/// CP949 / EUC-KR을 읽되 잘못된 바이트는 버린다. 인코딩 판별 휴리스틱의 마지막 단계가 쓰는 관대한 디코더다.
///
/// 해석할 수 없는 선두 바이트는 1바이트만 버리고 바로 다음 바이트부터 다시 읽어, 손상 구간 뒤에서 빨리 정상 위치를 되찾는다.
/// encoding_rs의 기본 동작은 비ASCII 후행 바이트까지 함께 버려 다시 읽는 위치가 달라지고, 중문·일문
/// 텍스트를 이 경로로 읽을 때 판정이 바뀐다(실측: 표본 400건 중 7건).
/// `ksx1001_only`가 참이면 확장 완성형(UHC)을 빼고 KS X 1001 영역만 받아들인다.
pub fn decode_korean_ignore(raw: &[u8], ksx1001_only: bool) -> String {
    // 흔한 경우: 전부 유효하면 한 번에 푼다. 잘못된 바이트가 있을 때만 아래 느린 경로를 탄다.
    if !ksx1001_only && let Some(all) = decode_cp949_strict(raw) {
        return all;
    }
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c < 0x80 {
            out.push(c as char);
            i += 1;
            continue;
        }
        let pair_ok = match raw.get(i + 1) {
            Some(&d) if c != 0x80 && c != 0xff && (!ksx1001_only || ((0xa1..=0xfe).contains(&c) && (0xa1..=0xfe).contains(&d))) => {
                let before = out.len();
                // `decode_to_string*`는 String을 키우지 않는다. 남은 용량이 모자라면 OutputFull로 실패하므로 미리 확보한다.
                out.reserve(4);
                let mut dec = encoding_rs::EUC_KR.new_decoder_without_bom_handling();
                let (res, _) = dec.decode_to_string_without_replacement(&[c, d], &mut out, true);
                if res == encoding_rs::DecoderResult::InputEmpty && out.len() > before {
                    true
                } else {
                    out.truncate(before);
                    false
                }
            }
            _ => false,
        };
        i += if pair_ok { 2 } else { 1 };
    }
    out
}
