mod common;
#[path = "common/cover_fixtures.rs"]
mod cover_fixtures;

use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use backend::book_manager::{BookManager, Thumbnailer};
use backend::cover_management::{ApplyImageModel, Backup, CoverHooks, CoverManagement, EpubThumbnailer};
use backend::cover_sources::{STORES, SearchError, SearchHooks};
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::epub_cover::{self, CoverError, EpubCoverHooks, Limits, MAX_IMAGE_BYTES};
use common::{
    clock::{Event, FakeClock},
    epub::{BODY, EpubSpec, append_child, attr_of_element_with_id, count_elements, make_epub, remove_element},
    es_dummy,
    faulty_fs::{FaultyFs, other_err},
    fsx::{count_matching, files_under, ino_mtime, sha256_hex, touch},
    image::{BLUE, RED, b64, encode_image, image_size, red_jpeg},
    log_capture::LogCapture,
    zipx::{member_names, read_zip_entry, rewrite_epub_members},
};
use cover_fixtures::cached_candidates;
use image::ImageFormat;
use serde_json::{Value, json};

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn backups(root: &Path) -> PathBuf {
    root.join(".cover_backups")
}

/// 현재 `revision`. Python `inspect_epub(path)["revision"]`.
fn rev(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap()["revision"].as_str().unwrap().to_string()
}

fn first_item(service: &CoverManagement) -> Value {
    service.missing("", "", 60, false).unwrap()["items"][0].clone()
}

fn not_found(_store: &str, _metadata: &Value) -> Result<Value, SearchError> {
    Ok(json!({"status": "not_found", "candidates": []}))
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap()
}

fn opf_of(path: &Path) -> Vec<u8> {
    read_zip_entry(path, "OPS/book.opf")
}

// TODO(common): `es_dummy::truthy(Option<&Value>)`를 `&Value`로 받는 얇은 래퍼. cover_fixtures::truthy를 대신한다.
fn truthy(value: &Value) -> bool {
    es_dummy::truthy(Some(value))
}

/// 클로저를 `ImageFetcher`로 감싼다.
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

/// 클로저를 `Thumbnailer`로 감싼다.
// TODO(common): `Thumbnailer` 클로저 어댑터가 `common`에 없다(docs/rust-test-seams.md 3.12는 trait 이름만 정한다).
struct FnThumbnailer<F>(F);

impl<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync> Thumbnailer for FnThumbnailer<F> {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        (self.0)(path, file_type, width)
    }
}

fn thumbnailer<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync + 'static>(f: F) -> Arc<dyn Thumbnailer> {
    Arc::new(FnThumbnailer(f))
}

#[test]
fn test_cover_listing_and_search_use_same_filename_title() {
    let cases = [
        ("[랜들 개릿] 다아시 경의 모험.epub", "랜들 개릿 다아시 경의 모험"),
        ("[작가] 구운몽—꿈(1) & 이야기!.EPUB", "작가 구운몽 꿈 1 이야기"),
        ("제목.v2.epub", "제목 v2"),
        ("  책\t 이름   .epub", "책 이름"),
        ("【作者】中文_タイトル〈第2巻〉.epub", "作者 中文 タイトル 第2巻"),
        ("한글 이름.epub", "한글 이름"),
        ("★[!]★.epub", "제목 없음"),
    ];
    for (filename, title) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("folder.epub").join(filename), &EpubSpec::cover("3.0", false));
        let original = read(&path);
        let mut service = CoverManagement::new(tmp.path());
        let item = service.missing("folder.epub", "", 60, false).unwrap()["items"][0].clone();
        assert_eq!(item["title"], title, "filename={filename:?}");
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = seen.clone();
        service.set_hooks(service.hooks().clone().with_search(move |_store, metadata| {
            sink.lock().unwrap().push(metadata["title"].as_str().unwrap().to_string());
            Ok(json!({"status": "not_found", "candidates": []}))
        }));
        service.candidates("yes24", str_of(&item, "file_path"), false, false).unwrap();
        assert_eq!(*seen.lock().unwrap(), [str_of(&item, "title")], "filename={filename:?}");
        assert_eq!(read(&path), original, "filename={filename:?}");
    }
}

#[test]
fn test_cover_management_accessors_and_normalized_book_cache_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let service = CoverManagement::with_hooks(tmp.path(), CoverHooks::default().with_search_hooks(SearchHooks::default()));

    assert_eq!(service.root(), tmp.path());
    assert_eq!(service.cache(), tmp.path().join(".cover_management"));
    assert_eq!(service.book_cache(""), service.book_cache("."));
    assert_ne!(service.book_cache("/book.epub"), service.book_cache("book.epub"));
    assert!(EpubThumbnailer.extract_cover_thumbnail(tmp.path(), "pdf", None).is_none());
    assert!(EpubThumbnailer.extract_cover_thumbnail(tmp.path(), "epub", None).is_none());
}

#[test]
fn test_cover_cache_write_error_removes_temporary_file() {
    let tmp = tempfile::tempdir().unwrap();
    let service = CoverManagement::new(tmp.path());
    let destination = tmp.path().join("destination");
    std::fs::create_dir(&destination).unwrap();

    assert!(service.try_write(&destination, b"data").is_err());
    assert!(std::fs::read_dir(tmp.path()).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".tmp-")));
}

#[test]
fn test_edit_images_rejects_blank_insert_url_before_mutating_epub() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", true));
    let before = read(&path);
    let service = CoverManagement::new(tmp.path());

    let error = service.edit_images("book.epub", &rev(&path), epub_cover::EditImages::default(), &[json!({"before_idref":"chapter","url":"  "})], Backup::default()).unwrap_err();

    assert!(error.to_string().contains("이미지 URL이 비어 있습니다"));
    assert_eq!(read(&path), before);
    assert!(!backups(tmp.path()).exists());
}

#[test]
fn test_filename_title_replaces_old_metadata_search_cache() {
    for covered in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("[저자] 제목 1.epub"), &EpubSpec::cover("3.0", covered));
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let mut service = CoverManagement::new(tmp.path());
        let revision = rev(&path);
        let calls = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let sink = calls.clone();
        service.set_hooks(service.hooks().clone().with_search(move |store, metadata| {
            sink.lock().unwrap().push((store.to_string(), metadata["title"].as_str().unwrap().to_string()));
            Ok(json!({"status": "not_found", "candidates": []}))
        }));
        for store in STORES {
            let old_key = format!("{store}:{name}:{revision}") + if *store == "aladin" { ":books-v2" } else { "" };
            let old_hash = sha256_hex(old_key);
            for parent in [service.cache().to_path_buf(), service.book_cache(&name)] {
                service.write(&parent.join(format!("search-{old_hash}.json")), br#"{"status":"not_found","candidates":[]}"#);
            }
            for _ in 0..2 {
                service.candidates(store, &name, covered, false).unwrap();
            }
        }
        let expected: Vec<(String, String)> = STORES.iter().map(|store| (store.to_string(), "저자 제목 1".to_string())).collect();
        assert_eq!(*calls.lock().unwrap(), expected, "covered={covered}");
        assert_eq!(service.missing("", "", 60, covered).unwrap()["items"][0]["title"], "저자 제목 1", "covered={covered}");
    }
}

