#[path = "../common/mod.rs"]
mod common;

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::common::cover_docs::{create_test_pdf, use_doc};
use crate::common::shared_fixtures::thumbnailer;
use backend::book_manager::{BookManager, BookManagerApi, BookManagerDeps};
use backend::comics_manager::ComicsManager;
use backend::main::{AppDeps, AuthMode, CategoryMappingApi, MappingError, app_with};
use common::{
    TestClient,
    epub::{EpubSpec, make_epub},
    es_fake::FakeEsManager,
    fsx::{set_times_ns, times_ns},
};
use serde_json::{Value, json};

#[test]
fn test_epubveri_cli_writes_epubcheck_compatible_json_for_valid_and_invalid_books() {
    let tmp = tempfile::tempdir().unwrap();
    let valid_path = tmp.path().join("valid.epub");
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="id">urn:uuid:123e4567-e89b-12d3-a456-426614174000</dc:identifier><dc:title>Rust wrapper</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2024-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let nav = r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc"><ol><li><a href="ch1.xhtml">Chapter</a></li></ol></nav></body></html>"#;
    let mut valid_spec = EpubSpec::minimal().with_opf(opf).with_extra("nav.xhtml", nav);
    valid_spec.chapters = vec![("ch1.xhtml".into(), "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Text</title></head><body><p>Text</p></body></html>".into())];
    make_epub(&valid_path, &valid_spec);
    let valid_json = tmp.path().join("valid.json");
    let valid = Command::new(env!("CARGO_BIN_EXE_epubveri")).arg(&valid_path).arg("--json").arg(&valid_json).output().unwrap();
    let valid_report: Value = serde_json::from_slice(&std::fs::read(valid_json).unwrap()).unwrap();
    assert_eq!(valid.status.code(), Some(0), "{}", valid_report);
    assert_eq!(valid_report["checker"]["nFatal"], 0);
    assert_eq!(valid_report["publication"]["title"], "Rust wrapper");

    let invalid_path = tmp.path().join("invalid.epub");
    make_epub(&invalid_path, &EpubSpec::minimal().without_mimetype());
    let invalid_json = tmp.path().join("invalid.json");
    let invalid = Command::new(env!("CARGO_BIN_EXE_epubveri")).arg(&invalid_path).arg("--json").arg(&invalid_json).output().unwrap();
    assert_eq!(invalid.status.code(), Some(1), "{}", String::from_utf8_lossy(&invalid.stderr));
    let invalid_report: Value = serde_json::from_slice(&std::fs::read(invalid_json).unwrap()).unwrap();
    let messages = invalid_report["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    assert!(messages.iter().all(|message| message.get("ID").is_some()));
}

#[test]
fn test_epubveri_cli_exits_2_on_bad_arguments_unreadable_books_and_unwritable_reports() {
    let tmp = tempfile::tempdir().unwrap();
    let book = tmp.path().join("book.epub");
    make_epub(&book, &EpubSpec::minimal());
    let run = |args: &[&std::ffi::OsStr]| Command::new(env!("CARGO_BIN_EXE_epubveri")).args(args).output().unwrap();
    let (b, j) = (book.as_os_str(), std::ffi::OsStr::new("--json"));
    let out = tmp.path().join("r.json");
    for args in [vec![], vec![b], vec![b, std::ffi::OsStr::new("--yaml"), out.as_os_str()], vec![b, j], vec![b, j, out.as_os_str(), b]] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&o.stderr).contains("usage: epubveri"), "{args:?}");
    }
    let o = run(&[tmp.path().join("none.epub").as_os_str(), j, out.as_os_str()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("epubveri validation failed"));
    // 리포트 경로가 디렉터리면 쓰지 못한다.
    let o = run(&[b, j, tmp.path().as_os_str()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("failed to write JSON report"));
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
    let token = crate::common::session::token("admin@test.com", "admin", Some("Test Admin"), None);
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
        let client = admin_client(app_with(AppDeps::new(AuthMode::Cookies).with_refresh_token_store(crate::common::session::store()).with_book_manager(env.manager.clone())));

        let response = client.get("/cover/201").await;

        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-type"), Some("image/jpeg"));
    }

    #[tokio::test]
    async fn test_requires_auth() {
        let env = endpoint_env();
        let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_refresh_token_store(crate::common::session::store()).with_book_manager(env.manager.clone())));

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
