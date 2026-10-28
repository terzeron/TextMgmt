//! 원본: tests/test_epub_image_edit.py
//!
//! EPUB 쪽 이미지의 크롭·분할·삭제·삽입: epub_cover.edit_images, BookManager.edit_epub_images, POST /epub-images/{id}.
//!
//! 가정한 API:
//! - `backend::epub_cover::{OPF_NS: &str, CoverError, revision(&Path) -> String}`; `CoverError`는 `Display`로 사유를 돌려준다
//!   (`pytest.raises(match=...)`는 부분 문자열로 확인한다)
//! - `backend::epub_cover::edit_images(path: &Path, revision: &str, trash_dir: &Path, args: EditImages) -> Result<Value, CoverError>`
//! - `backend::epub_cover::EditImages { crops: Vec<Value>, splits: Vec<Value>, deletes: Vec<Value>, inserts: Vec<(String, Vec<u8>)>, backup_name: Option<String> }` (`Default`)
//!   - crops 원소: `{"idref", "index"?, "rect": [x0, y0, x1, y1, angle]}`, splits 원소: `{"idref", "index"?, "first": rect, "second": rect}`
//!   - deletes 원소: `{"idref", "index"?}` 또는 idref 문자열, inserts 원소: `(before_idref, 이미지 바이트)`
//! - `backend::epub_cover::read_package(&mut zip::ZipArchive<std::fs::File>) -> Result<Package, CoverError>` (Python `_read_package`)
//! - `backend::epub_cover::edit_page_images(html: &[u8], image_count: usize, edits: &[(usize, &str, Option<Value>)]) -> Result<Vec<u8>, CoverError>` (Python `_edit_page_images`)
//!   (두 함수의 밑줄 접두사는 뗀다. 테스트에서 쓰려면 `pub`이어야 한다)
//! - `BookManager::edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> (Option<Value>, Option<String>)`
//! - `BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>)`: 인자 순서는 (path, es) (docs/rust-test-seams.md 3.12)
//!   - `.with_book_lookup(Arc<dyn BookLookup>)` (`get_book` 대역. 구 `.with_book(path, type)`), `.with_image_fetcher(Arc<dyn ImageFetcher>)` (`backend.cover_management.fetch_url_image` 대체. 구 `with_fetch_url_image`)
//!   - `backend::book_manager::{BookLookup, BookRef}`: `#[async_trait] trait BookLookup { async fn get_book(&self, book_id: i64) -> (Option<BookRef>, Option<String>) }`, `BookRef { book_id, file_path, file_type }`
//!   - `backend::direct_cover_url::{ImageFetcher, FetchError}`: `trait ImageFetcher: Send + Sync { fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> }`
//! - `backend::main::{app_with, AppDeps, AuthMode}`: `app_with(AppDeps::new(AuthMode::Fixed(json!({"role": "admin"}))).with_book_manager(Arc<dyn BookManagerApi>))`. 구 `app_with_overrides`. `app() -> Router`도 그대로 쓴다
//!
//! `lxml`은 `roxmltree`로, PIL은 `image`로, `zipfile`은 `zip`으로 옮겼다. PIL 색 이름은 `common::image::color`로 푼다.
//! ZIP 쓰기·읽기와 이미지 인코딩은 `common::{zipx, image}`를, ES 대역은 `common::es_fake::FakeEsManager`를 쓴다.
//! `book_fixtures`는 옛 API(`loader`, `AddBook` 등)에 묶여 컴파일되지 않아 `FakeManager`(라우트 테스트용 `BookManagerApi` 대역)를 이 파일에 둔다.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use backend::book_manager::{BookLookup, BookManager, BookManagerDeps, BookRef};
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::epub_cover::{CoverError, EditImages, EpubCoverHooks, Limits, OPF_NS, edit_images, edit_images_with, edit_page_images, read_package, revision};
use backend::main::{AppDeps, AuthMode, BookManagerApi, app_with};
use common::TestClient;
use common::es_fake::FakeEsManager;
use common::image::{b64, color, encode, image_size, jpeg_bytes};
use common::zipx::{Members as ZipMembers, ZipMode, write_zip_with};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, RgbaImage};
use serde_json::{Value, json};
use tempfile::TempDir;
use zip::ZipArchive;

fn full() -> Value {
    json!([0, 0, 1, 1, 0])
}

fn jpeg_sized(c: &str, size: (u32, u32)) -> Vec<u8> {
    jpeg_bytes(&RgbImage::from_pixel(size.0, size.1, Rgb(color(c))), 75)
}

fn jpeg(c: &str) -> Vec<u8> {
    jpeg_sized(c, (200, 100))
}

fn png(c: &str) -> Vec<u8> {
    encode(DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 100, Rgb(color(c)))), ImageFormat::Png)
}

fn gif() -> Vec<u8> {
    encode(DynamicImage::ImageRgba8(RgbaImage::new(20, 20)), ImageFormat::Gif)
}

