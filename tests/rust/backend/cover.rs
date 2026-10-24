//! 원본: tests/test_cover.py
//!
//! `BookManager::get_cover` 단위 테스트와 `/cover/{book_id}` 엔드포인트 테스트. Docker/ES 없이 가짜 ES로 돈다.
//! Python 모듈 스코프 fixture(`temp_dir`, `book_manager_module`)는 테스트마다 새 임시 디렉터리와 관리자로 옮긴다(테스트 사이에 공유 상태가 없다).
//!
//! 실제 PDF·EPUB과 주입한 Thumbnailer로 표지 선택·크기·캐시 revision을 검증한다.
//! HTTP 테스트는 viewer 카테고리 정책과 인증을 실제 라우터에서 확인한다.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use backend::auth::create_jwt_token;
use backend::book_manager::{BookManager, BookManagerApi, BookManagerDeps, Thumbnailer};
use backend::comics_manager::ComicsManager;
use backend::main::{AppDeps, AuthMode, CategoryMappingApi, MappingError, app_with};
use common::{
    TestClient,
    epub::{EpubSpec, make_epub},
    es_fake::FakeEsManager,
    fsx::{count_matching, set_times_ns, times_ns},
    image::{paste, png_bytes, solid},
    pdf::{write_blank_pdf, write_image_pdf},
};
use image::{GenericImageView, ImageFormat};
use serde_json::{Value, json};

/// OPF metadata/manifest 조각과 이미지 멤버를 받아 최소 EPUB을 만든다.
fn create_test_epub(output_path: &Path, metadata: &str, manifest_items: &str, images: &[(&str, Vec<u8>)]) {
    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title>{metadata}</metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    {manifest_items}
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#
    );
    let mut spec = EpubSpec::cover("3.0", false).with_opf(&opf).with_opf_path("OEBPS/content.opf");
    spec.chapters = vec![("ch1.xhtml".to_string(), "<html><body><p>x</p></body></html>".to_string())];
    spec.include_style = false;
    for (name, data) in images {
        spec = spec.with_extra(&format!("OEBPS/{name}"), data.clone());
    }
    make_epub(output_path, &spec);
}

fn create_test_pdf(output_path: &Path) {
    write_blank_pdf(output_path, 612, 792);
}

fn create_test_spread_pdf(output_path: &Path) {
    let mut spread = solid(1200, 800, [30, 30, 200]);
    paste(&mut spread, (0, 0, 600, 800), [200, 30, 30]);
    write_image_pdf(output_path, &[spread]);
}

fn create_test_webtoon_pdf(output_path: &Path) {
    let mut webtoon = solid(600, 2400, [30, 30, 200]);
    paste(&mut webtoon, (0, 0, 600, 900), [200, 30, 30]);
    write_image_pdf(output_path, &[webtoon]);
}

fn create_test_pdf_with_blank_first_cover_area(output_path: &Path, background: [u8; 3]) {
    let first_page = solid(600, 900, background);
    let second_page = solid(600, 900, [30, 30, 200]);
    write_image_pdf(output_path, &[first_page, second_page]);
}

fn create_test_pdf_with_detailed_first_cover(output_path: &Path) {
    let mut first_page = solid(600, 900, [255, 255, 255]);
    for x in (0..600).step_by(20) {
        paste(&mut first_page, (x, 0, x + 2, 900), [0, 0, 0]);
    }
    for y in (0..900).step_by(20) {
        paste(&mut first_page, (0, y, 600, y + 2), [0, 0, 0]);
    }
    let second_page = solid(600, 900, [30, 30, 200]);
    write_image_pdf(output_path, &[first_page, second_page]);
}

fn make_doc(relative_path: &str, file_type: &str) -> Value {
    json!({"category": "test_category", "title": "Test Book", "author": "Author", "file_path": relative_path, "file_type": file_type, "file_size": 1, "line_count": 0, "page_count": 1, "isbn": "", "summary": "test", "updated_time": "2024-01-01T00:00:00.000000"})
}

fn assert_jpeg_thumbnail(body: &[u8]) {
    assert_eq!(image::guess_format(body).unwrap(), ImageFormat::Jpeg);
    let img = image::load_from_memory(body).unwrap();
    assert!(img.width() <= 300, "너비 {}", img.width());
}

fn decode(body: &[u8]) -> image::DynamicImage {
    image::load_from_memory(body).unwrap()
}