#[test]
fn test_broken_epub_title_also_uses_clean_filename() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("[저자] 손상된—책.epub"), b"invalid zip").unwrap();
    let item = first_item(&CoverManagement::new(tmp.path()));
    assert_eq!((str_of(&item, "title"), str_of(&item, "status")), ("저자 손상된 책", "error"));
}

#[test]
fn test_original_preview_handles_changes_between_inspection_and_read() {
    for change in ["missing_image", "bad_zip", "writer_lock", "size_limit"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
        let writers = Arc::new(Mutex::new(Vec::<File>::new()));
        let held = writers.clone();
        let mut hooks = CoverHooks::default().with_inspect_epub(move |target| {
            let result = epub_cover::inspect_epub(target);
            match change {
                "missing_image" => rewrite_epub_members(target, |members| {
                    members.remove("OPS/cover.jpg");
                }),
                "bad_zip" => std::fs::write(target, b"external write").unwrap(),
                "writer_lock" => {
                    let writer = std::fs::OpenOptions::new().read(true).write(true).open(target).unwrap();
                    writer.try_lock().unwrap();
                    held.lock().unwrap().push(writer);
                }
                _ => {}
            }
            result
        });
        if change == "size_limit" {
            hooks = hooks.with_max_image_bytes(1);
        }
        let outcome = CoverManagement::with_hooks(root, hooks).preview("book.epub", true);
        writers.lock().unwrap().clear();
        assert!(outcome.is_err(), "change={change}");
        assert!(!backups(root).exists(), "change={change}");
    }
}

#[test]
fn test_apply_image_rejects_malformed_base64_without_changing_book() {
    for image_data in ["한글", "YQ=", "YQ==extra", "\nYQ=="] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
        let original = read(&path);
        let outcome = CoverManagement::new(root).apply_image("book.epub", image_data, &rev(&path));
        assert!(outcome.is_err(), "image_data={image_data:?}");
        assert_eq!(read(&path), original, "image_data={image_data:?}");
        assert!(!backups(root).exists(), "image_data={image_data:?}");
    }
}

#[test]
fn test_apply_image_payload_limit_is_validated_before_decoding() {
    let encoded_limit = MAX_IMAGE_BYTES.div_ceil(3) * 4;
    let err = ApplyImageModel::new("book.epub", "r1", &"A".repeat(encoded_limit + 1)).expect_err("한도를 넘는 payload가 통과했다");
    assert!(err.to_string().contains("string_too_long"), "{err}");
}

#[test]
fn test_cover_replacement_preserves_content_and_has_one_generated_cover() {
    for version in ["2.0", "3.0"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover(version, false));
        let stat = ino_mtime(&path);
        let first = epub_cover::add_cover(&path, &red_jpeg(), &rev(&path), &backups(root), false).unwrap();
        let mut before = read(&path);
        // 600px 축소 경로만 지나면 된다. 크게 만들면 디버그 빌드에서 Lanczos 축소가 수 초씩 걸린다.
        let png = encode_image(900, 1350, BLUE, ImageFormat::Png);
        let mut plates: Vec<usize> = Vec::new();
        let mut result = Value::Null;
        for data in [png.clone(), png.clone(), red_jpeg()] {
            result = epub_cover::add_cover(&path, &data, &rev(&path), &backups(root), true).unwrap();
            assert_eq!(read(Path::new(str_of(&result, "backup_path"))), before, "version={version}");
            assert_eq!(ino_mtime(&path), stat, "version={version}");
            let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
            let member = BookManager::find_epub_cover_member(&mut archive).unwrap();
            drop(archive);
            assert_eq!(read_zip_entry(&path, &member), epub_cover::normalize_image(&data, Some(600)).unwrap(), "version={version}");
            let names = member_names(&path);
            assert_eq!(names.iter().filter(|name| name.contains("tm-cover-")).count(), 2, "version={version}");
            // 표지 페이지와 본문 1쪽에, 교체로 밀려난 이전 표지가 도판 페이지로 남는다.
            let plate_pages = names.iter().filter(|name| name.contains("tm-plate-page-")).count();
            assert_eq!(count_elements(&opf_of(&path), "itemref", None), 2 + plate_pages, "version={version}");
            plates.push(plate_pages);
            assert_eq!(read_zip_entry(&path, "OPS/chapter.xhtml"), BODY.as_bytes(), "version={version}");
            assert_eq!(read_zip_entry(&path, "OPS/style.css"), b"body { color: red; }", "version={version}");
            assert_eq!(names.len(), names.iter().collect::<std::collections::BTreeSet<_>>().len(), "version={version}");
            before = read(&path);
        }
        // 같은 이미지로 다시 교체하면 도판이 늘지 않고, 다른 이미지로 바꿀 때만 늘어난다.
        assert_eq!(plates, [1, 1, 2], "version={version}");
        assert_ne!(first["revision"], result["revision"], "version={version}");
    }
}

#[test]
fn test_browser_image_replaces_cover_after_server_candidates_are_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let saved = service.apply("book.epub", cached.id(0), &rev(&path)).unwrap();
    assert!(!service.book_cache("book.epub").exists());
    let before = read(&path);
    assert!(service.apply_image("book.epub", "not valid base64!", str_of(&saved, "revision")).is_err());
    assert!(service.apply_image("book.epub", &b64(b"invalid image"), str_of(&saved, "revision")).is_err());
    let err = service.apply_image("book.epub", &b64(&red_jpeg()), "stale").unwrap_err();
    assert!(err.to_string().contains("변경"), "{err}");
    assert_eq!(read(&path), before);
    let replaced = service.apply_image("book.epub", &b64(&red_jpeg()), str_of(&saved, "revision")).unwrap();
    assert_ne!(replaced["revision"], saved["revision"]);
    assert_eq!(read(&root.join(str_of(&replaced, "backup_path"))), before);
}

#[test]
fn test_research_and_direct_url_replacement_allow_saved_book_explicitly() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
    let hooks = CoverHooks::default().with_search(not_found).with_image_fetcher(fetcher(|_url| Ok(red_jpeg())));
    let service = CoverManagement::with_hooks(root, hooks);
    let err = service.candidates("yes24", "book.epub", false, false).unwrap_err();
    assert!(err.to_string().contains("이미 표지"), "{err}");
    let original = read(&path);
    assert_eq!(service.candidates("yes24", "book.epub", true, false).unwrap()["status"], "not_found");
    assert_eq!(read(&path), original);
    let saved = service.apply_url("book.epub", "https://example.org/a", &rev(&path), true).unwrap();
    assert_eq!(read(&root.join(str_of(&saved, "backup_path"))), original);
}