fn page(images: &[&str], text: &str) -> String {
    let body: String = images.iter().map(|src| format!(r#"<div><img src="{src}"/></div>"#)).collect();
    format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body>{text}{body}</body></html>"#)
}

type Members = Vec<(String, Vec<u8>)>;

/// mimetype을 맨 앞에 두고 나머지 순서는 그대로 둔다. 멤버는 모두 무압축으로 쓴다.
fn write_zip(path: &Path, mut members: Members) {
    let pos = members.iter().position(|(n, _)| n == "mimetype").expect("mimetype");
    let mimetype = members.remove(pos);
    members.insert(0, mimetype);
    let entries: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    write_zip_with(path, &entries, ZipMode::Stored);
}

/// p1·p2·p3 는 이미지만 있는 쪽, text 는 글만 있는 쪽, two 는 이미지 둘, nav 는 p3 를 가리키는 목차.
fn make_book(path: &Path, extra: &[(&str, Vec<u8>)]) -> PathBuf {
    let opf = format!(
        r##"<package xmlns="{OPF_NS}" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>책</dc:title></metadata><manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="p1" href="p1.xhtml" media-type="application/xhtml+xml"/><item id="p2" href="p2.xhtml" media-type="application/xhtml+xml"/>
<item id="p3" href="p3.xhtml" media-type="application/xhtml+xml"/><item id="text" href="text.xhtml" media-type="application/xhtml+xml"/>
<item id="two" href="two.xhtml" media-type="application/xhtml+xml"/><item id="shared" href="shared.xhtml" media-type="application/xhtml+xml"/>
<item id="gif" href="gif.xhtml" media-type="application/xhtml+xml"/><item id="mixed" href="mixed.xhtml" media-type="application/xhtml+xml"/>
<item id="i1" href="img/1.jpg" media-type="image/jpeg"/><item id="i2" href="img/2.png" media-type="image/png"/>
<item id="i3" href="img/3.jpg" media-type="image/jpeg"/><item id="i4" href="img/4.jpg" media-type="image/jpeg"/>
<item id="ig" href="img/g.gif" media-type="image/gif"/></manifest>
<spine><itemref idref="p1"/><itemref idref="p2"/><itemref idref="p3"/><itemref idref="text"/><itemref idref="two"/><itemref idref="shared"/><itemref idref="gif"/><itemref idref="mixed"/></spine></package>"##
    );
    let mut members: Members = vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        ("META-INF/container.xml".into(), br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#.to_vec()),
        ("OPS/book.opf".into(), opf.into_bytes()),
        (
            "OPS/nav.xhtml".into(),
            br#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="p3.xhtml">3</a></li></ol></nav></body></html>"#
                .to_vec(),
        ),
        ("OPS/p1.xhtml".into(), page(&["img/1.jpg"], "").into_bytes()),
        ("OPS/p2.xhtml".into(), page(&["img/2.png"], "").into_bytes()),
        ("OPS/p3.xhtml".into(), page(&["img/3.jpg"], "").into_bytes()),
        ("OPS/text.xhtml".into(), page(&[], "본문 글").into_bytes()),
        ("OPS/two.xhtml".into(), page(&["img/4.jpg", "img/5.jpg"], "").into_bytes()),
        ("OPS/shared.xhtml".into(), page(&["img/4.jpg"], "").into_bytes()),
        ("OPS/gif.xhtml".into(), page(&["img/g.gif"], "").into_bytes()),
        ("OPS/mixed.xhtml".into(), page(&["img/5.jpg"], "설명 글").into_bytes()),
        ("OPS/img/1.jpg".into(), jpeg("red")),
        ("OPS/img/2.png".into(), png("green")),
        ("OPS/img/3.jpg".into(), jpeg("blue")),
        ("OPS/img/4.jpg".into(), jpeg("yellow")),
        ("OPS/img/5.jpg".into(), jpeg("pink")),
        ("OPS/img/g.gif".into(), gif()),
    ];
    for (name, data) in extra {
        match members.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = data.clone(),
            None => members.push((name.to_string(), data.clone())),
        }
    }
    write_zip(path, members);
    path.to_path_buf()
}

fn read(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let members = ZipMembers::read(path);
    members.names().into_iter().map(|name| (name.clone(), members.get(&name).to_vec())).collect()
}

fn opf_ids(path: &Path, opf: &str, tag: &str, attr: &str) -> Vec<String> {
    let text = String::from_utf8(read(path)[opf].clone()).unwrap();
    let doc = roxmltree::Document::parse(&text).unwrap();
    doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == tag && n.tag_name().namespace() == Some(OPF_NS)).filter_map(|n| n.attribute(attr).map(str::to_string)).collect()
}

fn spine(path: &Path) -> Vec<String> {
    opf_ids(path, "OPS/book.opf", "itemref", "idref")
}

fn manifest_ids(path: &Path) -> BTreeSet<String> {
    opf_ids(path, "OPS/book.opf", "item", "id").into_iter().collect()
}

fn manifest_ids_of(path: &Path, opf: &str) -> BTreeSet<String> {
    opf_ids(path, opf, "item", "id").into_iter().collect()
}

fn size(data: &[u8]) -> (u32, u32) {
    image_size(data)
}

fn text_of(members: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(members[name].clone()).unwrap()
}

/// `MagicMock` book_manager. 호출 인자를 기록하고 정해 둔 `(result, error)`를 돌려준다.
struct FakeManager {
    edit_epub_images: Mutex<(Option<Value>, Option<String>)>,
    /// (메서드 이름, 인자 목록): `await_args.args`에 대응한다.
    calls: Mutex<Vec<(String, Vec<Value>)>>,
}

impl FakeManager {
    fn new() -> Arc<Self> {
        Arc::new(Self { edit_epub_images: Mutex::new((Some(json!({"idrefs": {"inserted": [], "split": {}}})), None)), calls: Mutex::new(Vec::new()) })
    }

    fn args(&self, method: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().rev().find(|(m, _)| m == method).map(|(_, a)| a.clone()).unwrap_or_else(|| panic!("{method}가 불리지 않았다"))
    }
}

#[async_trait]
impl BookManagerApi for FakeManager {
    async fn edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> Result<Value, String> {
        self.calls.lock().unwrap().push(("edit_epub_images".into(), vec![json!(book_id), json!(crops), json!(splits), json!(deletes), json!(inserts)]));
        let (value, error) = self.edit_epub_images.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }
}

/// `book` fixture: (책 경로, 휴지통 경로)
struct Book {
    path: PathBuf,
    trash: PathBuf,
    _tmp: TempDir,
}

fn book() -> Book {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("책").join("book.epub"), &[]);
    let trash = tmp.path().join("0_trash");
    Book { path, trash, _tmp: tmp }
}

fn edit(path: &Path, trash: &Path, args: EditImages) -> Result<Value, CoverError> {
    edit_images(path, &revision(path), trash, EditImages { backup_name: Some("book.epub".to_string()), ..args })
}

impl Book {
    fn edit(&self, args: EditImages) -> Result<Value, CoverError> {
        edit(&self.path, &self.trash, args)
    }
}

fn err_of(result: Result<Value, CoverError>) -> String {
    result.expect_err("CoverError가 나야 한다").to_string()
}

fn assert_refused(result: Result<Value, CoverError>, reason: &str, label: &str) {
    let text = err_of(result);
    assert!(text.contains(reason), "{label}: {text:?} 에 {reason:?}가 없다");
}