fn pixel(img: &image::DynamicImage, x: u32, y: u32) -> (u8, u8, u8) {
    let p = img.get_pixel(x, y);
    (p[0], p[1], p[2])
}

/// 클로저를 `Thumbnailer`로 감싼다.
// TODO(common): `common`에 `Thumbnailer` 클로저 어댑터가 없다(docs/rust-test-seams.md 3.12는 trait 이름만 정한다). 생기면 지운다.
struct FnThumbnailer<F>(F);

impl<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync> Thumbnailer for FnThumbnailer<F> {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        (self.0)(path, file_type, width)
    }
}

fn thumbnailer<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync + 'static>(f: F) -> Arc<dyn Thumbnailer> {
    Arc::new(FnThumbnailer(f))
}

fn use_doc(es: &FakeEsManager, relative_path: &str, file_type: &str) {
    es.set_doc(Some(make_doc(relative_path, file_type)));
}

struct Env {
    tmp: tempfile::TempDir,
    es: Arc<FakeEsManager>,
    manager: BookManager,
}

impl Env {
    fn dir(&self) -> &Path {
        self.tmp.path()
    }
}

/// Python `book_manager_module` fixture: ESManager를 대역으로 바꾼 BookManager.
fn env() -> Env {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()));
    Env { tmp, es, manager }
}

mod test_get_cover {
    use super::*;

    #[tokio::test]
    async fn test_pdf_pixel_stream_formats_and_decode_errors() {
        for (kind, expected_status) in [("rgb", 200), ("gray", 200), ("rgb_png", 200), ("corrupt", 404), ("cmyk", 404), ("smask", 404)] {
            let e = env();
            let path = e.dir().join("pixels.pdf");
            common::pdf::make_pixel_pdf(&path, &[kind]);
            use_doc(&e.es, "pixels.pdf", "pdf");
            let response = e.manager.get_cover(1, None).await;
            assert_eq!(response.status_code, expected_status, "{kind}");
            if expected_status == 200 {
                assert_jpeg_thumbnail(&response.body);
            }
        }
    }

    #[tokio::test]
    async fn test_pdf_pixel_stream_size_mismatch_is_rejected() {
        let e = env();
        let path = e.dir().join("pixels.pdf");
        common::pdf::make_pixel_pdf(&path, &["rgb"]);
        common::pdf::edit_pdf(&path, |doc| {
            let image = common::pdf::first_xobject_mut(doc, 1);
            image.dict.set("Width", 1);
            image.dict.set("Height", 1);
        });
        use_doc(&e.es, "pixels.pdf", "pdf");
        assert_eq!(e.manager.get_cover(1, None).await.status_code, 404);
    }

    #[tokio::test]
    async fn test_epub_cover_image_property() {
        let e = env();
        create_test_epub(
            &e.dir().join("test_category/prop.epub"),
            "",
            r#"<item id="img" href="images/front.png" media-type="image/png" properties="cover-image"/>"#,
            &[("images/front.png", png_bytes(600, 900))],
        );
        use_doc(&e.es, "test_category/prop.epub", "epub");

        let response = e.manager.get_cover(101, None).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(response.media_type.as_deref(), Some("image/jpeg"));
        assert_jpeg_thumbnail(&response.body);
    }

    #[tokio::test]
    async fn test_epub_meta_cover() {
        let e = env();
        create_test_epub(
            &e.dir().join("test_category/meta.epub"),
            r#"<meta name="cover" content="img1"/>"#,
            r#"<item id="img1" href="a.jpg" media-type="image/png"/>"#,
            &[("a.jpg", png_bytes(600, 900))],
        );
        use_doc(&e.es, "test_category/meta.epub", "epub");

        let response = e.manager.get_cover(102, None).await;

        assert_eq!(response.status_code, 200);
        assert_jpeg_thumbnail(&response.body);
    }

    #[tokio::test]
    async fn test_epub_cover_named_item() {
        let e = env();
        create_test_epub(
            &e.dir().join("test_category/named.epub"),
            "",
            r#"<item id="i1" href="img/other.png" media-type="image/png"/><item id="i2" href="img/Cover.png" media-type="image/png"/>"#,
            &[("img/other.png", png_bytes(10, 10)), ("img/Cover.png", png_bytes(600, 900))],
        );
        use_doc(&e.es, "test_category/named.epub", "epub");

        let response = e.manager.get_cover(103, None).await;

        assert_eq!(response.status_code, 200);
        // other.png(10x10)가 아니라 Cover.png(600x900)를 골라야 세로 비율이 나온다
        let img = decode(&response.body);
        assert!(img.height() > img.width());
    }

