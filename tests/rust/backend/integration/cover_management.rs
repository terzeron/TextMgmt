#[path = "../common/mod.rs"]
mod common;
#[path = "../common/cover_fixtures.rs"]
mod cover_fixtures;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::common::shared_fixtures::fetcher;
use crate::common::shared_fixtures::{backups, opf_of, read, rev, str_of, truthy};
use backend::book_manager::{BookManager, BookManagerDeps};
use backend::cover_management::{CoverHooks, CoverManagement, check_cancelled};
use backend::epub_cover::{self, CoverError, OPF_NS};
use backend::main::{AppDeps, AuthMode, app_with};
use common::{
    TestClient,
    clock::Event,
    epub::{BODY, EpubSpec, append_child, count_elements, make_epub},
    es_fake::FakeEsManager,
    fsx::{count_matching, dir_names, ino_mtime},
    image::{BLUE, b64, encode_image, image_size, red_jpeg},
    zipx::{member_names, read_zip_entry, rewrite_epub_members, testzip_ok},
};
use cover_fixtures::{cached_candidates, cover_api};
use image::ImageFormat;
use serde_json::{Value, json};
use zip::CompressionMethod;

/// 항목마다 `keys`의 값을 배열로 묶는다. Python `[(item[a], item[b]) for item in items]`.
fn pick(items: &Value, keys: &[&str]) -> Vec<Value> {
    items.as_array().unwrap().iter().map(|item| Value::Array(keys.iter().map(|k| item[*k].clone()).collect())).collect()
}

/// 실제 쿠키 인증으로 표지 라우터를 올린 앱. 구 `create_cover_router` + `AdminGate` 조립을 `AppDeps`가 대신한다.
fn cover_app(root: &Path) -> axum::Router {
    let manager = Arc::new(BookManager::with_deps(BookManagerDeps::new(root.to_path_buf(), FakeEsManager::new())));
    app_with(AppDeps::new(AuthMode::Cookies).with_refresh_token_store(crate::common::session::store()).with_book_manager(manager))
}

fn admin_client(app: axum::Router) -> TestClient {
    TestClient::with_app(app).cookie("tm_access_token", &crate::common::session::token("admin@test.com", "admin", Some("Test Admin"), None))
}

#[tokio::test]
async fn test_existing_cover_listing_exposes_original_dimensions_and_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let api = cover_api(tmp.path(), CoverHooks::default());
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", true));
    let data = encode_image(2400, 3600, BLUE, ImageFormat::Jpeg);
    rewrite_epub_members(&path, |members| members.set("OPS/cover.jpg", data.clone()));
    let original = read(&path);
    let response = api.client.get("/admin/book-covers/missing?covered=true").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["result"]["items"][0]["cover_image"], json!({"width": 2400, "height": 3600, "byte_size": data.len()}));
    assert_eq!(read(&path), original);
}

#[tokio::test]
async fn test_saved_cover_size_matches_embedded_image() {
    for endpoint in ["apply", "apply-url", "apply-image"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let data = encode_image(900, 1200, BLUE, ImageFormat::Png);
        let served = data.clone();
        let api = cover_api(root, CoverHooks::default().with_image_fetcher(fetcher(move |_url| Ok(served.clone()))));
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
        let service = CoverManagement::new(root);
        let mut body = json!({"file_path": "book.epub", "revision": rev(&path)});
        if endpoint == "apply" {
            let candidate_id = "a".repeat(64);
            let mut record = body.clone();
            record["store"] = json!("yes24");
            service.write(&service.cache().join(format!("candidate-{candidate_id}.json")), record.to_string().as_bytes());
            service.write(&service.cache().join(format!("image-{candidate_id}.jpg")), &data);
            body["candidate_id"] = json!(candidate_id);
        } else if endpoint == "apply-url" {
            body["url"] = json!("https://example.org/cover.png");
        } else {
            body["image_data"] = json!(b64(&data));
        }
        let response = api.client.post_json(&format!("/admin/book-covers/{endpoint}"), &body).await;
        assert_eq!(response.status, 200, "endpoint={endpoint}");
        let name = member_names(&path).into_iter().find(|name| name.contains("tm-cover-") && name.ends_with(".jpg")).unwrap();
        let embedded = read_zip_entry(&path, &name);
        assert_eq!(image_size(&embedded), (600, 800), "endpoint={endpoint}");
        assert_eq!(response.json()["result"]["cover_image"], json!({"width": 600, "height": 800, "byte_size": embedded.len()}), "endpoint={endpoint}");
    }
}

