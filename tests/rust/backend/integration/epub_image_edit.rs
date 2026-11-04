#[path = "../common/mod.rs"]
mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use backend::book_metadata::BookLookupError;
use backend::book_manager::{BookLookup, BookManager, BookManagerDeps, BookRef};
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::epub_cover::{EditImages, OPF_NS, read_package};
use backend::main::{AppDeps, AuthMode, BookManagerApi, app_with};
use common::TestClient;
use common::es_fake::FakeEsManager;
use common::image::{b64};
use common::zipx::{Members as ZipMembers};
use serde_json::{Value, json};
use tempfile::TempDir;
use zip::ZipArchive;
use crate::common::shared_fixtures::{assert_refused, edit, full, jpeg, size, text_of};
use crate::common::shared_fixtures::{MULTI_PAGE};
use crate::common::epub_book::{book, make_book, make_multi, multi, page, write_zip};

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
    async fn get_book(&self, book_id: i64) -> Result<BookRef, BookLookupError> {
        Ok(BookRef { book_id, file_path: self.path.clone(), file_type: self.file_type.clone() })
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
fn test_crop_refuses_when_two_tags_in_the_page_use_the_same_image() {
    let tmp = tempfile::tempdir().unwrap();
    let page = MULTI_PAGE.replace("m2.jpg", "m1.jpg");
    let path = make_multi(&tmp.path().join("dup.epub"), page.as_bytes());
    let result = edit(&path, &tmp.path().join("0_trash"), EditImages { crops: vec![json!({"idref": "multi", "index": 0, "rect": full()})], ..EditImages::default() });
    assert_refused(result, "공유", "dup");
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
