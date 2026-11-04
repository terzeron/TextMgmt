//! `backend::epub_cover` 단위 테스트.

#[path = "common/cover_fixtures.rs"]
mod cover_fixtures;

mod common;
#[path = "common/book_fixtures.rs"]
mod fx;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering, Mutex};
use backend::book_metadata::BookLookupError;
use backend::book_manager::{BookLookup, BookManager, BookManagerDeps, BookRef};
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::epub_cover;
use backend::epub_cover::{CoverError, EpubCoverHooks, Limits};
use common::epub::{EpubSpec, make_epub};
use common::es_fake::FakeEsManager;
use common::faulty_fs::{FaultyFs, other_err};
use common::fsx::{ino_mtime, sha256_hex};
use common::image::{color_jpeg, red_jpeg};
use common::zipx::{read_zip_entry, rewrite_epub_members};
use serde_json::{json, Value};
use std::collections::VecDeque;
use crate::common::{epub::append_child, epub::attr_of_element_with_id, epub::count_elements, epub::remove_element, fsx::count_matching, image::encode_image, image::image_size, image::RED};
use image::ImageFormat;
use crate::common::shared_fixtures::{assert_cover_error, backups, cover_tmp_files, epub_with_cover, has_cover, hooks_with, opf_of, publisher_cover_epub, read, rev, revision, str_of, truthy};

const URL: &str = "https://img.example.com/cover.jpg";

// ── 대역 (env fixture) ─────────────────────────────────────

type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync>;

/// `monkeypatch.setattr("backend.cover_management.fetch_url_image", ...)`의 대응. 테스트 중간에 바꿀 수 있다.
/// `BookManagerDeps::with_image_fetcher`로 주입한다. 오류 타입은 `FetchError`(구 `EpubCoverError`).
struct FetchSlot(Mutex<Fetch>);

impl FetchSlot {}

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

mod member_path {
    use backend::epub_cover::member_path;

    #[test]
    fn member_path_resolves_like_python() {
        assert_eq!(member_path("OPS/Text", "../a%20b.jpg"), "OPS/a b.jpg");
        assert_eq!(member_path("OPS", "https://example.com/a.jpg"), "");
        assert_eq!(member_path("OPS", "../../x.jpg"), "");
        assert_eq!(member_path("OPS", "#top"), "");
        assert_eq!(member_path("OPS", ""), "");
        assert_eq!(member_path("OPS", "a.xhtml#frag"), "OPS/a.xhtml");
        assert_eq!(member_path("", "a.jpg?x=1"), "a.jpg");
    }
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

mod image_page_cropping {
    use crate::common::shared_fixtures::{assert_refused, size, text_of};
    use super::common;
    use std::collections::BTreeMap;
    use std::path::Path;
    use backend::epub_cover::{edit_images, EditImages, revision};
    use common::image::{jpeg_bytes};
    use common::zipx::{Members as ZipMembers, write_zip_with, ZipMode};
    use image::{Rgb, RgbImage};
    use serde_json::{json};

    type Members = Vec<(String, Vec<u8>)>;

    /// mimetype을 맨 앞에 두고 나머지 순서는 그대로 둔다. 멤버는 모두 무압축으로 쓴다.
    fn write_zip(path: &Path, mut members: Members) {
        let pos = members.iter().position(|(n, _)| n == "mimetype").expect("mimetype");
        let mimetype = members.remove(pos);
        members.insert(0, mimetype);
        let entries: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        write_zip_with(path, &entries, ZipMode::Stored);
    }

    fn read(path: &Path) -> BTreeMap<String, Vec<u8>> {
        let members = ZipMembers::read(path);
        members.names().into_iter().map(|name| (name.clone(), members.get(&name).to_vec())).collect()
    }

    // ── <svg><image> 로 이미지를 넣은 쪽 ──────────────────────────────────────────