    #[tokio::test]
    async fn test_epub_without_cover_returns_404() {
        let e = env();
        create_test_epub(&e.dir().join("test_category/nocover.epub"), "", r#"<item id="i1" href="fig.png" media-type="image/png"/>"#, &[("fig.png", png_bytes(600, 900))]);
        use_doc(&e.es, "test_category/nocover.epub", "epub");

        let response = e.manager.get_cover(104, None).await;

        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_broken_epub_returns_404() {
        let e = env();
        let broken = e.dir().join("test_category/broken.epub");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, b"not a zip").unwrap();
        use_doc(&e.es, "test_category/broken.epub", "epub");

        let response = e.manager.get_cover(105, None).await;

        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_pdf_first_page() {
        let e = env();
        create_test_pdf(&e.dir().join("test_category/doc.pdf"));
        use_doc(&e.es, "test_category/doc.pdf", "pdf");

        let response = e.manager.get_cover(106, None).await;

        assert_eq!(response.status_code, 200);
        assert_jpeg_thumbnail(&response.body);
        let img = decode(&response.body);
        assert_eq!(img.width(), 300);
        let expected = (300.0_f64 * 792.0 / 612.0).round() as i64;
        assert!((img.height() as i64 - expected).abs() <= 1, "높이 {}, 기대 {expected}", img.height());
    }

    #[tokio::test]
    async fn test_pdf_spread_uses_left_half_as_cover() {
        let e = env();
        create_test_spread_pdf(&e.dir().join("test_category/spread.pdf"));
        use_doc(&e.es, "test_category/spread.pdf", "pdf");

        let response = e.manager.get_cover(109, None).await;

        assert_eq!(response.status_code, 200);
        let img = decode(&response.body);
        assert_eq!((img.width(), img.height()), (300, 400));
        for x in [img.width() / 4, img.width() * 3 / 4] {
            let (red, _green, blue) = pixel(&img, x, img.height() / 2);
            assert!(red > blue, "x={x}: red={red}, blue={blue}");
        }
    }

    #[tokio::test]
    async fn test_pdf_webtoon_uses_top_cover_area() {
        let e = env();
        create_test_webtoon_pdf(&e.dir().join("test_category/webtoon.pdf"));
        use_doc(&e.es, "test_category/webtoon.pdf", "pdf");

        let response = e.manager.get_cover(110, None).await;

        assert_eq!(response.status_code, 200);
        let img = decode(&response.body);
        assert_eq!((img.width(), img.height()), (300, 450));
        let (red, _green, blue) = pixel(&img, img.width() / 2, img.height() - 10);
        assert!(red > blue, "red={red}, blue={blue}");
    }

    #[tokio::test]
    async fn test_pdf_blank_first_cover_area_uses_second_page() {
        for (background, book_id) in [([255, 255, 255], 111), ([0, 0, 0], 112)] {
            let e = env();
            create_test_pdf_with_blank_first_cover_area(&e.dir().join(format!("test_category/blank-{book_id}.pdf")), background);
            use_doc(&e.es, &format!("test_category/blank-{book_id}.pdf"), "pdf");

            let response = e.manager.get_cover(book_id, None).await;

            assert_eq!(response.status_code, 200, "background={background:?}");
            let img = decode(&response.body);
            assert_eq!((img.width(), img.height()), (300, 450), "background={background:?}");
            let (red, green, blue) = pixel(&img, img.width() / 2, img.height() / 2);
            assert!(blue > red, "background={background:?}");
            assert!(blue > green, "background={background:?}");
        }
    }

    #[tokio::test]
    async fn test_pdf_detailed_first_cover_area_is_kept() {
        let e = env();
        create_test_pdf_with_detailed_first_cover(&e.dir().join("test_category/detailed.pdf"));
        use_doc(&e.es, "test_category/detailed.pdf", "pdf");

        let response = e.manager.get_cover(113, None).await;

        assert_eq!(response.status_code, 200);
        let img = decode(&response.body);
        let (red, green, blue) = pixel(&img, img.width() / 2, img.height() / 2);
        assert!(red.max(green).max(blue) < 100, "({red}, {green}, {blue})");
    }

    #[tokio::test]
    async fn test_text_format_returns_404() {
        let e = env();
        let txt = e.dir().join("test_category/a.txt");
        std::fs::create_dir_all(txt.parent().unwrap()).unwrap();
        std::fs::write(&txt, "hello").unwrap();
        use_doc(&e.es, "test_category/a.txt", "txt");

        let response = e.manager.get_cover(107, None).await;

        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_book_not_found() {
        let e = env();
        e.es.set_doc(None);

        let response = e.manager.get_cover(999, None).await;

        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_cache_hit_skips_extraction() {
        let e = env();
        create_test_pdf(&e.dir().join("test_category/cached.pdf"));
        use_doc(&e.es, "test_category/cached.pdf", "pdf");

        let first = e.manager.get_cover(108, None).await;
        assert_eq!(first.status_code, 200);
        assert_eq!(count_matching(&e.dir().join(".cover_cache"), &format!("108.v{}.", BookManager::COVER_CACHE_VERSION), ".jpg"), 1);

        // 두 번째 호출에서 추출기를 부르면 실패해야 한다.
        let strict = thumbnailer(|_path, _type, _width| panic!("must use cache"));
        let second_manager = BookManager::with_deps(BookManagerDeps::new(e.dir().to_path_buf(), e.es.clone()).with_thumbnailer(strict));
        let second = second_manager.get_cover(108, None).await;

        assert_eq!(second.status_code, 200);
    }

    #[tokio::test]
    async fn test_width_tier_is_whitelisted() {
        for (requested, expected) in [(Some(150), 150), (Some(300), 300), (None, 300), (Some(0), 300), (Some(999), 300), (Some(-1), 300)] {
            let e = env();
            create_test_pdf(&e.dir().join("test_category/tier.pdf"));
            use_doc(&e.es, "test_category/tier.pdf", "pdf");

            let response = e.manager.get_cover(130, requested).await;

            assert_eq!(response.status_code, 200, "requested={requested:?}");
            assert!(decode(&response.body).width() <= expected, "requested={requested:?}");
            assert!(count_matching(&e.dir().join(".cover_cache"), &format!("130.v{}.w{expected}.", BookManager::COVER_CACHE_VERSION), ".jpg") > 0, "requested={requested:?}");
        }
    }

    #[tokio::test]
    async fn test_width_tiers_do_not_share_cache() {
        let e = env();
        create_test_pdf(&e.dir().join("test_category/tiers.pdf"));
        use_doc(&e.es, "test_category/tiers.pdf", "pdf");

        let small = e.manager.get_cover(131, Some(150)).await;
        let large = e.manager.get_cover(131, Some(300)).await;

        assert!(decode(&small.body).width() <= 150);
        assert!(decode(&large.body).width() > 150);
        assert_eq!(count_matching(&e.dir().join(".cover_cache"), &format!("131.v{}.w", BookManager::COVER_CACHE_VERSION), ".jpg"), 2);
    }

    #[test]
    fn test_normalize_cover_width_rejects_non_int() {
        assert_eq!(BookManager::normalize_cover_width(json!(150).as_i64()), 150);
        assert_eq!(BookManager::normalize_cover_width(json!("150").as_i64()), BookManager::COVER_THUMBNAIL_WIDTH);
    }
}

/// 한 콘텐츠 종류(책/만화)에 대해 표지 캐시가 원본 파일을 따라가는지 확인한다.
async fn check_cover_cache_tracks_source_file(make: fn(BookManagerDeps) -> Arc<dyn BookManagerApi>, content_type: &str, change: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let extract_calls = Arc::new(AtomicUsize::new(0));
    let calls = extract_calls.clone();
    let extractor = thumbnailer(move |path, _type, _width| {
        calls.fetch_add(1, Ordering::SeqCst);
        std::fs::read(path).ok()
    });
    let manager = make(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_thumbnailer(extractor));
    let mut relative = "여성향/작품A/[0001] 0001화.pdf".to_string();
    let mut source: std::path::PathBuf = tmp.path().join(&relative);
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, b"first cover").unwrap();
    set_times_ns(&source, 1_000_000_000, 1_000_000_000);
    let (original_atime, original_mtime) = times_ns(&source);
    use_doc(&es, &relative, "pdf");

    let first = manager.get_cover(123, None).await;
    assert_eq!(first.body, b"first cover", "{content_type}/{change}");
    assert_eq!(manager.get_cover(123, None).await.body, first.body, "{content_type}/{change}");
    assert_eq!(extract_calls.load(Ordering::SeqCst), 1, "{content_type}/{change}");

    match change {
        "path" => {
            relative = "0_웹툰/작품B/[0001] 0001화.pdf".to_string();
            source = tmp.path().join(&relative);
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(&source, b"other cover").unwrap();
        }
        "replacement" => {
            let replacement = source.with_extension("tmp");
            std::fs::write(&replacement, b"other cover").unwrap();
            std::fs::rename(&replacement, &source).unwrap();
        }
        _ => std::fs::write(&source, b"other cover").unwrap(),
    }
    // 원본보다 cache 생성 시각이 최신인 상태에서도 다른 파일을 구분해야 한다.
    set_times_ns(&source, original_atime, original_mtime);
    use_doc(&es, &relative, "pdf");

    let second = manager.get_cover(123, None).await;

    assert_eq!(second.status_code, 200, "{content_type}/{change}");
    assert_eq!(second.body, b"other cover", "{content_type}/{change}");
    assert_eq!(extract_calls.load(Ordering::SeqCst), 2, "{content_type}/{change}");
}

/// `BookManager`와 `ComicsManager`를 같은 `BookManagerApi`로 다룬다. 구 `CoverManaging` 제네릭을 대신한다.
fn book_manager(deps: BookManagerDeps) -> Arc<dyn BookManagerApi> {
    Arc::new(BookManager::with_deps(deps))
}

fn comics_manager(deps: BookManagerDeps) -> Arc<dyn BookManagerApi> {
    Arc::new(ComicsManager::with_deps(deps))
}

#[tokio::test]
async fn test_cover_cache_tracks_source_file() {
    // ID 재사용, 파일 교체, mtime을 보존한 수정으로 다른 표지를 재사용하지 않는다.
    for content_type in ["book", "comic"] {
        for change in ["path", "replacement", "preserved_mtime"] {
            let make = if content_type == "book" { book_manager } else { comics_manager };
            check_cover_cache_tracks_source_file(make, content_type, change).await;
        }
    }
}

#[tokio::test]
async fn test_cover_ignores_legacy_id_only_cache() {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let relative = "test_category/legacy-cache.pdf";
    create_test_pdf(&tmp.path().join(relative));
    use_doc(&es, relative, "pdf");
    let legacy = tmp.path().join(".cover_cache/124.v6.jpg");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"wrong book cover").unwrap();

    let extractor = thumbnailer(|_path, _type, _width| Some(b"correct cover".to_vec()));
    let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_thumbnailer(extractor));
    let response = manager.get_cover(124, None).await;

    assert_eq!(response.body, b"correct cover");
}

/// `/cover/{book_id}` 엔드포인트를 위한 앱. Python `TestCoverEndpoint.setup_client` fixture.
struct EndpointEnv {
    _tmp: tempfile::TempDir,
    manager: Arc<BookManager>,
}

fn endpoint_env() -> EndpointEnv {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let manager = Arc::new(BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone())));
    create_test_pdf(&tmp.path().join("test_category/endpoint.pdf"));
    use_doc(&es, "test_category/endpoint.pdf", "pdf");
    EndpointEnv { _tmp: tmp, manager }
}

