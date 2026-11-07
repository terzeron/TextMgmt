mod common;
use crate::common::cover_candidates::cached_candidates;
use crate::common::epub_fixtures::{backups, epub_with_cover, hooks_with, read, rev, str_of, truthy};
use crate::common::epub_fixtures::{fetcher, thumbnailer};
use common::clock::{Event, FakeClock};
use common::epub::{EpubSpec, make_epub};
use common::fsx::{files_under, ino_mtime, sha256_hex, touch};
use common::image::{b64, red_jpeg};
use common::log_capture::LogCapture;
use common::zipx::rewrite_epub_members;
use cover_manager::cover_management::CandidateSearch;
use cover_manager::cover_management::Thumbnailer;
use cover_manager::cover_management::{ApplyImageModel, Backup, CoverHooks, CoverManagement, EpubThumbnailer};
use cover_manager::cover_sources::{STORES, SearchError, SearchHooks};
use cover_manager::cover_sources::{SearchStatus, StoreSearch};
use cover_manager::epub_cover;
use cover_manager::epub_cover::EpubInspection;
use cover_manager::epub_cover::{CoverError, Limits, MAX_IMAGE_BYTES};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// 표지 목록 한 쪽의 JSON 모양. 프런트엔드가 받는 응답과 같다.
fn page(service: &CoverManagement, directory: &str, limit: usize, covered: bool) -> Value {
    serde_json::to_value(service.missing(directory, "", limit, covered).unwrap()).unwrap()
}

fn first_item(service: &CoverManagement) -> Value {
    page(service, "", 60, false)["items"][0].clone()
}

/// 표지 후보 검색 결과의 JSON 모양. 프런트엔드가 받는 응답과 같다.
fn found(service: &CoverManagement, store: &str, relative: &str, include_covered: bool, refresh: bool) -> Value {
    serde_json::to_value(service.candidates(store, relative, include_covered, refresh).unwrap()).unwrap()
}