#[tokio::test]
async fn test_covered_listing_mode_filters_pages_and_preserves_epub() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let api = cover_api(root, CoverHooks::default());
    let paths = [
        make_epub(&root.join("0-covered.epub"), &EpubSpec::cover("3.0", true)),
        make_epub(&root.join("1-missing.epub"), &EpubSpec::cover("3.0", false)),
        make_epub(&root.join("2-covered.epub"), &EpubSpec::cover("3.0", true)),
    ];
    std::fs::write(root.join("3-broken.epub"), b"invalid zip").unwrap();
    let originals: Vec<(PathBuf, Vec<u8>)> = paths.iter().map(|p| (p.clone(), read(p))).collect();
    let default = api.client.get("/admin/book-covers/missing?limit=2").await.json()["result"].clone();
    assert_eq!(pick(&default["items"], &["file_path"]), [json!(["1-missing.epub"])]);
    let page = api.client.get("/admin/book-covers/missing?covered=true&limit=2").await.json()["result"].clone();
    assert_eq!(pick(&page["items"], &["file_path", "status", "has_cover"]), [json!(["0-covered.epub", "covered", true])]);
    assert!(page["next_cursor"] == "2" && page["scanned"] == 2 && page["total"] == 4, "{page}");
    let next_page = api.client.get("/admin/book-covers/missing?covered=true&limit=2&cursor=2").await.json()["result"].clone();
    assert_eq!(pick(&next_page["items"], &["file_path", "status"]), [json!(["2-covered.epub", "covered"]), json!(["3-broken.epub", "error"])]);
    assert!(next_page["next_cursor"].is_null());
    assert_eq!(api.client.get("/admin/book-covers/missing?covered=invalid").await.status, 422);
    assert!(originals.iter().all(|(path, original)| &read(path) == original));
    assert!(!backups(root).exists());
}

#[tokio::test]
async fn test_listing_scans_past_160_books_without_recursing_or_modifying_files() {
    for covered in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let inspected = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
        let sink = inspected.clone();
        let api = cover_api(
            root,
            CoverHooks::default().with_inspect_epub(move |path| {
                sink.lock().unwrap().push(path.to_path_buf());
                epub_cover::inspect_epub(path)
            }),
        );
        let selected = root.join("selected");
        let paths: Vec<PathBuf> = (0..241).map(|index| make_epub(&selected.join(format!("book-{index:03}.epub")), &EpubSpec::cover("3.0", if index == 240 { covered } else { !covered }))).collect();
        make_epub(&selected.join("nested").join("nested.epub"), &EpubSpec::cover("3.0", covered));
        make_epub(&root.join("sibling").join("sibling.epub"), &EpubSpec::cover("3.0", covered));
        make_epub(&selected.join(".hidden.epub"), &EpubSpec::cover("3.0", covered));
        std::fs::write(selected.join("other.txt"), "대상 제외").unwrap();
        let originals: Vec<(PathBuf, Vec<u8>)> = paths.iter().map(|p| (p.clone(), read(p))).collect();
        let mut cursor = String::new();
        let mut pages = Vec::new();
        for _ in 0..5 {
            let response = api.client.get(&format!("/admin/book-covers/missing?directory=selected&cursor={cursor}&covered={covered}")).await;
            assert_eq!(response.status, 200, "covered={covered}");
            let page = response.json()["result"].clone();
            cursor = page["next_cursor"].as_str().unwrap_or("").to_string();
            pages.push(page);
        }
        assert_eq!(pages.iter().map(|p| p["scanned"].clone()).collect::<Vec<_>>(), [60, 120, 180, 240, 241].map(|n| json!(n)), "covered={covered}");
        assert_eq!(pages.iter().map(|p| p["next_cursor"].clone()).collect::<Vec<_>>(), [json!("60"), json!("120"), json!("180"), json!("240"), Value::Null], "covered={covered}");
        assert!(pages.iter().all(|p| p["total"] == 241), "covered={covered}");
        assert!(pages[..4].iter().all(|p| p["items"].as_array().unwrap().is_empty()), "covered={covered}");
        assert_eq!(pick(&pages[4]["items"], &["file_path"]), [json!(["selected/book-240.epub"])], "covered={covered}");
        assert_eq!(*inspected.lock().unwrap(), paths, "covered={covered}");
        assert!(originals.iter().all(|(path, original)| &read(path) == original), "covered={covered}");
        assert!(!backups(root).exists(), "covered={covered}");
    }
}

