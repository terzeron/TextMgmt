//! 원본: tests/test_cover_repair.py
//!
//! 손상된 기존 표지를 교체하되 새 이미지와 EPUB 구조는 검증한다. Python이 `tests.test_cover_management`에서 가져오던
//! `make_epub`, `image_bytes`, `rewrite_epub_members`는 `common::{epub, image, zipx}`에 있다.
//!
//! 가정한 API (아직 구현 전이다. Python 이름 그대로이고 비공개 `_x`는 `pub fn x`로 본다. 설계: docs/rust-test-seams.md 3.7, 3.12):
//! - `backend::epub_cover::{inspect_epub(path: &Path), inspect_epub_with(path: &Path, hooks: &EpubCoverHooks)}` (`Result<Value, CoverError>`, `has_cover`, `cover_error`, `cover_image`, `revision`)
//! - `backend::epub_cover::add_cover(path: &Path, image: &[u8], revision: &str, backup_dir: &Path, replace_existing: bool) -> Result<Value, CoverError>`
//! - `backend::epub_cover::CoverError` (`impl Display`)
//! - `backend::epub_cover::{Limits, EpubCoverHooks}`: `Limits { max_image_pixels: usize, .. }`(`Default`는 운영 값), `EpubCoverHooks::default().with_limits(Limits)`(Python `MAX_IMAGE_PIXELS` 등 모듈 상수 patch 대체),
//!   `.with_pixel_decode_observer(Arc<dyn Fn() + Send + Sync>)`(이미지 전체 디코딩 때마다 호출. Python `Image.Image.load` patch 대체)
//! - `backend::cover_management::CoverManagement::{new, with_hooks, hooks, set_hooks, missing(directory, cursor, limit, covered: bool), candidates(store, relative, include_covered: bool, refresh: bool), apply_image(relative, image_data: &str, revision), apply_url(relative, url, revision, replace_existing: bool)}`
//!   (모두 `Result<Value, CoverError>`)
//! - `backend::cover_management::CoverHooks::{with_search, with_image_fetcher(Arc<dyn ImageFetcher>)}`
//! - `backend::direct_cover_url::{ImageFetcher, FetchError}`: `trait ImageFetcher: Send + Sync { fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError>; }`

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use backend::cover_management::CoverManagement;
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::epub_cover::{self, CoverError, EpubCoverHooks, Limits};
use common::{
    epub::{BODY, EpubSpec, make_epub},
    es_dummy,
    fsx::ino_mtime,
    image::{b64, red_jpeg},
    zipx::{read_zip_entry, rewrite_epub_members},
};
use serde_json::{Value, json};

/// `common::fx::fetcher`가 생기면 지운다. 클로저를 `ImageFetcher`로 감싼다.
// TODO(common): `common::fx::fetcher(|url| ...)` (docs/rust-test-seams.md 3.7). 아직 없어 로컬로 둔다.
struct FnFetcher<F>(F);

impl<F: Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync> ImageFetcher for FnFetcher<F> {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        (self.0)(url)
    }
}

fn fetcher<F: Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync + 'static>(f: F) -> Arc<dyn ImageFetcher> {
    Arc::new(FnFetcher(f))
}

// TODO(common): `es_dummy::truthy(Option<&Value>)`를 `&Value`로 받는 얇은 래퍼. cover_fixtures::truthy를 대신한다.
fn truthy(value: &Value) -> bool {
    es_dummy::truthy(Some(value))
}

fn broken_cover(tmp: &Path) -> PathBuf {
    let path = make_epub(&tmp.join("book.epub"), &EpubSpec::cover("3.0", true));
    rewrite_epub_members(&path, |members| members.set("OPS/cover.jpg", &b"not a JPEG"[..]));
    path
}

fn first_item(service: &CoverManagement, covered: bool) -> Value {
    service.missing("", "", 60, covered).unwrap()["items"][0].clone()
}