#[test]
fn test_crop_replaces_the_image_in_place_keeping_the_format() {
    let b = book();
    let before = read(&b.path);
    let result = b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0.25, 0, 0.75, 1, 0]}), json!({"idref": "p2", "rect": [0, 0, 0.5, 0.5, 0]})], ..EditImages::default() }).unwrap();
    let after = read(&b.path);
    assert_eq!(size(&after["OPS/img/1.jpg"]), (100, 100));
    assert_eq!(size(&after["OPS/img/2.png"]), (100, 50));
    assert_eq!(&after["OPS/img/2.png"][..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(after["OPS/p1.xhtml"], before["OPS/p1.xhtml"]);
    assert!(b.trash.join("book.epub").exists());
    assert!(result["cover_image"].is_null() && result["idrefs"] == json!({"inserted": [], "split": {}}), "{result}");
}

#[test]
fn test_tilted_crop_straightens_the_image() {
    let b = book();
    b.edit(EditImages { crops: vec![json!({"idref": "p3", "rect": [0.3, 0.3, 0.7, 0.7, 3]})], ..EditImages::default() }).unwrap();
    assert_eq!(size(&read(&b.path)["OPS/img/3.jpg"]), (80, 40));
}

#[test]
fn test_crop_refusals_leave_the_book_untouched() {
    let cases = [("text", "이미지가 0개"), ("two", "공유"), ("p1:5", "이미지를 찾을 수 없음"), ("p1:-1", "위치"), ("gif", "지원하지 않는 이미지 형식"), ("nope", "찾을 수 없습니다")];
    for (spec, reason) in cases {
        let b = book();
        let original = fs::read(&b.path).unwrap();
        let (idref, index) = match spec.split_once(':') {
            Some((idref, index)) => (idref, index.parse::<i64>().unwrap()),
            None => (spec, 0),
        };
        let result = b.edit(EditImages { crops: vec![json!({"idref": idref, "index": index, "rect": full()})], ..EditImages::default() });
        assert_refused(result, reason, spec);
        assert_eq!(fs::read(&b.path).unwrap(), original, "{spec}");
    }
}

#[test]
fn test_crop_refuses_an_image_shared_with_another_page() {
    let b = book();
    let result = b.edit(EditImages { crops: vec![json!({"idref": "shared", "rect": full()})], ..EditImages::default() });
    assert_refused(result, "공유", "shared");
}

#[test]
fn test_crop_region_errors_are_reported_in_korean() {
    let b = book();
    let result = b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.01, 1, 0]})], ..EditImages::default() });
    assert_refused(result, "크롭할 수 없습니다 (크롭 영역이 너무 작음)", "p1");
}

#[test]
fn test_split_replaces_the_original_and_adds_a_page_right_after() {
    let b = book();
    let result = b.edit(EditImages { splits: vec![json!({"idref": "p1", "first": [0, 0, 0.5, 1, 0], "second": [0.5, 0, 1, 1, 0]})], ..EditImages::default() }).unwrap();
    let new_id = result["idrefs"]["split"]["p1"].as_str().unwrap().to_string();
    assert!(new_id.starts_with("tm-split-page-"), "{new_id}");
    let order = spine(&b.path);
    assert_eq!(order[..3], ["p1".to_string(), new_id.clone(), "p2".to_string()]);
    let after = read(&b.path);
    assert_eq!(size(&after["OPS/img/1.jpg"]), (100, 100));
    let page_member = format!("OPS/{new_id}.xhtml");
    let image_member = format!("OPS/{}.jpg", new_id.replace("-page", ""));
    assert!(after.contains_key(&page_member), "{page_member}");
    assert_eq!(size(&after[&image_member]), (100, 100));
    assert!(manifest_ids(&b.path).contains(&new_id));
}

#[test]
fn test_split_in_a_page_with_text_happens_inside_the_page() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("t").join("book.epub"), &[("OPS/p1.xhtml", page(&["img/1.jpg"], "<p>설명 글</p>").into_bytes())]);
    let result =
        edit(&path, &tmp.path().join("0_trash"), EditImages { splits: vec![json!({"idref": "p1", "first": [0, 0, 0.5, 1, 0], "second": [0.5, 0, 1, 1, 0]})], ..EditImages::default() }).unwrap();
    assert!(result["idrefs"]["split"] == json!({}) && spine(&path)[..2] == ["p1".to_string(), "p2".to_string()], "{result}");
    let text = text_of(&read(&path), "OPS/p1.xhtml");
    assert!(text.matches("<img").count() == 2 && text.contains("설명 글"), "{text}");
}

#[test]
fn test_split_refuses_an_image_shared_with_another_page() {
    let b = book();
    let result = b.edit(EditImages { splits: vec![json!({"idref": "mixed", "first": full(), "second": full()})], ..EditImages::default() });
    assert_refused(result, "공유", "mixed");
}

#[test]
fn test_split_refuses_a_manifest_page_outside_the_spine() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("b.epub"), &[]);
    let mut members = read(&path);
    let opf = members.get("OPS/book.opf").unwrap().clone();
    let opf = replace_bytes(&opf, b"</manifest>", b"<item id=\"orphan\" href=\"p1.xhtml\" media-type=\"application/xhtml+xml\"/></manifest>");
    members.insert("OPS/book.opf".into(), opf);
    write_zip(&path, members.into_iter().collect());

    let result = edit(&path, &tmp.path().join("0_trash"), EditImages { splits: vec![json!({"idref": "orphan", "first": full(), "second": full()})], ..EditImages::default() });
    assert_refused(result, "편집할 쪽", "다시 열어");
}

#[test]
fn test_delete_removes_the_page_and_its_unused_image() {
    let b = book();
    b.edit(EditImages { deletes: vec![json!({"idref": "p2"})], ..EditImages::default() }).unwrap();
    let after = read(&b.path);
    let ids = manifest_ids(&b.path);
    assert!(!spine(&b.path).contains(&"p2".to_string()) && !ids.contains("p2") && !ids.contains("i2"));
    assert!(!after.contains_key("OPS/p2.xhtml") && !after.contains_key("OPS/img/2.png"));
}

#[test]
fn test_delete_keeps_an_image_other_pages_still_use() {
    let b = book();
    b.edit(EditImages { deletes: vec![json!({"idref": "shared", "index": 0})], ..EditImages::default() }).unwrap();
    assert!(read(&b.path).contains_key("OPS/img/4.jpg"));
}

#[test]
fn test_delete_refuses_pages_the_toc_points_to_and_edits_text_pages_in_place() {
    let b = book();
    assert_refused(b.edit(EditImages { deletes: vec![json!("p3")], ..EditImages::default() }), "목차가 가리키는", "p3");
    // 글이 함께 있는 쪽은 쪽 안에서 이미지만 지운다(5.jpg 는 two 쪽도 쓰므로 파일은 남는다).
    b.edit(EditImages { deletes: vec![json!({"idref": "mixed"})], ..EditImages::default() }).unwrap();
    let after = read(&b.path);
    let mixed = text_of(&after, "OPS/mixed.xhtml");
    assert!(!mixed.contains("<img") && mixed.contains("설명 글"), "{mixed}");
    assert!(after.contains_key("OPS/img/5.jpg"));
}