#[tokio::test]
async fn test_existing_cover_listing_can_recommend_and_replace_with_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let hooks = CoverHooks::default()
        .with_search(|_store, _metadata| {
            Ok(json!({"status": "success", "candidates": [{"title": "교체 후보", "image_url": "https://image.yes24.com/cover.jpg", "detail_url": "https://www.yes24.com/book"}]}))
        })
        .with_fetch_bytes(|_store, _url| Ok(red_jpeg()));
    let api = cover_api(root, hooks);
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
    let original = read(&path);
    let item = api.client.get("/admin/book-covers/missing?covered=true").await.json()["result"]["items"][0].clone();
    assert_eq!(api.client.get("/admin/book-covers/candidates/yes24?file_path=book.epub").await.status, 409);
    let candidate = api.client.get("/admin/book-covers/candidates/yes24?file_path=book.epub&include_covered=true").await.json()["result"]["candidates"][0].clone();
    let image = api.client.get(&format!("/admin/book-covers/candidate-image/{}", str_of(&candidate, "candidate_id"))).await;
    let response = api.client.post_json("/admin/book-covers/apply-image", &json!({"file_path": "book.epub", "revision": item["revision"], "image_data": b64(&image.body)})).await;
    assert_eq!(response.status, 200);
    let result = response.json()["result"].clone();
    assert_ne!(result["revision"], item["revision"]);
    assert_eq!(read(&root.join(str_of(&result, "backup_path"))), original);
    assert!(!CoverManagement::new(root).book_cache("book.epub").exists());
    assert_eq!(read_zip_entry(&path, "OPS/chapter.xhtml"), BODY.as_bytes());
}

#[tokio::test]
async fn test_original_cover_preview_uses_saved_600px_image() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let api = cover_api(root, CoverHooks::default());
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let data = encode_image(1200, 1800, BLUE, ImageFormat::Jpeg);
    epub_cover::add_cover(&path, &data, &rev(&path), &backups(root), false).unwrap();
    let original = read(&path);
    let service = CoverManagement::new(root);
    assert_eq!(image_size(&service.preview("book.epub", false).unwrap()).0, 300);
    assert_eq!(image_size(&service.preview("book.epub", true).unwrap()), (600, 900));
    let response = api.client.get("/admin/book-covers/preview?file_path=book.epub&original=true").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("image/jpeg"));
    assert_eq!(image_size(&response.body), (600, 900));
    assert_eq!(read(&path), original);
}

#[test]
fn test_replacement_preserves_unrelated_assets_with_generated_cover_names() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    epub_cover::add_cover(&path, &red_jpeg(), &rev(&path), &backups(root), false).unwrap();
    let assets: [(&str, Vec<u8>); 3] = [
        ("OPS/tm-cover-0000000000000000.jpg", red_jpeg()),
        ("OPS/tm-cover-page-0000000000000000.xhtml", br#"<html xmlns="http://www.w3.org/1999/xhtml"><body>appendix</body></html>"#.to_vec()),
        ("OPS/fonts/font.otf", b"\x00\x01font".to_vec()),
    ];
    // add_cover가 OPF를 기본 네임스페이스(접두사 없음)로 다시 쓴다고 가정하고 문자열로 요소를 붙인다.
    rewrite_epub_members(&path, |members| {
        for (name, content) in &assets {
            members.set(name, content.clone());
        }
        let opf = members.get("OPS/book.opf").to_vec();
        let opf = append_child(
            &opf,
            "manifest",
            r#"<item id="illustration" href="tm-cover-0000000000000000.jpg" media-type="image/jpeg"/><item id="appendix" href="tm-cover-page-0000000000000000.xhtml" media-type="application/xhtml+xml"/>"#,
        );
        let opf = append_child(&opf, "spine", r#"<itemref idref="appendix"/>"#);
        members.set("OPS/book.opf", opf);
    });
    let result = epub_cover::add_cover(&path, &red_jpeg(), &rev(&path), &backups(root), true).unwrap();
    for (name, content) in &assets {
        assert_eq!(&read_zip_entry(&path, name), content, "name={name}");
    }
    assert_eq!(count_elements(&opf_of(&path), "itemref", Some(("idref", "appendix"))), 1);
    assert!(testzip_ok(&path));
    assert!(Path::new(str_of(&result, "backup_path")).is_file());
}

#[test]
fn test_replacement_rejects_external_change_before_creating_another_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let backup_dir = backups(root);
    let saved = epub_cover::add_cover(&path, &red_jpeg(), &rev(&path), &backup_dir, false).unwrap();
    let existing = dir_names(&backup_dir);
    let mut appended = read(&path);
    appended.extend_from_slice(b"external update");
    std::fs::write(&path, &appended).unwrap();
    let changed = read(&path);
    let err = epub_cover::add_cover(&path, &red_jpeg(), str_of(&saved, "revision"), &backup_dir, true).unwrap_err();
    assert!(err.to_string().contains("변경"), "{err}");
    assert_eq!(read(&path), changed);
    assert_eq!(dir_names(&backup_dir), existing);
    assert_eq!(count_matching(root, ".cover-", ".tmp"), 0);
}

