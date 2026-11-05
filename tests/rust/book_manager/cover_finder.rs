//! `book_manager::cover_finder` 단위 테스트.

#[path = "../shared/book_fixtures.rs"]
mod fx;

#[path = "common/mod.rs"]
mod common;
use crate::common::cover_docs::{create_test_pdf, use_doc};
use crate::common::cover_env::{env, split};
use crate::common::shared_fixtures::{URL, has_cover, spine_idrefs, xml_text};
use crate::common::shared_fixtures::{broken_cover, error_text, first_item};
use crate::common::shared_fixtures::{fetcher, thumbnailer};
use book_manager::{BookManager, BookManagerDeps};
use common::epub::BODY;
use common::epub::{EpubSpec, make_epub};
use common::es_fake::FakeEsManager;
use common::fsx::ino_mtime;
use common::image::{b64, red_jpeg};
use common::zipx::Members;
use common::zipx::read_zip_entry;
use cover_manager::cover_management::{CoverManagement, CoverSave};
use cover_manager::cover_sources::{SearchStatus, StoreSearch};
use cover_manager::epub_cover;
use cover_manager::epub_cover::{CoverError, MAX_IMAGE_BYTES};
use fx::{Edit, Opts};
use serde_json::json;
use std::fs::File;

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
        service.set_hooks(service.hooks().clone().with_search(|_store, _metadata| Ok(StoreSearch { status: SearchStatus::NotFound, ..StoreSearch::default() })));
        assert_eq!(service.candidates("yes24", "book.epub", true, false).unwrap().status, SearchStatus::NotFound, "method={method}");
        // 기존 표지 존재 여부와 덮어쓰기 동의는 손상 여부와 별개다.
        let revision = item["revision"].as_str().unwrap();
        let err = epub_cover::add_cover(&path, &red_jpeg(), revision, &root.join(".cover_backups"), false).unwrap_err();
        assert!(err.to_string().contains("이미 표지"), "method={method}: {err}");
        let bad: Result<CoverSave, CoverError> = service.apply_image("book.epub", &b64(b"bad replacement"), revision);
        assert!(bad.is_err(), "method={method}");
        assert_eq!(std::fs::read(&path).unwrap(), original, "method={method}");
        assert!(!root.join(".cover_backups").exists(), "method={method}");
        let result = if method == "image" {
            service.apply_image("book.epub", &b64(&red_jpeg()), revision).unwrap()
        } else {
            service.set_hooks(service.hooks().clone().with_image_fetcher(fetcher(|_url| Ok(red_jpeg()))));
            service.apply_url("book.epub", "https://example.org/cover.jpg", revision, true).unwrap()
        };
        assert!(epub_cover::inspect_epub(&path).unwrap().cover_error.is_none(), "method={method}");
        assert_eq!(std::fs::read(root.join(&result.backup_path)).unwrap(), original, "method={method}");
        assert_eq!(ino_mtime(&path), stat, "method={method}");
        assert_eq!(read_zip_entry(&path, "OPS/chapter.xhtml"), BODY.as_bytes(), "method={method}");
    }
}

mod upload_limits {
    use super::{common, fx};
    use crate::common::shared_fixtures::error_text;
    use common::edits;
    use cover_manager::epub_cover::{EpubCoverHooks, Limits};
    use std::fs;

    use common::image::png_solid;
    use fx::{Edit, Opts, image_env, with_deps};

    /// `fx::env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
    // TODO(common): `book_fixtures::Opts`가 Deps 설정 훅(`configure: Option<Box<dyn FnOnce(BookManagerDeps) -> BookManagerDeps>>`)을 받으면 이 헬퍼가 필요 없다.
    #[tokio::test]
    async fn test_upload_insert_with_too_many_pixels_changes_nothing() {
        let e = with_deps(image_env(Opts::loader()), true, |d| d.with_epub_hooks(EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() })));
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { inserts: Some(vec![edits::upload(2, &png_solid(20, 20, [10, 120, 200]))]), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains("해상도가 너무 큽니다"), "{error:?}");
        e.assert_nothing_changed(&original);
    }
}