#[test]
fn test_retry_bypasses_error_cache_and_aladin_old_results() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = calls.clone();
    let hooks = CoverHooks::default().with_search(move |store, _metadata| {
        sink.lock().unwrap().push(store.to_string());
        Err(SearchError::Value("offline".to_string()))
    });
    let service = CoverManagement::with_hooks(root, hooks);
    assert_eq!(service.candidates("yes24", "book.epub", false, false).unwrap()["status"], "error");
    assert_eq!(service.candidates("yes24", "book.epub", false, false).unwrap()["status"], "error");
    assert_eq!(*calls.lock().unwrap(), ["yes24"]);
    assert_eq!(service.candidates("yes24", "book.epub", false, true).unwrap()["status"], "error");
    assert_eq!(*calls.lock().unwrap(), ["yes24", "yes24"]);
    let key = sha256_hex(format!("aladin:book.epub:{}", rev(&path)));
    service.write(&service.book_cache("book.epub").join(format!("search-{key}.json")), br#"{"status":"not_found","candidates":[]}"#);
    assert_eq!(service.candidates("aladin", "book.epub", false, false).unwrap()["status"], "error");
    assert_eq!(calls.lock().unwrap().last().unwrap(), "aladin");
}

#[test]
fn test_failed_replacement_restores_previous_cover() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
    let original = read(&path);
    let stat = ino_mtime(&path);
    let calls = AtomicUsize::new(0);
    let fs = FaultyFs::new();
    fs.set_sync_file(move |_file| if calls.fetch_add(1, Ordering::SeqCst) + 1 == 3 { Some(other_err("write failed")) } else { None });
    let hooks = EpubCoverHooks::default().with_fs(fs);
    let err = epub_cover::add_cover_with(&path, &red_jpeg(), &epub_cover::revision(&path), &backups(root), true, &hooks).unwrap_err();
    assert!(err.is_io(), "{err}");
    assert_eq!(read(&path), original);
    assert_eq!(ino_mtime(&path), stat);
}

#[test]
fn test_missing_opf_namespace_recognizes_existing_cover_without_modifying_book() {
    for version in ["2.0", "3.0"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover(version, true));
        rewrite_epub_members(&path, |members| members.replace_in("OPS/book.opf", b"<dc:creator>", br#"<dc:creator opf:role="aut">"#));
        let original = read(&path);
        let stat = ino_mtime(&path);

        let metadata = epub_cover::inspect_epub(&path).unwrap();
        assert!(truthy(&metadata["has_cover"]), "version={version}");
        assert_eq!(metadata["author"], "저자", "version={version}");
        assert_eq!(CoverManagement::new(root).missing("", "", 60, false).unwrap()["items"], json!([]), "version={version}");
        let err = epub_cover::add_cover(&path, &red_jpeg(), str_of(&metadata, "revision"), &backups(root), false).unwrap_err();
        assert!(err.to_string().contains("이미 표지"), "version={version}: {err}");
        assert_eq!(read(&path), original, "version={version}");
        assert_eq!(ino_mtime(&path), stat, "version={version}");
        assert!(!backups(root).exists(), "version={version}");
    }
}

#[test]
fn test_direct_url_saves_cover_and_clear_cache_preserves_epub() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let metadata = epub_cover::inspect_epub(&path).unwrap();
    let downloaded = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = downloaded.clone();
    service.set_hooks(service.hooks().clone().with_image_fetcher(fetcher(move |url| {
        sink.lock().unwrap().push(url.to_string());
        Ok(red_jpeg())
    })));
    let result = service.apply_url("book.epub", "https://example.org/cover.jpg", str_of(&metadata, "revision"), false).unwrap();
    assert_eq!(*downloaded.lock().unwrap(), ["https://example.org/cover.jpg"]);
    assert!(truthy(&epub_cover::inspect_epub(&path).unwrap()["has_cover"]));
    assert!(!service.candidate_path(cached.id(0)).exists());
    let saved = read(&path);
    service.clear_cache("book.epub", str_of(&result, "revision")).unwrap();
    assert_eq!(read(&path), saved);
    assert!(root.join(str_of(&result, "backup_path")).exists());
}

#[test]
fn test_clear_cache_is_book_scoped_and_revision_checked() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    make_epub(&root.join("other.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let other = cached_candidates(&mut service, "other.epub", &["yes24"]);
    let original = read(&path);
    let err = service.clear_cache("book.epub", "stale").unwrap_err();
    assert!(err.to_string().contains("변경"), "{err}");
    assert!(service.candidate_path(cached.id(0)).exists());
    service.clear_cache("book.epub", &rev(&path)).unwrap();
    assert!(!service.candidate_path(cached.id(0)).exists());
    assert!(service.candidate_path(other.id(0)).exists());
    assert_eq!(read(&path), original);
}

#[test]
fn test_failed_write_restores_original() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("test.epub"), &EpubSpec::cover("3.0", false));
    let original = read(&path);
    let revision = epub_cover::inspect_epub(&path).unwrap()["revision"].as_str().unwrap().to_string();
    let calls = AtomicUsize::new(0);
    let fs = FaultyFs::new();
    fs.set_sync_file(move |_file| if calls.fetch_add(1, Ordering::SeqCst) + 1 == 3 { Some(other_err("write failed")) } else { None });
    let hooks = EpubCoverHooks::default().with_fs(fs);
    let err = epub_cover::add_cover_with(&path, &red_jpeg(), &revision, &backups(root), false, &hooks).unwrap_err();
    assert!(err.is_io(), "{err}");
    assert_eq!(read(&path), original);
}

