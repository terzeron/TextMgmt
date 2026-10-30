//! HWP 압축 스트림 해제. 명세는 zlib이라 쓰지만 오픈소스 구현은 헤더 없는 raw deflate로 읽는다(`docs/bookdoctor-hwp5-notes.md`).
//! 둘 다 받고, 출력은 항상 상한 안에서 읽는다.

use std::io::Read;

use flate2::read::{DeflateDecoder, ZlibDecoder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InflateError {
    /// 풀린 크기가 상한을 넘었다. 압축 폭탄일 수 있다.
    TooLarge,
    /// 어느 형식으로도 읽히지 않거나 중간에 끊겼다.
    Invalid,
}

/// zlib 헤더(RFC 1950)로 유효한가: CMF의 방식이 8(deflate)이고 `CMF*256+FLG`가 31의 배수.
fn looks_like_zlib(data: &[u8]) -> bool {
    matches!(data, [cmf, flg, ..] if cmf & 0x0F == 8 && (u32::from(*cmf) * 256 + u32::from(*flg)) % 31 == 0)
}

/// `reader`를 `limit`바이트까지만 읽는다. 상한을 넘기면 `TooLarge`, 읽기 오류면 `Invalid`다.
fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>, InflateError> {
    let mut out = Vec::new();
    // 한 바이트 더 읽어 상한 초과를 알아낸다. 그보다 더 읽지 않는다.
    reader.take(limit as u64 + 1).read_to_end(&mut out).map_err(|_| InflateError::Invalid)?;
    if out.len() > limit { Err(InflateError::TooLarge) } else { Ok(out) }
}

/// 한 형식으로 풀어 보는 함수.
type Attempt = fn(&[u8], usize) -> Result<Vec<u8>, InflateError>;

fn zlib(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    read_limited(ZlibDecoder::new(data), limit)
}

fn raw(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    read_limited(DeflateDecoder::new(data), limit)
}

/// 압축 스트림을 풀어 `limit`바이트 이하로 돌려준다. zlib 헤더가 유효하면 zlib을, 아니면 raw deflate를 먼저 시도하고,
/// 첫 시도가 `Invalid`면 다른 쪽을 한 번 더 시도한다. `TooLarge`는 바로 돌려준다. 빈 입력은 빈 출력이다.
pub fn decompress(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let (first, second): (Attempt, Attempt) = if looks_like_zlib(data) { (zlib, raw) } else { (raw, zlib) };
    match first(data, limit) {
        Err(InflateError::Invalid) => second(data, limit),
        other => other,
    }
}
