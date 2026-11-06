//! `cover_manager::epub_cover::image_editor` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use crate::common::epub_book::{Book, book};
use crate::common::epub_fixtures::{assert_refused, full};
use cover_manager::epub_cover::edit_page_images;
use cover_manager::epub_cover::{CoverError, EditImages, EpubCoverHooks, EpubRewrite, Limits, edit_images, edit_images_with, revision};
use serde_json::json;
use std::fs;
use std::sync::Arc;

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

// ---- 편집 요청 거절 (book_manager epub_inspector 테스트에서 옮김) ----

fn edit_with(b: &Book, hooks: &EpubCoverHooks, args: EditImages) -> Result<EpubRewrite, CoverError> {
    edit_images_with(&b.path, &revision(&b.path), &b.trash, EditImages { backup_name: Some("book.epub".to_string()), ..args }, hooks)
}

#[test]
fn test_empty_and_duplicate_requests_are_rejected() {
    let b = book();
    assert_refused(b.edit(EditImages::default()), "편집이 없습니다", "빈 요청");
    assert_refused(b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], deletes: vec![json!("p1")], ..EditImages::default() }), "두 번", "중복 요청");
}

#[test]
fn test_stale_revision_is_rejected_and_backup_is_kept_once() {
    let b = book();
    let stale = edit_images(&b.path, "stale", &b.trash, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() });
    assert_refused(stale, "변경", "stale");
    let original = fs::read(&b.path).unwrap();
    b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
    b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
    assert_eq!(fs::read(b.trash.join("book.epub")).unwrap(), original);
}

#[test]
fn test_oversized_image_is_refused_before_decoding() {
    let b = book();
    let original = fs::read(&b.path).unwrap();
    let observer = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = observer.clone();
    let observe = move || {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    };
    // 바이트 크기: 읽기 전에 거절한다.
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_image_bytes: 10, ..Limits::default() }).with_pixel_decode_observer(Arc::new(observe.clone()));
    assert_refused(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }), "너무 큼", "bytes");
    // 화소 수: 디코딩 전에 거절한다.
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() }).with_pixel_decode_observer(Arc::new(observe));
    assert_refused(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }), "해상도가 너무 큼", "pixels");
    assert_eq!(observer.load(std::sync::atomic::Ordering::SeqCst), 0, "상한을 넘는 이미지를 디코딩했다");
    assert_eq!(fs::read(&b.path).unwrap(), original);
}

#[test]
fn test_oversized_page_document_is_refused() {
    let b = book();
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_xml_bytes: 50, ..Limits::default() });
    assert!(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }).is_err());
}