#[test]
fn test_cover_writer_respects_actual_reader_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let original = read(&path);
    let expected = rev(&path);
    {
        let reader = File::open(&path).unwrap();
        reader.try_lock_shared().unwrap();
        let err = epub_cover::add_cover(&path, &red_jpeg(), &expected, &backups(root), false).unwrap_err();
        assert!(err.to_string().contains("저장 중"), "{err}");
    }
    assert_eq!(read(&path), original);
    assert!(!backups(root).exists());
}

#[tokio::test]
async fn test_cover_replacement_api_requires_admin_role() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let api = cover_api(root, CoverHooks::default());
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
    let original = read(&path);
    api.gate.deny(403, "관리자만 사용할 수 있습니다");
    let response = api.client.post_json("/admin/book-covers/apply-image", &json!({"file_path": "book.epub", "revision": rev(&path), "image_data": "invalid"})).await;
    assert_eq!(response.status, 403);
    assert_eq!(read(&path), original);
}

#[tokio::test]
async fn test_cover_replacement_api_rejects_paths_outside_book_scope() {
    for relative in ["../book.epub", ".cover_backups/book.epub", "/etc/passwd"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let api = cover_api(root, CoverHooks::default());
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
        let original = read(&path);
        let response = api.client.post_json("/admin/book-covers/apply-image", &json!({"file_path": relative, "revision": "r1", "image_data": b64(&red_jpeg())})).await;
        assert_eq!(response.status, 409, "relative={relative}");
        assert_eq!(read(&path), original, "relative={relative}");
    }
}

#[tokio::test]
async fn test_apply_image_api_auth_revision_and_index_update() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let api = cover_api(root, CoverHooks::default());
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
    let body = json!({"file_path": "book.epub", "revision": rev(&path), "image_data": b64(&red_jpeg())});
    api.gate.clear();
    assert_eq!(api.client.post_json("/admin/book-covers/apply-image", &body).await.status, 401);
    api.gate.allow(json!({"role": "admin"}));
    let original = read(&path);
    let with = |key: &str, value: Value| {
        let mut changed = body.clone();
        changed[key] = value;
        changed
    };
    assert_eq!(api.client.post_json("/admin/book-covers/apply-image", &with("image_data", json!(""))).await.status, 422);
    assert_eq!(api.client.post_json("/admin/book-covers/apply-image", &with("image_data", json!("invalid!"))).await.status, 409);
    assert_eq!(api.client.post_json("/admin/book-covers/apply-image", &with("revision", json!("stale"))).await.status, 409);
    assert_eq!(read(&path), original);
    let response = api.client.post_json("/admin/book-covers/apply-image", &body).await;
    assert_eq!(response.status, 200);
    assert!(api.manager.update_called());
    assert_eq!(read(&root.join(str_of(&response.json()["result"], "backup_path"))), original);
}

