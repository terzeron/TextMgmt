//! 바이트 열의 고정 위치에서 little-endian 정수를 읽는다. OLE·VBA·HWP 레코드 해석이 함께 쓴다.
//! 범위를 벗어나면(위치 덧셈이 넘치는 경우 포함) `None`이다.

pub(crate) fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at.checked_add(2)?).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

pub(crate) fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at.checked_add(8)?).map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}