#[test]
fn test_filesystem_listing_includes_unindexed_epub_and_reports_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_epub(&root.join("目录").join("missing.EPUB"), &EpubSpec::cover("3.0", false));
    make_epub(&root.join("目录").join("covered.epub"), &EpubSpec::cover("3.0", true));
    std::fs::write(root.join("目录").join("broken.epub"), b"bad zip").unwrap();
    std::fs::create_dir(backups(root)).unwrap();
    let manager = CoverManagement::new(root);
    let labels: Vec<Value> = manager.directories("").unwrap().iter().map(|entry| entry["label"].clone()).collect();
    assert_eq!(labels, [json!("目录")]);
    let page = manager.missing("目录", "", 60, false).unwrap();
    let statuses: std::collections::BTreeSet<&str> = page["items"].as_array().unwrap().iter().map(|item| item["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["error", "missing"].into_iter().collect());
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert!(page["next_cursor"].is_null());
    for item in page["items"].as_array().unwrap() {
        assert_eq!(item["book_id"].as_u64().unwrap(), ino_mtime(&root.join(str_of(item, "file_path"))).0);
        assert_eq!(item["category"], "目录");
        assert!(truthy(&item["revision"]));
    }
}

#[test]
fn filesystem_listing_order_matches_python_casefold() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir(root.join("zeta")).unwrap();
    std::fs::create_dir(root.join("ßeta")).unwrap();
    std::fs::write(root.join("z.epub"), b"bad zip").unwrap();
    std::fs::write(root.join("ß.epub"), b"bad zip").unwrap();

    let manager = CoverManagement::new(root);
    let labels: Vec<String> = manager.directories("").unwrap().into_iter().map(|entry| entry["label"].as_str().unwrap().to_owned()).collect();
    assert_eq!(labels, ["ßeta", "zeta"]);

    let page = manager.missing("", "", 10, false).unwrap();
    let paths: Vec<&str> = page["items"].as_array().unwrap().iter().map(|item| item["file_path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["ß.epub", "z.epub"]);
}

#[test]
fn test_cover_page_image_detection_and_existing_cover_protection() {
    let img = r#"<img src="../Images/120%28550%29.jpg" />"#;
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="../Images/120%28550%29.jpg" /></svg>"#;
    let commented = format!("<!-- 표지 설명 -->{img}");
    // (page_href, metadata, guide, image_markup, has_cover)
    let cases: Vec<(&str, &str, &str, String, bool)> = vec![
        ("Text/cover.xhtml", "", "", commented, true),
        ("Text/front.xhtml", "", r#"<guide><reference type="cover" href="Text/front.xhtml#cover" /></guide>"#, img.to_string(), true),
        ("Text/front.xhtml", r#"<meta name="cover" content="page" />"#, "", img.to_string(), true),
        ("Text/cover.xhtml", "", "", svg.to_string(), true),
        ("Text/chapter.xhtml", "", "", img.to_string(), false),
        ("Text/cover.xhtml", "", "", r#"<img src="missing.jpg" />"#.to_string(), false),
        ("Text/cover.xhtml", "", "", r#"<img src="https://example.com/cover.jpg" />"#.to_string(), false),
    ];
    for (page_href, metadata, guide, image_markup, has_cover) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = root.join("html-cover.epub");
        let opf = format!(
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>요재지이 형식</dc:title>{metadata}</metadata><manifest><item id="page" href="{page_href}" media-type="application/xhtml+xml"/><item id="x120_550_.jpg" href="Images/120%28550%29.jpg" media-type="image/jpeg"/></manifest><spine><itemref idref="page"/></spine>{guide}</package>"#
        );
        let mut spec = EpubSpec::cover("2.0", false).with_opf(&opf).with_opf_path("OEBPS/content.opf").with_extra("OEBPS/Images/120(550).jpg", red_jpeg());
        spec.chapters = vec![(page_href.to_string(), format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>{image_markup}</body></html>"#))];
        spec.include_style = false;
        make_epub(&path, &spec);
        let original = read(&path);
        let inspected = epub_cover::inspect_epub(&path).unwrap();
        assert_eq!(inspected["has_cover"], json!(has_cover), "page_href={page_href}, markup={image_markup}");
        let mut zip_archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert_eq!(BookManager::find_epub_cover_member(&mut zip_archive), has_cover.then(|| "OEBPS/Images/120(550).jpg".to_string()), "page_href={page_href}, markup={image_markup}");
        drop(zip_archive);
        let listed = !CoverManagement::new(root).missing("", "", 60, false).unwrap()["items"].as_array().unwrap().is_empty();
        assert_eq!(listed, !has_cover, "page_href={page_href}, markup={image_markup}");
        if has_cover {
            assert!(BookManager::extract_cover_thumbnail(&path, "epub", None).is_some(), "page_href={page_href}, markup={image_markup}");
            let err = epub_cover::add_cover(&path, &red_jpeg(), str_of(&inspected, "revision"), &backups(root), false).unwrap_err();
            assert!(err.to_string().contains("이미 표지"), "page_href={page_href}: {err}");
            assert_eq!(read(&path), original, "page_href={page_href}, markup={image_markup}");
        }
    }
}

#[test]
fn test_root_guard_rejects_unsafe_paths() {
    for relative in ["../outside.epub", "/etc/passwd", ".cover_backups/a.epub"] {
        let tmp = tempfile::tempdir().unwrap();
        assert!(CoverManagement::new(tmp.path()).resolve(relative, false).is_err(), "relative={relative}");
    }
}

#[test]
fn test_symlink_is_excluded() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("real.epub"), &EpubSpec::cover("3.0", false));
    std::os::unix::fs::symlink(&path, root.join("link.epub")).unwrap();
    let manager = CoverManagement::new(root);
    assert!(manager.resolve("link.epub", false).is_err());
    assert_eq!(manager.missing("", "", 60, false).unwrap()["items"].as_array().unwrap().len(), 1);
}

#[test]
fn test_pending_cache_survives_age_and_service_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let old_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 30 * 86400;
    for path in files_under(service.cache()) {
        touch(&path, old_time);
    }
    let mut restarted = CoverManagement::with_hooks(root, service.hooks().clone());
    for candidate in &cached.candidates {
        assert!(!restarted.image(str_of(candidate, "candidate_id")).unwrap().is_empty());
    }
    assert_eq!(cached.fetches.lock().unwrap().len(), 2);
    restarted.set_hooks(restarted.hooks().clone().with_search(|_store, _metadata| panic!("캐시한 검색을 다시 요청했다")));
    let restored = restarted.candidates("yes24", "book.epub", false, false).unwrap()["candidates"].clone();
    let restored_ids: Vec<&str> = restored.as_array().unwrap().iter().map(|item| str_of(item, "candidate_id")).collect();
    let cached_ids: Vec<&str> = cached.candidates.iter().map(|item| str_of(item, "candidate_id")).collect();
    assert_eq!(restored_ids, cached_ids);
    assert!(restored.as_array().unwrap().iter().all(|item| item["width"] == 120 && item["height"] == 180 && item["byte_size"].as_u64().unwrap() > 0));
}

#[test]
fn test_success_removes_all_book_candidates_but_keeps_other_book_and_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    make_epub(&root.join("other.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", STORES);
    let other = cached_candidates(&mut service, "other.epub", &["yes24"]);
    let result = service.apply("book.epub", cached.id(0), &rev(&path)).unwrap();
    assert!(truthy(&epub_cover::inspect_epub(&path).unwrap()["has_cover"]));
    assert!(root.join(str_of(&result, "backup_path")).is_file());
    let files = files_under(service.cache());
    assert_eq!(files.iter().filter(|f| f.extension().is_some_and(|e| e == "jpg")).count(), 2);
    for record in files.iter().filter(|f| f.file_name().unwrap().to_string_lossy().starts_with("candidate-") && f.extension().is_some_and(|e| e == "json")) {
        let parsed: Value = serde_json::from_slice(&read(record)).unwrap();
        assert_ne!(parsed.get("file_path").and_then(Value::as_str), Some("book.epub"), "{}", record.display());
    }
    assert!(!service.image(other.id(0)).unwrap().is_empty());
    assert!(service.image(cached.id(1)).is_err());
}

#[test]
fn test_failed_apply_keeps_candidates_for_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let original = read(&path);
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let snapshot = |service: &CoverManagement| -> BTreeMap<PathBuf, Vec<u8>> {
        files_under(service.cache()).into_iter().filter(|f| f.extension().is_some_and(|e| e == "json" || e == "jpg")).map(|f| (f.clone(), read(&f))).collect()
    };
    let before = snapshot(&service);
    assert!(service.apply("book.epub", cached.id(0), "stale revision").is_err());
    assert_eq!(read(&path), original);
    assert_eq!(before, snapshot(&service));
    assert!(!service.image(cached.id(0)).unwrap().is_empty());
    assert_eq!(cached.fetches.lock().unwrap().len(), 2);
}

#[test]
fn test_apply_waits_for_download_and_removes_its_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let pending_id = cached.id(1).to_string();
    let first_id = cached.id(0).to_string();
    for image in files_under(service.cache()) {
        if image.file_name().unwrap().to_string_lossy() == format!("image-{pending_id}.jpg") {
            std::fs::remove_file(image).unwrap();
        }
    }
    let started = Event::new();
    let release = Event::new();
    let waiting_for_download = Event::new();
    let (started_in, release_in, waiting_in) = (started.clone(), release.clone(), waiting_for_download.clone());
    service.set_hooks(service.hooks().clone().with_lock_wait_observer(move || waiting_in.set()).with_fetch_bytes(move |_store, _url| {
        started_in.set();
        assert!(release_in.wait(3));
        Ok(red_jpeg())
    }));
    let old_revision = rev(&path);
    let service = &service;
    std::thread::scope(|scope| {
        let downloading = scope.spawn(|| service.image(&pending_id));
        assert!(started.wait(3));
        let applying = scope.spawn(|| service.apply("book.epub", &first_id, &old_revision));
        let waited = waiting_for_download.wait(3);
        let applying_done = applying.is_finished();
        release.set();
        assert!(waited, "apply가 다운로드 잠금을 기다리지 않았다");
        assert!(!applying_done, "apply가 다운로드보다 먼저 끝났다");
        assert!(!downloading.join().unwrap().unwrap().is_empty());
        applying.join().unwrap().unwrap();
    });
    assert!(files_under(service.cache()).iter().all(|f| f.extension().is_none_or(|e| e != "jpg")));
    assert!(service.image(&pending_id).is_err());
}

#[test]
fn test_legacy_cache_provides_image_size_without_external_fetch() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let hooks = CoverHooks::default().with_fetch_bytes(|_store, _url| panic!("외부 이미지 요청")).with_search(|_store, _metadata| panic!("외부 검색 요청"));
    let service = CoverManagement::with_hooks(root, hooks);
    let old_revision = rev(&path);
    let candidate_id = "b".repeat(64);
    let record = json!({"store": "yes24", "file_path": "book.epub", "revision": old_revision, "image_url": "https://image.yes24.com/book.jpg"});
    let key = sha256_hex(format!("yes24:book.epub:{old_revision}:filename-v1"));
    service.write(&service.cache().join(format!("candidate-{candidate_id}.json")), record.to_string().as_bytes());
    service.write(&service.cache().join(format!("image-{candidate_id}.jpg")), &red_jpeg());
    service.write(&service.cache().join(format!("search-{key}.json")), json!({"status": "success", "candidates": [{"candidate_id": candidate_id}]}).to_string().as_bytes());
    let result = service.candidates("yes24", "book.epub", false, false).unwrap()["candidates"][0].clone();
    assert_eq!((&result["width"], &result["height"]), (&json!(120), &json!(180)));
    assert_eq!(result["byte_size"].as_u64().unwrap() as usize, red_jpeg().len());
}

#[test]
fn test_service_rejects_invalid_and_stale_operations() {
    let operations = ["wrong_store", "covered", "stale_image", "missing_image", "bad_id", "unknown_id", "no_cover_preview", "unrenderable", "stale_url", "covered_url"];
    for operation in operations {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
        let mut service = CoverManagement::new(root);
        let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
        let candidate_id = cached.id(0).to_string();
        let mut revision = rev(&path);
        let mut original = read(&path);
        if ["covered", "covered_url", "unrenderable"].contains(&operation) {
            service.apply("book.epub", &candidate_id, &revision).unwrap();
            revision = rev(&path);
            original = read(&path);
        }
        if operation == "stale_image" {
            let mut record = service.candidate(&candidate_id).unwrap();
            record["revision"] = json!("stale");
            service.write(&service.candidate_path(&candidate_id), record.to_string().as_bytes());
        }
        if operation == "missing_image" {
            std::fs::remove_file(service.candidate_path(&candidate_id).parent().unwrap().join(format!("image-{candidate_id}.jpg"))).unwrap();
        }
        if operation == "unrenderable" {
            service.set_hooks(service.hooks().clone().with_thumbnailer(thumbnailer(|_path, _type, _width| None)));
        }
        let outcome: Result<(), CoverError> = match operation {
            "wrong_store" => service.candidates("unknown", "book.epub", false, false).map(|_| ()),
            "covered" => service.candidates("yes24", "book.epub", false, false).map(|_| ()),
            "stale_image" => service.image(&candidate_id).map(|_| ()),
            "missing_image" => service.apply("book.epub", &candidate_id, &revision).map(|_| ()),
            "bad_id" => service.image("bad").map(|_| ()),
            "unknown_id" => service.image(&"e".repeat(64)).map(|_| ()),
            "no_cover_preview" => service.preview("book.epub", false).map(|_| ()),
            "unrenderable" => service.preview("book.epub", false).map(|_| ()),
            "stale_url" => service.apply_url("book.epub", "https://example.org/a", "stale", false).map(|_| ()),
            _ => service.apply_url("book.epub", "https://example.org/a", &revision, false).map(|_| ()),
        };
        assert!(outcome.is_err(), "operation={operation}");
        assert_eq!(read(&path), original, "operation={operation}");
    }
}

#[test]
fn test_search_error_cache_expiry_cancel_and_cache_double_check() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::with_hooks(root, CoverHooks::default().with_search(|_store, _metadata| Err(SearchError::Value("blocked".to_string()))));
    assert_eq!(service.candidates("yes24", "book.epub", false, false).unwrap()["status"], "error");
    assert_eq!(service.candidates("yes24", "book.epub", false, false).unwrap()["status"], "error");
    let book_cache = service.book_cache("book.epub");
    for cached in files_under(&book_cache) {
        let name = cached.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("search-") && name.ends_with(".json") {
            touch(&cached, 0);
        }
    }
    service.set_hooks(service.hooks().clone().with_search(|_store, _metadata| Err(SearchError::Cancelled)));
    let err = service.candidates("yes24", "book.epub", false, false).unwrap_err();
    assert!(err.is_cancelled(), "{err}");
    let values = Mutex::new(VecDeque::from([None, None, Some(json!({"status": "not_found", "candidates": []}))]));
    service.set_hooks(service.hooks().clone().with_cached_search(move |_store, _relative| values.lock().unwrap().pop_front().expect("_cached_search를 세 번만 부른다")));
    assert_eq!(service.candidates("yes24", "book.epub", false, false).unwrap()["status"], "not_found");
}