fn not_found(_store: &str, _metadata: &EpubInspection) -> Result<StoreSearch, SearchError> {
    Ok(StoreSearch { status: SearchStatus::NotFound, ..StoreSearch::default() })
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
        let item = page(&service, "folder.epub", 60, false)["items"][0].clone();
        assert_eq!(item["title"], title, "filename={filename:?}");
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = seen.clone();
        service.set_hooks(service.hooks().clone().with_search(move |_store, metadata| {
            sink.lock().unwrap().push(metadata.title.clone());
            Ok(StoreSearch { status: SearchStatus::NotFound, ..StoreSearch::default() })
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
            sink.lock().unwrap().push((store.to_string(), metadata.title.clone()));
            Ok(StoreSearch { status: SearchStatus::NotFound, ..StoreSearch::default() })
        }));
        for store in STORES {
            let old_key = format!("{store}:{name}:{revision}") + if *store == "aladin" { ":books-v2" } else { "" };
            let old_hash = sha256_hex(old_key);
            for parent in [service.cache().to_path_buf(), service.book_cache(&name)] {
                service.write(&parent.join(format!("search-{old_hash}.json")), br#"{"status":"not_found","candidates":[]}"#);
            }
            for _ in 0..2 {
                found(&service, store, &name, covered, false);
            }
        }
        let expected: Vec<(String, String)> = STORES.iter().map(|store| (store.to_string(), "저자 제목 1".to_string())).collect();
        assert_eq!(*calls.lock().unwrap(), expected, "covered={covered}");
        assert_eq!(page(&service, "", 60, covered)["items"][0]["title"], "저자 제목 1", "covered={covered}");
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
fn test_browser_image_replaces_cover_after_server_candidates_are_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let saved = service.apply("book.epub", cached.id(0), &rev(&path)).unwrap();
    assert!(!service.book_cache("book.epub").exists());
    let before = read(&path);
    assert!(service.apply_image("book.epub", "not valid base64!", &saved.revision).is_err());
    assert!(service.apply_image("book.epub", &b64(b"invalid image"), &saved.revision).is_err());
    let err = service.apply_image("book.epub", &b64(&red_jpeg()), "stale").unwrap_err();
    assert!(err.to_string().contains("변경"), "{err}");
    assert_eq!(read(&path), before);
    let replaced = service.apply_image("book.epub", &b64(&red_jpeg()), &saved.revision).unwrap();
    assert_ne!(replaced.revision, saved.revision);
    assert_eq!(read(&root.join(&replaced.backup_path)), before);
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
    assert_eq!(found(&service, "yes24", "book.epub", true, false)["status"], "not_found");
    assert_eq!(read(&path), original);
    let saved = service.apply_url("book.epub", "https://example.org/a", &rev(&path), true).unwrap();
    assert_eq!(read(&root.join(&saved.backup_path)), original);
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
    assert_eq!(found(&service, "yes24", "book.epub", false, false)["status"], "error");
    assert_eq!(found(&service, "yes24", "book.epub", false, false)["status"], "error");
    assert_eq!(*calls.lock().unwrap(), ["yes24"]);
    assert_eq!(found(&service, "yes24", "book.epub", false, true)["status"], "error");
    assert_eq!(*calls.lock().unwrap(), ["yes24", "yes24"]);
    let key = sha256_hex(format!("aladin:book.epub:{}", rev(&path)));
    service.write(&service.book_cache("book.epub").join(format!("search-{key}.json")), br#"{"status":"not_found","candidates":[]}"#);
    assert_eq!(found(&service, "aladin", "book.epub", false, false)["status"], "error");
    assert_eq!(calls.lock().unwrap().last().unwrap(), "aladin");
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
        assert!(metadata.has_cover, "version={version}");
        assert_eq!(metadata.author, "저자", "version={version}");
        assert_eq!(page(&CoverManagement::new(root), "", 60, false)["items"], json!([]), "version={version}");
        let err = epub_cover::add_cover(&path, &red_jpeg(), &metadata.revision, &backups(root), false).unwrap_err();
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
    let result = service.apply_url("book.epub", "https://example.org/cover.jpg", &metadata.revision, false).unwrap();
    assert_eq!(*downloaded.lock().unwrap(), ["https://example.org/cover.jpg"]);
    assert!(epub_cover::inspect_epub(&path).unwrap().has_cover);
    assert!(!service.candidate_path(cached.id(0)).exists());
    let saved = read(&path);
    service.clear_cache("book.epub", &result.revision).unwrap();
    assert_eq!(read(&path), saved);
    assert!(root.join(&result.backup_path).exists());
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
    // 이전 flat cache에 남은 다른 책의 후보는 지우지 않는다.
    let legacy_other = service.cache().join(format!("candidate-{}.json", "c".repeat(64)));
    service.write(&legacy_other, json!({"store": "yes24", "file_path": "other.epub", "revision": "r", "image_url": "https://image.yes24.com/o.jpg"}).to_string().as_bytes());
    let original = read(&path);
    let err = service.clear_cache("book.epub", "stale").unwrap_err();
    assert!(err.to_string().contains("변경"), "{err}");
    assert!(service.candidate_path(cached.id(0)).exists());
    service.clear_cache("book.epub", &rev(&path)).unwrap();
    assert!(!service.candidate_path(cached.id(0)).exists());
    assert!(service.candidate_path(other.id(0)).exists());
    assert!(legacy_other.exists());
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
    let page = page(&manager, "目录", 60, false);
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
/// 이름은 소문자로 바꾼 뒤 코드포인트 순서로 정렬한다. `ß`(U+00DF)는 `z`보다 뒤다.
fn filesystem_listing_orders_by_lowercased_name() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir(root.join("zeta")).unwrap();
    std::fs::create_dir(root.join("ßeta")).unwrap();
    std::fs::write(root.join("z.epub"), b"bad zip").unwrap();
    std::fs::write(root.join("ß.epub"), b"bad zip").unwrap();

    let manager = CoverManagement::new(root);
    let labels: Vec<String> = manager.directories("").unwrap().into_iter().map(|entry| entry["label"].as_str().unwrap().to_owned()).collect();
    assert_eq!(labels, ["zeta", "ßeta"]);

    let page = page(&manager, "", 10, false);
    let paths: Vec<&str> = page["items"].as_array().unwrap().iter().map(|item| item["file_path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["z.epub", "ß.epub"]);
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
        // 표지 ID와 첫 쪽이 같은 쪽을 가리키면 한 번만 본다.
        ("Text/front.xhtml", r#"<meta name="cover" content="page" />"#, "", r#"<img src="missing.jpg" />"#.to_string(), false),
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
        assert_eq!(inspected.has_cover, has_cover, "page_href={page_href}, markup={image_markup}");
        let mut zip_archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert_eq!(epub_cover::find_epub_cover_member(&mut zip_archive), has_cover.then(|| "OEBPS/Images/120(550).jpg".to_string()), "page_href={page_href}, markup={image_markup}");
        drop(zip_archive);
        let listed = !page(&CoverManagement::new(root), "", 60, false)["items"].as_array().unwrap().is_empty();
        assert_eq!(listed, !has_cover, "page_href={page_href}, markup={image_markup}");
        if has_cover {
            assert!(epub_cover::extract_epub_thumbnail(&path, None).is_some(), "page_href={page_href}, markup={image_markup}");
            let err = epub_cover::add_cover(&path, &red_jpeg(), &inspected.revision, &backups(root), false).unwrap_err();
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
    assert_eq!(page(&manager, "", 60, false)["items"].as_array().unwrap().len(), 1);
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
        assert!(!restarted.image(&candidate.candidate_id).unwrap().is_empty());
    }
    assert_eq!(cached.fetches.lock().unwrap().len(), 2);
    restarted.set_hooks(restarted.hooks().clone().with_search(|_store, _metadata| panic!("캐시한 검색을 다시 요청했다")));
    let restored = found(&restarted, "yes24", "book.epub", false, false)["candidates"].clone();
    let restored_ids: Vec<&str> = restored.as_array().unwrap().iter().map(|item| str_of(item, "candidate_id")).collect();
    let cached_ids: Vec<&str> = cached.candidates.iter().map(|item| item.candidate_id.as_str()).collect();
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
    assert!(epub_cover::inspect_epub(&path).unwrap().has_cover);
    assert!(root.join(&result.backup_path).is_file());
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
    let result = found(&service, "yes24", "book.epub", false, false)["candidates"][0].clone();
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
            record.revision = "stale".to_string();
            service.write(&service.candidate_path(&candidate_id), &serde_json::to_vec(&record).unwrap());
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
    assert_eq!(found(&service, "yes24", "book.epub", false, false)["status"], "error");
    assert_eq!(found(&service, "yes24", "book.epub", false, false)["status"], "error");
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
    let values = Mutex::new(VecDeque::from([None, None, Some(CandidateSearch { status: SearchStatus::NotFound, ..CandidateSearch::default() })]));
    service.set_hooks(service.hooks().clone().with_cached_search(move |_store, _relative| values.lock().unwrap().pop_front().expect("_cached_search를 세 번만 부른다")));
    assert_eq!(found(&service, "yes24", "book.epub", false, false)["status"], "not_found");
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
    assert_eq!(found(&service, "yes24", "book.epub", false, false)["status"], "success");
    let bad = service.cache().join("bad.json");
    service.write(&bad, b"[]");
    assert!(service.read_json::<Value>(&bad).is_none());
    service.write(&bad, b"not json");
    assert!(service.read_json::<Value>(&bad).is_none());
    assert_eq!(record.file_path, "book.epub");
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
    assert!(result.warning.is_some(), "{result:?}");
    assert!(epub_cover::inspect_epub(&path).unwrap().has_cover);
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
    record.image = None;
    service.write(&record_path, &serde_json::to_vec(&record).unwrap());
    std::fs::remove_file(record_path.parent().unwrap().join(format!("image-{}.jpg", cached.id(0)))).unwrap();
    let result = found(&service, "yes24", "book.epub", false, false)["candidates"][0].clone();
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
            epub_cover::find_epub_cover_member_with(&mut archive, &|name| if name == "OPS/cover.xhtml" { Some(5 * 1024 * 1024) } else { None })
        } else {
            epub_cover::find_epub_cover_member(&mut archive)
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
    assert_eq!(epub_cover::find_epub_cover_member(&mut archive), None);
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
            sink.lock().unwrap().push(metadata.title.clone());
            Err(http_error(418))
        }),
    );
    let first = found(&service, "naver", "a.epub", false, false);
    assert!(first["status"] == "error" && first["blocked"] == true && first["retry_after"].as_f64() == Some(60.0), "{first}");
    assert!(str_of(&first, "error").contains("1분"), "{first}");
    // 차단 중에는 다른 책도, 수동 재시도(refresh)도 서점에 요청하지 않는다
    assert_eq!(found(&service, "naver", "b.epub", false, false)["blocked"], true);
    assert_eq!(found(&service, "naver", "a.epub", false, true)["blocked"], true);
    assert_eq!(calls.lock().unwrap().len(), 1);
    // 차단 응답은 책별 캐시에 남기지 않는다
    assert!(files_under(service.cache()).iter().all(|f| !f.file_name().unwrap().to_string_lossy().starts_with("search-")));
    // 다른 서점은 영향이 없다
    service.set_hooks(service.hooks().clone().with_search(not_found));
    assert_eq!(found(&service, "yes24", "a.epub", false, false)["status"], "not_found");
    // 대기 후 시험 요청이 성공하면 차단이 풀리고 backoff가 초기화된다
    clock.advance(61.0);
    assert_eq!(found(&service, "naver", "b.epub", false, false)["status"], "not_found");
    service.set_hooks(service.hooks().clone().with_search(|_store, _metadata| Err(http_error(429))));
    assert_eq!(found(&service, "naver", "a.epub", false, false)["retry_after"].as_f64(), Some(60.0));
}

#[test]
fn test_store_block_backoff_grows_when_probe_is_still_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let clock = FakeClock::new(5000.0);
    make_epub(&root.join("a.epub"), &EpubSpec::cover("3.0", false));
    let service = CoverManagement::with_hooks(root, CoverHooks::default().with_clock(clock.clone()).with_search(|_store, _metadata| Err(http_error(418))));
    assert_eq!(found(&service, "naver", "a.epub", false, false)["retry_after"].as_f64(), Some(60.0));
    clock.advance(61.0);
    let second = found(&service, "naver", "a.epub", false, true);
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
            sink.lock().unwrap().push(metadata.title.clone());
            Err(http_error(500))
        }),
    );
    let log = LogCapture::start(log::Level::Warn);
    let result = found(&service, "naver", "a.epub", false, false);
    assert!(result["status"] == "error" && !result.as_object().unwrap().contains_key("blocked"), "{result}");
    assert!(log.text().contains("500"), "{}", log.text());
    assert_eq!(found(&service, "naver", "a.epub", false, false)["status"], "error");
    assert_eq!(found(&service, "naver", "b.epub", false, false)["status"], "error");
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