#[test]
fn test_missing_opf_namespace_is_valid_xml_after_cover_save() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    rewrite_epub_members(&path, |members| members.replace_in("OPS/book.opf", b"<dc:creator>", br#"<dc:creator opf:role="aut">"#));
    let original = read(&path);

    let result = epub_cover::add_cover(&path, &red_jpeg(), &rev(&path), &backups(root), false).unwrap();
    assert!(truthy(&epub_cover::inspect_epub(&path).unwrap()["has_cover"]));
    assert_eq!(read(Path::new(str_of(&result, "backup_path"))), original);
    let opf = String::from_utf8(opf_of(&path)).unwrap();
    let package = roxmltree::Document::parse(&opf).expect("저장한 OPF가 올바른 XML이 아니다");
    let creator = package.descendants().find(|n| n.is_element() && n.tag_name().name() == "creator").unwrap();
    assert_eq!(creator.attribute((OPF_NS, "role")), Some("aut"));
    assert_eq!(read_zip_entry(&path, "OPS/chapter.xhtml"), BODY.as_bytes());
}

#[test]
fn test_namespace_compatibility_does_not_accept_other_xml_errors() {
    let contents: [&[u8]; 4] = [
        b"",
        br#"<package xmlns="http://www.idpf.org/2007/opf"><metadata></package>"#,
        br#"<package xmlns="http://www.idpf.org/2007/opf"><metadata><creator unknown:role="aut"/></metadata></package>"#,
        br#"<package xmlns="http://www.idpf.org/2007/opf"><metadata><creator opf:role="aut"></metadata></package>"#,
    ];
    for content in contents {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
        rewrite_epub_members(&path, |members| members.set("OPS/book.opf", content));
        let original = read(&path);
        let err = epub_cover::inspect_epub(&path).unwrap_err();
        assert!(err.to_string().contains("검사할 수 없습니다"), "content={:?}: {err}", String::from_utf8_lossy(content));
        assert_eq!(read(&path), original, "content={:?}", String::from_utf8_lossy(content));
    }
}

#[test]
fn test_namespace_compatibility_does_not_resolve_external_entities() {
    let tmp = tempfile::tempdir().unwrap();
    let secret = tmp.path().join("private.txt");
    std::fs::write(&secret, "private-value").unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
    let content = format!(
        r#"<!DOCTYPE package [<!ENTITY external SYSTEM "file://{}">]><package xmlns="http://www.idpf.org/2007/opf"><metadata><creator opf:role="aut">&external;</creator></metadata></package>"#,
        secret.display()
    );
    rewrite_epub_members(&path, |members| members.set("OPS/book.opf", content.clone()));
    assert!(epub_cover::inspect_epub(&path).is_err());
}

#[test]
fn test_cover_save_preserves_book_id_and_content() {
    for version in ["2.0", "3.0"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("한글 책.epub"), &EpubSpec::cover(version, false));
        let original = read(&path);
        let inode = ino_mtime(&path).0;
        let metadata = epub_cover::inspect_epub(&path).unwrap();
        assert_eq!(metadata["title"], "테스트 책", "version={version}");
        assert!(!truthy(&metadata["has_cover"]), "version={version}");
        let result = epub_cover::add_cover(&path, &red_jpeg(), str_of(&metadata, "revision"), &backups(root), false).unwrap();
        assert_eq!(ino_mtime(&path).0, inode, "version={version}");
        assert!(truthy(&epub_cover::inspect_epub(&path).unwrap()["has_cover"]), "version={version}");
        assert_ne!(result["revision"], metadata["revision"], "version={version}");
        assert_eq!(read(Path::new(str_of(&result, "backup_path"))), original, "version={version}");
        let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        {
            let first = archive.by_index(0).unwrap();
            assert_eq!(first.name().unwrap(), "mimetype", "version={version}");
            assert_eq!(first.compression(), CompressionMethod::Stored, "version={version}");
        }
        drop(archive);
        assert_eq!(read_zip_entry(&path, "OPS/chapter.xhtml"), BODY.as_bytes(), "version={version}");
        assert_eq!(read_zip_entry(&path, "OPS/style.css"), b"body { color: red; }", "version={version}");
    }
}

#[test]
fn test_stale_revision_and_existing_cover_do_not_modify_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("test.epub"), &EpubSpec::cover("3.0", false));
    let original = read(&path);
    let err = epub_cover::add_cover(&path, &red_jpeg(), "old", &backups(root), false).unwrap_err();
    assert!(err.to_string().contains("변경"), "{err}");
    assert_eq!(read(&path), original);
    let revision = rev(&path);
    epub_cover::add_cover(&path, &red_jpeg(), &revision, &backups(root), false).unwrap();
    let saved = read(&path);
    let err = epub_cover::add_cover(&path, &red_jpeg(), &rev(&path), &backups(root), false).unwrap_err();
    assert!(err.to_string().contains("표지"), "{err}");
    assert_eq!(read(&path), saved);
}