#[test]
fn test_invalid_cached_records_refresh_and_image_lock_double_check() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let candidate_id = cached.id(0).to_string();
    let record_path = service.candidate_path(&candidate_id);
    let record = service.candidate(&candidate_id).unwrap();
    let image_path = record_path.parent().unwrap().join(format!("image-{candidate_id}.jpg"));
    std::fs::remove_file(&image_path).unwrap();
    // 잠금을 잡는 사이 다른 요청이 이미지를 캐시에 만들어 놓은 상황.
    let appears = image_path.clone();
    service.set_hooks(service.hooks().clone().with_store_lock(move |_cache, _store| {
        std::fs::write(&appears, red_jpeg()).unwrap();
        Box::new(())
    }));
    assert_eq!(service.image(&candidate_id).unwrap(), red_jpeg());
    std::fs::remove_file(&record_path).unwrap();
    assert_eq!(service.candidates("yes24", "book.epub", false, false).unwrap()["status"], "success");
    let bad = service.cache().join("bad.json");
    service.write(&bad, b"[]");
    assert!(service.read_json(&bad).is_none());
    service.write(&bad, b"not json");
    assert!(service.read_json(&bad).is_none());
    assert_eq!(record["file_path"], "book.epub");
}

#[test]
fn test_success_warns_on_candidate_cleanup_failure_and_invalidates_only_own_previews() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let book_id = ino_mtime(&path).0;
    for name in [".cover_cache", ".preview_cache"] {
        let directory = root.join(name);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join(format!("{book_id}.jpg")), b"stale").unwrap();
        std::fs::write(directory.join(format!("{book_id}_ch10.epub")), b"stale").unwrap();
        std::fs::write(directory.join(format!("{book_id}0.jpg")), b"other book").unwrap();
        std::fs::create_dir(directory.join(format!("{book_id}_directory"))).unwrap();
    }
    service.set_hooks(service.hooks().clone().with_delete_book_cache(|_path| Err(std::io::Error::other("disk"))));
    let result = service.apply("book.epub", cached.id(0), &rev(&path)).unwrap();
    assert!(result.as_object().unwrap().contains_key("warning"), "{result}");
    assert!(truthy(&epub_cover::inspect_epub(&path).unwrap()["has_cover"]));
    for name in [".cover_cache", ".preview_cache"] {
        assert!(!root.join(name).join(format!("{book_id}.jpg")).exists(), "{name}");
        assert!(!root.join(name).join(format!("{book_id}_ch10.epub")).exists(), "{name}");
        assert!(root.join(name).join(format!("{book_id}0.jpg")).exists(), "{name}");
        assert!(root.join(name).join(format!("{book_id}_directory")).is_dir(), "{name}");
    }
}

