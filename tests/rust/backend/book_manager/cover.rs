//! `backend::book_manager::cover` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

#[path = "../common/mod.rs"]
mod common;

// `tests/rust/backend/cover.rs`에서 옮긴 테스트.
mod cover {
    use super::{common};

    use std::path::Path;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use backend::book_manager::{BookManager, BookManagerDeps, Thumbnailer};

    use common::{
        epub::{EpubSpec, make_epub},
        es_fake::FakeEsManager,
        fsx::count_matching,
        image::{paste, png_bytes, solid},
        pdf::{write_blank_pdf, write_image_pdf},
    };
    use image::{DynamicImage, GenericImageView, ImageFormat, Rgb, RgbImage};
    use serde_json::{Value, json};

    /// OPF metadata/manifest 조각과 이미지 멤버를 받아 최소 EPUB을 만든다.
    fn create_test_epub(output_path: &Path, metadata: &str, manifest_items: &str, images: &[(&str, Vec<u8>)]) {
        create_test_epub_with_chapters(output_path, metadata, manifest_items, &["<html><body><p>x</p></body></html>"], images);
    }

    /// `create_test_epub`에 본문 쪽(spine 순서)을 직접 정해서 준다. 쪽 파일 이름은 `ch1.xhtml`, `ch2.xhtml`, … 이다.
    fn create_test_epub_with_chapters(output_path: &Path, metadata: &str, manifest_items: &str, chapters: &[&str], images: &[(&str, Vec<u8>)]) {
        let pages: String = (1..=chapters.len()).map(|n| format!(r#"<item id="ch{n}" href="ch{n}.xhtml" media-type="application/xhtml+xml"/>"#)).collect();
        let spine: String = (1..=chapters.len()).map(|n| format!(r#"<itemref idref="ch{n}"/>"#)).collect();
        let opf = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title>{metadata}</metadata>
  <manifest>
    {pages}
    {manifest_items}
  </manifest>
  <spine>{spine}</spine>
</package>"#
        );
        let mut spec = EpubSpec::cover("3.0", false).with_opf(&opf).with_opf_path("OEBPS/content.opf");
        spec.chapters = chapters.iter().enumerate().map(|(index, html)| (format!("ch{}.xhtml", index + 1), html.to_string())).collect();
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

    #[test]
    fn cover_blank_detection_does_not_merge_sixteen_nearby_colors_into_one_background() {
        let image = RgbImage::from_fn(64, 96, |x, y| {
            if y < 58 {
                let color_x = (x / 16).min(3);
                let color_y = (y / 15).min(3);
                Rgb([64 + color_x as u8 * 4, 64 + color_y as u8 * 4, 64])
            } else {
                Rgb([0, 0, 0])
            }
        });

        assert!(!BookManager::_is_mostly_blank_cover(&DynamicImage::ImageRgb8(image)));
    }

    #[test]
    fn cover_blank_detection_matches_pillow_on_repository_image_fixtures() {
        // Python Pillow 10.4.0 oracle for the repository's actual RGB/GIF/JPEG fixtures.
        let fixtures: [(&str, &[u8], bool); 5] = [
            ("test.png", include_bytes!("../../../books/_image/test.png"), false),
            ("150.gif", include_bytes!("../../../books/_image/150.gif"), false),
            ("151.gif", include_bytes!("../../../books/_image/151.gif"), false),
            ("42sik.jpg", include_bytes!("../../../books/_image/42sik.jpg"), false),
            ("42siks.jpg", include_bytes!("../../../books/_image/42siks.jpg"), false),
        ];
        for (name, bytes, expected) in fixtures {
            let image = image::load_from_memory(bytes).unwrap();
            assert_eq!(BookManager::_is_mostly_blank_cover(&image), expected, "fixture={name}");
        }
    }

    #[test]
    fn cover_blank_detection_matches_pillow_on_synthetic_color_patterns() {
        // Python Pillow 10.4.0 MEDIANCUT + FIND_EDGES oracle, generated from the same patterns.
        let expected = [false, false, false, false, false, false, true, true];
        for seed in 0..32u32 {
            let kind = seed % 8;
            let image = RgbImage::from_fn(64, 96, |x, y| match kind {
                0 => {
                    let delta = ((x * 17 + y * 31 + seed * 13) % 9) as i16 - 4;
                    Rgb([(128 + delta) as u8, (128 - delta) as u8, (128 + delta) as u8])
                }
                1 => {
                    let value = if (x / 8 + y / 12) % 2 == 0 { 220 } else { 20 };
                    Rgb([value, value, value])
                }
                2 if y < 58 => Rgb([64 + (x / 16).min(3) as u8 * 4, 64 + (y / 15).min(3) as u8 * 4, 64]),
                2 => Rgb([0, 0, 0]),
                3 => Rgb([(x * 4 % 256) as u8, (y * 3 % 256) as u8, ((x + y) * 2 % 256) as u8]),
                4 if (x * 7 + y * 11 + seed) % 23 < 3 => Rgb([0, 0, 0]),
                4 => Rgb([255, 255, 255]),
                5 => {
                    let n = (x * 92821 + y * 68917 + seed * 421) % 256;
                    Rgb([n as u8, (n * 37 % 256) as u8, (n * 83 % 256) as u8])
                }
                6 if x < 55 && y < 85 => Rgb([180, 175, 160]),
                6 => Rgb([0, 0, 0]),
                _ => Rgb([(seed * 31 % 256) as u8, (seed * 17 % 256) as u8, (seed * 7 % 256) as u8]),
            });
            assert_eq!(BookManager::_is_mostly_blank_cover(&DynamicImage::ImageRgb8(image)), expected[kind as usize], "seed={seed}");
        }
    }

    #[test]
    fn cover_blank_detection_matches_pillow_after_bilinear_resizing() {
        // Pillow 10.4.0 oracle for 600x900 sources resized to 64x96 with BILINEAR.
        let expected = "10001011100010111000101110001011";
        let results: String = (0..32u32)
            .map(|seed| {
                let kind = seed % 8;
                let image = RgbImage::from_fn(600, 900, |x, y| {
                    let sx = x * 64 / 600;
                    let sy = y * 96 / 900;
                    match kind {
                        0 => {
                            let delta = ((x * 17 + y * 31 + seed * 13) % 9) as i16 - 4;
                            Rgb([(128 + delta) as u8, (128 - delta) as u8, (128 + delta) as u8])
                        }
                        1 => {
                            let value = if (sx / 8 + sy / 12) % 2 == 0 { 220 } else { 20 };
                            Rgb([value, value, value])
                        }
                        2 if sy < 58 => Rgb([64 + (sx / 16).min(3) as u8 * 4, 64 + (sy / 15).min(3) as u8 * 4, 64]),
                        2 => Rgb([0, 0, 0]),
                        3 => Rgb([(x * 4 % 256) as u8, (y * 3 % 256) as u8, ((x + y) * 2 % 256) as u8]),
                        4 if (x * 7 + y * 11 + seed) % 23 < 3 => Rgb([0, 0, 0]),
                        4 => Rgb([255, 255, 255]),
                        5 => {
                            let n = (x * 92821 + y * 68917 + seed * 421) % 256;
                            Rgb([n as u8, (n * 37 % 256) as u8, (n * 83 % 256) as u8])
                        }
                        6 if sx < 55 && sy < 85 => Rgb([180, 175, 160]),
                        6 => Rgb([0, 0, 0]),
                        _ => Rgb([(seed * 31 % 256) as u8, (seed * 17 % 256) as u8, (seed * 7 % 256) as u8]),
                    }
                });
                if BookManager::_is_mostly_blank_cover(&DynamicImage::ImageRgb8(image)) { '1' } else { '0' }
            })
            .collect();
        assert_eq!(results, expected);
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
        async fn cover_slot_io_error_returns_503_without_running_extractor() {
            let e = env();
            std::fs::write(e.dir().join("source.pdf"), b"pdf").unwrap();
            std::fs::write(e.dir().join(".cover_cache"), b"not a directory").unwrap();
            use_doc(&e.es, "source.pdf", "pdf");
            let manager = BookManager::with_deps(BookManagerDeps::new(e.dir().to_path_buf(), e.es.clone()).with_thumbnailer(thumbnailer(|_, _, _| panic!("must not run without a slot"))));
            assert_eq!(manager.get_cover(1, None).await.status_code, 503);
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn concurrent_cover_generation_stays_at_ten_across_managers_after_cancellation() {
            let e = env();
            std::fs::write(e.dir().join("slow.pdf"), b"pdf").unwrap();
            use_doc(&e.es, "slow.pdf", "pdf");
            let active = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let started = Arc::new(AtomicUsize::new(0));
            let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
            let extractor = {
                let (active, peak, started, release) = (active.clone(), peak.clone(), started.clone(), release.clone());
                thumbnailer(move |_, _, _| {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    started.fetch_add(1, Ordering::SeqCst);
                    let (lock, ready) = &*release;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        let (next, timeout) = ready.wait_timeout(released, std::time::Duration::from_secs(5)).unwrap();
                        released = next;
                        if timeout.timed_out() {
                            break;
                        }
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                    Some(b"cover".to_vec())
                })
            };
            let first = Arc::new(BookManager::with_deps(BookManagerDeps::new(e.dir().to_path_buf(), e.es.clone()).with_thumbnailer(extractor.clone())));
            let second = Arc::new(BookManager::with_deps(BookManagerDeps::new(e.dir().to_path_buf(), e.es.clone()).with_thumbnailer(extractor)));
            let mut tasks = Vec::new();
            for id in 0..10 {
                let manager = first.clone();
                tasks.push(tokio::spawn(async move { manager.get_cover(id, None).await }));
            }
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while started.load(Ordering::SeqCst) < 10 {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            // HTTP handler를 버려도 이미 시작한 blocking 추출은 끝날 때까지 슬롯을 쥐어야 한다.
            for task in &tasks {
                task.abort();
            }
            for id in 10..20 {
                let manager = second.clone();
                tasks.push(tokio::spawn(async move { manager.get_cover(id, None).await }));
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let before_release = started.load(Ordering::SeqCst);
            *release.0.lock().unwrap() = true;
            release.1.notify_all();
            for task in tasks {
                let _ = task.await;
            }
            assert_eq!(before_release, 10, "취소된 추출과 다음 관리자 요청이 겹치면 안 된다");
            assert_eq!(started.load(Ordering::SeqCst), 20);
            assert_eq!(peak.load(Ordering::SeqCst), 10);
        }

        #[tokio::test]
        async fn test_pdf_pixel_stream_formats_and_decode_errors() {
            // pypdfium2 renders malformed Flate data as black and ignores mismatched image dimensions.
            for (kind, expected_status) in [("rgb", 200), ("gray", 200), ("rgb_png", 200), ("corrupt", 200), ("cmyk", 200), ("smask", 200)] {
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
        async fn test_pdf_pixel_stream_size_mismatch_matches_pdfium_rendering() {
            let e = env();
            let path = e.dir().join("pixels.pdf");
            common::pdf::make_pixel_pdf(&path, &["rgb"]);
            common::pdf::edit_pdf(&path, |doc| {
                let image = common::pdf::first_xobject_mut(doc, 1);
                image.dict.set("Width", 1);
                image.dict.set("Height", 1);
            });
            use_doc(&e.es, "pixels.pdf", "pdf");
            assert_eq!(e.manager.get_cover(1, None).await.status_code, 200);
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

        const FIGURE: &str = r#"<item id="i1" href="fig.png" media-type="image/png"/>"#;
        const FIGURE_PAGE: &str = r#"<html><body><img src="fig.png"/></body></html>"#;
        const TEXT_PAGE: &str = "<html><body><p>x</p></body></html>";

        #[tokio::test]
        async fn test_epub_without_explicit_cover_uses_first_spine_image() {
            let e = env();
            create_test_epub_with_chapters(&e.dir().join("test_category/first.epub"), "", FIGURE, &[FIGURE_PAGE], &[("fig.png", png_bytes(600, 900))]);
            use_doc(&e.es, "test_category/first.epub", "epub");

            let response = e.manager.get_cover(140, None).await;

            assert_eq!(response.status_code, 200);
            assert_eq!(response.media_type.as_deref(), Some("image/jpeg"));
            assert_jpeg_thumbnail(&response.body);
        }

        #[tokio::test]
        async fn test_explicit_cover_wins_over_first_spine_image() {
            let e = env();
            let manifest = r#"<item id="front" href="front.png" media-type="image/png" properties="cover-image"/><item id="i1" href="fig.png" media-type="image/png"/>"#;
            let images = [("front.png", png_bytes(600, 900)), ("fig.png", png_bytes(900, 600))];
            create_test_epub_with_chapters(&e.dir().join("test_category/explicit.epub"), "", manifest, &[FIGURE_PAGE], &images);
            use_doc(&e.es, "test_category/explicit.epub", "epub");

            let response = e.manager.get_cover(141, None).await;

            assert_eq!(response.status_code, 200);
            let img = decode(&response.body);
            assert!(img.height() > img.width(), "세로 표지가 아니라 본문 그림(가로)을 썼다: {}x{}", img.width(), img.height());
        }

        #[tokio::test]
        async fn test_epub_image_after_page_limit_is_not_a_cover() {
            let e = env();
            let mut chapters = vec![TEXT_PAGE; 5];
            chapters.push(FIGURE_PAGE);
            create_test_epub_with_chapters(&e.dir().join("test_category/late.epub"), "", FIGURE, &chapters, &[("fig.png", png_bytes(600, 900))]);
            use_doc(&e.es, "test_category/late.epub", "epub");

            assert_eq!(e.manager.get_cover(142, None).await.status_code, 404);
        }

        #[tokio::test]
        async fn test_epub_image_on_fifth_page_is_still_a_cover() {
            let e = env();
            let mut chapters = vec![TEXT_PAGE; 4];
            chapters.push(FIGURE_PAGE);
            create_test_epub_with_chapters(&e.dir().join("test_category/fifth.epub"), "", FIGURE, &chapters, &[("fig.png", png_bytes(600, 900))]);
            use_doc(&e.es, "test_category/fifth.epub", "epub");

            assert_eq!(e.manager.get_cover(143, None).await.status_code, 200);
        }

        #[tokio::test]
        async fn test_epub_page_image_missing_from_zip_is_not_a_cover() {
            let e = env();
            create_test_epub_with_chapters(&e.dir().join("test_category/dangling.epub"), "", FIGURE, &[FIGURE_PAGE], &[]);
            use_doc(&e.es, "test_category/dangling.epub", "epub");

            assert_eq!(e.manager.get_cover(144, None).await.status_code, 404);
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
            assert_eq!(img.width(), 200);
            let expected = (200.0_f64 * 792.0 / 612.0).round() as i64;
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
            assert_eq!((img.width(), img.height()), (200, 267));
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
            assert_eq!((img.width(), img.height()), (200, 300));
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
                assert_eq!((img.width(), img.height()), (200, 300), "background={background:?}");
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
            assert!(red.max(green).max(blue) < 150, "({red}, {green}, {blue})");
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
            for (requested, expected) in [(Some(100), 100), (Some(200), 200), (None, 200), (Some(0), 200), (Some(300), 200), (Some(-1), 200)] {
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

            let small = e.manager.get_cover(131, Some(100)).await;
            let large = e.manager.get_cover(131, Some(200)).await;

            assert!(decode(&small.body).width() <= 100);
            assert!(decode(&large.body).width() > 100);
            assert_eq!(count_matching(&e.dir().join(".cover_cache"), &format!("131.v{}.w", BookManager::COVER_CACHE_VERSION), ".jpg"), 2);
        }

        #[test]
        fn test_normalize_cover_width_rejects_non_int() {
            assert_eq!(BookManager::normalize_cover_width(json!(100).as_i64()), 100);
            assert_eq!(BookManager::normalize_cover_width(json!("100").as_i64()), BookManager::COVER_THUMBNAIL_WIDTH);
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
}

// `tests/rust/backend/cover_repair.rs`에서 옮긴 테스트.
mod cover_repair {
    use super::{common};

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
}