#[test]
fn test_delete_refuses_the_tool_cover_page() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("b.epub"), &[]);
    let mut members = read(&path);
    let opf = members.get("OPS/book.opf").unwrap().clone();
    let opf = replace_bytes(&opf, b"<spine>", b"<spine><itemref idref=\"tm-cover-page-x\"/>");
    let opf = replace_bytes(&opf, b"</manifest>", b"<item id=\"tm-cover-page-x\" href=\"p1.xhtml\" media-type=\"application/xhtml+xml\"/></manifest>");
    members.insert("OPS/book.opf".into(), opf);
    write_zip(&path, members.into_iter().collect());
    let result = edit(&path, &tmp.path().join("0_trash"), EditImages { deletes: vec![json!("tm-cover-page-x")], ..EditImages::default() });
    assert_refused(result, "표지", "tm-cover-page-x");
}

#[test]
fn test_split_and_insert_refuse_tool_cover_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("b.epub"), &[]);
    let mut members = read(&path);
    let opf = members.get("OPS/book.opf").unwrap().clone();
    let opf = replace_bytes(&opf, b"<spine>", b"<spine><itemref idref=\"tm-cover-page-x\"/>");
    let opf = replace_bytes(&opf, b"</manifest>", b"<item id=\"tm-cover-page-x\" href=\"p1.xhtml\" media-type=\"application/xhtml+xml\"/></manifest>");
    members.insert("OPS/book.opf".into(), opf);
    write_zip(&path, members.into_iter().collect());

    let trash = tmp.path().join("0_trash");
    let split = edit(&path, &trash, EditImages { splits: vec![json!({"idref": "tm-cover-page-x", "first": full(), "second": full()})], ..EditImages::default() });
    assert_refused(split, "표지", "분할");

    let insert = edit(&path, &trash, EditImages { inserts: vec![("tm-cover-page-x".into(), jpeg("red"))], ..EditImages::default() });
    assert_refused(insert, "표지", "삽화");
}

fn replace_bytes(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

fn find_bytes(data: &[u8], needle: &[u8]) -> Option<usize> {
    data.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn test_delete_refuses_to_empty_the_book() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("b.epub"), &[]);
    let mut members = read(&path);
    // Python의 `.replace(b'<spine>', ..., 0)`은 아무것도 바꾸지 않는다.
    let only = b"<spine><itemref idref=\"p1\"/></spine>";
    let opf = members["OPS/book.opf"].clone();
    let start = find_bytes(&opf, b"<spine>").expect("<spine>");
    let mut rewritten = opf[..start].to_vec();
    rewritten.extend_from_slice(only);
    rewritten.extend_from_slice(b"</package>");
    members.insert("OPS/book.opf".into(), rewritten);
    write_zip(&path, members.into_iter().collect());
    let result = edit(&path, &tmp.path().join("0_trash"), EditImages { deletes: vec![json!("p1")], ..EditImages::default() });
    assert_refused(result, "모든 쪽", "p1");
}

#[test]
fn test_multiple_inserts_keep_their_order_before_the_same_page() {
    let b = book();
    let result = b.edit(EditImages { inserts: vec![("p2".into(), jpeg("red")), ("p2".into(), jpeg("blue")), ("p1".into(), jpeg("green"))], ..EditImages::default() }).unwrap();
    let ids: Vec<String> = result["idrefs"]["inserted"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let (first, second, third) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
    let order = spine(&b.path);
    assert_eq!(order[..5], [third, "p1".to_string(), first, second, "p2".to_string()]);
}

#[test]
fn test_insert_may_target_a_page_deleted_in_the_same_request() {
    let b = book();
    let result = b.edit(EditImages { inserts: vec![("p2".into(), jpeg("red"))], deletes: vec![json!("p2")], ..EditImages::default() }).unwrap();
    let inserted = result["idrefs"]["inserted"][0].as_str().unwrap().to_string();
    let order = spine(&b.path);
    assert!(order[..2] == ["p1".to_string(), inserted] && !order.contains(&"p2".to_string()), "{order:?}");
}

#[test]
fn test_insert_refuses_unknown_positions() {
    let b = book();
    assert_refused(b.edit(EditImages { inserts: vec![("nope".into(), jpeg("red"))], ..EditImages::default() }), "위치", "nope");
}

#[test]
fn test_combined_request_is_applied_atomically() {
    let b = book();
    let original = fs::read(&b.path).unwrap();
    let failing = b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], deletes: vec![json!("p3")], ..EditImages::default() });
    assert!(failing.is_err());
    assert_eq!(fs::read(&b.path).unwrap(), original);
    let result = b
        .edit(EditImages {
            crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})],
            splits: vec![json!({"idref": "p2", "first": [0, 0, 0.5, 1, 0], "second": [0.5, 0, 1, 1, 0]})],
            deletes: vec![json!("shared")],
            inserts: vec![("text".into(), jpeg("red"))],
            ..EditImages::default()
        })
        .unwrap();
    assert_eq!(size(&read(&b.path)["OPS/img/1.jpg"]), (100, 100));
    assert!(result["idrefs"]["inserted"].as_array().unwrap().len() == 1 && result["idrefs"]["split"].get("p2").is_some(), "{result}");
    // archive.testzip() is None: 모든 멤버를 끝까지 읽어 CRC를 확인한다.
    let mut archive = ZipArchive::new(File::open(&b.path).unwrap()).unwrap();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        std::io::copy(&mut f, &mut std::io::sink()).unwrap_or_else(|e| panic!("{:?}: {e}", f.name()));
    }
}

#[test]
fn test_empty_and_duplicate_requests_are_rejected() {
    let b = book();
    assert_refused(b.edit(EditImages::default()), "편집이 없습니다", "빈 요청");
    assert_refused(b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], deletes: vec![json!("p1")], ..EditImages::default() }), "두 번", "중복 요청");
}

#[test]
fn test_stale_revision_is_rejected_and_backup_is_kept_once() {
    let b = book();
    let stale = edit_images(&b.path, "stale", &b.trash, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() });
    assert_refused(stale, "변경", "stale");
    let original = fs::read(&b.path).unwrap();
    b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
    b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
    assert_eq!(fs::read(b.trash.join("book.epub")).unwrap(), original);
}