#[test]
fn test_missing_and_internal_paths_are_rejected() {
    for (relative, directory) in [("missing", true), ("book.txt", false), (".hidden/book.epub", false)] {
        let tmp = tempfile::tempdir().unwrap();
        assert!(CoverManagement::new(tmp.path()).resolve(relative, directory).is_err(), "relative={relative}, directory={directory}");
    }
}

#[test]
fn test_invalid_listing_cursor_is_rejected() {
    let long = "1".repeat(11);
    for cursor in ["abc", long.as_str()] {
        let tmp = tempfile::tempdir().unwrap();
        assert!(CoverManagement::new(tmp.path()).missing("", cursor, 60, false).is_err(), "cursor={cursor}");
    }
}

#[test]
fn test_epub_validation_limits_and_svg_cover() {
    let modes = ["size", "container", "opf", "package", "opf_path", "count", "pixels", "empty_image", "large_image", "svg", "large_existing"];
    for mode in modes {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", mode == "svg" || mode == "large_existing"));
        let mut hooks = EpubCoverHooks::default();
        let mut limits = Limits::default();
        match mode {
            "size" => limits.max_epub_bytes = 1,
            "container" => limits.max_xml_bytes = 1,
            "opf" => limits.max_xml_bytes = 300,
            "package" => rewrite_epub_members(&path, |members| members.set("OPS/book.opf", &b"<wrong/>"[..])),
            "opf_path" => hooks = hooks.with_find_opf_path(Arc::new(|| "../outside.opf".to_string())),
            "count" => {
                assert!(epub_cover::check_entry_count(20001).is_err(), "mode={mode}");
                continue;
            }
            "pixels" | "empty_image" | "large_image" => {
                if mode == "pixels" {
                    limits.max_image_pixels = 1;
                } else if mode == "large_image" {
                    limits.max_image_bytes = 1;
                }
                let data = if mode == "empty_image" { Vec::new() } else { red_jpeg() };
                assert!(epub_cover::normalize_image_with(&data, None, &hooks.with_limits(limits)).is_err(), "mode={mode}");
                continue;
            }
            "svg" => {
                rewrite_epub_members(&path, |members| {
                    members.replace_in("OPS/book.opf", b"cover.jpg", b"cover.svg");
                    members.replace_in("OPS/book.opf", b"image/jpeg", b"image/svg+xml");
                    members.set("OPS/cover.svg", &br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#[..]);
                    members.remove("OPS/cover.jpg");
                });
                assert!(truthy(&epub_cover::inspect_epub(&path).unwrap()["has_cover"]), "mode={mode}");
                continue;
            }
            _ => {
                limits.max_image_bytes = 1;
                let result = epub_cover::inspect_epub_with(&path, &hooks.with_limits(limits)).unwrap();
                assert!(truthy(&result["has_cover"]), "mode={mode}");
                assert!(result["cover_image"].is_null(), "mode={mode}");
                assert!(str_of(&result, "cover_error").contains("크기 제한"), "mode={mode}: {result}");
                continue;
            }
        }
        assert!(epub_cover::inspect_epub_with(&path, &hooks.with_limits(limits)).is_err(), "mode={mode}");
    }
}

#[test]
fn test_cover_rebuild_guards_and_old_declaration_cleanup() {
    let modes = ["manifest", "metadata", "spine", "id", "path", "old_declarations", "revision", "invalid_stage", "no_staged_cover", "locked"];
    for mode in modes {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
        let image = epub_cover::normalize_image(&red_jpeg(), None).unwrap();
        let cover_id = format!("tm-cover-{}", &sha256_hex(&image)[..16]);
        rewrite_epub_members(&path, |members| {
            let mut opf = members.get("OPS/book.opf").to_vec();
            match mode {
                "manifest" | "metadata" | "spine" => opf = remove_element(&opf, mode),
                "id" => opf = append_child(&opf, "manifest", &format!(r#"<item id="{cover_id}" href="chapter.xhtml" media-type="application/xhtml+xml"/>"#)),
                "path" => members.set(&format!("OPS/{cover_id}.jpg"), &b"unused member"[..]),
                "old_declarations" => {
                    let mut items = String::new();
                    for (index, properties) in ["cover-image", "cover-image other"].iter().enumerate() {
                        items += &format!(r#"<item id="stale{index}" href="missing{index}.jpg" properties="{properties}" media-type="image/jpeg"/>"#);
                    }
                    opf = append_child(&opf, "manifest", &items);
                    opf = append_child(&opf, "metadata", r#"<meta name="cover" content="stale0"/><meta name="other" content="preserve"/>"#);
                }
                _ => {}
            }
            members.set("OPS/book.opf", opf);
        });
        let original = read(&path);
        let revision = epub_cover::revision(&path);
        let mut hooks = EpubCoverHooks::default();
        match mode {
            "locked" => {
                let fs = FaultyFs::new();
                fs.set_lock_exclusive(|_file, _nonblocking| Some(std::io::ErrorKind::WouldBlock.into()));
                hooks = hooks.with_fs(fs);
                let err = epub_cover::inspect_epub_with(&path, &hooks).unwrap_err();
                assert!(err.to_string().contains("저장 중"), "mode={mode}: {err}");
            }
            "revision" => {
                let values = Mutex::new(VecDeque::from([revision.clone(), "changed".to_string()]));
                hooks = hooks.with_revision(Arc::new(move |_path: &Path| values.lock().unwrap().pop_front().expect("revision을 두 번만 부른다")));
            }
            "invalid_stage" => hooks = hooks.with_testzip(Arc::new(|_path: &Path| Some("corrupt".to_string()))),
            "no_staged_cover" => {
                hooks = hooks.with_inspect_archive(Arc::new(|mut inspected: Value| {
                    inspected["has_cover"] = json!(false);
                    inspected
                }))
            }
            _ => {}
        }
        if mode == "old_declarations" {
            epub_cover::add_cover_with(&path, &red_jpeg(), &revision, &backups(root), false, &hooks).unwrap();
            let opf = opf_of(&path);
            assert_eq!(count_elements(&opf, "meta", Some(("name", "cover"))), 1, "mode={mode}");
            assert_eq!(attr_of_element_with_id(&opf, "item", "stale1", "properties"), "other", "mode={mode}");
            assert!(truthy(&epub_cover::inspect_epub(&path).unwrap()["has_cover"]), "mode={mode}");
        } else {
            assert!(epub_cover::add_cover_with(&path, &red_jpeg(), &revision, &backups(root), false, &hooks).is_err(), "mode={mode}");
            assert_eq!(read(&path), original, "mode={mode}");
            assert_eq!(count_matching(root, ".cover-", ".tmp"), 0, "mode={mode}");
        }
    }
}

#[test]
fn test_resolved_escape_and_incomplete_legacy_cache_are_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    {
        let outside = root.parent().unwrap().join("outside.epub");
        let target = path.clone();
        let escaping = CoverManagement::with_hooks(root, service.hooks().clone().with_resolve(move |value| if value == target { outside.clone() } else { value.to_path_buf() }));
        let err = escaping.resolve("book.epub", false).unwrap_err();
        assert!(err.to_string().contains("디렉토리 밖"), "{err}");
    }
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let record_path = service.candidate_path(cached.id(0));
    let mut record = service.candidate(cached.id(0)).unwrap();
    record.as_object_mut().unwrap().remove("width");
    service.write(&record_path, record.to_string().as_bytes());
    std::fs::remove_file(record_path.parent().unwrap().join(format!("image-{}.jpg", cached.id(0)))).unwrap();
    let result = service.candidates("yes24", "book.epub", false, false).unwrap()["candidates"][0].clone();
    assert!(!result.as_object().unwrap().contains_key("width"), "{result}");
    service.write(&service.cache().join("candidate-invalid.json"), b"[]");
    service.clear_cache("book.epub", &rev(&path)).unwrap();
    assert!(service.cache().join("candidate-invalid.json").exists());
}

#[test]
fn test_unreadable_or_oversized_cover_page_is_not_a_cover() {
    let cases: [(&[u8], bool); 3] = [(b"", false), (b"\xff", false), (b"<html/>", true)];
    for (markup, oversized) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
        rewrite_epub_members(&path, |members| {
            members.replace_in("OPS/book.opf", b"chapter.xhtml", b"cover.xhtml");
            members.set("OPS/cover.xhtml", markup);
        });
        let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let found = if oversized {
            BookManager::find_epub_cover_member_with(&mut archive, &|name| if name == "OPS/cover.xhtml" { Some(5 * 1024 * 1024) } else { None })
        } else {
            BookManager::find_epub_cover_member(&mut archive)
        };
        assert_eq!(found, None, "markup={markup:?}, oversized={oversized}");
    }
}

#[test]
fn test_guide_missing_cover_page_is_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
    rewrite_epub_members(&path, |members| members.replace_in("OPS/book.opf", b"</package>", br#"<guide><reference type="cover" href="missing.xhtml"/></guide></package>"#));
    let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
    assert_eq!(BookManager::find_epub_cover_member(&mut archive), None);
}

#[test]
fn test_saved_cover_is_at_most_600_pixels_wide_and_keeps_aspect_ratio() {
    // 마지막 항목은 가로 2000 제한을 넘는 이미지다. 디버그 빌드에서는 축소 시간이 화소 수에 비례해
    // (2400x3200 은 약 3초) 가로만 2000을 넘는 납작한 2100x700 을 쓴다. 2000 제한과 600px 저장 경로를 모두 지난다.
    for (width, height, format) in [(900, 1350, ImageFormat::Jpeg), (1200, 600, ImageFormat::Png), (300, 450, ImageFormat::Png), (2100, 700, ImageFormat::Jpeg)] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let raw = encode_image(width, height, RED, format);
        let candidate = epub_cover::normalize_image(&raw, None).unwrap();
        let (candidate_width, _) = image_size(&candidate);
        assert!(candidate_width <= 2000, "{width}x{height}");
        if width > 600 {
            assert!(candidate_width > 600, "{width}x{height}");
        }
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
        epub_cover::add_cover(&path, &raw, &rev(&path), &backups(root), false).unwrap();
        let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let member = BookManager::find_epub_cover_member(&mut archive).unwrap();
        drop(archive);
        let (saved_width, saved_height) = image_size(&read_zip_entry(&path, &member));
        assert_eq!(saved_width, width.min(600), "{width}x{height}");
        // Python round()는 .5를 짝수로 보내지만 이 값들은 모두 정수로 나누어떨어진다.
        assert_eq!(saved_height, (height as f64 * width.min(600) as f64 / width as f64).round() as u32, "{width}x{height}");
    }
}

#[test]
fn test_direct_store_url_uses_same_store_lock() {
    for host in ["image.yes24.com", "yes24.com"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
        let stores = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = stores.clone();
        let hooks = CoverHooks::default()
            .with_store_lock(move |_cache, store| {
                sink.lock().unwrap().push(store.to_string());
                Box::new(())
            })
            .with_image_fetcher(fetcher(|_url| Ok(red_jpeg())));
        CoverManagement::with_hooks(root, hooks).apply_url("book.epub", &format!("https://{host}/cover.jpg"), &rev(&path), false).unwrap();
        assert_eq!(*stores.lock().unwrap(), ["yes24"], "host={host}");
    }
}

fn http_error(status: u16) -> SearchError {
    SearchError::Http { status }
}

#[test]
fn test_store_block_returns_wait_message_without_cache_and_stops_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let clock = FakeClock::new(5000.0);
    make_epub(&root.join("a.epub"), &EpubSpec::cover("3.0", false));
    make_epub(&root.join("b.epub"), &EpubSpec::cover("3.0", false));
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = calls.clone();
    let mut service = CoverManagement::with_hooks(
        root,
        CoverHooks::default().with_clock(clock.clone()).with_search(move |_store, metadata| {
            sink.lock().unwrap().push(metadata["title"].as_str().unwrap().to_string());
            Err(http_error(418))
        }),
    );
    let first = service.candidates("naver", "a.epub", false, false).unwrap();
    assert!(first["status"] == "error" && first["blocked"] == true && first["retry_after"].as_f64() == Some(60.0), "{first}");
    assert!(str_of(&first, "error").contains("1분"), "{first}");
    // 차단 중에는 다른 책도, 수동 재시도(refresh)도 서점에 요청하지 않는다
    assert_eq!(service.candidates("naver", "b.epub", false, false).unwrap()["blocked"], true);
    assert_eq!(service.candidates("naver", "a.epub", false, true).unwrap()["blocked"], true);
    assert_eq!(calls.lock().unwrap().len(), 1);
    // 차단 응답은 책별 캐시에 남기지 않는다
    assert!(files_under(service.cache()).iter().all(|f| !f.file_name().unwrap().to_string_lossy().starts_with("search-")));
    // 다른 서점은 영향이 없다
    service.set_hooks(service.hooks().clone().with_search(not_found));
    assert_eq!(service.candidates("yes24", "a.epub", false, false).unwrap()["status"], "not_found");
    // 대기 후 시험 요청이 성공하면 차단이 풀리고 backoff가 초기화된다
    clock.advance(61.0);
    assert_eq!(service.candidates("naver", "b.epub", false, false).unwrap()["status"], "not_found");
    service.set_hooks(service.hooks().clone().with_search(|_store, _metadata| Err(http_error(429))));
    assert_eq!(service.candidates("naver", "a.epub", false, false).unwrap()["retry_after"].as_f64(), Some(60.0));
}

#[test]
fn test_store_block_backoff_grows_when_probe_is_still_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let clock = FakeClock::new(5000.0);
    make_epub(&root.join("a.epub"), &EpubSpec::cover("3.0", false));
    let service = CoverManagement::with_hooks(root, CoverHooks::default().with_clock(clock.clone()).with_search(|_store, _metadata| Err(http_error(418))));
    assert_eq!(service.candidates("naver", "a.epub", false, false).unwrap()["retry_after"].as_f64(), Some(60.0));
    clock.advance(61.0);
    let second = service.candidates("naver", "a.epub", false, true).unwrap();
    assert!(second["retry_after"].as_f64() == Some(120.0) && str_of(&second, "error").contains("2분"), "{second}");
}

#[test]
fn test_non_rate_limit_http_error_is_cached_briefly_and_logs_status() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let clock = FakeClock::new(5000.0);
    make_epub(&root.join("a.epub"), &EpubSpec::cover("3.0", false));
    make_epub(&root.join("b.epub"), &EpubSpec::cover("3.0", false));
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = calls.clone();
    let service = CoverManagement::with_hooks(
        root,
        CoverHooks::default().with_clock(clock).with_search(move |_store, metadata| {
            sink.lock().unwrap().push(metadata["title"].as_str().unwrap().to_string());
            Err(http_error(500))
        }),
    );
    let log = LogCapture::start(log::Level::Warn);
    let result = service.candidates("naver", "a.epub", false, false).unwrap();
    assert!(result["status"] == "error" && !result.as_object().unwrap().contains_key("blocked"), "{result}");
    assert!(log.text().contains("500"), "{}", log.text());
    assert_eq!(service.candidates("naver", "a.epub", false, false).unwrap()["status"], "error");
    assert_eq!(service.candidates("naver", "b.epub", false, false).unwrap()["status"], "error");
    assert_eq!(*calls.lock().unwrap(), ["a", "b"]);
}