fn admin_client(app: axum::Router) -> TestClient {
    let token = create_jwt_token("admin@test.com", "admin", Some("Test Admin"), None);
    TestClient::with_app(app).cookie("tm_access_token", &token)
}

/// viewer가 접근할 수 없는 카테고리 정책.
struct DenyAccess;

impl CategoryMappingApi for DenyAccess {
    fn get_hidden_categories(&self, _content_type: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec!["test_category".into()])
    }
}

mod test_cover_endpoint {
    use super::*;

    #[tokio::test]
    async fn test_returns_jpeg() {
        let env = endpoint_env();
        let client = admin_client(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(env.manager.clone())));

        let response = client.get("/cover/201").await;

        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-type"), Some("image/jpeg"));
    }

    #[tokio::test]
    async fn test_requires_auth() {
        let env = endpoint_env();
        let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(env.manager.clone())));

        let response = client.get("/cover/201").await;

        assert_eq!(response.status, 401);
    }

    #[tokio::test]
    async fn test_checks_viewer_access() {
        let env = endpoint_env();
        let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::AuthOnly(json!({"role":"viewer"}))).with_book_manager(env.manager.clone()).with_category_mapping(Arc::new(DenyAccess))));

        let response = client.get("/cover/201").await;

        assert_eq!(response.status, 403);
    }
}
