//! `cover_manager::epub_cover::image_editor` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use cover_manager::epub_cover::edit_page_images;
use serde_json::json;

#[test]
fn test_mismatched_tag_count_is_refused() {
    let err = edit_page_images(b"<html><body><img src='a.jpg'/></body></html>", 2, &[(0, "delete", None)]).expect_err("CoverError가 나야 한다");
    assert!(err.to_string().contains("쪽 구조를 해석할 수 없습니다"), "{err}");
}

#[test]
fn test_page_image_edit_preserves_utf8_signature() {
    let raw = b"\xEF\xBB\xBF<html><body><img src='old.jpg'/></body></html>";

    let edited = edit_page_images(raw, 1, &[(0, "split", Some(json!("new image.jpg")))]).unwrap();

    assert!(edited.starts_with(b"\xEF\xBB\xBF"));
    assert!(String::from_utf8_lossy(&edited).contains("src='new%20image.jpg'"));
}

#[test]
fn test_page_image_edit_reads_and_writes_utf16_bom() {
    let text = "<html><body><img src='old.jpg'/></body></html>";
    let raw: Vec<u8> = [vec![0xFF, 0xFE], text.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat();

    let edited = edit_page_images(&raw, 1, &[(0, "delete", None)]).unwrap();

    assert!(edited.starts_with(&[0xFF, 0xFE]));
    let words: Vec<u16> = edited[2..].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    assert!(!String::from_utf16(&words).unwrap().contains("<img"));
}

#[test]
fn test_page_image_edit_round_trips_latin1_meta_charset() {
    let raw = [b"<html><head><meta charset='ISO-8859-1'></head><body>caf".as_slice(), &[0xE9], b"<img src='old.jpg'/></body></html>"].concat();

    let edited = edit_page_images(&raw, 1, &[(0, "delete", None)]).unwrap();

    assert!(edited.windows(4).any(|window| window == b"caf\xE9"));
    assert!(!edited.windows(4).any(|window| window == b"<img"));
}

#[test]
fn test_page_image_edit_rejects_invalid_encoding_declarations() {
    let raw = b"<?xml version='1.0' encoding='windows-1252'?><html><body><img src='a'/></body></html>";

    let error = edit_page_images(raw, 1, &[(0, "delete", None)]).unwrap_err();

    assert!(error.to_string().contains("문자 인코딩을 읽을 수 없음"));
}

#[test]
fn test_page_image_split_does_not_rewrite_data_src_attribute() {
    let raw = b"<html><body><img data-src='keep.jpg' src='old.jpg'/></body></html>";

    let edited = edit_page_images(raw, 1, &[(0, "split", Some(json!("new.jpg")))]).unwrap();
    let edited = String::from_utf8(edited).unwrap();

    assert!(edited.contains("data-src='keep.jpg'"));
    assert!(edited.contains("src='new.jpg'"));
}