#[test]
fn test_insert_illustration_rejects_malformed_base64_without_changing_book() {
    for image_data in ["한글", "YQ=", "YQ==extra"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
        let original = read(&path);
        let err = CoverManagement::new(root).insert_illustration("book.epub", &rev(&path), "chapter", Some(image_data)).unwrap_err();
        assert!(err.to_string().contains("삽화 이미지 데이터"), "image_data={image_data:?}: {err}");
        assert_eq!(read(&path), original, "image_data={image_data:?}");
        assert!(!backups(root).exists(), "image_data={image_data:?}");
    }
}

/// 보안 검토 지적: 썸네일 경로가 표지 크기·화소 상한을 보지 않고 읽고 디코딩했다. 상한을 넘으면 읽거나 디코딩하기 전에 거부해야 한다.
mod added_thumbnail_limits {
    use super::*;

    fn epub_with_cover(root: &Path, width: u32, height: u32) -> PathBuf {
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
        let data = encode_image(width, height, BLUE, ImageFormat::Jpeg);
        rewrite_epub_members(&path, |members| members.set("OPS/cover.jpg", data.clone()));
        path
    }

    fn hooks_with(limits: Limits) -> EpubCoverHooks {
        EpubCoverHooks::default().with_limits(limits)
    }

    #[test]
    fn thumbnail_is_made_within_the_default_limits() {
        let tmp = tempfile::tempdir().unwrap();
        let path = epub_with_cover(tmp.path(), 40, 60);
        let thumbnail = epub_cover::extract_epub_thumbnail_with(&path, Some(150), &EpubCoverHooks::default()).expect("기본 상한 안에서는 만든다");
        assert_eq!(image_size(&thumbnail), (40, 60));
    }

