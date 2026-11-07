//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]

pub fn le16(v: u16) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

pub fn codes(text: &str) -> Vec<u16> {
    text.chars().map(|c| c as u16).collect()
}

pub fn pack(codes: &[u16]) -> Vec<u8> {
    codes.iter().flat_map(|c| c.to_le_bytes()).collect()
}