mod clean_title {
    use cover_manager::cover_management::clean_title;

    #[test]
    fn titles_match_reference_cases() {
        assert_eq!(clean_title("[랜들 개릿] 다아시 경의 모험"), "랜들 개릿 다아시 경의 모험");
        assert_eq!(clean_title("[작가] 구운몽—꿈(1) & 이야기!"), "작가 구운몽 꿈 1 이야기");
        assert_eq!(clean_title("제목.v2"), "제목 v2");
        assert_eq!(clean_title("  책\t 이름   "), "책 이름");
        assert_eq!(clean_title("【作者】中文_タイトル〈第2巻〉"), "作者 中文 タイトル 第2巻");
        assert_eq!(clean_title("★[!]★"), "제목 없음");
    }

    #[test]
    fn nfd_hangul_is_composed() {
        let nfd = "\u{1112}\u{1161}\u{11AB}\u{1100}\u{1173}\u{11AF}";
        assert_eq!(clean_title(nfd), "한글");
    }
}

mod cover_repair {
    use super::common;
    use crate::common::epub_fixtures::{broken_cover, first_item, truthy};

    use cover_manager::cover_management::CoverManagement;

    use common::zipx::rewrite_epub_members;

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
    fn test_broken_package_remains_a_fatal_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = broken_cover(tmp.path());
        rewrite_epub_members(&path, |members| members.set("OPS/book.opf", &b"not XML"[..]));
        let item = first_item(&CoverManagement::new(tmp.path()), false);
        assert_eq!(item["status"], "error");
        assert!(!item.as_object().unwrap().contains_key("cover_error"), "{item}");
    }
}

