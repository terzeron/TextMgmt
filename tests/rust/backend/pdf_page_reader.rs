//! `backend::pdf_page_reader` 단위 테스트.

mod common;
#[path = "common/book_fixtures.rs"]
mod fx;
use std::fs;
use common::edits;
use common::image::assert_approx;
use common::pdf::{edit_pdf, first_image_size, first_xobject, first_xobject_mut, load, mediabox_of, page_count_bytes, page_dict, page_dict_mut, srgb_profile};
use fx::{Edit, image_env, Opts, pixel_env};
use lopdf::{dictionary, Dictionary, Document, Object, Stream};
use std::path::Path;
use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering};
use backend::pdf_page_reader::{LopdfPageOps, PageCountCache, PdfCacheLimits, PdfPageOps, PdfReaderCache, PdfReaderHandle};
use crate::common::shared_fixtures::{create_test_pdf, error_text};
use crate::common::pdf_env::{ResponseExt, build};

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

impl ResponseExt for backend::main::BookResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

#[test]
fn pdf_caches_expose_consistent_empty_clear_and_lookup_state() {
    let reader_cache = PdfReaderCache::new(PdfCacheLimits::default());
    assert!(reader_cache.is_empty());
    assert_eq!(reader_cache.len(), 0);
    assert!(!reader_cache.contains("book.pdf"));
    reader_cache.insert_raw("book.pdf", 1, 5, 12);
    assert!(!reader_cache.is_empty());
    assert!(reader_cache.contains("book.pdf"));
    reader_cache.clear();
    assert!(reader_cache.is_empty());

    let page_cache = PageCountCache::new(2);
    let first = ("one.pdf".to_string(), 10);
    let second = ("two.pdf".to_string(), 20);
    assert!(page_cache.is_empty());
    assert!(!page_cache.contains(&first));
    page_cache.insert(first.clone(), 3);
    page_cache.insert(second.clone(), 4);
    assert_eq!(page_cache.get(&first), Some(3));
    assert!(page_cache.contains(&second));
    page_cache.evict();
    assert_eq!(page_cache.len(), 2);
    page_cache.clear();
    assert!(page_cache.is_empty());
}

#[test]
fn lopdf_page_ops_falls_back_to_path_for_a_reader_without_a_document() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("reader-fallback.pdf");
    create_test_pdf(2, &path);
    let reader = PdfReaderHandle::new(path, 2);

    let bytes = LopdfPageOps.extract_pages_from_reader(&reader, 0, 1).unwrap();

    assert_eq!(page_count_bytes(&bytes), 1);
}

/// `_pdf_reader_cache`가 개수가 아니라 바이트로 제한되는지 검증한다.
///
/// cached PdfReader 1개는 PDF 파일 크기와 거의 1:1로 RSS를 차지한다
/// (pod 실측: 134MiB 파일 -> RSS +134.2MiB). 개수만 제한하면 /comics 의 1.5GiB 급
/// PDF 8개가 한 worker 에서 12GiB 를 점유해 노드 메모리를 고갈시킨다.
mod pdf_reader_cache_budget {
    use super::*;

    /// 전체 예산을 넘으면 개수와 무관하게 LRU 로 축출한다.
    #[test]
    fn test_byte_budget_evicts_lru() {
        let e = build(|dir, d| {
            let first = dir.join("test_category").join("budget_0.pdf");
            create_test_pdf(2, &first);
            let one_size = fs::metadata(&first).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits {
                max_file_bytes: Some(one_size + 1),
                max_bytes: Some(one_size * 2 + 1),
                max_entries: Some(100), // 개수는 넉넉히
            })
        });
        let mut pdfs = Vec::new();
        for i in 0..3 {
            let p = e.temp_dir.join("test_category").join(format!("budget_{i}.pdf"));
            create_test_pdf(2, &p);
            pdfs.push(p);
        }

        for p in &pdfs {
            e.bm._get_cached_pdf_reader(p);
        }

        assert_eq!(e.bm.pdf_cache().len(), 2);
        assert!(!e.bm.pdf_cache().contains(&pdfs[0].to_string_lossy())); // 가장 오래된 것이 축출
        assert!(e.bm.pdf_cache().contains(&pdfs[2].to_string_lossy()));
        let total: u64 = pdfs.iter().filter(|p| e.bm.pdf_cache().contains(&p.to_string_lossy())).map(|p| fs::metadata(p).unwrap().len()).sum();
        assert!(total <= e.bm.pdf_cache().limits().max_bytes.unwrap());
    }

    /// 실제 페이지 수 조회와 추출에서 PDF를 읽는 횟수를 센다.
    struct CountingOpener {
        inner: backend::pdf_page_reader::LopdfPageOps,
        opened: Arc<AtomicUsize>,
    }

    impl backend::pdf_page_reader::PdfPageOps for CountingOpener {
        fn page_count(&self, path: &Path) -> Result<usize, backend::pdf_page_reader::PdfError> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            self.inner.page_count(path)
        }

        fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, backend::pdf_page_reader::PdfError> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            self.inner.extract_pages(path, start, end)
        }
    }

    /// 디스크 .preview_cache 히트 시 PdfReader 를 만들지 않아야 한다.
    ///
    /// 예전에는 reader 를 디스크 캐시 확인보다 먼저 만들었기 때문에, 캐시 히트여도
    /// GB급 PDF 를 통째로 파싱했다 (관측된 최장 응답 125초).
    #[tokio::test]
    async fn test_disk_cache_hit_does_not_open_pdf() {
        let opened = Arc::new(AtomicUsize::new(0));
        let counter = opened.clone();
        let e = build(move |_, d| d.with_pdf_pages(Arc::new(CountingOpener { inner: backend::pdf_page_reader::LopdfPageOps, opened: counter })));

        let first = e.bm.get_pdf_pages(1, 1, 2).await;
        assert_eq!(first.status_code, 200);

        // reader 캐시만 비운다. 페이지 수 캐시와 디스크 캐시는 남은 상태.
        e.bm.pdf_cache().clear();
        let before = opened.load(Ordering::SeqCst);
        assert!(before > 0);

        let second = e.bm.get_pdf_pages(1, 1, 2).await;

        assert_eq!(second.status_code, 200);
        assert_eq!(second.header("X-Total-Pages"), Some("5"));
        // `assert not mock_reader.called`: 두 번째 호출은 PDF를 열지 않는다.
        assert_eq!(opened.load(Ordering::SeqCst), before);
    }
}