#[test]
fn test_inspection_reads_headers_without_decoding_pixels() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", true));
    let original = std::fs::read(&path).unwrap();
    let byte_size = red_jpeg().len();

    let hooks = EpubCoverHooks::default().with_pixel_decode_observer(Arc::new(|| panic!("목록 검사에서 이미지 전체를 디코딩했다")));
    let result = epub_cover::inspect_epub_with(&path, &hooks).unwrap();
    assert_eq!(result["cover_image"], json!({"width": 120, "height": 180, "byte_size": byte_size}));
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn test_broken_cover_is_repairable_in_both_listing_modes() {
    for covered in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let path = broken_cover(tmp.path());
        let original = std::fs::read(&path).unwrap();
        let item = first_item(&CoverManagement::new(tmp.path()), covered);
        assert_eq!(item["status"], "covered", "covered={covered}");
        assert_eq!(item["has_cover"], true, "covered={covered}");
        assert!(truthy(&item["cover_error"]), "covered={covered}");
        assert!(item["cover_image"].is_null(), "covered={covered}");
        assert_eq!(std::fs::read(&path).unwrap(), original, "covered={covered}");
    }
}

#[test]
fn test_broken_cover_search_and_replacement_preserve_body_and_backup() {
    for method in ["image", "url"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = broken_cover(root);
        let original = std::fs::read(&path).unwrap();
        let stat = ino_mtime(&path);
        let mut service = CoverManagement::new(root);
        let item = first_item(&service, false);
        service.set_hooks(service.hooks().clone().with_search(|_store, _metadata| Ok(json!({"status": "not_found", "candidates": []}))));
        assert_eq!(service.candidates("yes24", "book.epub", true, false).unwrap()["status"], "not_found", "method={method}");
        // 기존 표지 존재 여부와 덮어쓰기 동의는 손상 여부와 별개다.
        let revision = item["revision"].as_str().unwrap();
        let err = epub_cover::add_cover(&path, &red_jpeg(), revision, &root.join(".cover_backups"), false).unwrap_err();
        assert!(err.to_string().contains("이미 표지"), "method={method}: {err}");
        let bad: Result<Value, CoverError> = service.apply_image("book.epub", &b64(b"bad replacement"), revision);
        assert!(bad.is_err(), "method={method}");
        assert_eq!(std::fs::read(&path).unwrap(), original, "method={method}");
        assert!(!root.join(".cover_backups").exists(), "method={method}");
        let result = if method == "image" {
            service.apply_image("book.epub", &b64(&red_jpeg()), revision).unwrap()
        } else {
            service.set_hooks(service.hooks().clone().with_image_fetcher(fetcher(|_url| Ok(red_jpeg()))));
            service.apply_url("book.epub", "https://example.org/cover.jpg", revision, true).unwrap()
        };
        assert!(!truthy(&epub_cover::inspect_epub(&path).unwrap()["cover_error"]), "method={method}");
        assert_eq!(std::fs::read(root.join(result["backup_path"].as_str().unwrap())).unwrap(), original, "method={method}");
        assert_eq!(ino_mtime(&path), stat, "method={method}");
        assert_eq!(read_zip_entry(&path, "OPS/chapter.xhtml"), BODY.as_bytes(), "method={method}");
    }
}

#[test]
fn test_broken_package_remains_a_fatal_error() {
    let tmp = tempfile::tempdir().unwrap();
    let path = broken_cover(tmp.path());
    rewrite_epub_members(&path, |members| members.set("OPS/book.opf", &b"not XML"[..]));
    let item = first_item(&CoverManagement::new(tmp.path()), false);
    assert_eq!(item["status"], "error");
    assert!(!item.as_object().unwrap().contains_key("cover_error"), "{item}");
}

#[test]
fn test_unreadable_existing_cover_metadata_is_a_warning() {
    for kind in ["svg", "pixels"] {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", true));
        let result = if kind == "svg" {
            rewrite_epub_members(&path, |members| {
                members.replace_in("OPS/book.opf", b"cover.jpg", b"cover.svg");
                members.replace_in("OPS/book.opf", b"image/jpeg", b"image/svg+xml");
                members.set("OPS/cover.svg", &b"not SVG"[..]);
            });
            epub_cover::inspect_epub(&path).unwrap()
        } else {
            epub_cover::inspect_epub_with(&path, &EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 1, ..Limits::default() })).unwrap()
        };
        assert!(truthy(&result["has_cover"]), "kind={kind}");
        assert!(truthy(&result["cover_error"]), "kind={kind}");
        assert!(result["cover_image"].is_null(), "kind={kind}");
    }
}
