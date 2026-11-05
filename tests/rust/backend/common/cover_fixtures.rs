// TODO(seams): common::{image,fsx,epub,zipx,pdf,clock}로 이전 중. 이전 후 삭제
//! 표지(cover) 테스트 공용 fixture.
//! `cached_candidates`, `cover_api`와 표지 판정 테스트의 PDF 생성 도구.
//!
//! 이 파일은 `common/mod.rs`를 건드리지 않으려고 따로 둔다. 쓰는 쪽은 `mod common;` 아래에서
//! `#[path = "common/cover_fixtures.rs"] mod cover_fixtures;`로 가져온다.
//!
//! 가정한 API (아직 구현 전이다):
//! - `backend::book_metadata::Book::override_path_prefix(path: &Path) -> PathPrefixGuard` (drop 시 원래 값 복원.
//! - `cover_manager::cover_management::CoverManagement::{new(root: &Path), with_hooks(root: &Path, hooks: CoverHooks), hooks(&self) -> &CoverHooks, set_hooks(&mut self, hooks: CoverHooks)}`
//! - `cover_manager::cover_management::CoverHooks` (Clone + Default): `with_search(Fn(&str, &EpubInspection) -> Result<StoreSearch, SearchError>)`, `with_fetch_bytes(Fn(&str, &str) -> Result<Vec<u8>, String>)`,
//!   `with_fetch_url_image(Fn(&str) -> Result<Vec<u8>, CoverError>)`, `with_clock(Arc<dyn Clock>)`. 테스트가 대역을 꽂는 주입 지점이다.
//! - `cover_manager::cover_management::{AdminGate, BookIndex, create_cover_router}`
//!   - `AdminGate::default()` 실제 인증, `.allow(Value)`, `.deny(status: u16, detail: &str)`, `.clear()`
//!   - `trait BookIndex: Send + Sync { fn search_by_id(&self, book_id: i64) -> Result<Option<Value>, String>; fn update(&self, book_id: i64, fields: &Value) -> Result<bool, String>; }`
//!   - `create_cover_router(get_manager: Arc<dyn Fn() -> Arc<dyn BookIndex> + Send + Sync>, gate: AdminGate, hooks: CoverHooks) -> axum::Router`
//! - `cover_manager::cover_sources::Clock` (`fn now(&self) -> f64`, `fn sleep(&self, seconds: f64)`)

#![allow(dead_code)]

#[allow(unused_imports)]
pub use crate::common::cover_candidates::{Cached, cached_candidates, encode_image, image_bytes};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use backend::api_router::{AppDeps, AuthMode, BookManagerApi, app_with};
use cover_manager::cover_management::CoverHooks;
use image::RgbImage;
use lopdf::{Document, Object, Stream, dictionary};
use search_manager::{EsError, EsManagerApi};
use serde_json::{Value, json};
use zip::CompressionMethod;

use crate::common::TestClient;
// 사본을 지우고 common 원본을 다시 내보낸다. 이 fixture를 쓰는 테스트 crate마다 일부만 쓴다.
#[allow(unused_imports)]
pub use crate::common::clock::FakeClock;
#[allow(unused_imports)]
pub use crate::common::epub::{BODY, CONTAINER_XML, append_child, attr_of_element_with_id, count_elements};
#[allow(unused_imports)]
pub use crate::common::fsx::{files_under, sha256_hex};
#[allow(unused_imports)]
pub use crate::common::image::{BLUE, RED, paste, solid};
use crate::common::zipx::options;
#[allow(unused_imports)]
pub use crate::common::zipx::{Members, bytes_replace};

/// 인코딩된 이미지의 (너비, 높이).
pub fn image_size(data: &[u8]) -> (u32, u32) {
    let image = image::load_from_memory(data).unwrap();
    (image.width(), image.height())
}