/// `env` fixture: BookManager.edit_epub_images를 부를 수 있는 최소 환경.
struct ManagerEnv {
    bm: BookManager,
    es: Arc<FakeEsManager>,
    path: PathBuf,
    tmp: TempDir,
}

/// 어떤 URL이든 같은 이미지를 돌려주는 `ImageFetcher`. 구 `FakeFetch { behavior: Ok(..) }`.
// TODO(common): 문서 3.7의 `common::fx::fetcher(|url| ...)`가 아직 없다. 생기면 `fetcher(|_| Ok(jpeg("red")))`로 바꾼다.
struct FixedFetcher(Vec<u8>);

impl ImageFetcher for FixedFetcher {
    fn fetch_url_image(&self, _url: &str) -> Result<Vec<u8>, FetchError> {
        Ok(self.0.clone())
    }
}

/// 어떤 책 ID든 같은 파일을 돌려주는 `BookLookup`. 구 `BookManagerDeps::with_book(path, file_type)`.
// TODO(common): 문서 3.12의 `common`의 `StaticBookLookup`이 아직 없다.
struct StaticBookLookup {
    path: PathBuf,
    file_type: String,
}

#[async_trait]
impl BookLookup for StaticBookLookup {
    async fn get_book(&self, book_id: i64) -> (Option<BookRef>, Option<String>) {
        (Some(BookRef { book_id, file_path: self.path.clone(), file_type: self.file_type.clone() }), None)
    }
}

fn manager_env() -> ManagerEnv {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("분류").join("book.epub"), &[]);
    let es = FakeEsManager::new();
    // monkeypatch.setattr("backend.cover_management.fetch_url_image", lambda url: jpeg("red"))
    let fetch = Arc::new(FixedFetcher(jpeg("red")));
    let deps = BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_image_fetcher(fetch).with_book_lookup(Arc::new(StaticBookLookup { path: path.clone(), file_type: "epub".to_string() }));
    ManagerEnv { bm: BookManager::with_deps(deps), es, path, tmp }
}

fn rect(x0: f64, x1: f64) -> Value {
    json!({"x0": x0, "y0": 0, "x1": x1, "y1": 1, "angle": 0})
}

/// `{"idref": idref, **rect}`
fn with_idref(idref: &str, r: Value) -> Value {
    let mut m = r.as_object().unwrap().clone();
    m.insert("idref".into(), json!(idref));
    Value::Object(m)
}

#[tokio::test]
async fn test_manager_applies_a_combined_edit_and_keeps_the_original_in_trash() {
    let m = manager_env();
    let original = fs::read(&m.path).unwrap();
    let data = b64(&jpeg("blue"));
    let result =
        m.bm.edit_epub_images(
            1,
            &[with_idref("p1", rect(0.0, 0.5))],
            &[json!({"idref": "p2", "first": rect(0.0, 0.5), "second": rect(0.5, 1.0)})],
            &[json!({"idref": "shared"})],
            &[json!({"before_idref": "text", "url": "https://img.example.com/a.jpg"}), json!({"before_idref": "text", "image_data": data})],
        )
        .await
        .unwrap();
    assert!(result["idrefs"]["inserted"].as_array().unwrap().len() == 2 && result["idrefs"]["split"].get("p2").is_some(), "{result}");
    assert_eq!(fs::read(m.tmp.path().join("0_trash").join("book.epub")).unwrap(), original);
    assert_eq!(m.es.updates().len(), 1);
}

#[tokio::test]
async fn test_manager_reports_errors_as_messages() {
    let m = manager_env();
    let original = fs::read(&m.path).unwrap();
    let error = m.bm.edit_epub_images(1, &[with_idref("text", rect(0.0, 1.0))], &[], &[], &[]).await.unwrap_err();
    assert!(error.contains("크롭할 수 없습니다"), "{error}");
    let error = m.bm.edit_epub_images(1, &[], &[], &[], &[json!({"before_idref": "p1", "image_data": "!!!"})]).await.unwrap_err();
    assert!(error.contains("읽을 수 없습니다"), "{error}");
    assert_eq!(fs::read(&m.path).unwrap(), original);
}

mod test_route {
    use super::*;

    fn client() -> (TestClient, Arc<FakeManager>) {
        let fake = FakeManager::new();
        let app = app_with(AppDeps::new(AuthMode::Fixed(json!({"role": "admin"}))).with_book_manager(fake.clone()));
        (TestClient::with_app(app), fake)
    }

    #[tokio::test]
    async fn test_success_passes_the_agreed_body_to_the_manager() {
        let (http, fake) = client();
        let body = json!({
            "crops": [{"idref": "p1", "index": 1, "x0": 0.1, "y0": 0.1, "x1": 0.9, "y1": 0.9, "angle": 2}],
            "splits": [{"idref": "p2", "first": rect(0.0, 0.5), "second": rect(0.5, 1.0)}],
            "deletes": [{"idref": "p3"}],
            "inserts": [{"before_idref": "p1", "url": "https://img.example.com/a.jpg"}],
        });
        let response = http.post_json("/epub-images/1", &body).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status": "success", "result": {"idrefs": {"inserted": [], "split": {}}}}));
        let args = fake.args("edit_epub_images");
        assert!(args[0] == json!(1) && args[1][0]["idref"] == "p1" && args[1][0]["index"] == json!(1) && args[1][0]["angle"].as_f64() == Some(2.0), "{args:?}");
        assert!(args[2][0]["first"]["x1"].as_f64() == Some(0.5) && args[3] == json!([{"idref": "p3", "index": 0}]) && args[4][0]["before_idref"] == "p1", "{args:?}");
    }

    #[tokio::test]
    async fn test_failure_is_reported_in_the_body() {
        let (http, fake) = client();
        *fake.edit_epub_images.lock().unwrap() = (None, Some("크롭할 수 없습니다 (이미지가 0개인 쪽)".to_string()));
        let response = http.post_json("/epub-images/1", &json!({"deletes": [{"idref": "x"}]})).await;
        assert_eq!(response.json(), json!({"status": "failure", "error": "크롭할 수 없습니다 (이미지가 0개인 쪽)"}));
    }

    #[tokio::test]
    async fn test_invalid_bodies_are_validation_errors() {
        let (http, _) = client();
        assert_eq!(http.post_json("/epub-images/1", &json!({"deletes": [{"idref": ""}]})).await.status, 422);
        assert_eq!(http.post_json("/epub-images/1", &json!({"deletes": [{"idref": "p1", "index": -1}]})).await.status, 422);
        assert_eq!(http.post_json("/epub-images/1", &json!({"inserts": [{"before_idref": "p1"}]})).await.status, 422);
    }