    #[test]
    fn thumbnail_decodes_lossy_webp_cover() {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(
            &tmp.path().join("webp-cover.epub"),
            &EpubSpec::cover("3.0", false)
                .with_opf(r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata/><manifest><item id="cover" href="cover.webp" media-type="image/webp" properties="cover-image"/></manifest><spine/></package>"#)
                .with_extra("OPS/cover.webp", include_bytes!("fixtures/cover.webp").to_vec()),
        );

        let thumbnail = epub_cover::extract_epub_thumbnail(&path, Some(150)).expect("WebP 표지를 JPEG 썸네일로 만든다");

        assert_eq!(image_size(&thumbnail), (64, 96));
    }

    #[test]
    fn thumbnail_refuses_an_image_over_the_byte_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let path = epub_with_cover(tmp.path(), 40, 60);
        let hooks = hooks_with(Limits { max_image_bytes: 10, ..Limits::default() });
        assert!(epub_cover::extract_epub_thumbnail_with(&path, Some(150), &hooks).is_none());
    }

    #[test]
    fn thumbnail_refuses_an_image_over_the_pixel_limit_and_accepts_the_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let path = epub_with_cover(tmp.path(), 20, 20);
        let over = hooks_with(Limits { max_image_pixels: 399, ..Limits::default() });
        assert!(epub_cover::extract_epub_thumbnail_with(&path, Some(150), &over).is_none(), "400화소가 399 상한을 넘었는데 만들었다");
        let boundary = hooks_with(Limits { max_image_pixels: 400, ..Limits::default() });
        assert!(epub_cover::extract_epub_thumbnail_with(&path, Some(150), &boundary).is_some(), "상한과 같은 400화소는 허용한다");
    }

    #[test]
    fn preview_refuses_when_the_cover_inspection_reported_an_error() {
        // 검사가 표지 이미지를 읽지 못했다고 보고하면(`cover_error`) 썸네일도 만들지 않는다.
        // 썸네일러를 항상 성공하는 가짜로 바꿔, 다른 상한 검사가 아니라 이 분기가 막는지 따로 확인한다.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        epub_with_cover(root, 40, 60);
        let fake = thumbnailer(|_path, _kind, _width| Some(b"fake thumbnail".to_vec()));
        let inspected = CoverHooks::default().with_thumbnailer(fake.clone()).with_epub_hooks(hooks_with(Limits { max_image_bytes: 10, ..Limits::default() }));
        let outcome = CoverManagement::with_hooks(root, inspected).preview("book.epub", false);
        assert!(outcome.is_err(), "표지 검사가 실패했는데 썸네일을 돌려줬다");
        // 같은 가짜 썸네일러도 검사가 성공하면 그대로 쓴다(위 거부가 가짜 때문이 아님을 확인한다).
        let healthy = CoverHooks::default().with_thumbnailer(fake);
        assert_eq!(CoverManagement::with_hooks(root, healthy).preview("book.epub", false).unwrap(), b"fake thumbnail");
    }
}
