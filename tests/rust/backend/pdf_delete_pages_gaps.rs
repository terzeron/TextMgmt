//! 원본: tests/test_pdf_delete_pages_gaps.py
//!
//! BookManager.delete_pdf_pages 의 드문 분기(깨진 PDF 구조, 저장 실패, 재색인 실패)를 확인한다.
//! ES/Docker 없이 실행된다. fixture와 PDF 생성 도구는 `test_pdf_delete_pages`와 같은 `common/book_fixtures.rs`를 쓴다.
//!
//! 실제 `BookManagerDeps` seam에 파일시스템·loader·image limit을 주입해 실패 경로를 검증한다.
//! ENOSPC는 backup 전 sync 단계에서 주입한다. 업로드 pixel limit은 `EpubCoverHooks` 설정을 공유한다.
//!
//! fixture는 `common/book_fixtures.rs`를 쓰고 PDF·이미지·편집 도구는 `common::{pdf, image, edits}`에서 가져온다.
//! `ImageCms.createProfile("sRGB")`는 `common::pdf::srgb_profile()`의 최소 ICC 바이트로 대신한다.

mod common;
#[path = "common/book_fixtures.rs"]
mod fx;

use std::fs;
use std::io;

use backend::book_manager::{BookManager, BookManagerDeps};
use backend::epub_cover::{EpubCoverHooks, Limits, MAX_IMAGE_BYTES};
use backend::es_manager::Docs;
use common::edits;
use common::faulty_fs::FaultyFs;
use common::image::{assert_approx, png_solid};
use common::pdf::{edit_pdf, first_image_size, first_xobject, first_xobject_mut, load, mediabox_of, page_dict, page_dict_mut, srgb_profile};
use fx::{Edit, Env, Opts, image_env, pixel_env};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};

type EditFn = fn(&mut Document);

#[tokio::test]
async fn test_delete_pdf_pages_rejects_malformed_es_book_before_edit() {
    let e = fx::env(Opts::plain());
    let doc = serde_json::json!({
        "title": "T", "author": "A", "file_path": "cat/book.pdf", "file_type": "pdf",
        "file_size": fs::metadata(&e.pdf).unwrap().len(), "page_count": 5, "updated_time": "2024-01-01T00:00:00.000000"
    });
    e.es.set_doc(Some(doc));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run(&Edit { pages: vec![1], ..Edit::default() }).await;

    assert!(result.is_none());
    assert!(error.unwrap().contains("category"));
    e.assert_nothing_changed(&original);
}

/// `fx::env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
// TODO(common): `book_fixtures::Opts`가 Deps 설정 훅(`configure: Option<Box<dyn FnOnce(BookManagerDeps) -> BookManagerDeps>>`)을 받으면 이 헬퍼가 필요 없다.
fn with_deps(mut e: Env, loader: bool, configure: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Env {
    let mut deps = BookManagerDeps::new(e.root.clone(), e.es.clone());
    if loader {
        deps = deps.with_file_reader(e.reader.clone());
    }
    e.bm = BookManager::with_deps(configure(deps));
    e
}

fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
}

fn error_text(error: &Option<String>) -> &str {
    error.as_deref().unwrap_or("")
}

// ── 크롭할 수 없는 PDF 구조 ──

fn no_contents(doc: &mut Document) {
    page_dict_mut(doc, 1).remove(b"Contents");
}

fn missing_image(doc: &mut Document) {
    page_dict_mut(doc, 1).get_mut(b"Resources").unwrap().as_dict_mut().unwrap().set("XObject", Object::Dictionary(Dictionary::new()));
}

fn set_contents(doc: &mut Document, data: &[u8]) {
    let id = doc.add_object(Stream::new(Dictionary::new(), data.to_vec()));
    page_dict_mut(doc, 1).set("Contents", Object::Reference(id));
}

fn bad_matrix(doc: &mut Document) {
    set_contents(doc, b"q abc 0 0 60 0 0 cm /Im0 Do Q");
}

fn partial_cover(doc: &mut Document) {
    set_contents(doc, b"q 50 0 0 30 0 0 cm /Im0 Do Q");
}

fn cmyk_jpeg(doc: &mut Document) {
    first_xobject_mut(doc, 1).dict.set("ColorSpace", name("DeviceCMYK"));
}

fn wrong_declared_size(doc: &mut Document) {
    first_xobject_mut(doc, 1).dict.set("Width", Object::Integer(99));
}