    #[tokio::test]
    async fn test_requires_admin() {
        let status = TestClient::new().post_json("/epub-images/1", &json!({})).await.status;
        assert!(status == 401 || status == 403, "{status}");
    }
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

// ── 한 쪽에 이미지가 여러 장인 책(실제 책처럼 xmlns 없는 비정형 HTML) ─────────────────────

const MULTI_PAGE: &str = concat!(
    "<html>\n  <head>\n    <title>엄마의 말뚝</title>\n  </head>\n    <body><li class=\"chapter\">\n",
    "<p><img alt=\"image\" height=\"597\" src=\"../Images/m1.jpg\" width=\"412\"/></p>\n",
    "<p><img alt=\"image\" height=\"597\" src=\"../Images/m2.jpg\" width=\"412\"/></p>\n",
    "</li></body></html>"
);

fn make_multi(path: &Path, page: &[u8]) -> PathBuf {
    let opf = format!(
        concat!(
            r#"<package xmlns="{ns}" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">x</dc:identifier><dc:title>t</dc:title></metadata>"#,
            r#"<manifest><item id="multi" href="Text/multi.xhtml" media-type="application/xhtml+xml"/><item id="t" href="Text/t.xhtml" media-type="application/xhtml+xml"/>"#,
            r#"<item id="m1" href="Images/m1.jpg" media-type="image/jpeg"/><item id="m2" href="Images/m2.jpg" media-type="image/jpeg"/></manifest>"#,
            r#"<spine><itemref idref="multi"/><itemref idref="t"/></spine></package>"#,
        ),
        ns = OPF_NS
    );
    let members: Members = vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        (
            "META-INF/container.xml".into(),
            br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#.to_vec(),
        ),
        ("OEBPS/content.opf".into(), opf.into_bytes()),
        ("OEBPS/Text/multi.xhtml".into(), page.to_vec()),
        ("OEBPS/Text/t.xhtml".into(), "<html><body><p>다른 쪽</p></body></html>".as_bytes().to_vec()),
        ("OEBPS/Images/m1.jpg".into(), jpeg("red")),
        ("OEBPS/Images/m2.jpg".into(), jpeg("green")),
    ];
    write_zip(path, members);
    path.to_path_buf()
}

/// `multi` fixture
fn multi() -> Book {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_multi(&tmp.path().join("multi.epub"), MULTI_PAGE.as_bytes());
    let trash = tmp.path().join("0_trash");
    Book { path, trash, _tmp: tmp }
}

fn verify(path: &Path) {
    let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        std::io::copy(&mut f, &mut std::io::sink()).unwrap_or_else(|e| panic!("{:?}: {e}", f.name()));
    }
    read_package(&mut archive).unwrap();
}

