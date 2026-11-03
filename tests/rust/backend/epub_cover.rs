//! `backend::epub_cover` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

mod common;
#[path = "common/book_fixtures.rs"]
mod fx;

// `tests/rust/backend/pdf_delete_pages.rs`에서 옮긴 테스트.
mod pdf_delete_pages {
    use super::fx;

    use fx::{Opts, image_env};

    /// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
    fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
        match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        }
    }

    fn error_text(error: &Option<String>) -> &str {
        error.as_deref().unwrap_or("")
    }

    #[tokio::test]
    async fn test_upload_preview_rejects_oversized_data() {
        let e = image_env(Opts::plain());

        let (jpeg, error) = split(e.bm.preview_insert_upload(&vec![0u8; backend::epub_cover::MAX_IMAGE_BYTES + 1]).await);

        assert!(jpeg.is_none() && error_text(&error).contains("큽니다"), "{error:?}");
    }
}

// `tests/rust/backend/pdf_delete_pages_gaps.rs`에서 옮긴 테스트.
mod pdf_delete_pages_gaps {
    use super::fx;

    use backend::epub_cover::MAX_IMAGE_BYTES;

    use fx::{Edit, Opts};

    fn error_text(error: &Option<String>) -> &str {
        error.as_deref().unwrap_or("")
    }

    #[tokio::test]
    async fn test_upload_rejects_encoded_data_over_byte_limit_before_decoding() {
        let e = fx::env(Opts::plain());
        let encoded_limit = MAX_IMAGE_BYTES.div_ceil(3) * 4;
        let insert = serde_json::json!({"page": 1, "image_data": "A".repeat(encoded_limit + 1)});

        let (result, error) = e.run(&Edit { inserts: Some(vec![insert]), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains("이미지가 비어 있거나 너무 큽니다"), "{error:?}");
    }
}

// `tests/rust/backend/epub_cover_set.rs`에서 옮긴 테스트.
mod epub_cover_set {
    use super::common;

    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use backend::book::BookLookupError;
    use backend::book_manager::{BookLookup, BookManager, BookManagerDeps, BookRef};
    use backend::direct_cover_url::{FetchError, ImageFetcher};
    use backend::epub_cover::{self, CoverError, EpubCoverHooks, Limits};

    use common::{
        epub::{EpubSpec, make_epub},
        es_fake::FakeEsManager,
        faulty_fs::{FaultyFs, other_err},
        fsx::{dir_names, sha256_hex},
        image::{color_jpeg, red_jpeg},
        zipx::{Members, rewrite_epub_members},
    };
    use serde_json::{Value, json};

    const URL: &str = "https://img.example.com/cover.jpg";

    fn has_cover(path: &Path) -> bool {
        epub_cover::inspect_epub(path).unwrap()["has_cover"].as_bool().unwrap()
    }

    fn revision(path: &Path) -> String {
        epub_cover::inspect_epub(path).unwrap()["revision"].as_str().unwrap().to_string()
    }

    /// `path.parent.glob(".cover-*.tmp")`
    fn cover_tmp_files(dir: &Path) -> Vec<String> {
        (if dir.exists() { dir_names(dir) } else { Vec::new() }).into_iter().filter(|n| n.starts_with(".cover-") && n.ends_with(".tmp")).collect()
    }

    // ── XML 확인 도구 (lxml xpath 대응) ────────────────────────

    fn xml_text(data: &[u8]) -> String {
        String::from_utf8(data.to_vec()).unwrap()
    }

    /// local-name이 `name`인 모든 요소를 문서 순서로 돌려준다.
    fn elements<'a, 'i>(doc: &'a roxmltree::Document<'i>, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
        doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == name).collect()
    }

    fn spine_idrefs(path: &Path) -> Vec<String> {
        let text = xml_text(Members::read(path).get("OPS/book.opf"));
        let doc = roxmltree::Document::parse(&text).unwrap();
        elements(&doc, "spine").iter().flat_map(|s| s.children().filter(|c| c.is_element() && c.tag_name().name() == "itemref")).filter_map(|c| c.attribute("idref").map(String::from)).collect()
    }