/// 모든 요청에 `status`와 빈 본문을 돌려주는 서점 세션.
struct StatusSession(u16);

impl ::http_client::HttpSession for StatusSession {
    fn get(&self, _url: &str, _params: &[(String, String)]) -> Result<::http_client::HttpResponse, ::http_client::HttpError> {
        Ok(::http_client::HttpResponse::from_chunks(self.0, Vec::new(), Vec::new(), Arc::new(std::sync::atomic::AtomicBool::new(false))))
    }
}

fn status_hooks(status: u16) -> SearchHooks {
    SearchHooks::default().with_session_factory(Arc::new(move |_store| Arc::new(StatusSession(status)) as Arc<dyn ::http_client::HttpSession>)).with_validate(|_store, url| Ok(url.to_string()))
}

#[test]
fn test_default_image_fetch_reports_cancel_rejected_url_and_http_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let candidate_id = cached.id(0).to_string();
    std::fs::remove_file(service.candidate_path(&candidate_id).parent().unwrap().join(format!("image-{candidate_id}.jpg"))).unwrap();
    let cases: [(SearchHooks, &str); 3] = [
        (SearchHooks::default().with_cancel(Arc::new(|| true)), "취소"),
        (SearchHooks::default().with_validate(|_store, _url| Err(cover_manager::cover_sources::CoverSourceError("허용하지 않는 주소".into()))), "허용하지 않는 주소"),
        (status_hooks(404), "HTTP 404"),
    ];
    for (hooks, expected) in cases {
        service.set_hooks(CoverHooks::default().with_search_hooks(hooks));
        let err = service.image(&candidate_id).unwrap_err();
        assert!(err.to_string().contains(expected), "expected={expected}: {err}");
    }
}