#[tokio::test]
async fn test_crop_rejects_broken_page_structure_and_changes_nothing() {
    let cases: [(&str, EditFn, &str); 6] = [
        ("no_contents", no_contents, "리소스 없음"),
        ("missing_image", missing_image, "이미지를 찾을 수 없음"),
        ("bad_matrix", bad_matrix, "배치 행렬"),
        ("partial_cover", partial_cover, "쪽 전체를 덮지 않음"),
        ("cmyk_jpeg", cmyk_jpeg, "색상 구성"),
        ("wrong_declared_size", wrong_declared_size, "크기 불일치"),
    ];
    for (label, edit, expected) in cases {
        let e = image_env(Opts::loader());
        edit_pdf(&e.pdf, edit);
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(1, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains("1쪽") && error_text(&error).contains(expected), "{label}: {error:?}");
        e.assert_nothing_changed(&original);
    }
}

// ── 크롭 결과의 구조 ──

#[tokio::test]
async fn test_crop_accepts_single_item_filter_array() {
    let e = image_env(Opts::loader());

    edit_pdf(&e.pdf, |doc| {
        first_xobject_mut(doc, 2).dict.set("Filter", Object::Array(vec![name("DCTDecode")]));
    });

    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(2, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["cropped_pages"] == 1, "{error:?}");
    assert_eq!(first_image_size(&e.pdf, 2), (50, 60));
}

#[tokio::test]
async fn test_crop_keeps_icc_color_space_when_channel_count_matches() {
    let e = pixel_env(Opts::loader());

    edit_pdf(&e.pdf, |doc| {
        let profile = doc.add_object(Stream::new(dictionary! {"N" => 3}, srgb_profile()));
        first_xobject_mut(doc, 1).dict.set("ColorSpace", Object::Array(vec![name("ICCBased"), Object::Reference(profile)]));
    });

    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(1, 0.3, 0.2, 0.9, 0.8)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["cropped_pages"] == 1, "{error:?}");
    let doc = load(&e.pdf);
    let image = first_xobject(&doc, 1);
    let cs = image.dict.get(b"ColorSpace").unwrap().as_array().unwrap();
    assert!(matches!(&cs[0], Object::Name(n) if n == b"ICCBased"));
    let (_, profile) = doc.dereference(&cs[1]).unwrap();
    assert_eq!(profile.as_stream().unwrap().dict.get(b"N").unwrap().as_i64().unwrap(), 3);
    assert!(matches!(image.dict.get(b"Filter").unwrap(), Object::Name(n) if n == b"FlateDecode"));
}

fn wrong_image_subtype(doc: &mut Document) {
    first_xobject_mut(doc, 1).dict.set("Subtype", name("Form"));
}

fn unsupported_icc_channel_count(doc: &mut Document) {
    let mut dict = Dictionary::new();
    dict.set("N", 2);
    let profile = doc.add_object(Stream::new(dict, srgb_profile()));
    first_xobject_mut(doc, 1).dict.set("ColorSpace", Object::Array(vec![name("ICCBased"), Object::Reference(profile)]));
}

fn zero_image_width(doc: &mut Document) {
    first_xobject_mut(doc, 1).dict.set("Width", Object::Integer(0));
}

fn unsupported_image_bit_depth(doc: &mut Document) {
    first_xobject_mut(doc, 1).dict.set("BitsPerComponent", Object::Integer(16));
}

fn unfiltered_compressed_pixels(doc: &mut Document) {
    first_xobject_mut(doc, 1).dict.remove(b"Filter");
}

fn indexed_palette_high_too_large(doc: &mut Document) {
    first_xobject_mut(doc, 5).dict.set("ColorSpace", Object::Array(vec![name("Indexed"), name("DeviceRGB"), Object::Integer(256), Object::String(vec![255, 0, 0], lopdf::StringFormat::Hexadecimal)]));
}

fn indexed_lookup_wrong_type(doc: &mut Document) {
    first_xobject_mut(doc, 5).dict.set("ColorSpace", Object::Array(vec![name("Indexed"), name("DeviceRGB"), Object::Integer(1), Object::Integer(7)]));
}

fn indexed_palette_too_short(doc: &mut Document) {
    first_xobject_mut(doc, 5).dict.set("ColorSpace", Object::Array(vec![name("Indexed"), name("DeviceRGB"), Object::Integer(1), Object::String(vec![255, 0, 0], lopdf::StringFormat::Hexadecimal)]));
}