#[test]
fn test_invalid_image_does_not_touch_original() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("test.epub"), &EpubSpec::cover("3.0", false));
    let original = read(&path);
    let outcome: Result<Value, CoverError> = epub_cover::add_cover(&path, b"not an image", &rev(&path), &backups(root), false);
    assert!(outcome.is_err());
    assert_eq!(read(&path), original);
}

#[tokio::test]
async fn test_admin_api_save_and_preview() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let service = CoverManagement::new(root);
    let metadata = epub_cover::inspect_epub(&path).unwrap();
    let candidate_id = "a".repeat(64);
    let record = json!({"store": "yes24", "file_path": "book.epub", "revision": metadata["revision"], "image_url": "https://image.yes24.com/book.jpg"});
    service.write(&service.cache().join(format!("candidate-{candidate_id}.json")), record.to_string().as_bytes());
    service.write(&service.cache().join(format!("image-{candidate_id}.jpg")), &red_jpeg());
    // 인증 없는 요청은 401, 관리자 쿠키를 붙인 요청은 통과한다. 구 `gate.allow({"role": "admin"})`가 중간에 바꾸던 값이다.
    let app = cover_app(root);
    let anonymous = TestClient::with_app(app.clone());
    let client = admin_client(app);
    let image_endpoint = format!("candidate-image/{candidate_id}");
    for endpoint in ["directories", "missing", "candidates/yes24?file_path=book.epub", image_endpoint.as_str(), "preview?file_path=book.epub"] {
        assert_eq!(anonymous.get(&format!("/admin/book-covers/{endpoint}")).await.status, 401, "endpoint={endpoint}");
    }
    let apply_body = json!({"file_path": "book.epub", "candidate_id": candidate_id, "revision": metadata["revision"]});
    assert_eq!(anonymous.post_json("/admin/book-covers/apply", &apply_body).await.status, 401);
    let inode = ino_mtime(&path).0;
    let listed = client.get("/admin/book-covers/missing").await.json()["result"]["items"][0].clone();
    assert_eq!(listed["book_id"].as_u64().unwrap(), inode);
    assert_eq!(listed["category"], "_root");
    let response = client.post_json("/admin/book-covers/apply", &apply_body).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["result"]["book_id"].as_u64().unwrap(), inode);
    assert_eq!(client.get("/admin/book-covers/preview?file_path=book.epub").await.header("content-type"), Some("image/jpeg"));
    assert_eq!(client.get("/admin/book-covers/missing").await.json()["result"]["items"], json!([]));
    assert!(!service.cache().join(format!("candidate-{candidate_id}.json")).exists());
    assert!(!service.cache().join(format!("image-{candidate_id}.jpg")).exists());
}

#[tokio::test]
async fn test_all_api_gets_and_clear_cache_response() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let hooks = service.hooks().clone();
    let api = cover_api(root, hooks.clone());
    assert_eq!(api.client.get("/admin/book-covers/directories").await.json()["result"], json!([]));
    assert_eq!(api.client.get("/admin/book-covers/candidates/yes24?file_path=book.epub").await.json()["result"]["status"], "success");
    let image = api.client.get(&format!("/admin/book-covers/candidate-image/{}", cached.id(0))).await;
    assert!(image.status == 200 && image.header("content-type") == Some("image/jpeg"));
    let cleared = api.client.post_json("/admin/book-covers/cache/clear", &json!({"file_path": "book.epub", "revision": rev(&path)})).await;
    assert!(truthy(&cleared.json()["result"]["cleared"]));
    assert_eq!(api.client.get("/admin/book-covers/candidate-image/bad").await.status, 409);
    assert_eq!(api.client.get("/admin/book-covers/missing?limit=0").await.status, 422);
    drop(api);
    let api = cover_api(root, hooks.with_directories(|_path| Err(std::io::Error::other("secret disk details"))));
    let response = api.client.get("/admin/book-covers/directories").await;
    assert_eq!(response.status, 503);
    assert!(!response.text().contains("secret"), "{}", response.text());
}