/// 임시 EPUB을 만든다. 표지가 있으면 `OPS/cover.jpg`를 `cover-image`로 선언한다.
pub fn make_epub(path: &Path, version: &str, cover: bool) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut archive = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let stored = options(CompressionMethod::Stored);
    archive.start_file("mimetype", stored).unwrap();
    archive.write_all(b"application/epub+zip").unwrap();
    archive.start_file("META-INF/container.xml", stored).unwrap();
    archive.write_all(CONTAINER_XML.as_bytes()).unwrap();
    let cover_item = if cover { r#"<item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>"# } else { "" };
    let opf = format!(
        r#"<package xmlns="http://www.idpf.org/2007/opf" version="{version}"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>테스트 책</dc:title><dc:creator>저자</dc:creator></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>{cover_item}</manifest><spine><itemref idref="chapter"/></spine></package>"#
    );
    archive.start_file("OPS/book.opf", stored).unwrap();
    archive.write_all(opf.as_bytes()).unwrap();
    archive.start_file("OPS/chapter.xhtml", stored).unwrap();
    archive.write_all(BODY.as_bytes()).unwrap();
    archive.start_file("OPS/style.css", stored).unwrap();
    archive.write_all(b"body { color: red; }").unwrap();
    if cover {
        archive.start_file("OPS/cover.jpg", stored).unwrap();
        archive.write_all(&image_bytes()).unwrap();
    }
    archive.finish().unwrap();
    path.to_path_buf()
}

/// 작은 fixture의 멤버를 중복 entry 없이 바꾼다. `mimetype`만 STORED, 나머지는 DEFLATED.
pub fn rewrite_epub_members(path: &Path, transform: impl FnOnce(&mut Members)) {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut members = Members { items: Vec::new() };
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        if file.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        members.items.push((file.name().unwrap().to_string(), data));
    }
    drop(archive);
    transform(&mut members);
    let mut out = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in &members.items {
        out.start_file(name, options(if name == "mimetype" { CompressionMethod::Stored } else { CompressionMethod::Deflated })).unwrap();
        out.write_all(data).unwrap();
    }
    out.finish().unwrap();
}

/// ZIP 멤버 하나를 읽는다.
pub fn read_member(path: &Path, name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut file = archive.by_name(name).unwrap_or_else(|_| panic!("멤버 없음: {name}"));
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    data
}

/// ZIP 멤버 이름을 저장 순서대로 돌려준다.
pub fn member_names(path: &Path) -> Vec<String> {
    let archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    archive.file_names().map(|n| n.unwrap().into_owned()).collect()
}

/// `<opf:manifest>` 같은 자식 요소 하나를 통째로 지운다.
pub fn remove_element(xml: &[u8], local: &str) -> Vec<u8> {
    let text = std::str::from_utf8(xml).unwrap();
    let open = format!("<{local}");
    let close = format!("</{local}>");
    let start = text.find(&open).unwrap_or_else(|| panic!("요소 없음: {local}"));
    let end = text[start..].find(&close).map(|e| start + e + close.len()).unwrap_or_else(|| {
        // 자기 닫힘 요소
        start + text[start..].find("/>").unwrap() + 2
    });
    format!("{}{}", &text[..start], &text[end..]).into_bytes()
}

// ---- cached_candidates -----------------------------------------------------------------------

// ---- cover_api -------------------------------------------------------------------------------

/// `BookManagerApi`와 `EsManagerApi` 대역. 표지 라우트가 쓰는 `path_prefix`, `search_by_id`, `update`만 구현한다.
pub struct MockManager {
    root: PathBuf,
    search: Mutex<Result<Option<Value>, String>>,
    update_result: Mutex<Result<bool, String>>,
    update_calls: Mutex<Vec<(i64, Value)>>,
}

struct MockEs(Arc<MockManager>);

impl EsManagerApi for MockEs {
    fn search_by_id(&self, _doc_id: i64) -> Result<Option<Value>, EsError> {
        self.0.search.lock().unwrap().clone().map_err(EsError::Other)
    }

