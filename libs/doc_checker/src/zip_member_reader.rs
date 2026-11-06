//! ZIP 멤버를 한도 안에서 읽는 공용 도구. 형식 검사기(EPUB·ZIP·OOXML·HWPX)와 중첩 검사가 함께 쓴다.
//!
//! - 읽기 한도: [`Limits`], 거대한 중앙 디렉터리를 열기 전에 거르는 [`directory_limit_note`], 멤버 하나를 상한까지 읽는 [`read_limited`]
//! - 마크업 바이트 해석: UTF-16·UTF-32를 알아보는 [`decode_text`]

use std::borrow::Cow;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use common::text_sanitizer::sanitize;
use common::zip_reader::Zip;

/// 사유 문구의 최대 글자 수.
const MAX_NOTE_CHARS: usize = 160;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// 멤버 하나에서 읽는 최대 압축 해제 바이트.
    pub max_member: u64,
    /// 파일 전체에서 읽는 최대 압축 해제 바이트.
    pub max_total: u64,
    pub max_files: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_member: 20 * 1024 * 1024, max_total: 200 * 1024 * 1024, max_files: 10_000 }
    }
}

/// 중앙 디렉터리가 이보다 크면 열지 않는다. ZIP 라이브러리는 항목마다 메타데이터를 메모리에 올린다.
const MAX_DIRECTORY_ENTRIES: u64 = 1_000_000;
const MAX_DIRECTORY_BYTES: u64 = 256 * 1024 * 1024;

/// 파일 끝의 EOCD(zip64 포함)에서 항목 수와 중앙 디렉터리 크기를 읽는다. 열기 전에 거대한 디렉터리를 거르려는 용도다.
/// 알아보지 못하면 None이고, 그때는 평소처럼 연다.
pub(crate) fn zip_directory_bounds(path: &Path) -> Option<(u64, u64)> {
    const TAIL: u64 = 22 + 65_535;
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    f.take(TAIL).read_to_end(&mut tail).ok()?;
    let eocd = tail.windows(4).rposition(|w| w == b"PK\x05\x06")?;
    let rec = tail.get(eocd..eocd + 22)?;
    let le16 = |b: &[u8], o: usize| u64::from(u16::from_le_bytes([b[o], b[o + 1]]));
    let le32 = |b: &[u8], o: usize| u64::from(u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]));
    let (mut entries, mut size) = (le16(rec, 10), le32(rec, 12));
    if entries == 0xFFFF || size == 0xFFFF_FFFF {
        // zip64: EOCD 바로 앞의 로케이터가 zip64 EOCD 레코드의 위치를 알려 준다.
        let loc = tail.get(eocd.checked_sub(20)?..eocd)?;
        if &loc[..4] != b"PK\x06\x07" {
            return None; // 표식 값(0xFFFF)을 실제 값으로 읽으면 안 된다.
        }
        let off = u64::from_le_bytes(loc[8..16].try_into().ok()?);
        let mut f = File::open(path).ok()?;
        f.seek(SeekFrom::Start(off)).ok()?;
        let mut z = [0u8; 56];
        f.read_exact(&mut z).ok()?;
        if &z[..4] != b"PK\x06\x06" {
            return None;
        }
        entries = u64::from_le_bytes(z[32..40].try_into().ok()?);
        size = u64::from_le_bytes(z[40..48].try_into().ok()?);
    }
    Some((entries, size))
}

/// 중앙 디렉터리가 한도를 넘으면 열지 않는 사유를 돌려준다. 어느 검사 단계든 ZIP을 열기 전에 먼저 확인한다.
pub fn directory_limit_note(path: &Path) -> Option<String> {
    let (entries, dir_bytes) = zip_directory_bounds(path)?;
    if entries > MAX_DIRECTORY_ENTRIES {
        return Some(sanitize(&format!("Too many entries: {entries} (max {MAX_DIRECTORY_ENTRIES}); not opened"), MAX_NOTE_CHARS, None));
    }
    if dir_bytes > MAX_DIRECTORY_BYTES {
        return Some(sanitize(&format!("Central directory too large: {dir_bytes} bytes (max {MAX_DIRECTORY_BYTES}); not opened"), MAX_NOTE_CHARS, None));
    }
    None
}

/// 마크업 바이트를 어떻게 읽었는지.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextEnc {
    /// UTF-8(BOM 유무 무관). 예전과 같다.
    Utf8,
    /// UTF-16을 BOM이나 NUL 교차로 알아보고 변환했다.
    Utf16,
    /// UTF-32 BOM. encoding_rs가 지원하지 않아 변환하지 못한다.
    Utf32,
    /// BOM 없는 UTF-8이 아닌 바이트. 대체 문자(U+FFFD)로 읽었다.
    Invalid,
}