#[tokio::test]
async fn test_direct_url_api_updates_index_or_reports_post_save_warning() {
    for index in ["ok", "missing", "failed", "exception"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let api = cover_api(root, CoverHooks::default().with_image_fetcher(fetcher(|_url| Ok(red_jpeg()))));
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
        match index {
            "missing" => api.manager.set_search(None),
            "failed" => api.manager.set_update(false),
            "exception" => api.manager.set_search_error("ES unavailable"),
            _ => {}
        }
        let response = api.client.post_json("/admin/book-covers/apply-url", &json!({"file_path": "book.epub", "revision": rev(&path), "url": "https://example.org/a"})).await;
        assert_eq!(response.status, 200, "index={index}");
        let result = response.json()["result"].clone();
        assert!(truthy(&epub_cover::inspect_epub(&path).unwrap()["has_cover"]), "index={index}");
        assert_eq!(result.as_object().unwrap().contains_key("warning"), index == "failed" || index == "exception", "index={index}");
        if index == "ok" {
            let size = std::fs::metadata(&path).unwrap().len();
            assert_eq!(api.manager.last_update(), Some((ino_mtime(&path).0 as i64, json!({"file_size": size}))), "index={index}");
        }
    }
}

#[tokio::test]
async fn test_new_api_auth_and_input_validation() {
    let tmp = tempfile::tempdir().unwrap();
    // 실제 쿠키 인증을 쓴다. 쿠키 없음은 401, viewer 쿠키는 403, 관리자 쿠키는 입력 검증까지 간다.
    let app = cover_app(tmp.path());
    let anonymous = TestClient::with_app(app.clone());
    let viewer = TestClient::with_app(app.clone()).cookie("tm_access_token", &crate::common::session::token("viewer@test.com", "viewer", None, None));
    let admin = admin_client(app);
    let body = json!({"file_path": "book.epub", "revision": "r1", "url": "https://example.org/a"});
    for endpoint in ["apply-url", "cache/clear"] {
        assert_eq!(anonymous.post_json(&format!("/admin/book-covers/{endpoint}"), &body).await.status, 401, "endpoint={endpoint}");
    }
    for endpoint in ["apply-url", "cache/clear"] {
        assert_eq!(viewer.post_json(&format!("/admin/book-covers/{endpoint}"), &body).await.status, 403, "endpoint={endpoint}");
    }
    let mut empty_url = body.clone();
    empty_url["url"] = json!("");
    assert_eq!(admin.post_json("/admin/book-covers/apply-url", &empty_url).await.status, 422);
    let mut bad_candidate = body.clone();
    bad_candidate["candidate_id"] = json!("bad");
    assert_eq!(admin.post_json("/admin/book-covers/apply", &bad_candidate).await.status, 422);
}

#[tokio::test]
async fn test_disconnected_search_cancels_worker() {
    let tmp = tempfile::tempdir().unwrap();
    let finished = Event::new();
    let done = finished.clone();
    let hooks = CoverHooks::default()
        .with_candidates(move |_store, _relative| {
            let outcome = (0..100).try_for_each(|_| {
                check_cancelled()?;
                std::thread::sleep(Duration::from_millis(10));
                Ok::<(), CoverError>(())
            });
            done.set();
            outcome.map(|_| Value::Null)
        })
        .with_is_disconnected(|| true);
    let api = cover_api(tmp.path(), hooks);
    let response = api.client.get("/admin/book-covers/candidates/yes24?file_path=book.epub").await;
    assert_eq!(response.status, 499);
    assert!(finished.wait(2));
}

#[test]
fn test_isbn_normalization_and_title_fallback() {
    for (isbn, expected) in [("urn:isbn:978-1234567890", "9781234567890"), ("1234567890", "1234567890"), ("invalid", "")] {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("fallback.epub"), &EpubSpec::cover("3.0", false));
        rewrite_epub_members(&path, |members| members.replace_in("OPS/book.opf", "<dc:title>테스트 책</dc:title>".as_bytes(), format!("<dc:identifier>{isbn}</dc:identifier>").as_bytes()));
        let result = epub_cover::inspect_epub(&path).unwrap();
        assert_eq!(result["title"], "fallback", "isbn={isbn}");
        assert_eq!(result["isbn"], expected, "isbn={isbn}");
    }
}