fn indexed_pixel_outside_palette(doc: &mut Document) {
    first_xobject_mut(doc, 5).dict.set("ColorSpace", Object::Array(vec![name("Indexed"), name("DeviceRGB"), Object::Integer(0), Object::String(vec![255, 0, 0], lopdf::StringFormat::Hexadecimal)]));
}

fn malformed_page_box_length(doc: &mut Document) {
    page_dict_mut(doc, 1).set("MediaBox", vec![0.into(), 0.into(), 100.into()]);
}

fn invalid_page_box_bounds(doc: &mut Document) {
    page_dict_mut(doc, 1).set("MediaBox", vec![0.into(), 0.into(), 0.into(), 60.into()]);
}

fn short_image_matrix(doc: &mut Document) {
    set_contents(doc, b"q 100 0 0 60 0 cm /Im0 Do Q");
}

#[tokio::test]
async fn test_crop_rejects_malformed_image_metadata_and_pixel_data() {
    let cases: [(&str, usize, EditFn, &str); 12] = [
        ("wrong_subtype", 1, wrong_image_subtype, "이미지를 찾을 수 없음"),
        ("unsupported_icc_channels", 1, unsupported_icc_channel_count, "지원하지 않는 이미지 색상 구성"),
        ("zero_width", 1, zero_image_width, "이미지 해상도가 너무 큽니다"),
        ("unsupported_bit_depth", 1, unsupported_image_bit_depth, "지원하지 않는 이미지 색상 구성"),
        ("pixel_size_mismatch", 1, unfiltered_compressed_pixels, "이미지 크기 불일치"),
        ("indexed_high_too_large", 5, indexed_palette_high_too_large, "이미지를 해독할 수 없음"),
        ("indexed_lookup_wrong_type", 5, indexed_lookup_wrong_type, "이미지를 해독할 수 없음"),
        ("indexed_palette_too_short", 5, indexed_palette_too_short, "이미지를 해독할 수 없음"),
        ("indexed_pixel_outside_palette", 5, indexed_pixel_outside_palette, "이미지를 해독할 수 없음"),
        ("page_box_wrong_length", 1, malformed_page_box_length, "페이지 크기가 잘못되었습니다"),
        ("page_box_invalid_bounds", 1, invalid_page_box_bounds, "페이지 크기가 잘못되었습니다"),
        ("short_matrix", 1, short_image_matrix, "이미지 배치 행렬을 읽을 수 없음"),
    ];
    for (label, page, edit, expected) in cases {
        let e = pixel_env(Opts::loader());
        edit_pdf(&e.pdf, edit);
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(page as i64, 0.3, 0.2, 0.9, 0.8)]), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains(expected), "{label}: {error:?}");
        e.assert_nothing_changed(&original);
    }
}

#[tokio::test]
async fn test_crop_preserves_single_channel_icc_profile() {
    let e = pixel_env(Opts::loader());

    edit_pdf(&e.pdf, |doc| {
        let mut dict = Dictionary::new();
        dict.set("N", 1);
        let profile = doc.add_object(Stream::new(dict, srgb_profile()));
        first_xobject_mut(doc, 3).dict.set("ColorSpace", Object::Array(vec![name("ICCBased"), Object::Reference(profile)]));
    });

    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(3, 0.3, 0.2, 0.9, 0.8)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["cropped_pages"] == 1, "{error:?}");
    let doc = load(&e.pdf);
    let cs = first_xobject(&doc, 3).dict.get(b"ColorSpace").unwrap().as_array().unwrap();
    assert!(matches!(&cs[0], Object::Name(n) if n == b"ICCBased"));
}

#[tokio::test]
async fn test_crop_accepts_indexed_palette_stream() {
    let e = pixel_env(Opts::loader());

    edit_pdf(&e.pdf, |doc| {
        let palette = doc.add_object(Stream::new(Dictionary::new(), vec![255, 0, 0, 0, 0, 255]));
        first_xobject_mut(doc, 5).dict.set("ColorSpace", Object::Array(vec![name("Indexed"), name("DeviceRGB"), Object::Integer(1), Object::Reference(palette)]));
    });

    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(5, 0.3, 0.2, 0.9, 0.8)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["cropped_pages"] == 1, "{error:?}");
    let doc = load(&e.pdf);
    let image = first_xobject(&doc, 5);
    assert_eq!(image.dict.get(b"Width").unwrap().as_i64().unwrap(), 60);
    assert_eq!(image.dict.get(b"Height").unwrap().as_i64().unwrap(), 36);
}