/// BOM 없는 UTF-16 추정: 앞 64바이트에서 (비NUL, NUL) 쌍이 4분의 3 이상이면 LE, (NUL, 비NUL) 쌍이면 BE다.
/// 단순한 휴리스틱이다. XHTML·XML은 ASCII 표식으로 시작하므로 이 정도로 충분하다.
fn bomless_utf16(data: &[u8]) -> Option<bool> {
    let head = &data[..data.len().min(64)];
    let pairs = head.chunks_exact(2).len();
    if pairs < 2 {
        return None;
    }
    let le = head.chunks_exact(2).filter(|p| p[0] != 0 && p[1] == 0).count();
    let be = head.chunks_exact(2).filter(|p| p[0] == 0 && p[1] != 0).count();
    if le * 4 >= pairs * 3 {
        Some(true)
    } else if be * 4 >= pairs * 3 {
        Some(false)
    } else {
        None
    }
}

thread_local! {
    /// 이 스레드에서 `decode_text`를 부른 횟수. 스레드별이라 같은 프로세스의 다른 검사와 섞이지 않는다.
    static DECODE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 이 스레드에서 `decode_text`를 부른 횟수. 비텍스트 멤버를 변환하지 않는지 테스트가 센다(성능 회귀 감시용, 검사 결과와 무관).
pub fn decode_calls() -> usize {
    DECODE_CALLS.with(std::cell::Cell::get)
}

/// 보안 리뷰 지적(from_utf8_lossy): UTF-16 마크업은 모든 정규식을 피해 간다. BOM·NUL 교차로 UTF-16을 알아봐 변환한다.
/// UTF-8 BOM은 호출부가 예전처럼 벗긴다.
pub(crate) fn decode_text(data: &[u8]) -> (Cow<'_, str>, TextEnc) {
    DECODE_CALLS.with(|calls| calls.set(calls.get() + 1));
    if data.starts_with(&[0xFF, 0xFE, 0, 0]) || data.starts_with(&[0, 0, 0xFE, 0xFF]) {
        return (String::from_utf8_lossy(data), TextEnc::Utf32);
    }
    if let Some(rest) = data.strip_prefix(&[0xFF, 0xFE]) {
        return (encoding_rs::UTF_16LE.decode_without_bom_handling(rest).0, TextEnc::Utf16);
    }
    if let Some(rest) = data.strip_prefix(&[0xFE, 0xFF]) {
        return (encoding_rs::UTF_16BE.decode_without_bom_handling(rest).0, TextEnc::Utf16);
    }
    if let Some(le) = bomless_utf16(data) {
        let enc = if le { encoding_rs::UTF_16LE } else { encoding_rs::UTF_16BE };
        return (enc.decode_without_bom_handling(data).0, TextEnc::Utf16);
    }
    match std::str::from_utf8(data) {
        Ok(t) => (Cow::Borrowed(t), TextEnc::Utf8),
        Err(_) => (String::from_utf8_lossy(data), TextEnc::Invalid),
    }
}

pub(crate) fn read_limited(zip: &mut Zip, index: usize, limit: u64) -> Result<Vec<u8>, String> {
    let entry = zip.by_index(index).map_err(|e| e.to_string())?;
    // 선언 크기만큼 미리 잡는다. 비워 두고 읽으면 read_to_end가 용량을 두 배씩 늘려 최대 2배를 쓴다. 선언이 거짓이어도 상한까지만 잡는다.
    let mut data = Vec::with_capacity(entry.size().min(limit).saturating_add(1) as usize);
    entry.take(limit + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use common::zip_reader::open_zip;

    use super::*;

    fn zip_with(dir: &tempfile::TempDir, members: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.path().join("a.zip");
        let mut w = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, data) in members {
            w.start_file(*name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        path
    }

    /// zip64 EOCD 레코드 + 로케이터 + EOCD를 이어 붙인 바이트열. 항목 수와 디렉터리 크기를 zip64 쪽에만 적는다.
    fn zip64_tail(entries: u64, dir_bytes: u64, record_sig: &[u8; 4]) -> Vec<u8> {
        let mut record = vec![0u8; 56];
        record[..4].copy_from_slice(record_sig);
        record[32..40].copy_from_slice(&entries.to_le_bytes());
        record[40..48].copy_from_slice(&dir_bytes.to_le_bytes());
        let mut locator = vec![0u8; 20];
        locator[..4].copy_from_slice(b"PK\x06\x07");
        let mut eocd = vec![0u8; 22];
        eocd[..4].copy_from_slice(b"PK\x05\x06");
        eocd[10..12].copy_from_slice(&0xFFFFu16.to_le_bytes());
        eocd[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        [record, locator, eocd].concat()
    }

    #[test]
    fn decode_text_reads_utf8_without_copying() {
        let (text, enc) = decode_text("가나다 abc".as_bytes());
        assert!(matches!(text, Cow::Borrowed("가나다 abc")) && enc == TextEnc::Utf8);
    }

    #[test]
    fn decode_text_converts_bom_utf16_in_both_byte_orders() {
        for raw in [&b"\xFF\xFE<\x00p\x00>\x00"[..], b"\xFE\xFF\x00<\x00p\x00>"] {
            let (text, enc) = decode_text(raw);
            assert_eq!(text, "<p>");
            assert!(enc == TextEnc::Utf16);
        }
    }

    #[test]
    fn decode_text_flags_utf32_boms_without_converting() {
        assert!(decode_text(b"\xFF\xFE\x00\x00<\x00\x00\x00").1 == TextEnc::Utf32);
        assert!(decode_text(b"\x00\x00\xFE\xFF\x00\x00\x00<").1 == TextEnc::Utf32);
    }

    #[test]
    fn decode_text_recognizes_utf16_without_a_bom_by_nul_interleaving() {
        assert_eq!(decode_text(b"<\x00s\x00c\x00r\x00i\x00p\x00t\x00").0, "<script");
        assert_eq!(decode_text(b"\x00<\x00s\x00c\x00r\x00i\x00p\x00t").0, "<script");
    }

    #[test]
    fn decode_text_reads_other_bytes_lossily_as_invalid() {
        let (text, enc) = decode_text(b"ab\xB0\xA1cd");
        assert!(enc == TextEnc::Invalid);
        assert!(text.contains('\u{fffd}') && text.starts_with("ab") && text.ends_with("cd"));
    }

    #[test]
    fn bomless_utf16_needs_three_quarters_of_the_pairs_in_one_order() {
        assert_eq!(bomless_utf16(b"a\x00b\x00c\x00d\x00"), Some(true));
        assert_eq!(bomless_utf16(b"\x00a\x00b\x00c\x00d"), Some(false));
        assert_eq!(bomless_utf16(b"a\x00b\x00cdef"), None, "절반만 맞으면 판단하지 않는다");
        assert_eq!(bomless_utf16(b"ab"), None, "쌍이 둘 미만이면 판단하지 않는다");
        assert_eq!(bomless_utf16(b""), None);
    }

    #[test]
    fn decode_calls_counts_each_decode_on_this_thread() {
        let before = decode_calls();
        decode_text(b"a");
        decode_text(b"b");
        assert_eq!(decode_calls() - before, 2);
    }

    #[test]
    fn read_limited_reads_one_byte_past_the_limit_so_callers_can_see_overflow() {
        let dir = tempfile::tempdir().unwrap();
        let mut zip = open_zip(&zip_with(&dir, &[("a.txt", b"hello world")])).unwrap();
        assert_eq!(read_limited(&mut zip, 0, 5).unwrap(), b"hello ");
        assert_eq!(read_limited(&mut zip, 0, 11).unwrap(), b"hello world");
        assert_eq!(read_limited(&mut zip, 0, 1000).unwrap(), b"hello world");
        assert!(read_limited(&mut zip, 7, 5).is_err(), "없는 인덱스는 오류다");
    }

    #[test]
    fn zip_directory_bounds_reads_the_end_of_central_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = zip_with(&dir, &[("a.txt", b"x"), ("b.txt", b"y")]);
        let (entries, bytes) = zip_directory_bounds(&path).unwrap();
        assert_eq!(entries, 2);
        assert!(bytes > 0);
        assert_eq!(directory_limit_note(&path), None);
    }

    #[test]
    fn zip_directory_bounds_is_none_for_files_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("a.txt");
        std::fs::write(&text, b"plain text, no zip trailer").unwrap();
        assert_eq!(zip_directory_bounds(&text), None);
        assert_eq!(zip_directory_bounds(&dir.path().join("none.zip")), None);
        assert_eq!(directory_limit_note(&text), None);
    }

    #[test]
    fn a_zip64_trailer_supplies_the_real_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.zip");
        std::fs::write(&path, zip64_tail(2_000_000, 100, b"PK\x06\x06")).unwrap();
        assert_eq!(zip_directory_bounds(&path), Some((2_000_000, 100)));
        let note = directory_limit_note(&path).unwrap();
        assert!(note.starts_with("Too many entries: 2000000"), "{note}");
    }

    #[test]
    fn a_huge_central_directory_is_refused_even_with_few_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.zip");
        std::fs::write(&path, zip64_tail(10, MAX_DIRECTORY_BYTES + 1, b"PK\x06\x06")).unwrap();
        let note = directory_limit_note(&path).unwrap();
        assert!(note.starts_with("Central directory too large"), "{note}");
    }

    #[test]
    fn a_zip64_locator_pointing_at_a_wrong_record_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.zip");
        std::fs::write(&path, zip64_tail(2_000_000, 100, b"XXXX")).unwrap();
        assert_eq!(zip_directory_bounds(&path), None);
    }
}