    /// 형님 EPUB과 같은 SVG 표지, guide, NCX 구조를 만든다.
    fn attach_publisher_cover_page(path: &Path, shared: bool) {
        rewrite_epub_members(path, |files| {
            let mut opf = xml_text(files.get("OPS/book.opf"));
            let version3 = opf.contains(r#"version="3.0""#);
            let mut items = String::from(r#"<item id="titlepage" href="Text/front.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#);
            if version3 {
                items.push_str(r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#);
            }
            opf = opf.replace("</manifest>", &format!("{items}</manifest>"));
            opf = opf.replace("<spine>", r#"<spine toc="ncx"><itemref idref="titlepage"/>"#);
            opf = opf.replace("</package>", r#"<guide><reference type="cover" href="Text/front.xhtml#cover"/></guide></package>"#);
            files.set("OPS/book.opf", opf.into_bytes());
            files.set(
                "OPS/Text/front.xhtml",
                br#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Cover</title></head><body><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="../cover.jpg"/></svg></body></html>"#.to_vec(),
            );
            files.set(
                "OPS/toc.ncx",
                br#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="cover" playOrder="1"><navLabel><text>Cover</text></navLabel><content src="Text/front.xhtml#cover"/></navPoint><navPoint id="chapter" playOrder="2"><navLabel><text>Chapter</text></navLabel><content src="chapter.xhtml"/></navPoint></navMap></ncx>"#.to_vec(),
            );
            if version3 {
                files.set(
                    "OPS/nav.xhtml",
                    br#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="Text/front.xhtml#cover">Cover</a></li><li><a href="chapter.xhtml">Chapter</a></li></ol></nav></body></html>"#.to_vec(),
                );
            }
            if shared {
                files.set("OPS/chapter.xhtml", br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Keep this chapter.</p><img src="cover.jpg"/></body></html>"#.to_vec());
            }
        });
    }

    // ── 대역 (env fixture) ─────────────────────────────────────

    type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync>;

    /// `monkeypatch.setattr("backend.cover_management.fetch_url_image", ...)`의 대응. 테스트 중간에 바꿀 수 있다.
    /// `BookManagerDeps::with_image_fetcher`로 주입한다. 오류 타입은 `FetchError`(구 `EpubCoverError`).
    struct FetchSlot(Mutex<Fetch>);

    impl FetchSlot {
        fn set(&self, f: impl Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync + 'static) {
            *self.0.lock().unwrap() = Box::new(f);
        }
    }

    impl ImageFetcher for FetchSlot {
        fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
            (self.0.lock().unwrap())(url)
        }
    }

    /// `AsyncMock(return_value=(book, None))` get_book.
    struct FakeBooks(Mutex<(Option<BookRef>, Option<String>)>);

    // 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
    #[allow(dead_code)]
    impl FakeBooks {
        fn set_file_type(&self, file_type: &str) {
            self.0.lock().unwrap().0.as_mut().unwrap().file_type = file_type.to_string();
        }

        fn set_missing(&self, message: &str) {
            *self.0.lock().unwrap() = (None, Some(message.to_string()));
        }
    }

    #[async_trait::async_trait]
    impl BookLookup for FakeBooks {
        async fn get_book(&self, book_id: i64) -> Result<BookRef, BookLookupError> {
            match self.0.lock().unwrap().clone() {
                (Some(book), _) => Ok(book),
                (None, Some(error)) => Err(BookLookupError::Failed(error)),
                (None, None) => Err(BookLookupError::NotFound(book_id)),
            }
        }
    }

    // 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
    #[allow(dead_code)]
    /// (manager 대역, EPUB 경로, 책). get_book은 이 EPUB를 가리키는 책을 돌려준다.
    struct Env {
        tmp: tempfile::TempDir,
        path: PathBuf,
        manager: BookManager,
        es: Arc<FakeEsManager>,
        books: Arc<FakeBooks>,
        fetcher: Arc<FetchSlot>,
    }

    fn env() -> Env {
        common::setup_env();
        let tmp = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(FetchSlot(Mutex::new(Box::new(|_| Ok(red_jpeg())))));
        let path = make_epub(&tmp.path().join("분류").join("book.epub"), &EpubSpec::cover("3.0", false));
        let es = FakeEsManager::new();
        let books = Arc::new(FakeBooks(Mutex::new((Some(BookRef { book_id: 1, file_path: path.clone(), file_type: "epub".into() }), None))));
        let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_trash_dirname("0_trash").with_book_lookup(books.clone()).with_image_fetcher(fetcher.clone()));
        Env { tmp, path, manager, es, books, fetcher }
    }

    // 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
    #[allow(dead_code)]
    impl Env {
        fn dir(&self) -> &Path {
            self.tmp.path()
        }

        async fn call(&self, url: &str) -> (Option<Value>, Option<String>) {
            split(self.manager.set_epub_cover(1, Some(url), None).await)
        }

        async fn upload(&self, image_data: &str) -> (Option<Value>, Option<String>) {
            split(self.manager.set_epub_cover(1, None, Some(image_data)).await)
        }

        async fn remove(&self) -> (Option<Value>, Option<String>) {
            split(self.manager.remove_epub_cover(1).await)
        }

        async fn illustrate(&self, before: &str) -> (Option<Value>, Option<String>) {
            split(self.manager.insert_epub_illustration(1, before, Some(URL), None).await)
        }

        fn size(&self) -> u64 {
            fs::metadata(&self.path).unwrap().len()
        }
    }

    /// `"x" in error`: error가 None이면 Python은 TypeError로 실패한다.
    /// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
    fn split(result: Result<Value, String>) -> (Option<Value>, Option<String>) {
        match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        }
    }

    fn err_has(error: &Option<String>, needle: &str) -> bool {
        error.as_deref().unwrap_or_else(|| panic!("error가 None이다 (기대: {needle:?} 포함)")).contains(needle)
    }

    /// `pytest.raises(CoverError, match=needle)`
    fn assert_cover_error<T: std::fmt::Debug>(result: Result<T, CoverError>, needle: &str) {
        match result {
            Err(e) if !e.is_io() => assert!(e.to_string().contains(needle), "CoverError 메시지에 {needle:?}가 없다: {e}"),
            other => panic!("CoverError({needle:?})를 기대했지만 {other:?}"),
        }
    }

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

    /// 백업이 이미 있으면 그것은 처음 원본이다. 쓰다가 실패하면 지금 원본으로 되돌려야 한다.
    #[test]
    fn test_failed_write_restores_the_current_file_not_the_older_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path();

        let path = make_epub(&tmp_path.join("book.epub"), &EpubSpec::cover("3.0", false));
        let trash = tmp_path.join("0_trash");
        epub_cover::add_cover_backup_with(&path, &red_jpeg(), &revision(&path), &trash, false, Some("book.epub"), &EpubCoverHooks::default()).unwrap();
        let current = fs::read(&path).unwrap();
        let older = fs::read(trash.join("book.epub")).unwrap();
        assert_ne!(current, older);

        let counter = AtomicUsize::new(0);
        let faulty = FaultyFs::new();
        faulty.set_sync_file(move |_file| {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            // 롤백 사본(1), 편집본(2), 본 파일에 쓴 뒤(3)
            if n == 3 { Some(other_err("write failed")) } else { None }
        });
        let hooks = EpubCoverHooks::default().with_fs(faulty);
        let result = epub_cover::add_cover_backup_with(&path, &color_jpeg("green"), &revision(&path), &trash, true, Some("book.epub"), &hooks);
        assert!(matches!(&result, Err(e) if e.is_io()), "OSError를 기대했지만 {result:?}");

        assert_eq!(fs::read(&path).unwrap(), current);
        assert_eq!(fs::read(trash.join("book.epub")).unwrap(), older);
        assert!(cover_tmp_files(tmp_path).is_empty());
    }

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
        assert!(BookManager::find_epub_cover_member(&mut archive).unwrap().contains("tm-cover-"));
    }

    // ── epub_cover 방어 경로: 표지 분리, 교체 전 표지 보존, 검증 실패 ──

    /// SVG 표지 페이지(Text/front.xhtml)를 가진 EPUB 3. `files`로 멤버를 덮어쓴다.
    fn publisher_cover_epub(tmp_path: &Path, files: &[(&str, Vec<u8>)]) -> PathBuf {
        let path = make_epub(&tmp_path.join("book.epub"), &EpubSpec::cover("3.0", true));
        attach_publisher_cover_page(&path, false);
        if !files.is_empty() {
            rewrite_epub_members(&path, |members_| {
                for (name, data) in files {
                    members_.set(name, data.clone());
                }
            });
        }
        path
    }

    fn remove_directly_with(path: &Path, tmp_path: &Path, hooks: &EpubCoverHooks) -> Result<Value, CoverError> {
        epub_cover::remove_cover_with(path, &revision(path), &tmp_path.join(".cover_backups"), hooks)
    }

    #[test]
    fn test_oversized_body_page_stops_cover_removal_without_touching_the_book() {
        let tmp = tempfile::tempdir().unwrap();
        let mut chapter = b"<html><body>".to_vec();
        chapter.extend(std::iter::repeat_n(b'x', 6000));
        chapter.extend_from_slice(b"</body></html>");
        let path = publisher_cover_epub(tmp.path(), &[("OPS/chapter.xhtml", chapter)]);
        let original = fs::read(&path).unwrap();
        let hooks = EpubCoverHooks::default().with_limits(Limits { max_xml_bytes: 5000, ..Limits::default() });

        assert_cover_error(remove_directly_with(&path, tmp.path(), &hooks), "너무 큽니다");

        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!tmp.path().join(".cover_backups").exists());
    }

