#[path = "../shared/book_fixtures.rs"]
mod fx;

#[path = "common/mod.rs"]
mod common;
use crate::common::pdf_edit_env::env;
use crate::common::shared_fixtures::FetchImage;
use crate::common::shared_fixtures::{error_text, split};
use book_manager::{BookManager, BookManagerDeps};
use common::edits;
use common::image::assert_approx;
use common::pdf::{edit_pdf, first_image_size, first_xobject, first_xobject_mut, load, mediabox_of, page_dict, page_dict_mut, srgb_profile};
use fx::{Edit, FetchBehavior, Opts, image_env, pixel_env};
use image_processor::image_fetcher::FetchError;
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use serde_json::json;
use std::fs;
use std::sync::Arc;

#[tokio::test]
async fn custom_trash_directory_is_used() {
    let e = env();
    let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_trash_dirname("backup"));
    let result = bm.delete_pdf_pages(1, &[1], None, None, None, None).await.unwrap();
    assert_eq!(result.backup, "backup/book.pdf");
    assert!(e.root.path().join("backup/book.pdf").is_file());
}

#[tokio::test]
async fn url_preview_is_the_same_jpeg_that_gets_inserted() {
    let e = env();
    let fetcher = Arc::new(FetchImage(Ok(common::image::png_solid(80, 160, [10, 20, 30]))));
    let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_image_fetcher(fetcher));
    let preview = bm.preview_insert_image("  https://example.com/image.png  ").await.unwrap();
    let inserts = [json!({"page":1,"url":"https://example.com/image.png"})];
    bm.delete_pdf_pages(1, &[], None, None, Some(&inserts), None).await.unwrap();
    assert_eq!(common::pdf::first_xobject(&common::pdf::load(&e.pdf), 1).content, preview);
}

#[tokio::test]
async fn url_preview_reports_safe_fetch_errors() {
    use image_processor::image_fetcher::FetchError;
    for (effect, expected) in [
        (FetchError::Value("내부 주소의 이미지는 사용할 수 없습니다".into()), "내부 주소의 이미지는 사용할 수 없습니다"),
        (FetchError::Os("dns error".into()), "이미지 서버 주소를 확인하지 못했습니다"),
        (FetchError::Other("private details".into()), "이미지를 가져오지 못했습니다"),
    ] {
        let e = env();
        let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_image_fetcher(Arc::new(FetchImage(Err(effect)))));
        assert_eq!(bm.preview_insert_image("https://example.com/image.png").await, Err(expected.into()));
    }
    let e = env();
    for url in ["", "  ", "http://127.0.0.1/a", "ftp://example.com/a"] {
        let (image, error) = split(e.bm.preview_insert_image(url).await);
        assert!(image.is_none() && error.is_some());
    }
}

mod upload_limits {
    use super::{common, fx};
    use crate::common::shared_fixtures::{error_text, split};
    use common::image::png_solid;
    use cover_manager::epub_cover::{EpubCoverHooks, Limits};
    use fx::{Opts, image_env, with_deps};

    /// `fx::env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
    // TODO(common): `book_fixtures::Opts`가 Deps 설정 훅(`configure: Option<Box<dyn FnOnce(BookManagerDeps) -> BookManagerDeps>>`)을 받으면 이 헬퍼가 필요 없다.
    // ── 올린 이미지 ──

    #[tokio::test]
    async fn test_upload_preview_rejects_too_many_pixels() {
        let e = with_deps(image_env(Opts::plain()), false, |d| d.with_epub_hooks(EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() })));

        let (jpeg, error) = split(e.bm.preview_insert_upload(&png_solid(20, 20, [10, 120, 200])).await);

        assert!(jpeg.is_none() && error_text(&error).contains("해상도가 너무 큽니다"), "{error:?}");
    }
}

#[tokio::test]
async fn test_upload_preview_rejects_oversized_data() {
    let e = image_env(Opts::plain());

    let (jpeg, error) = split(e.bm.preview_insert_upload(&vec![0u8; image_processor::MAX_IMAGE_BYTES + 1]).await);

    assert!(jpeg.is_none() && error_text(&error).contains("큽니다"), "{error:?}");
}

mod backup_reporting {
    use super::fx;
    use crate::common::shared_fixtures::error_text;
    use fx::Opts;
    use search_manager::Docs;

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
}

// ---- 이미지 URL 받기 실패 (cover_manager direct_cover_url 테스트에서 옮김) ----

#[tokio::test]
async fn test_fetch_failure_changes_nothing() {
    let cases = [
        (FetchError::Value("공개 HTTP/HTTPS 이미지 URL만 사용할 수 있습니다".into()), "1쪽 앞에 넣을 이미지를 가져올 수 없습니다"),
        (FetchError::Os("Name or service not known".into()), "이미지 서버 주소를 확인하지 못했습니다"),
    ];
    for (effect, expected) in cases {
        let e = image_env(Opts::loader_fetch());
        let original = fs::read(&e.pdf).unwrap();
        e.fetch.set(FetchBehavior::Err(effect));

        let (result, error) = e.run(&Edit { inserts: Some(vec![edits::insert(1)]), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains(expected), "{expected}: {error:?}");
        assert_eq!(fs::read(&e.pdf).unwrap(), original, "{expected}");
        assert!(!e.root.join("0_trash").exists(), "{expected}");
        assert!(e.es.deleted().is_empty(), "{expected}");
    }
}

#[tokio::test]
async fn test_preview_reports_fetch_failures() {
    let cases = [
        (FetchError::Value("내부 주소의 이미지는 사용할 수 없습니다".into()), "내부 주소의 이미지는 사용할 수 없습니다"),
        (FetchError::Os("Name or service not known".into()), "이미지 서버 주소를 확인하지 못했습니다"),
        (FetchError::Other("boom".into()), "이미지를 가져오지 못했습니다"),
    ];
    for (effect, expected) in cases {
        let e = image_env(Opts::fetch());
        e.fetch.set(FetchBehavior::Err(effect));

        let (jpeg, error) = split(e.bm.preview_insert_image("https://img.example.com/a.png").await);

        assert!(jpeg.is_none() && error.as_deref() == Some(expected), "{expected}: {error:?}");
    }
}

// ---- PDF 자르기·쪽 삭제 실패 처리 (pdf_page_reader 테스트 파일에서 옮김) ----

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

fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
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

    assert!(error.is_none() && result.unwrap().cropped_pages == 1, "{error:?}");
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

    assert!(error.is_none() && result.unwrap().cropped_pages == 1, "{error:?}");
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

    assert!(error.is_none() && result.unwrap().cropped_pages == 1, "{error:?}");
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

    assert!(error.is_none() && result.unwrap().cropped_pages == 1, "{error:?}");
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
async fn test_es_delete_failure_stops_before_reindexing() {
    let e = fx::env(Opts::loader());
    e.es.set_delete_result(false);

    let (result, error) = e.run_pages(&[3]).await;

    assert!(result.is_none());
    assert!(error_text(&error).contains("ES 문서 삭제 실패") && error_text(&error).contains("0_trash/book.pdf"), "{error:?}");
    assert!(e.es.inserted().is_empty());
}
