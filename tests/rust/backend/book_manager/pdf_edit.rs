#[path = "../common/book_fixtures.rs"]
mod fx;

#[path = "../common/mod.rs"]
mod common;
use backend::book_manager::{BookManager, BookManagerDeps};
use serde_json::json;
use std::sync::Arc;
use fx::{image_env, Opts};
use crate::common::shared_fixtures::{error_text, split};
use crate::common::shared_fixtures::{FetchImage};
use crate::common::pdf_edit_env::env;

#[tokio::test]
async fn custom_trash_directory_is_used() {
    let e = env();
    let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_trash_dirname("backup"));
    let result = bm.delete_pdf_pages(1, &[1], None, None, None, None).await.unwrap();
    assert_eq!(result["backup"], "backup/book.pdf");
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
    use backend::direct_cover_url::FetchError;
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
    use crate::common::shared_fixtures::{error_text, split};
    use super::{common, fx};
    use backend::epub_cover::{EpubCoverHooks, Limits};
    use common::image::png_solid;
    use fx::{image_env, Opts, with_deps};

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

    let (jpeg, error) = split(e.bm.preview_insert_upload(&vec![0u8; backend::epub_cover::MAX_IMAGE_BYTES + 1]).await);

    assert!(jpeg.is_none() && error_text(&error).contains("큽니다"), "{error:?}");
}

mod backup_reporting {
    use crate::common::shared_fixtures::{error_text};
    use backend::es_manager::{Docs};
    use super::fx;
    use fx::Opts;

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