    fn update(&self, doc_id: i64, fields: &Value) -> Result<bool, EsError> {
        self.0.update_calls.lock().unwrap().push((doc_id, fields.clone()));
        self.0.update_result.lock().unwrap().clone().map_err(EsError::Other)
    }
}

#[async_trait::async_trait]
impl BookManagerApi for MockManagerHandle {
    fn path_prefix(&self) -> PathBuf {
        self.0.root.clone()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        Arc::new(MockEs(self.0.clone()))
    }
}

/// `BookManagerApi`로 앱에 꽂는 손잡이. 호출 기록은 `MockManager`가 갖는다.
pub struct MockManagerHandle(pub Arc<MockManager>);

/// 표지 관리 라우트의 인증 상태. 구 `AdminGate`: `allow`는 관리자, `deny(403)`은 viewer, `clear`와 `deny(401)`은 로그인 없음이다.
/// 실제 쿠키 인증(`AuthMode::Cookies`)을 쓰므로 요청마다 상태에 맞는 쿠키를 붙인다.
#[derive(Clone)]
pub struct AdminGate {
    state: Arc<Mutex<Option<&'static str>>>,
}

impl AdminGate {
    fn new() -> Self {
        Self { state: Arc::new(Mutex::new(Some("admin"))) }
    }

    pub fn allow(&self, _payload: Value) {
        *self.state.lock().unwrap() = Some("admin");
    }

    pub fn deny(&self, status: u16, _detail: &str) {
        *self.state.lock().unwrap() = if status == 401 { None } else { Some("viewer") };
    }

    pub fn clear(&self) {
        *self.state.lock().unwrap() = None;
    }

    fn token(&self) -> Option<String> {
        match *self.state.lock().unwrap() {
            Some("admin") => Some(crate::common::session::token("admin@test.com", "admin", Some("Test Admin"), None)),
            Some(_) => Some(crate::common::session::token("viewer@test.com", "viewer", None, None)),
            None => None,
        }
    }
}

/// 게이트 상태를 요청마다 반영하는 클라이언트.
pub struct GateClient {
    app: axum::Router,
    gate: AdminGate,
}

impl GateClient {
    fn client(&self) -> TestClient {
        let client = TestClient::with_app(self.app.clone());
        match self.gate.token() {
            Some(token) => client.cookie("tm_access_token", &token),
            None => client,
        }
    }

    pub async fn get(&self, path: &str) -> crate::common::Response {
        self.client().get(path).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> crate::common::Response {
        self.client().post_json(path, body).await
    }
}

impl MockManager {
    pub fn new(root: &Path) -> Arc<Self> {
        Arc::new(Self { root: root.to_path_buf(), search: Mutex::new(Ok(Some(json!({"title": "책"})))), update_result: Mutex::new(Ok(true)), update_calls: Mutex::new(Vec::new()) })
    }

    /// `search_by_id.return_value = value`
    pub fn set_search(&self, value: Option<Value>) {
        *self.search.lock().unwrap() = Ok(value);
    }

    /// `search_by_id.side_effect = BookManagerError(message)`
    pub fn set_search_error(&self, message: &str) {
        *self.search.lock().unwrap() = Err(message.to_string());
    }

    /// `update.return_value = value`
    pub fn set_update(&self, value: bool) {
        *self.update_result.lock().unwrap() = Ok(value);
    }

    pub fn update_called(&self) -> bool {
        !self.update_calls.lock().unwrap().is_empty()
    }