#[tokio::test]
async fn test_crop_removes_stale_page_boxes() {
    let e = image_env(Opts::loader());

    edit_pdf(&e.pdf, |doc| {
        let page = page_dict_mut(doc, 2);
        for key in ["CropBox", "TrimBox", "BleedBox", "ArtBox"] {
            page.set(key, Object::Array([0, 0, 100, 60].map(Object::Integer).to_vec()));
        }
    });

    let (_, error) = e.run(&Edit { crops: Some(vec![edits::crop(2, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let doc = load(&e.pdf);
    let page = page_dict(&doc, 2);
    for key in ["CropBox", "TrimBox", "BleedBox", "ArtBox"] {
        assert!(!page.has(key.as_bytes()), "{key}가 남았다");
    }
    assert_approx(mediabox_of(&doc, 2)[2], 50.0, "mediabox 너비");
}

// ── 올린 이미지 ──

#[tokio::test]
async fn test_upload_preview_rejects_too_many_pixels() {
    let e = with_deps(image_env(Opts::plain()), false, |d| d.with_epub_hooks(EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() })));

    let (jpeg, error) = e.bm.preview_insert_upload(&png_solid(20, 20, [10, 120, 200])).await;

    assert!(jpeg.is_none() && error_text(&error).contains("해상도가 너무 큽니다"), "{error:?}");
}

#[tokio::test]
async fn test_upload_rejects_encoded_data_over_byte_limit_before_decoding() {
    let e = fx::env(Opts::plain());
    let encoded_limit = MAX_IMAGE_BYTES.div_ceil(3) * 4;
    let insert = serde_json::json!({"page": 1, "image_data": "A".repeat(encoded_limit + 1)});

    let (result, error) = e.run(&Edit { inserts: Some(vec![insert]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("이미지가 비어 있거나 너무 큽니다"), "{error:?}");
}

#[tokio::test]
async fn test_upload_insert_with_too_many_pixels_changes_nothing() {
    let e = with_deps(image_env(Opts::loader()), true, |d| d.with_epub_hooks(EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() })));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::upload(2, &png_solid(20, 20, [10, 120, 200]))]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("해상도가 너무 큽니다"), "{error:?}");
    e.assert_nothing_changed(&original);
}

// ── 저장 단계 실패 ──

#[tokio::test]
async fn test_missing_file_is_reported() {
    let e = fx::env(Opts::loader());
    fs::remove_file(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("File not found: book.pdf"), "{error:?}");
    assert!(e.es.deleted().is_empty());
}

#[tokio::test]
async fn test_os_error_while_saving_reports_the_system_reason() {
    let faulty = FaultyFs::new();
    // OSError(28): 편집 파일을 백업하기 전에 동기화 단계에서 디스크 부족이 난다.
    faulty.set_sync_file(|_| Some(io::Error::from_raw_os_error(28)));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("파일 처리 실패: No space left on device"), "{error:?}");
    e.assert_nothing_changed(&original);
}

#[tokio::test]
async fn test_unexpected_error_while_saving_does_not_leak_panic_details() {
    let faulty = FaultyFs::new();
    faulty.set_sync_file(|_| panic!("internal path /x/y"));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("PDF 페이지 삭제 실패 (RuntimeError)"), "{error:?}");
    e.assert_nothing_changed(&original);
}

// ── 재색인 실패 ──

#[tokio::test]
async fn test_unreadable_saved_file_reports_backup_location() {
    let e = fx::env(Opts::loader());
    // patch("utils.loader.Loader.read_file", return_value={})
    *e.reader.docs.lock().unwrap() = Docs::new();

    let (result, error) = e.run_pages(&[3]).await;

    assert!(result.is_none());
    assert!(error_text(&error).contains("재색인할 문서를 읽지 못했습니다") && error_text(&error).contains("0_trash/book.pdf"), "{error:?}");
    assert!(e.es.inserted().is_empty());
    assert!(e.root.join("0_trash").join("book.pdf").is_file());
}

#[tokio::test]
async fn test_es_delete_failure_stops_before_reindexing() {
    let e = fx::env(Opts::loader());
    e.es.set_delete_result(false);

    let (result, error) = e.run_pages(&[3]).await;

    assert!(result.is_none());
    assert!(error_text(&error).contains("ES 문서 삭제 실패") && error_text(&error).contains("0_trash/book.pdf"), "{error:?}");
    assert!(e.es.inserted().is_empty());
}