#[test]
fn test_store_search_failures_are_reported_as_search_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::with_hooks(root, CoverHooks::default().with_search_hooks(status_hooks(500)));
    let log = LogCapture::start(log::Level::Warn);
    assert_eq!(found(&service, "yes24", "book.epub", false, true)["status"], "error");
    assert!(log.text().contains("HTTP 500"), "{}", log.text());
    service.set_hooks(service.hooks().clone().with_search(|_store, _metadata| Err(SearchError::Transport("connection reset".into()))));
    assert_eq!(found(&service, "yes24", "book.epub", false, true)["status"], "error");
    assert!(log.text().contains("RequestException"), "{}", log.text());
    service.set_hooks(service.hooks().clone().with_search(|_store, _metadata| {
        let candidate = cover_manager::cover_sources::StoreCandidate { title: "제목".into(), ..Default::default() };
        Ok(StoreSearch { status: SearchStatus::Success, search_url: String::new(), candidates: vec![candidate] })
    }));
    assert_eq!(found(&service, "yes24", "book.epub", false, true)["status"], "error");
    assert!(log.text().contains("후보에 image_url이 없습니다"), "{}", log.text());
}

#[test]
fn test_cancel_after_the_book_lock_stops_at_the_store_lock_or_the_request_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 책 잠금을 잡은 뒤(메타데이터를 읽을 때) 화면이 요청을 버린다.
    let flag = cancelled.clone();
    let hooks = CoverHooks::default().with_clock(FakeClock::new(5000.0)).with_search(not_found).with_inspect_epub(move |path| {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        epub_cover::inspect_epub(path)
    });
    let mut service = CoverManagement::with_hooks(root, hooks);
    let probe = cancelled.clone();
    let cancel: cover_manager::cover_sources::CancelFn = Arc::new(move || probe.load(std::sync::atomic::Ordering::SeqCst));
    let err = cover_manager::cover_management::with_cancel_scope(cancel.clone(), || service.candidates("yes24", "book.epub", false, false)).unwrap_err();
    assert!(err.is_cancelled(), "{err}");
    // naver는 요청 간격을 기다리는 동안 취소를 확인한다.
    service.set_hooks(service.hooks().clone().with_store_lock(|_cache, _store| Box::new(())));
    assert_eq!(found(&service, "naver", "book.epub", false, false)["status"], "not_found");
    cancelled.store(false, std::sync::atomic::Ordering::SeqCst);
    let err = cover_manager::cover_management::with_cancel_scope(cancel, || service.candidates("naver", "book.epub", false, true)).unwrap_err();
    assert!(err.is_cancelled(), "{err}");
    assert!(epub_cover::inspect_epub(&path).is_ok());
}