    /// 마지막 `update(book_id, **fields)` 호출.
    pub fn last_update(&self) -> Option<(i64, Value)> {
        self.update_calls.lock().unwrap().last().cloned()
    }
}

pub struct CoverApi {
    pub client: GateClient,
    pub gate: AdminGate,
    pub manager: Arc<MockManager>,
}

/// 표지 라우터가 올라간 앱. 책 경로를 `root`로 두고 관리자 쿠키를 붙인다.
pub fn cover_api(root: &Path, hooks: CoverHooks) -> CoverApi {
    let manager = MockManager::new(root);
    let gate = AdminGate::new();
    let app =
        app_with(AppDeps::new(AuthMode::Cookies).with_refresh_token_store(crate::common::session::store()).with_book_manager(Arc::new(MockManagerHandle(manager.clone()))).with_cover_hooks(hooks));
    CoverApi { client: GateClient { app, gate: gate.clone() }, gate, manager }
}

// ---- PDF 생성 --------------------------------------------------------------------------------

/// 이미지 한 장당 한 쪽인 PDF. 쪽 크기는 픽셀 크기와 같다.
pub fn write_image_pdf(path: &Path, pages: &[RgbImage]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for image in pages {
        let (width, height) = image.dimensions();
        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => Object::Integer(width as i64),
                "Height" => Object::Integer(height as i64),
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => Object::Integer(8),
            },
            image.as_raw().clone(),
        ));
        let content_id = doc.add_object(Stream::new(dictionary! {}, format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q").into_bytes()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Integer(width as i64), Object::Integer(height as i64)]),
            "Contents" => Object::Reference(content_id),
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => Object::Reference(image_id) } },
        });
        kids.push(Object::Reference(page_id));
    }
    let count = kids.len() as i64;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => Object::Array(kids), "Count" => Object::Integer(count) }));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => Object::Reference(pages_id) });
    doc.trailer.set("Root", Object::Reference(catalog_id));
    doc.compress();
    doc.save(path).unwrap();
}

/// 이미지 없는 빈 한 쪽짜리 PDF.
pub fn write_blank_pdf(path: &Path, width: i64, height: i64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(pages_id),
        "MediaBox" => Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Integer(width), Object::Integer(height)]),
    });
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => Object::Array(vec![Object::Reference(page_id)]), "Count" => Object::Integer(1) }));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => Object::Reference(pages_id) });
    doc.trailer.set("Root", Object::Reference(catalog_id));
    doc.save(path).unwrap();
}

// ---- 작은 도우미 -----------------------------------------------------------------------------

pub fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn ino_mtime(path: &Path) -> (u64, i128) {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).unwrap();
    (meta.ino(), meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128)
}

pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub fn testzip_ok(path: &Path) -> bool {
    let mut archive = match zip::ZipArchive::new(std::fs::File::open(path).unwrap()) {
        Ok(a) => a,
        Err(_) => return false,
    };
    for i in 0..archive.len() {
        let mut file = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut sink = Vec::new();
        if file.read_to_end(&mut sink).is_err() {
            return false;
        }
    }
    true
}

/// 이름이 `prefix`로 시작하고 `suffix`로 끝나는 직속 항목 수.
pub fn count_matching(dir: &Path, prefix: &str, suffix: &str) -> usize {
    std::fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with(prefix) && e.file_name().to_string_lossy().ends_with(suffix)).count()).unwrap_or(0)
}

/// 직속 항목 이름을 정렬해서 돌려준다.
pub fn dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// 접근·수정 시각을 `seconds_since_epoch`로 맞춘다.
pub fn touch(path: &Path, seconds_since_epoch: u64) {
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds_since_epoch);
    let times = std::fs::FileTimes::new().set_accessed(at).set_modified(at);
    std::fs::OpenOptions::new().write(true).open(path).unwrap().set_times(times).unwrap();
}

pub struct Event {
    flag: Mutex<bool>,
    condvar: std::sync::Condvar,
}

impl Event {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { flag: Mutex::new(false), condvar: std::sync::Condvar::new() })
    }

    pub fn set(&self) {
        *self.flag.lock().unwrap() = true;
        self.condvar.notify_all();
    }

    /// 켜졌으면 true, 제한 시간 안에 안 켜졌으면 false.
    pub fn wait(&self, seconds: u64) -> bool {
        let guard = self.flag.lock().unwrap();
        let (guard, _) = self.condvar.wait_timeout_while(guard, std::time::Duration::from_secs(seconds), |set| !*set).unwrap();
        *guard
    }
}