#[test]
fn test_multi_image_page_crops_each_image_separately() {
    for (index, member) in [(0, "OEBPS/Images/m1.jpg"), (1, "OEBPS/Images/m2.jpg")] {
        let b = multi();
        let before = read(&b.path);
        b.edit(EditImages { crops: vec![json!({"idref": "multi", "index": index, "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
        let after = read(&b.path);
        assert_eq!(size(&after[member]), (100, 100), "index={index}");
        let other = if index == 0 { "OEBPS/Images/m2.jpg" } else { "OEBPS/Images/m1.jpg" };
        assert!(after[other] == before[other] && after["OEBPS/Text/multi.xhtml"] == before["OEBPS/Text/multi.xhtml"], "index={index}");
        verify(&b.path);
    }
}

#[test]
fn test_multi_image_page_split_adds_an_image_inside_the_page() {
    let b = multi();
    let before = read(&b.path);
    let result = b.edit(EditImages { splits: vec![json!({"idref": "multi", "index": 1, "first": [0, 0, 0.5, 1, 0], "second": [0.5, 0, 1, 1, 0]})], ..EditImages::default() }).unwrap();
    assert_eq!(result["idrefs"]["split"], json!({}));
    let after = read(&b.path);
    let page_text = text_of(&after, "OEBPS/Text/multi.xhtml");
    assert_eq!(page_text.matches("<img").count(), 3);
    let new_names: Vec<&String> = after.keys().filter(|n| n.contains("tm-split-")).collect();
    assert!(new_names.len() == 1 && size(&after[new_names[0]]) == (100, 100) && size(&after["OEBPS/Images/m2.jpg"]) == (100, 100), "{new_names:?}");
    // 새 이미지는 opf 와 같은 디렉터리(OEBPS)에 두고, 쪽(OEBPS/Text)에서는 상대경로로 가리킨다.
    let base = new_names[0].rsplit('/').next().unwrap();
    assert!(new_names[0].starts_with("OEBPS/tm-split-") && page_text.contains(&format!(r#"src="../{base}""#)), "{page_text}");
    assert!(manifest_ids_of(&b.path, "OEBPS/content.opf").iter().any(|item| item.starts_with("tm-split-")));
    // 이미지 태그 하나가 늘었을 뿐 나머지 문서·파일은 그대로다.
    assert!(page_text.contains("<br/>"), "{page_text}");
    assert_eq!(page_text.replace("<br/>", "").matches("<p>").count(), 2);
    assert!(after["OEBPS/Images/m1.jpg"] == before["OEBPS/Images/m1.jpg"] && after["OEBPS/Text/t.xhtml"] == before["OEBPS/Text/t.xhtml"]);
    assert!(page_text.starts_with("<html>\n  <head>") && page_text.contains("<li class=\"chapter\">"), "{page_text}");
    verify(&b.path);
}

#[test]
fn test_multi_image_page_delete_removes_the_wrapper_and_the_unused_image() {
    let b = multi();
    b.edit(EditImages { deletes: vec![json!({"idref": "multi", "index": 0})], ..EditImages::default() }).unwrap();
    let after = read(&b.path);
    let page_text = text_of(&after, "OEBPS/Text/multi.xhtml");
    assert!(page_text.matches("<img").count() == 1 && !page_text.contains("m1.jpg") && page_text.matches("<p>").count() == 1, "{page_text}");
    assert!(!after.contains_key("OEBPS/Images/m1.jpg") && !manifest_ids_of(&b.path, "OEBPS/content.opf").contains("m1"));
    assert!(after.contains_key("OEBPS/Images/m2.jpg"));
    verify(&b.path);
}

#[test]
fn test_multi_image_page_mixed_edits_apply_without_shifting_indexes() {
    let b = multi();
    b.edit(EditImages {
        crops: vec![json!({"idref": "multi", "index": 0, "rect": [0, 0, 0.5, 1, 0]})],
        splits: vec![json!({"idref": "multi", "index": 1, "first": [0, 0, 0.5, 1, 0], "second": [0.5, 0, 1, 1, 0]})],
        ..EditImages::default()
    })
    .unwrap();
    let after = read(&b.path);
    assert!(size(&after["OEBPS/Images/m1.jpg"]) == (100, 100) && size(&after["OEBPS/Images/m2.jpg"]) == (100, 100));
    assert_eq!(text_of(&after, "OEBPS/Text/multi.xhtml").matches("<img").count(), 3);
    verify(&b.path);
}

#[test]
fn test_split_then_delete_on_the_same_page_use_original_indexes() {
    let b = multi();
    b.edit(EditImages {
        splits: vec![json!({"idref": "multi", "index": 0, "first": [0, 0, 0.5, 1, 0], "second": [0.5, 0, 1, 1, 0]})],
        deletes: vec![json!({"idref": "multi", "index": 1})],
        ..EditImages::default()
    })
    .unwrap();
    let page_text = text_of(&read(&b.path), "OEBPS/Text/multi.xhtml");
    assert!(page_text.matches("<img").count() == 2 && !page_text.contains("m2.jpg") && page_text.contains("m1.jpg"), "{page_text}");
    verify(&b.path);
}

#[test]
fn test_same_image_twice_is_refused() {
    let b = multi();
    let original = fs::read(&b.path).unwrap();
    let first = b.edit(EditImages { crops: vec![json!({"idref": "multi", "index": 1, "rect": full()})], deletes: vec![json!({"idref": "multi", "index": 1})], ..EditImages::default() });
    assert_refused(first, "두 번", "crop+delete");
    let second = b.edit(EditImages { crops: vec![json!({"idref": "multi", "rect": full()}), json!({"idref": "multi", "index": 0, "rect": full()})], ..EditImages::default() });
    assert_refused(second, "두 번", "crop+crop");
    assert_eq!(fs::read(&b.path).unwrap(), original);
}

#[test]
fn test_index_out_of_range_is_refused() {
    let b = multi();
    let original = fs::read(&b.path).unwrap();
    for kind in ["crops", "splits", "deletes"] {
        let mut entry = json!({"idref": "multi", "index": 2});
        match kind {
            "crops" => entry["rect"] = full(),
            "splits" => {
                entry["first"] = full();
                entry["second"] = full();
            }
            _ => {}
        }
        let mut args = EditImages::default();
        match kind {
            "crops" => args.crops = vec![entry],
            "splits" => args.splits = vec![entry],
            _ => args.deletes = vec![entry],
        }
        assert_refused(b.edit(args), "이미지를 찾을 수 없음", kind);
    }
    assert_eq!(fs::read(&b.path).unwrap(), original);
}

#[test]
fn test_crop_refuses_when_two_tags_in_the_page_use_the_same_image() {
    let tmp = tempfile::tempdir().unwrap();
    let page = MULTI_PAGE.replace("m2.jpg", "m1.jpg");
    let path = make_multi(&tmp.path().join("dup.epub"), page.as_bytes());
    let result = edit(&path, &tmp.path().join("0_trash"), EditImages { crops: vec![json!({"idref": "multi", "index": 0, "rect": full()})], ..EditImages::default() });
    assert_refused(result, "공유", "dup");
}

#[test]
fn test_mismatched_tag_count_is_refused() {
    let err = edit_page_images(b"<html><body><img src='a.jpg'/></body></html>", 2, &[(0, "delete", None)]).expect_err("CoverError가 나야 한다");
    assert!(err.to_string().contains("쪽 구조를 해석할 수 없습니다"), "{err}");
}

#[test]
fn test_page_image_edit_preserves_utf8_signature() {
    let raw = b"\xEF\xBB\xBF<html><body><img src='old.jpg'/></body></html>";

    let edited = edit_page_images(raw, 1, &[(0, "split", Some(json!("new image.jpg")))]).unwrap();

    assert!(edited.starts_with(b"\xEF\xBB\xBF"));
    assert!(String::from_utf8_lossy(&edited).contains("src='new%20image.jpg'"));
}

#[test]
fn test_page_image_edit_reads_and_writes_utf16_bom() {
    let text = "<html><body><img src='old.jpg'/></body></html>";
    let raw: Vec<u8> = [vec![0xFF, 0xFE], text.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat();

    let edited = edit_page_images(&raw, 1, &[(0, "delete", None)]).unwrap();

    assert!(edited.starts_with(&[0xFF, 0xFE]));
    let words: Vec<u16> = edited[2..].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    assert!(!String::from_utf16(&words).unwrap().contains("<img"));
}

#[test]
fn test_page_image_edit_round_trips_latin1_meta_charset() {
    let raw = [b"<html><head><meta charset='ISO-8859-1'></head><body>caf".as_slice(), &[0xE9], b"<img src='old.jpg'/></body></html>"].concat();

    let edited = edit_page_images(&raw, 1, &[(0, "delete", None)]).unwrap();

    assert!(edited.windows(4).any(|window| window == b"caf\xE9"));
    assert!(!edited.windows(4).any(|window| window == b"<img"));
}

#[test]
fn test_page_image_edit_rejects_invalid_encoding_declarations() {
    let raw = b"<?xml version='1.0' encoding='windows-1252'?><html><body><img src='a'/></body></html>";

    let error = edit_page_images(raw, 1, &[(0, "delete", None)]).unwrap_err();

    assert!(error.to_string().contains("문자 인코딩을 읽을 수 없음"));
}

#[test]
fn test_page_image_split_does_not_rewrite_data_src_attribute() {
    let raw = b"<html><body><img data-src='keep.jpg' src='old.jpg'/></body></html>";

    let edited = edit_page_images(raw, 1, &[(0, "split", Some(json!("new.jpg")))]).unwrap();
    let edited = String::from_utf8(edited).unwrap();

    assert!(edited.contains("data-src='keep.jpg'"));
    assert!(edited.contains("src='new.jpg'"));
}

#[test]
fn test_comment_images_are_not_counted() {
    let tmp = tempfile::tempdir().unwrap();
    let page = MULTI_PAGE.replace("<li", "<!-- <img src=\"../Images/m1.jpg\"/> --><li");
    let path = make_multi(&tmp.path().join("comment.epub"), page.as_bytes());
    edit(&path, &tmp.path().join("0_trash"), EditImages { deletes: vec![json!({"idref": "multi", "index": 1})], ..EditImages::default() }).unwrap();
    let text = text_of(&read(&path), "OEBPS/Text/multi.xhtml");
    assert!(text.matches("<img").count() == 2 && !text.replace("<!-- <img", "").contains("m2.jpg"), "{text}");
}

#[test]
fn test_delete_of_repeated_image_reference_keeps_the_shared_member() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("b.epub"), &[("OPS/two.xhtml", page(&["img/4.jpg", "img/4.jpg"], "").into_bytes())]);
    let result = edit(&path, &tmp.path().join("0_trash"), EditImages { deletes: vec![json!({"idref": "two", "index": 0})], ..EditImages::default() });
    assert!(result.is_ok(), "{result:?}");
    let members = read(&path);
    assert!(members.contains_key("OPS/img/4.jpg"));
    assert!(text_of(&members, "OPS/two.xhtml").contains("img/4.jpg"));
}

#[test]
fn test_undecodable_page_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut page = MULTI_PAGE.as_bytes().to_vec();
    page.extend_from_slice(b"\xff\xfe");
    let path = make_multi(&tmp.path().join("bad.epub"), &page);
    let result = edit(&path, &tmp.path().join("0_trash"), EditImages { deletes: vec![json!({"idref": "multi", "index": 0})], ..EditImages::default() });
    assert!(result.is_err());
}

// ── 입력 검증과 상한(Python보다 엄격한 부분) ───────────────────────────────────

fn edit_with(b: &Book, hooks: &EpubCoverHooks, args: EditImages) -> Result<Value, CoverError> {
    edit_images_with(&b.path, &revision(&b.path), &b.trash, EditImages { backup_name: Some("book.epub".to_string()), ..args }, hooks)
}

#[test]
fn test_edit_counts_and_indexes_are_capped() {
    let b = book();
    let original = fs::read(&b.path).unwrap();
    let deletes: Vec<Value> = (0..501).map(|i| json!({"idref": "multi", "index": i})).collect();
    assert_refused(b.edit(EditImages { deletes, ..EditImages::default() }), "항목 수", "501 deletes");
    let inserts: Vec<(String, Vec<u8>)> = (0..101).map(|_| ("p1".to_string(), jpeg("red"))).collect();
    assert_refused(b.edit(EditImages { inserts, ..EditImages::default() }), "항목 수", "101 inserts");
    assert_refused(b.edit(EditImages { deletes: vec![json!({"idref": "p1", "index": 10001})], ..EditImages::default() }), "위치", "index 10001");
    assert_refused(b.edit(EditImages { deletes: vec![json!({"idref": "p1", "index": 1.5})], ..EditImages::default() }), "위치", "index 1.5");
    assert_eq!(fs::read(&b.path).unwrap(), original);
}

#[test]
fn test_malformed_crop_rectangles_are_refused() {
    let b = book();
    let original = fs::read(&b.path).unwrap();
    let cases = [
        json!([0, 0, "a", 1, 0]),
        json!([0, 0, 1]),
        json!([0, 0, 1, 1, 0, 0]),
        json!(null),
        json!("0,0,1,1"),
        json!([0, 0, f64::NAN, 1, 0]),
        json!([0, 0, 1, 1, 400]),
        json!([0, 0, 1, 1e308, 0]),
        json!([-0.5, 0, 1, 1, 0]),
    ];
    for rect in cases {
        let label = rect.to_string();
        assert!(b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": rect})], ..EditImages::default() }).is_err(), "{label}");
    }
    assert_refused(b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 1, 1, 90]})], ..EditImages::default() }), "기울어진", "90도로 이미지 밖");
    assert_eq!(fs::read(&b.path).unwrap(), original);
}

#[test]
fn test_oversized_image_is_refused_before_decoding() {
    let b = book();
    let original = fs::read(&b.path).unwrap();
    let observer = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = observer.clone();
    let observe = move || {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    };
    // 바이트 크기: 읽기 전에 거절한다.
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_image_bytes: 10, ..Limits::default() }).with_pixel_decode_observer(Arc::new(observe.clone()));
    assert_refused(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }), "너무 큼", "bytes");
    // 화소 수: 디코딩 전에 거절한다.
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() }).with_pixel_decode_observer(Arc::new(observe));
    assert_refused(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }), "해상도가 너무 큼", "pixels");
    assert_eq!(observer.load(std::sync::atomic::Ordering::SeqCst), 0, "상한을 넘는 이미지를 디코딩했다");
    assert_eq!(fs::read(&b.path).unwrap(), original);
}