    #[test]
    fn test_corrupt_staged_epub_is_rejected_for_illustration_and_removal() {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
        let original = fs::read(&path).unwrap();
        let hooks = EpubCoverHooks::default().with_testzip(Arc::new(|_path: &Path| Some("OPS/chapter.xhtml".to_string())));

        assert_cover_error(epub_cover::add_illustration_with(&path, &red_jpeg(), &revision(&path), &tmp.path().join(".cover_backups"), "chapter", &hooks), "검증에 실패");
        assert_eq!(fs::read(&path).unwrap(), original);

        let removal_dir = tmp.path().join("removal");
        let covered = publisher_cover_epub(&removal_dir, &[]);
        let original = fs::read(&covered).unwrap();
        assert_cover_error(remove_directly_with(&covered, &removal_dir, &hooks), "검증에 실패");
        assert_eq!(fs::read(&covered).unwrap(), original);
    }

    #[test]
    fn test_removal_is_rejected_when_the_staged_epub_still_reports_a_cover() {
        let tmp = tempfile::tempdir().unwrap();
        let path = publisher_cover_epub(tmp.path(), &[]);
        let original = fs::read(&path).unwrap();
        let rev = revision(&path);
        let hooks = EpubCoverHooks::default().with_inspect_archive(Arc::new(|_inspected: Value| json!({"has_cover": true})));

        assert_cover_error(epub_cover::remove_cover_with(&path, &rev, &tmp.path().join(".cover_backups"), &hooks), "표지를 삭제한 EPUB의 검증에 실패");

        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn test_unnamed_backup_collision_is_an_error_not_a_silent_reuse() {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
        let original = fs::read(&path).unwrap();
        let rev = revision(&path);
        let hooks = EpubCoverHooks::default().with_uuid_hex(Arc::new(|| "f".repeat(32)));
        let backup_dir = tmp.path().join(".cover_backups");
        fs::create_dir(&backup_dir).unwrap();
        let existing = backup_dir.join(format!("{}-{}.epub", sha256_hex(format!("{}:{}", path.display(), rev).as_bytes()), "f".repeat(32)));
        fs::write(&existing, b"older backup").unwrap();

        let result = epub_cover::add_cover_with(&path, &red_jpeg(), &rev, &backup_dir, false, &hooks);
        match result {
            Err(e) if e.is_io() => assert_eq!(e.io_kind(), Some(std::io::ErrorKind::AlreadyExists)),
            other => panic!("FileExistsError를 기대했지만 {other:?}"),
        }

        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(&existing).unwrap(), b"older backup");
    }
}