#[test]
fn test_image_fetch_reports_unknown_store_and_unopenable_store_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let mut service = CoverManagement::new(root);
    let cached = cached_candidates(&mut service, "book.epub", &["yes24"]);
    let candidate_id = cached.id(0).to_string();
    let record_path = service.candidate_path(&candidate_id);
    std::fs::remove_file(record_path.parent().unwrap().join(format!("image-{candidate_id}.jpg"))).unwrap();
    service.set_hooks(CoverHooks::default());
    let lock = service.cache().join("yes24.lock");
    let _ = std::fs::remove_file(&lock);
    std::fs::create_dir(&lock).unwrap();
    assert!(service.image(&candidate_id).is_err());
    let mut record = service.candidate(&candidate_id).unwrap();
    record.store = "unknown".to_string();
    service.write(&record_path, &serde_json::to_vec(&record).unwrap());
    let err = service.image(&candidate_id).unwrap_err();
    assert!(err.to_string().contains("지원하지 않는 서점입니다"), "{err}");
}

#[test]
fn test_cache_write_into_a_read_only_directory_fails_and_is_logged() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let service = CoverManagement::new(tmp.path());
    let locked = tmp.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = service.try_write(&locked.join("a.json"), b"{}");
    let log = LogCapture::start(log::Level::Warn);
    service.write(&locked.join("a.json"), b"{}");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(outcome.is_err());
    assert!(log.text().contains("Cover cache write failed"), "{}", log.text());
    assert_eq!(std::fs::read_dir(&locked).unwrap().count(), 0);
}

#[test]
fn test_clearing_a_book_cache_skips_an_unlistable_legacy_cache_directory() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", false));
    let service = CoverManagement::new(root);
    std::fs::create_dir_all(service.cache().join(".book_locks")).unwrap();
    // 쓰고 들어갈 수는 있지만 목록은 읽을 수 없는 캐시 디렉토리.
    std::fs::set_permissions(service.cache(), std::fs::Permissions::from_mode(0o300)).unwrap();
    let outcome = service.clear_cache("book.epub", &rev(&path));
    std::fs::set_permissions(service.cache(), std::fs::Permissions::from_mode(0o755)).unwrap();
    outcome.unwrap();
}