#[tokio::test]
async fn test_upload_rejects_encoded_data_over_byte_limit_before_decoding() {
    let e = fx::env(Opts::plain());
    let encoded_limit = MAX_IMAGE_BYTES.div_ceil(3) * 4;
    let insert = serde_json::json!({"page": 1, "image_data": "A".repeat(encoded_limit + 1)});

    let (result, error) = e.run(&Edit { inserts: Some(vec![insert]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("이미지가 비어 있거나 너무 큽니다"), "{error:?}");
}

mod download_failures {
    use crate::common::cover_env::env;
    use crate::common::shared_fixtures::URL;
    use crate::common::shared_fixtures::err_has;
    use std::fs;

    use cover_manager::direct_cover_url::FetchError;

    #[tokio::test]
    async fn test_download_failure_returns_the_reason_and_keeps_the_file() {
        let env = env();
        env.fetcher.set(|_| Err(FetchError::Value("내부 주소의 이미지는 사용할 수 없습니다".to_string())));
        let before = fs::read(&env.path).unwrap();

        let (result, error) = env.call(URL).await;

        assert!(result.is_none() && err_has(&error, "내부 주소"));
        assert_eq!(fs::read(&env.path).unwrap(), before);
        assert!(env.es.updates().is_empty(), "update는 불리면 안 된다");
    }

    #[tokio::test]
    async fn test_unexpected_io_error_hides_internal_details() {
        let env = env();
        env.fetcher.set(|_| Err(FetchError::Os("/secret/disk/path failed".to_string())));

        let (result, error) = env.call(URL).await;

        assert!(result.is_none());
        assert!(!err_has(&error, "secret"));
    }
}

// ---- BookManager로 EPUB 표지 바꾸기 (cover_manager 테스트에서 옮김) ----

#[tokio::test]
async fn test_resolves_the_book_from_elasticsearch_without_a_lookup_override() {
    let env = env();
    let relative_path = env.path.strip_prefix(env.dir()).unwrap();
    env.es.set_doc(Some(json!({
        "category": "분류",
        "title": "book",
        "author": "",
        "file_path": relative_path.to_string_lossy(),
        "file_type": "epub",
        "file_size": env.size(),
        "updated_time": "2024-01-01T00:00:00.000000"
    })));
    let manager = BookManager::with_deps(BookManagerDeps::new(env.dir().to_path_buf(), env.es.clone()).with_trash_dirname("0_trash").with_image_fetcher(env.fetcher.clone()));

    let (result, error) = split(manager.set_epub_cover(1, Some(URL), None).await);

    assert!(error.is_none());
    assert!(result.is_some());
    assert!(has_cover(&env.path));
    assert_eq!(env.es.updates().len(), 1);
}

/// 교체로 빠지는 기존 표지는 새 표지 바로 뒤, 본문 앞의 도판으로 남는다.
#[tokio::test]
async fn test_replaced_cover_stays_as_an_illustration_after_the_new_cover() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    let old_cover = Members::read(&env.path).get("OPS/cover.jpg").to_vec();

    let (_, error) = env.call(URL).await;

    assert!(error.is_none());
    let after = Members::read(&env.path);
    assert!(!after.contains("OPS/cover.jpg"));
    let plates: Vec<String> = after.names().into_iter().filter(|name| name.starts_with("OPS/tm-plate-") && name.ends_with(".jpg")).collect();
    assert!(plates.len() == 1 && after.get(&plates[0]) == old_cover);
    let ids = spine_idrefs(&env.path);
    assert!(ids[0].starts_with("tm-cover-page-") && ids[1].starts_with("tm-plate-page-") && ids[2] == "chapter");
    let page = xml_text(after.get(&format!("OPS/{}.xhtml", ids[1])));
    assert!(page.contains(plates[0].strip_prefix("OPS/").unwrap()));
    // 도판은 표지로 오인되지 않는다. 표지는 새 이미지다.
    let mut archive = zip::ZipArchive::new(File::open(&env.path).unwrap()).unwrap();
    assert!(epub_cover::find_epub_cover_member(&mut archive).unwrap().contains("tm-cover-"));
}
