//! 원본: tests/test_pdf_delete_pages_gaps.py
//!
//! BookManager.delete_pdf_pages 의 드문 분기(깨진 PDF 구조, 저장 실패, 재색인 실패)를 확인한다.
//! ES/Docker 없이 실행된다. fixture와 PDF 생성 도구는 `test_pdf_delete_pages`와 같은 `common/book_fixtures.rs`를 쓴다.
//!
//! 가정한 API (나머지는 `pdf_delete_pages.rs`와 `common/book_fixtures.rs` 머리말을 본다. 설계는 docs/rust-test-seams.md 3.5, 3.12):
//! - `BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>)`, `.with_book_adder(..)`, `.with_file_reader(..)`, `.with_fs(Arc<dyn FsOps>)`, `.with_epub_hooks(EpubCoverHooks)`
//!   - 구 `with_remove_pdf_pages_with_backup_failure`는 없앴다. 저장 단계 OSError는 `common::faulty_fs::FaultyFs`의 `set_rename`으로 낸다
//!   - 구 `with_normalize_uploaded_image_failure`와 `SaveFailure::Other`(RuntimeError)는 통일 seam에 대응이 없어 해당 테스트만 `#[ignore]`로 둔다
//! - `backend::epub_cover::{EpubCoverHooks, Limits}`: `EpubCoverHooks::default().with_limits(Limits { max_image_pixels, .. })` (`Limits`는 `Default`).
//!   구 `set_max_image_pixels` 가드를 대신한다
//! - `BookManager::preview_insert_upload(&self, data: &[u8]) -> (Option<Vec<u8>>, Option<String>)`
//!
//! `book_fixtures`의 `Env`/`Opts`/`env`/`image_env`/`pixel_env`/`Edit`는 아직 `#[path]`로 쓴다(TODO). 나머지 PDF·이미지·편집 도구는 `common::{pdf, image, edits}`에서 가져온다.
//! `ImageCms.createProfile("sRGB")`는 `common::pdf::srgb_profile()`의 최소 ICC 바이트로 대신한다.

mod common;
#[path = "common/book_fixtures.rs"]
mod fx;

use std::fs;
use std::io;

use backend::book_manager::{BookManager, BookManagerDeps, SaveFailure};
use backend::epub_cover::{EpubCoverHooks, Limits};
use common::edits;
use common::faulty_fs::FaultyFs;
use common::image::{assert_approx, png_solid};
use common::pdf::{edit_pdf, first_image_size, first_xobject, first_xobject_mut, load, mediabox_of, page_dict, page_dict_mut, srgb_profile};
use fx::{Edit, Env, Opts, image_env, pixel_env};
use loader::record::ReadOutcome;
use lopdf::{Dictionary, Document, Object, Stream, dictionary};

type EditFn = fn(&mut Document);

/// `fx::env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
// TODO(common): `book_fixtures::Opts`가 Deps 설정 훅(`configure: Option<Box<dyn FnOnce(BookManagerDeps) -> BookManagerDeps>>`)을 받으면 이 헬퍼가 필요 없다.
fn with_deps(mut e: Env, loader: bool, configure: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Env {
    let mut deps = BookManagerDeps::new(e.root.clone(), e.es.clone()).with_book_adder(e.add_book.clone());
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
async fn test_upload_insert_with_too_many_pixels_changes_nothing() {
    let e = with_deps(image_env(Opts::loader()), true, |d| d.with_epub_hooks(EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() })));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::upload(2, &png_solid(20, 20, [10, 120, 200]))]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("해상도가 너무 큽니다"), "{error:?}");
    e.assert_nothing_changed(&original);
}

#[tokio::test]
#[ignore = "통일 seam에 대응이 없다: _normalize_uploaded_image가 RuntimeError(\"secret detail\")를 내게 바꾸던 with_normalize_uploaded_image_failure는 없앴다"]
async fn test_upload_preview_hides_unexpected_errors() {
    let e = image_env(Opts { normalize_failure: Some("secret detail".to_string()), ..Opts::plain() });

    let (jpeg, error) = e.bm.preview_insert_upload(b"data").await;

    assert!(jpeg.is_none() && error.as_deref() == Some("이미지를 읽지 못했습니다"), "{error:?}");
}

// ── 저장 단계 실패 ──

#[tokio::test]
async fn test_missing_file_is_reported() {
    let e = fx::env(Opts::loader());
    fs::remove_file(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("File not found: book.pdf"), "{error:?}");
    assert!(e.es.deleted.lock().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "백업 단계가 FsOps::rename을 쓰는지와 ENOSPC 문구(os error 접미사 제거)를 구현 후 확정해야 한다"]
async fn test_os_error_while_saving_reports_the_system_reason() {
    let faulty = FaultyFs::new();
    // OSError(28): 저장 단계(0_trash로 백업하는 rename)가 디스크 부족으로 실패한다.
    faulty.set_rename(|_, _| Some(io::Error::from_raw_os_error(28)));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("파일 처리 실패: No space left on device"), "{error:?}");
    e.assert_nothing_changed(&original);
}

#[tokio::test]
#[ignore = "통일 seam에 대응이 없다: FsOps는 io::Error만 내므로 RuntimeError 같은 그 밖의 예외는 만들 수 없다(with_remove_pdf_pages_with_backup_failure는 없앴다)"]
async fn test_unexpected_error_while_saving_names_only_the_exception_type() {
    let e = fx::env(Opts { loader: true, save_failure: Some(SaveFailure::Other { type_name: "RuntimeError".to_string(), message: "internal path /x/y".to_string() }), ..Opts::plain() });
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
    *e.reader.outcome.lock().unwrap() = ReadOutcome::default();

    let (result, error) = e.run_pages(&[3]).await;

    assert!(result.is_none());
    assert!(error_text(&error).contains("재색인할 문서를 읽지 못했습니다") && error_text(&error).contains("0_trash/book.pdf"), "{error:?}");
    assert!(e.add_book.calls.lock().unwrap().is_empty());
    assert!(e.root.join("0_trash").join("book.pdf").is_file());
}

#[tokio::test]
async fn test_es_delete_failure_stops_before_reindexing() {
    let e = fx::env(Opts::loader());
    *e.es.delete_result.lock().unwrap() = false;

    let (result, error) = e.run_pages(&[3]).await;

    assert!(result.is_none());
    assert!(error_text(&error).contains("ES 문서 삭제 실패") && error_text(&error).contains("0_trash/book.pdf"), "{error:?}");
    assert!(e.add_book.calls.lock().unwrap().is_empty());
}