#[test]
fn test_oversized_page_document_is_refused() {
    let b = book();
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_xml_bytes: 50, ..Limits::default() });
    assert!(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }).is_err());
}

#[test]
fn test_edit_result_never_exceeds_the_source_image() {
    // 크롭은 원본 안쪽만 쓰므로 어떤 영역·각도로도 결과가 원본보다 커지지 않는다.
    let b = book();
    b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 1, 1, 0]})], ..EditImages::default() }).unwrap();
    assert_eq!(size(&read(&b.path)["OPS/img/1.jpg"]), (200, 100));
}

mod test_route_limits {
    use super::*;

    #[tokio::test]
    async fn test_too_many_items_and_bad_numbers_are_validation_errors() {
        let (http, fake) = test_route_client();
        let many: Vec<Value> = (0..501).map(|_| json!({"idref": "p1"})).collect();
        assert_eq!(http.post_json("/epub-images/1", &json!({"deletes": many})).await.status, 422);
        assert_eq!(http.post_json("/epub-images/1", &json!({"deletes": [{"idref": "p1", "index": 10001}]})).await.status, 422);
        assert_eq!(http.post_json("/epub-images/1", &json!({"crops": [{"idref": "p1", "x0": 0, "y0": 0, "x1": "a", "y1": 1}]})).await.status, 422);
        assert_eq!(http.post_json("/epub-images/1", &json!({"crops": [{"idref": "p1", "x0": 0, "y0": 0, "y1": 1}]})).await.status, 422);
        assert_eq!(http.post_json("/epub-images/x", &json!({})).await.status, 422);
        assert!(fake.calls.lock().unwrap().is_empty());
    }
}

fn test_route_client() -> (TestClient, Arc<FakeManager>) {
    let fake = FakeManager::new();
    let app = app_with(AppDeps::new(AuthMode::Fixed(json!({"role": "admin"}))).with_book_manager(fake.clone()));
    (TestClient::with_app(app), fake)
}