    fn svg_epub(path: &Path, size: (u32, u32), title_text: &str, cover: bool) {
        let jpeg_data = jpeg_bytes(&RgbImage::from_pixel(size.0, size.1, Rgb([200, 30, 30])), 75);
        let (w, h) = size;
        let page = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:xlink="http://www.w3.org/1999/xlink"><head><title>c</title></head><body><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="100%" height="100%"><title>{title_text}</title><image width="{w}" height="{h}" xlink:href="c.jpg"/></svg></body></html>"#
        );
        let opf = concat!(
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">"#,
            r#"<dc:identifier id="id">x</dc:identifier><dc:title>t</dc:title><dc:language>ko</dc:language></metadata><manifest>"#,
            r#"<item id="c" href="c.xhtml" media-type="application/xhtml+xml"/><item id="ci" href="c.jpg" media-type="image/jpeg" __COVER__/>"#,
            r#"<item id="d" href="d.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c"/><itemref idref="d"/></spine></package>"#,
        )
        .replace("__COVER__", if cover { r#"properties="cover-image""# } else { "" });
        let members: Members = vec![
            ("mimetype".into(), b"application/epub+zip".to_vec()),
            ("META-INF/container.xml".into(), br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.to_vec()),
            ("OEBPS/content.opf".into(), opf.into_bytes()),
            ("OEBPS/c.xhtml".into(), page.into_bytes()),
            ("OEBPS/c.jpg".into(), jpeg_data),
            ("OEBPS/d.xhtml".into(), r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>d</title></head><body><p>본문</p></body></html>"#.as_bytes().to_vec()),
        ];
        write_zip(path, members);
    }

    #[test]
    fn test_svg_image_page_can_be_cropped_and_viewbox_follows() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("svg.epub");
        svg_epub(&path, (300, 400), "표지", false);
        edit_images(&path, &revision(&path), &tmp.path().join("trash"), EditImages { crops: vec![json!({"idref": "c", "rect": [0.0, 0.0, 0.5, 0.5, 0.0]})], ..EditImages::default() }).unwrap();
        let members = read(&path);
        assert_eq!(size(&members["OEBPS/c.jpg"]), (150, 200));
        let page = text_of(&members, "OEBPS/c.xhtml");
        assert!(page.contains(r#"viewBox="0 0 150 200""#), "{page}");
        assert!(page.contains(r#"width="150""#) && page.contains(r#"height="200""#), "{page}");
    }

    #[test]
    fn test_svg_image_page_counts_as_image_only_despite_svg_title() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("svg.epub");
        svg_epub(&path, (300, 400), "표지", false);
        // Python은 `deletes=[{"idref": "c"}] and ["c"]`를 넘긴다. 이 식은 ["c"]다.
        let result = edit_images(&path, &revision(&path), &tmp.path().join("trash"), EditImages { deletes: vec![json!("c")], ..EditImages::default() }).unwrap();
        assert!(result["revision"].as_str().is_some_and(|r| !r.is_empty()), "{result}");
    }

    #[test]
    fn test_cover_target_crops_the_cover_image_and_fits_its_svg_page() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cover.epub");
        svg_epub(&path, (300, 400), "표지", true);
        edit_images(&path, &revision(&path), &tmp.path().join("trash"), EditImages { crops: vec![json!({"idref": "@cover", "rect": [0.0, 0.0, 0.5, 1.0, 0.0]})], ..EditImages::default() }).unwrap();
        let members = read(&path);
        assert_eq!(size(&members["OEBPS/c.jpg"]), (150, 400));
        assert!(text_of(&members, "OEBPS/c.xhtml").contains(r#"viewBox="0 0 150 400""#));
    }

    #[test]
    fn test_cover_target_without_a_cover_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nocover.epub");
        svg_epub(&path, (300, 400), "표지", false);
        let result = edit_images(&path, &revision(&path), &tmp.path().join("trash"), EditImages { crops: vec![json!({"idref": "@cover", "rect": [0.0, 0.0, 0.5, 1.0, 0.0]})], ..EditImages::default() });
        assert_refused(result, "표지 이미지를 찾을 수 없음", "@cover");
    }
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
