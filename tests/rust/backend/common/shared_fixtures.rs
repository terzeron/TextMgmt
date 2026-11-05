//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]
use super::env::{EnvGuard, EnvReadGuard};
use super::epub::{EpubSpec, make_epub};
use super::es_fake::{Docs, FakeEsManager};
use super::fsx::dir_names;
use super::image::{BLUE, color, encode, encode_image, image_size, jpeg_bytes};
use super::pdf::make_blank_pdf;
use super::zipx::{Members, ZipMode, read_zip_entry, rewrite_epub_members, write_zip, write_zip_with};
use super::{Response, TestClient, es_dummy};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use backend::auth_service::Claims;
use backend::auth_service::RequestParts;
use backend::book_manager::{AddBookError, BookAdder, BookContent, BookLookup, BookManager, BookManagerDeps, BookRef, BookResponse, CoverPage, FileReader, ProposeOptions, ReadFileError, Thumbnailer};
use backend::book_manager::{EpubValidation, PdfValidation};
use backend::book_manager::{FsOnlyFile, MismatchDetails, MismatchReload, ReloadCounts, ReloadedFiles};
use backend::book_metadata::{Book, BookLookupError, BookPage};
use backend::cover_management::CoverManagement;
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::epub_cover;
use backend::epub_cover::{CoverError, EditImages, EpubCoverHooks, EpubRewrite, Limits, edit_images};
use backend::es_manager::{EsError, EsManagerApi, HttpTransport};
use backend::main::{AppDeps, ApplyCategoryChanges, AuthMode, AuthSlots, BookManagerApi, CategoryMappingApi, MappingError, ProposeCategoryChanges, ReloadProgress, TaskHandle, TaskRunner};
use backend::refresh_token_store::{RefreshTokenStoreApi, Rotation, SessionQuery, StoreError};
use db::category_mapping::CategoryMapping;
use db::{DbValue, Row};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, RgbaImage};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::io::Read;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, atomic::AtomicUsize, atomic::Ordering};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use zip::ZipArchive;

/// `CallArgs`를 위치 인자 배열과 키워드 인자 객체로 만든다.
macro_rules! call_args {
    ([$($a:expr),* $(,)?], {$($k:literal : $v:expr),* $(,)?}) => {
        CallArgs::new(vec![$(json!($a)),*], json!({$($k: $v),*}))
    };
}

pub fn admin_payload() -> Claims {
    Claims { email: "admin@test.com".into(), role: "admin".into(), name: "Admin".into(), exp: now_plus_3600() as i64, ..Claims::default() }
}

pub fn allowed_set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

// JSON -> trait 반환형 변환.

pub fn arr(v: &Value, i: usize) -> Value {
    v.get(i).cloned().unwrap_or(Value::Null)
}

/// `Result<(), _>`를 `("Ok" | "Error", 메시지)`로 펼친다.
pub fn as_status(result: Result<(), String>) -> (String, Option<String>) {
    match result {
        Ok(()) => ("Ok".to_string(), None),
        Err(error) => ("Error".to_string(), Some(error)),
    }
}

/// `CoverError`이고 메시지에 `needle`이 들었는지 확인한다.
pub fn assert_cover_error<T: std::fmt::Debug>(result: Result<T, CoverError>, needle: &str) {
    match result {
        Err(e) if !e.is_io() => assert!(e.to_string().contains(needle), "CoverError 메시지에 {needle:?}가 없다: {e}"),
        other => panic!("CoverError({needle:?})를 기대했지만 {other:?}"),
    }
}

pub fn assert_refused<T: std::fmt::Debug>(result: Result<T, CoverError>, reason: &str, label: &str) {
    let text = err_of(result);
    assert!(text.contains(reason), "{label}: {text:?} 에 {reason:?}가 없다");
}

/// 동기 테스트 안에서 future를 끝까지 돌린다.
pub fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut)
}

/// 형님 EPUB과 같은 SVG 표지, guide, NCX 구조를 만든다.
pub fn attach_publisher_cover_page(path: &Path, shared: bool) {
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

pub fn backups(root: &Path) -> PathBuf {
    root.join(".cover_backups")
}

/// 612x792 빈 페이지 `pages`장짜리 PDF.
pub fn blank_pdf_bytes(pages: usize, info: &[(&str, &str)]) -> Vec<u8> {
    pdf_bytes(pages, 612, 792, info)
}

pub fn blank_pdf_bytes_sized(pages: usize, width: u32, height: u32) -> Vec<u8> {
    pdf_bytes(pages, width, height, &[])
}

pub fn books(v: Value) -> Vec<Book> {
    v.as_array().map(|a| a.iter().cloned().map(|b| opt_book(b).expect("Book JSON")).collect()).unwrap_or_default()
}

pub fn broken_cover(tmp: &Path) -> PathBuf {
    let path = make_epub(&tmp.join("book.epub"), &EpubSpec::cover("3.0", true));
    rewrite_epub_members(&path, |members| members.set("OPS/cover.jpg", &b"not a JPEG"[..]));
    path
}

pub fn category_with_files(root: &Path, category: &str, names: &[&str]) {
    fs::create_dir_all(root.join(category)).unwrap();
    for name in names {
        fs::write(root.join(category).join(name), "본문").unwrap();
    }
}

pub fn chapter(index: usize) -> String {
    make_chapter_xhtml(index, false)
}

pub fn count(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

/// `[항목, 전체 수, 다음 커서, 오류]`를 `Result<CoverPage, _>`로 바꾼다. 전체 수가 null이면 모르는 것이다.
pub fn cover_page(v: &Value) -> Result<CoverPage, String> {
    let total = arr(v, 1).as_u64().map(|n| n as usize);
    opt_string(arr(v, 3)).map_or_else(|| Ok(CoverPage { items: titled_books(arr(v, 0)), total, next_cursor: opt_string(arr(v, 2)) }), Err)
}

/// `path.parent.glob(".cover-*.tmp")`
pub fn cover_tmp_files(dir: &Path) -> Vec<String> {
    (if dir.exists() { dir_names(dir) } else { Vec::new() }).into_iter().filter(|n| n.starts_with(".cover-") && n.ends_with(".tmp")).collect()
}

/// `create_test_pdf`
pub fn create_test_pdf(num_pages: usize, output_path: &Path) {
    make_blank_pdf(output_path, num_pages);
}

/// 기본 구성의 `BookManagerDeps`. `with_*`를 더 붙인 뒤 `BookManager::with_deps`로 만든다.
pub fn deps(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManagerDeps {
    super::setup_env();
    BookManagerDeps::new(tmp_path.to_path_buf(), es.clone())
}

pub fn edit(path: &Path, trash: &Path, args: EditImages) -> Result<EpubRewrite, CoverError> {
    edit_images(path, &revision(path), trash, EditImages { backup_name: Some("book.epub".to_string()), ..args })
}

/// local-name이 `name`인 모든 요소를 문서 순서로 돌려준다.
pub fn elements<'a, 'i>(doc: &'a roxmltree::Document<'i>, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == name).collect()
}

/// URL 경로와 쿼리에 들어가는 한글을 퍼센트 인코딩한다. `requests`가 하는 일과 같다.
pub fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn entry(name: &str, data: impl Into<Vec<u8>>) -> (String, Vec<u8>) {
    (name.to_string(), data.into())
}

pub fn epub_with_cover(root: &Path, width: u32, height: u32) -> PathBuf {
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
    let data = encode_image(width, height, BLUE, ImageFormat::Jpeg);
    rewrite_epub_members(&path, |members| members.set("OPS/cover.jpg", data.clone()));
    path
}

pub fn err_has(error: &Option<String>, needle: &str) -> bool {
    error.as_deref().unwrap_or_else(|| panic!("error가 None이다 (기대: {needle:?} 포함)")).contains(needle)
}

pub fn err_of<T: std::fmt::Debug>(result: Result<T, CoverError>) -> String {
    result.expect_err("CoverError가 나야 한다").to_string()
}

pub fn error_text(error: &Option<String>) -> &str {
    error.as_deref().unwrap_or("")
}

/// 표지 목록 첫 항목의 JSON 모양.
pub fn first_item(service: &CoverManagement, covered: bool) -> Value {
    serde_json::to_value(service.missing("", "", 60, covered).unwrap()).unwrap()["items"][0].clone()
}

pub fn flag(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}

pub fn full() -> Value {
    json!([0, 0, 1, 1, 0])
}

pub fn gif() -> Vec<u8> {
    encode(DynamicImage::ImageRgba8(RgbaImage::new(20, 20)), ImageFormat::Gif)
}

pub fn has_cover(path: &Path) -> bool {
    epub_cover::inspect_epub(path).unwrap().has_cover
}

pub fn hooks_with(limits: Limits) -> EpubCoverHooks {
    EpubCoverHooks::default().with_limits(limits)
}

/// 이미지 한 장의 바이트. 색은 고정이다.
pub fn image_bytes(fmt: image::ImageFormat, size: (u32, u32)) -> Vec<u8> {
    encode_image(size.0, size.1, [10, 120, 200], fmt)
}

pub fn ip_headers(headers: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in headers {
        map.insert(HeaderName::from_bytes(k.to_lowercase().as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
    }
    map
}

pub fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
    }
}

pub fn jpeg(c: &str) -> Vec<u8> {
    jpeg_sized(c, (200, 100))
}

pub fn jpeg_sized(c: &str, size: (u32, u32)) -> Vec<u8> {
    jpeg_bytes(&RgbImage::from_pixel(size.0, size.1, Rgb(color(c))), 75)
}

pub fn make_chapter_xhtml(index: usize, link_css: bool) -> String {
    let css_link = if link_css { r#"<link rel="stylesheet" href="style.css"/>"# } else { "" };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter {index}</title>{css_link}</head>
<body><h1>Chapter {index}</h1><p>Content of chapter {index}.</p></body>
</html>"#
    )
}

/// SVG `<image>` 태그로 커버 이미지를 참조하는 커버 페이지.
pub fn make_cover_xhtml_with_svg() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Cover</title></head>
<body>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
 width="100%" height="100%" viewBox="0 0 600 800">
<image xlink:href="../media/cover.jpg" width="600" height="800"/>
</svg>
</body>
</html>"#
        .to_string()
}

/// ES 대역을 쓰는 `BookManagerDeps`. 대역(`with_*`)을 꽂을 테스트는 이걸로 시작한다.
pub fn make_deps(tmp_path: &Path, es: Arc<dyn EsManagerApi>) -> BookManagerDeps {
    BookManagerDeps::new(tmp_path.to_path_buf(), es)
}

pub fn make_doc(rel_path: &str, file_type: &str) -> Value {
    json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
}

pub fn make_manager(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManager {
    BookManager::with_deps(deps(tmp_path, es))
}

/// 챕터 N개를 포함한 OPF XML을 생성.
pub fn make_opf(chapter_count: usize, include_css: bool, include_svg_cover: bool) -> String {
    let mut items = Vec::new();
    let mut spine_refs = Vec::new();
    for i in 1..=chapter_count {
        items.push(format!(r#"    <item id="ch{i}" href="ch{i}.xhtml" media-type="application/xhtml+xml"/>"#));
        spine_refs.push(format!(r#"    <itemref idref="ch{i}"/>"#));
    }
    if include_css {
        items.push(r#"    <item id="style" href="style.css" media-type="text/css"/>"#.to_string());
    }
    if include_svg_cover {
        items.push(r#"    <item id="cover-image" href="../media/cover.jpg" media-type="image/jpeg"/>"#.to_string());
    }
    let items = items.join("\n");
    let spine_refs = spine_refs.join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:title>Test Book</dc:title>
<dc:creator>Test Author</dc:creator>
</metadata>
<manifest>
<item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
{items}
</manifest>
<spine toc="toc">
{spine_refs}
</spine>
</package>"#
    )
}

/// 임시 디렉토리 `A/<name>`을 만들어 내용 "x"인 파일 경로를 돌려준다.
pub fn make_source_file(tmp_path: &Path, category: &str, name: &str) -> PathBuf {
    let source = tmp_path.join(category);
    fs::create_dir_all(&source).unwrap();
    let target = source.join(name);
    fs::write(&target, "x").unwrap();
    target
}

pub fn namelist<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>) -> Vec<String> {
    (0..zf.len()).map(|i| zf.by_index(i).unwrap().name().unwrap().into_owned()).collect()
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

// ── 공용 상수와 헬퍼 ─────────────────────────────────────────────────────────

pub fn now_plus_3600() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

pub fn ns_children<'a, 'i>(node: roxmltree::Node<'a, 'i>, ns: &str, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    node.children().filter(|n| n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)).collect()
}

/// ElementTree의 `.//{ns}name`처럼 네임스페이스까지 맞는 요소를 문서 순서로 돌려준다.
pub fn ns_nodes<'a, 'i>(doc: &'a roxmltree::Document<'i>, ns: &str, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)).collect()
}

pub fn opf_of(path: &Path) -> Vec<u8> {
    read_zip_entry(path, "OPS/book.opf")
}

pub fn opt_book(v: Value) -> Option<Book> {
    if v.is_null() { None } else { Some(Book::new(v["book_id"].as_i64().unwrap_or_default(), &v, v["score"].as_f64().unwrap_or_default())) }
}

pub fn opt_json(v: Value) -> Option<Value> {
    if v.is_null() { None } else { Some(v) }
}

pub fn opt_string(v: Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// `Result<Value, _>`를 `(결과, 오류)`로 펼친다. 실패하면 빈 객체다.
pub fn outcome(result: Result<serde_json::Value, String>) -> (serde_json::Value, Option<String>) {
    match result {
        Ok(value) => (value, None),
        Err(error) => (serde_json::json!({}), Some(error)),
    }
}

/// struct 결과를 `(결과, 오류)`로 펼친다. 실패하면 결과는 기본값이다.
pub fn outcome_or_default<T: Default>(result: Result<T, String>) -> (T, Option<String>) {
    match result {
        Ok(value) => (value, None),
        Err(error) => (T::default(), Some(error)),
    }
}

/// `[책 목록, 전체 수, 오류]`를 `Result<BookPage, _>`로 바꾼다.
pub fn page(v: &Value) -> Result<BookPage, String> {
    opt_string(arr(v, 2)).map_or_else(|| Ok(BookPage { books: books(arr(v, 0)), total: count(&arr(v, 1)), next_cursor: None }), Err)
}

// TODO(common): lopdf가 아니라 /Info 사전을 직접 쓰는 최소 PDF 생성기. super::pdf::PdfBuilder에 `info` 지원이 생기면 지운다.
/// 빈 쪽 PDF. `info`는 /Info 사전의 (키, 값)이다.
pub fn pdf_bytes(pages: usize, width: u32, height: u32, info: &[(&str, &str)]) -> Vec<u8> {
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
    for _ in 0..pages {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] >>"));
    }
    if !info.is_empty() {
        let body: String = info.iter().map(|(k, v)| format!("/{k} ({v}) ")).collect();
        objs.push(format!("<< {body}>>"));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
    }
    let xref_pos = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    let info_ref = if info.is_empty() { String::new() } else { format!(" /Info {} 0 R", 3 + pages) };
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R{info_ref} >>\nstartxref\n{xref_pos}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

pub fn png(c: &str) -> Vec<u8> {
    encode(DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 100, Rgb(color(c)))), ImageFormat::Png)
}

/// `load_jwt_secret(env: EnvFn)`에 넘기는 실제 프로세스 환경.
pub fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

pub fn proposal_result(total: usize, items: &[Value]) -> Value {
    json!({"content_type": "book", "source_category": "0_inbox", "total_count": total, "processed_count": total, "items": items, "failures": []})
}

pub fn propose(m: &BookManager, opts: ProposeOptions<'_>) -> Value {
    let (result, err) = outcome(run(m.propose_category_changes("0_inbox", Some(&json!({})), opts)));
    assert!(err.is_none(), "{err:?}");
    result
}

// ── epub_cover 방어 경로: 표지 분리, 교체 전 표지 보존, 검증 실패 ──

/// SVG 표지 페이지(Text/front.xhtml)를 가진 EPUB 3. `files`로 멤버를 덮어쓴다.
pub fn publisher_cover_epub(tmp_path: &Path, files: &[(&str, Vec<u8>)]) -> PathBuf {
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

pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// 현재 `revision`.
pub fn rev(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap().revision
}

pub fn revision(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap().revision
}

pub fn run<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut)
}

pub fn set_cookies(r: &Response) -> Vec<String> {
    r.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.clone()).collect()
}

pub fn size(data: &[u8]) -> (u32, u32) {
    image_size(data)
}

pub fn spine_idrefs(path: &Path) -> Vec<String> {
    let text = xml_text(Members::read(path).get("OPS/book.opf"));
    let doc = roxmltree::Document::parse(&text).unwrap();
    elements(&doc, "spine").iter().flat_map(|s| s.children().filter(|c| c.is_element() && c.tag_name().name() == "itemref")).filter_map(|c| c.attribute("idref").map(String::from)).collect()
}

/// `Result`를 `(결과, 오류)` 모양으로 펼쳐 단언을 짧게 쓴다.
pub fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

pub fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap()
}

pub fn strings(v: Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

pub fn text_of(members: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(members[name].clone()).unwrap()
}

/// `[[book, display_title], ...]`
pub fn titled_books(v: Value) -> Vec<(Book, String)> {
    v.as_array().map(|a| a.iter().map(|row| (opt_book(arr(row, 0)).expect("Book JSON"), arr(row, 1).as_str().unwrap_or("").to_string())).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 공용 헬퍼 (전반부 정의를 복사)
// ---------------------------------------------------------------------------

/// 임시 디렉터리. 반환값이 drop되면 디렉터리가 지워진다.
pub fn tmp_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// `{"__response__": {"content": .., "media_type": ..}}` -> 200 응답.
pub fn to_response(v: Value) -> BookResponse {
    let r = v.get("__response__").cloned().unwrap_or(Value::Null);
    BookResponse { status_code: 200, media_type: r["media_type"].as_str().map(str::to_string), headers: Vec::new(), body: r["content"].as_str().unwrap_or("").as_bytes().to_vec() }
}

pub fn transport(url: &str) -> HttpTransport {
    HttpTransport::new(url, "elastic", "pw", "books", Duration::from_secs(5))
}

// TODO(common): `es_dummy::truthy(Option<&Value>)`를 `&Value`로 받는 얇은 래퍼. cover_fixtures::truthy를 대신한다.
pub fn truthy(value: &Value) -> bool {
    es_dummy::truthy(Some(value))
}

pub fn unlink(path: &Path) {
    let _ = fs::remove_file(path);
}

/// `(Option<Value>, Option<String>)` 쌍. `[결과, 오류]`
/// `[값, 오류]`를 `Result`로 바꾼다. 오류가 있으면 값은 보지 않는다.
pub fn value_result(v: &Value) -> Result<Value, String> {
    opt_string(arr(v, 1)).map_or_else(|| Ok(arr(v, 0)), Err)
}

/// `value_result`를 struct로 읽는다. 빠진 필드는 기본값이다.
pub fn typed_result<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, String> {
    value_result(v).and_then(|value| serde_json::from_value(value).map_err(|e| e.to_string()))
}

/// 검증 결과를 `(유효 여부, 사유)`로 펼친다.
pub fn verdict(result: Result<(), String>) -> (bool, Option<String>) {
    match result {
        Ok(()) => (true, None),
        Err(reason) => (false, Some(reason)),
    }
}

/// mimetype(무압축)을 맨 앞에 두고 나머지를 압축해서 쓴다.
pub fn write_epub(path: &Path, entries: Vec<(String, Vec<u8>)>) {
    let mimetype: &[u8] = b"application/epub+zip";
    let mut all: Vec<(&str, &[u8])> = vec![("mimetype", mimetype)];
    all.extend(entries.iter().map(|(n, d)| (n.as_str(), d.as_slice())));
    write_zip_with(path, &all, ZipMode::EpubStyle);
}

/// `zipfile.ZipFile(path, "w")`: 압축 인자가 없으면 ZIP_STORED다.
pub fn write_zip_stored(path: &Path, entries: &[(&str, &[u8])]) {
    write_zip_with(path, entries, ZipMode::Stored);
}

// ── XML 확인 도구 ────────────────────────

pub fn xml_text(data: &[u8]) -> String {
    String::from_utf8(data.to_vec()).unwrap()
}

pub fn zip_read<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>, name: &str) -> Vec<u8> {
    let mut data = Vec::new();
    zf.by_name(name).unwrap_or_else(|_| panic!("ZIP 항목이 없다: {name}")).read_to_end(&mut data).unwrap();
    data
}

pub fn zip_read_string<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>, name: &str) -> String {
    String::from_utf8(zip_read(zf, name)).unwrap()
}

pub type AddBookFn = dyn Fn() -> Result<i64, AddBookError> + Send + Sync;

pub const CATEGORY: &str = "_epub";

pub const CONTAINER: &str = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;

// ── EPUB 생성 헬퍼 ────────────────────────────────────────

pub const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

/// 호출 하나의 기록. 위치 인자, 이름 붙은 인자(JSON 객체), 콜백.
#[derive(Clone)]
pub struct CallArgs<'a> {
    pub args: Vec<Value>,
    pub kwargs: Value,
    pub callbacks: HashMap<String, Callback<'a>>,
}

impl<'a> CallArgs<'a> {
    pub fn new(args: Vec<Value>, kwargs: Value) -> Self {
        Self { args, kwargs, callbacks: HashMap::new() }
    }

    pub fn with_callback(mut self, name: &str, callback: Option<Callback<'a>>) -> Self {
        if let Some(callback) = callback {
            self.callbacks.insert(name.to_string(), callback);
        }
        self
    }
}

/// `on_progress`/`on_item_done`/`should_continue`를 "JSON 하나를 받아 JSON 하나를 돌려주는 awaitable"로 통일한 것.
pub type Callback<'a> = Arc<dyn Fn(Value) -> Fut<'a, Value> + Send + Sync + 'a>;

// ========== 접속 IP ==========

pub const DEFAULT_CLIENT_HOST: &str = "192.0.2.9";

/// `cm()`이 돌려주는 값. 테이블을 공유하는 테스트끼리 병렬로 돌지 않게 잠금을 쥐고 있다.
pub struct Db {
    pub _lock: MutexGuard<'static, ()>,
    pub mapping: CategoryMapping,
}

impl Deref for Db {
    type Target = CategoryMapping;
    fn deref(&self) -> &CategoryMapping {
        &self.mapping
    }
}

/// 작업을 큐에 보관해 HTTP 응답 뒤까지 실행을 미룬다.
pub type DeferredJobs = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

pub struct DeferredRunner(pub DeferredJobs);

impl TaskRunner for DeferredRunner {
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        self.0.lock().unwrap().push(job);
        Box::new(JoinedHandle)
    }
}

/// `book_manager.es_manager.<메서드>` 호출은 점 표기 이름(`es_manager.delete`)으로 같은 `Mock`에 기록한다.
pub struct EsProxy(pub Mock);

impl EsManagerApi for EsProxy {
    fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        self.0.invoke_sync_try("es_manager.delete", call_args!([doc_id], {})).map(|v| flag(&v)).map_err(EsError::Other)
    }
}

/// `AsyncMock(return_value=(book, None))` get_book.
pub struct FakeBooks(pub Mutex<(Option<BookRef>, Option<String>)>);

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
impl FakeBooks {
    pub fn set_file_type(&self, file_type: &str) {
        self.0.lock().unwrap().0.as_mut().unwrap().file_type = file_type.to_string();
    }

    pub fn set_missing(&self, message: &str) {
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

pub struct FakeRequest {
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
}

impl RequestParts for FakeRequest {
    fn header(&self, name: &str) -> Option<String> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }

    fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.get(name).cloned()
    }
}

/// `RefreshTokenStoreApi` fake. 호출 인자를 기록하고 정해 둔 결과를 돌려준다.
/// 안 정한 메서드는 trait의 기본 구현(Unimplemented 오류)을 쓴다.
pub struct FakeStore {
    pub rotate_status: Mutex<Option<String>>,
    pub observation: Mutex<Option<Result<Option<Value>, String>>>,
    pub sessions: Mutex<Option<Result<Value, String>>>,
    pub revoke_result: Mutex<Option<Result<Value, String>>>,
    pub issued: Mutex<Vec<Value>>,
    pub rotated: Mutex<Vec<Value>>,
    pub revoked_families: Mutex<Vec<(String, String)>>,
    pub list_calls: Mutex<Vec<Value>>,
    pub revoke_session_calls: Mutex<Vec<Value>>,
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
impl FakeStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            rotate_status: Mutex::new(None),
            observation: Mutex::new(None),
            sessions: Mutex::new(None),
            revoke_result: Mutex::new(None),
            issued: Mutex::new(Vec::new()),
            rotated: Mutex::new(Vec::new()),
            revoked_families: Mutex::new(Vec::new()),
            list_calls: Mutex::new(Vec::new()),
            revoke_session_calls: Mutex::new(Vec::new()),
        })
    }

    pub fn rotating(status: &str) -> Arc<Self> {
        let store = Self::new();
        *store.rotate_status.lock().unwrap() = Some(status.to_string());
        store
    }

    pub fn with_observation(self: Arc<Self>, observation: Result<Option<Value>, String>) -> Arc<Self> {
        *self.observation.lock().unwrap() = Some(observation);
        self
    }

    pub fn listing(page: Result<Value, String>) -> Arc<Self> {
        let store = Self::new();
        *store.sessions.lock().unwrap() = Some(page);
        store
    }

    pub fn revoking(result: Result<Value, String>) -> Arc<Self> {
        let store = Self::new();
        *store.revoke_result.lock().unwrap() = Some(result);
        store
    }
}

// 실제 `StoreError`에는 `Other`가 없어서 임의 사유 문자열은 `Unsupported(String)`으로 옮긴다.
#[allow(clippy::too_many_arguments)]
impl RefreshTokenStoreApi for FakeStore {
    fn store_issued(&self, token_id: &str, family_id: &str, email: &str, _issued_at: i64, _expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError> {
        self.issued.lock().unwrap().push(json!({"token_id": token_id, "family_id": family_id, "email": email, "client_ip": client_ip, "user_agent": user_agent}));
        Ok(())
    }

    fn rotate(
        &self,
        current_token_id: &str,
        new_token_id: &str,
        family_id: &str,
        email: &str,
        _issued_at: i64,
        _expires_at: i64,
        client_ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<Rotation, StoreError> {
        self.rotated
            .lock()
            .unwrap()
            .push(json!({"current_token_id": current_token_id, "new_token_id": new_token_id, "family_id": family_id, "email": email, "client_ip": client_ip, "user_agent": user_agent}));
        Ok(self.rotate_status.lock().unwrap().as_deref().map_or(Rotation::Ok, |status| status.parse().unwrap()))
    }

    fn revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError> {
        self.revoked_families.lock().unwrap().push((family_id.to_string(), reason.to_string()));
        Ok(())
    }

    fn get_token_observation(&self, _token_id: &str) -> Result<Option<Value>, StoreError> {
        match self.observation.lock().unwrap().clone() {
            Some(result) => result.map_err(StoreError::Unsupported),
            None => Ok(None),
        }
    }

    fn session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
        if let Some(Err(error)) = self.sessions.lock().unwrap().as_ref() {
            return Err(StoreError::Unsupported(error.clone()));
        }
        let revoked = self.revoked_families.lock().unwrap();
        let revoked_sessions = self.revoke_session_calls.lock().unwrap();
        Ok(self
            .issued
            .lock()
            .unwrap()
            .iter()
            .filter(|row| email.is_none_or(|e| row["email"] == e) && family_id.is_none_or(|fid| row["family_id"] == fid))
            .map(|row| {
                let fid = row["family_id"].as_str().unwrap();
                let active = !revoked.iter().any(|(family, _)| family == fid) && !revoked_sessions.iter().any(|session| session["family_id"] == fid);
                Row::from([("family_id".into(), fid.into()), ("email".into(), row["email"].as_str().unwrap().into()), ("valid_token_count".into(), i64::from(active).into())])
            })
            .collect())
    }

    fn list_sessions(&self, q: &SessionQuery) -> Result<Value, StoreError> {
        // 이름 붙은 인자를 객체로 담아 둔다.
        self.list_calls.lock().unwrap().push(json!({"status": q.status, "current_family_id": q.current_family_id, "page": q.page, "page_size": q.page_size, "email": q.email}));
        self.sessions.lock().unwrap().clone().unwrap_or_else(|| Err("unset".to_string())).map_err(StoreError::Unsupported)
    }

    fn revoke_session(&self, family_id: &str) -> Result<Value, StoreError> {
        self.revoke_session_calls.lock().unwrap().push(json!({"family_id": family_id}));
        self.revoke_result.lock().unwrap().clone().unwrap_or_else(|| Err("unset".to_string())).map_err(StoreError::Unsupported)
    }
}

// ── 대역 (env fixture) ─────────────────────────────────────

pub type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync>;

pub struct FetchImage(pub Result<Vec<u8>, backend::direct_cover_url::FetchError>);

impl backend::direct_cover_url::ImageFetcher for FetchImage {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, backend::direct_cover_url::FetchError> {
        assert_eq!(url, "https://example.com/image.png");
        self.0.clone()
    }
}

/// 표지 이미지 내려받기 대역. 테스트 중간에 바꿀 수 있다.
/// `BookManagerDeps::with_image_fetcher`로 주입한다. 오류 타입은 `FetchError`(구 `EpubCoverError`).
pub struct FetchSlot(pub Mutex<Fetch>);

impl FetchSlot {
    pub fn set(&self, f: impl Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync + 'static) {
        *self.0.lock().unwrap() = Box::new(f);
    }
}

impl ImageFetcher for FetchSlot {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        (self.0.lock().unwrap())(url)
    }
}

// TODO(common): 클로저를 `BookAdder`로 감싸는 어댑터.
pub struct FnBookAdder(pub Box<AddBookFn>);

impl BookAdder for FnBookAdder {
    fn add_book(&self) -> Result<i64, AddBookError> {
        (self.0)()
    }
}

/// 클로저를 `ImageFetcher`로 감싼다.
// TODO(common): `super::fx::fetcher(|url| ...)` (docs/rust-test-seams.md 3.7). 아직 없어 로컬로 둔다.
pub struct FnFetcher<F>(pub F);

impl<F: Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync> ImageFetcher for FnFetcher<F> {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        (self.0)(url)
    }
}

// TODO(common): 클로저를 `FileReader`로 감싸는 어댑터. 문서 3.7의 `fx::fetcher`처럼 `super::fx`에 두면 된다.
pub struct FnFileReader(pub Box<ReadFileFn>);

impl FileReader for FnFileReader {
    /// 객체 `{"<book_id>": doc}`를 문서 묶음으로 바꾼다. 오류와 객체가 아닌 값은 빈 묶음이다.
    fn read_file(&self, path: &Path, _path_prefix: Option<&Path>) -> Docs {
        match (self.0)(path) {
            Ok(Value::Object(map)) => map.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect(),
            _ => Docs::new(),
        }
    }

    fn try_read_file(&self, path: &Path, path_prefix: Option<&Path>, _skip_text: bool) -> Result<Docs, ReadFileError> {
        match (self.0)(path) {
            Err(LoaderReadError::Timeout(timeout)) => Err(ReadFileError::Timeout(timeout.message)),
            Err(LoaderReadError::Other(message)) => Err(ReadFileError::Other(message)),
            Ok(_) => Ok(self.read_file(path, path_prefix)),
        }
    }
}

/// 클로저를 `Thumbnailer`로 감싼다.
// TODO(common): `Thumbnailer` 클로저 어댑터가 `common`에 없다(docs/rust-test-seams.md 3.12는 trait 이름만 정한다).
pub struct FnThumbnailer<F>(pub F);

impl<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync> Thumbnailer for FnThumbnailer<F> {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        (self.0)(path, file_type, width)
    }
}

// ── 목 매니저: AsyncMock/MagicMock 대응 ──────────────────────────────────────
//
// 설계 3.2: 앱에는 `AppDeps`로 `Arc<dyn BookManagerApi>`/`Arc<dyn CategoryMappingApi>`를 넣는다. `Mock`은 그 trait를 구현하는
// 테스트 로컬 어댑터다. trait 메서드마다 호출을 `Mock::call(이름, CallArgs)`로 넘기고, 테스트가 정한 `returns`/`side_effect`의
// JSON 결과를 trait의 반환형으로 바꿔 준다. 호출 기록은 위치 인자와 이름 붙은 인자로 나눈다.
// 반환형이 `Result`가 아닌 메서드에서 `Err(String)`이 나오면 패닉으로 올린다.

/// 비동기 결과. 콜백과 side effect가 쓴다.
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── fixture: client / mock_bm / mock_cm / mock_cat / override_auth ───────────

/// 테스트 client와 manager 대역, 인증 우회를 한데 묶는다.
/// 인증 우회는 기본이 관리자이고, 테스트가 `set_auth`로 viewer로 바꾼다.
/// 원본에서 이 fixture들을 요청하지 않은 테스트도 같은 가짜 매니저를 쓴다. 그 테스트들은 매니저를 호출하지 않는다.
pub struct Fx {
    pub app: axum::Router,
    pub slots: Arc<AuthSlots>,
    pub bm: Mock,
    pub cm: Mock,
    pub cat: Mock,
    pub store: Store,
    pub _guard: Guard,
}

impl Fx {
    pub fn new() -> Self {
        super::setup_env();
        Self::build(Guard::Read(EnvGuard::read()), |deps| deps)
    }

    /// 환경 변수를 바꾸는 테스트용.
    pub fn with_env(vars: &[(&str, Option<&str>)]) -> Self {
        Self::build(Guard::Env(EnvGuard::apply(vars)), |deps| deps)
    }

    /// 환경 변수와 대역을 함께 바꾸는 테스트용.
    pub fn with_env_tweaked(vars: &[(&str, Option<&str>)], tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        Self::build(Guard::Env(EnvGuard::apply(vars)), tweak)
    }

    /// `mock.patch`로 내부 함수를 바꾸던 테스트용. `AppDeps`의 `with_*` 빌더를 `tweak`에서 이어 붙인다.
    pub fn tweaked(tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        super::setup_env();
        Self::build(Guard::Read(EnvGuard::read()), tweak)
    }

    /// `AppDeps`를 만드는 유일한 곳이다. 인증은 `AuthMode::Shared`라 테스트가 중간에 바꿀 수 있다.
    pub fn build(guard: Guard, tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        let (bm, cm, cat) = (Mock::new(), Mock::new(), Mock::new());
        let store = install_classify_status_store(&cat, None);
        let slots = Arc::new(AuthSlots { auth: Mutex::new(admin_payload()), admin: Mutex::new(admin_payload()) });
        let deps = AppDeps::new(AuthMode::Shared(slots.clone()))
            .with_book_manager(Arc::new(bm.clone()))
            .with_comics_manager(Arc::new(cm.clone()))
            .with_category_mapping(Arc::new(cat.clone()))
            .with_refresh_token_store(Arc::new(Mock::new()))
            .with_task_runner(Arc::new(JoinedRunner));
        Self { app: backend::main::app_with(tweak(deps)), slots, bm, cm, cat, store, _guard: guard }
    }

    /// 인증 결과를 `payload`로 고정한다.
    pub fn set_auth(&self, payload: Claims) {
        *self.slots.auth.lock().unwrap() = payload;
    }

    pub fn client(&self) -> TestClient {
        TestClient::with_app(self.app.clone()).header("host", "testserver")
    }

    pub async fn get(&self, path: &str) -> Response {
        self.client().get(path).await
    }

    pub async fn delete(&self, path: &str) -> Response {
        self.client().delete(path).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Response {
        self.client().post_json(path, body).await
    }

    pub async fn put_json(&self, path: &str, body: &Value) -> Response {
        self.client().put_json(path, body).await
    }

    /// 본문 없는 POST.
    pub async fn post_empty(&self, path: &str) -> Response {
        self.client().request(Method::POST, path, None, Vec::new()).await
    }

    pub async fn post_with_cookie(&self, path: &str, name: &str, value: &str) -> Response {
        self.client().cookie(name, value).request(Method::POST, path, None, Vec::new()).await
    }
}

// ── 환경 변수 가드 ───────────────────────────────────────────────────────────

/// 환경 변수를 바꾸는 테스트는 `super::env::EnvGuard`의 쓰기 잠금을, 일반 테스트는 읽기 잠금(`EnvGuard::read`)을 쥔다.
#[allow(dead_code)]
pub enum Guard {
    Read(EnvReadGuard),
    Env(EnvGuard),
}

pub struct JoinedHandle;

impl TaskHandle for JoinedHandle {}

/// 백그라운드 동작은 실제 작업 스레드에서 실행하고 응답 전에 끝낸다.
pub struct JoinedRunner;

impl TaskRunner for JoinedRunner {
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        let _ = std::thread::spawn(job).join();
        Box::new(JoinedHandle)
    }
}

/// 구 `Loader.read_file`이 던지는 오류.
#[derive(Debug)]
#[allow(dead_code)]
pub enum LoaderReadError {
    Timeout(ParserTimeout),
    Other(String),
}

// ── 한 쪽에 이미지가 여러 장인 책(실제 책처럼 xmlns 없는 비정형 HTML) ─────────────────────

pub const MULTI_PAGE: &str = concat!(
    "<html>\n  <head>\n    <title>엄마의 말뚝</title>\n  </head>\n    <body><li class=\"chapter\">\n",
    "<p><img alt=\"image\" height=\"597\" src=\"../Images/m1.jpg\" width=\"412\"/></p>\n",
    "<p><img alt=\"image\" height=\"597\" src=\"../Images/m2.jpg\" width=\"412\"/></p>\n",
    "</li></body></html>"
);

/// 메서드별 `return_value`/`side_effect`를 정하고 호출을 기록한다. 설정이 없는 메서드는 null을 돌려준다.
#[derive(Clone, Default)]
pub struct Mock(pub Arc<Mutex<MockState>>);

impl Mock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn returns(&self, method: &str, value: Value) {
        let mut s = self.0.lock().unwrap();
        s.side_effects.remove(method);
        s.returns.insert(method.to_string(), value);
    }

    pub fn side_effect(&self, method: &str, f: impl for<'a> Fn(CallArgs<'a>) -> Fut<'a, Result<Value, String>> + Send + Sync + 'static) {
        self.0.lock().unwrap().side_effects.insert(method.to_string(), Arc::new(f));
    }

    /// `side_effect=BookManagerError(msg)`에 대응한다.
    pub fn raises(&self, method: &str, msg: &str) {
        let msg = msg.to_string();
        self.side_effect(method, move |_| {
            let m = msg.clone();
            Box::pin(async move { Err(m) })
        });
    }

    pub fn set_path_prefix(&self, p: &Path) {
        self.0.lock().unwrap().path_prefix = Some(p.to_path_buf());
    }

    pub fn calls(&self, method: &str) -> Vec<RecordedArgs> {
        self.0.lock().unwrap().calls.iter().filter(|(m, _)| m == method).map(|(_, c)| c.clone()).collect()
    }

    pub fn call_count(&self, method: &str) -> usize {
        self.calls(method).len()
    }

    pub fn assert_not_called(&self, method: &str) {
        assert_eq!(self.call_count(method), 0, "{method}는 호출되지 않아야 한다");
    }

    pub fn assert_called_once_with(&self, method: &str, args: Value, kwargs: Value) {
        let calls = self.calls(method);
        assert_eq!(calls.len(), 1, "{method} 호출 횟수");
        assert_eq!((Value::Array(calls[0].args.clone()), calls[0].kwargs.clone()), (args, kwargs), "{method} 인자");
    }

    /// 마지막 호출 기준 `assert_called_with`.
    pub fn assert_called_with(&self, method: &str, args: Value, kwargs: Value) {
        let calls = self.calls(method);
        let last = calls.last().unwrap_or_else(|| panic!("{method}가 호출되지 않았다"));
        assert_eq!((Value::Array(last.args.clone()), last.kwargs.clone()), (args, kwargs), "{method} 마지막 인자");
    }

    pub fn assert_any_call(&self, method: &str, args: Value, kwargs: Value) {
        let found = self.calls(method).iter().any(|c| Value::Array(c.args.clone()) == args && c.kwargs == kwargs);
        assert!(found, "{method}를 이 인자로 부른 적이 없다: {args} {kwargs}");
    }

    /// 호출을 기록하고 side effect 또는 `returns` 값을 돌려준다. `Err`는 실패다.
    pub fn call<'a>(&self, method: &str, args: CallArgs<'a>) -> Fut<'a, Result<Value, String>> {
        let (effect, ret) = {
            let mut s = self.0.lock().unwrap();
            s.calls.push((
                method.to_string(),
                RecordedArgs { args: args.args.clone(), kwargs: args.kwargs.clone(), callback_names: args.callbacks.keys().cloned().collect(), thread_id: std::thread::current().id() },
            ));
            (s.side_effects.get(method).cloned(), s.returns.get(method).cloned())
        };
        match effect {
            Some(f) => f(args),
            None => Box::pin(async move { Ok(ret.unwrap_or(Value::Null)) }),
        }
    }

    /// 반환형이 `Result`가 아닌 비동기 메서드용. 예외는 패닉이다.
    pub async fn invoke<'a>(&self, method: &str, args: CallArgs<'a>) -> Value {
        match self.call(method, args).await {
            Ok(v) => v,
            Err(e) => panic!("{method}가 예외를 던졌다: {e}"),
        }
    }

    /// 동기 trait 메서드용. side effect가 바로 끝나는 future여야 한다.
    pub fn invoke_sync_try(&self, method: &str, args: CallArgs<'_>) -> Result<Value, String> {
        poll_ready(self.call(method, args))
    }

    pub fn invoke_sync(&self, method: &str, args: CallArgs<'_>) -> Value {
        match self.invoke_sync_try(method, args) {
            Ok(v) => v,
            Err(e) => panic!("{method}가 예외를 던졌다: {e}"),
        }
    }
}

#[async_trait::async_trait]
impl BookManagerApi for Mock {
    fn path_prefix(&self) -> PathBuf {
        self.0.lock().unwrap().path_prefix.clone().unwrap_or_default()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        Arc::new(EsProxy(self.clone()))
    }

    /// `[책, null]`은 찾음, `[null, 오류]`는 조회 실패, `[null, null]`은 없음이다.
    async fn get_book(&self, id: i64) -> Result<Book, BookLookupError> {
        let v = self.invoke("get_book", call_args!([id], {})).await;
        match (opt_book(arr(&v, 0)), opt_string(arr(&v, 1))) {
            (Some(book), _) => Ok(book),
            (None, Some(error)) => Err(BookLookupError::Failed(error)),
            (None, None) => Err(BookLookupError::NotFound(id)),
        }
    }

    async fn get_book_content(&self, id: i64) -> BookContent {
        let v = self.invoke("get_book_content", call_args!([id], {})).await;
        match v.get("__error__").and_then(Value::as_str) {
            Some(message) => BookContent::Error(message.to_string()),
            None => BookContent::Text(v.as_str().unwrap_or("").to_string()),
        }
    }

    async fn get_book_preview(&self, id: i64, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        to_response(self.invoke("get_book_preview", call_args!([], {"book_id": id, "chapters": chapters, "resource_base_url": resource_base_url})).await)
    }

    async fn get_book_preview_with_pages(&self, id: i64, pages: Option<i64>, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        to_response(
            self.invoke("get_book_preview", call_args!([], {"book_id": id, "pages": pages.unwrap_or(5), "chapters": chapters.unwrap_or(10), "resource_base_url": resource_base_url.unwrap_or("")}))
                .await,
        )
    }

    async fn get_pdf_pages(&self, id: i64, start: i64, end: i64) -> BookResponse {
        to_response(self.invoke("get_pdf_pages", call_args!([], {"book_id": id, "start": start, "end": end})).await)
    }

    async fn get_html_resource(&self, book_id: i64, resource_path: &str) -> BookResponse {
        to_response(self.invoke("get_html_resource", call_args!([], {"book_id": book_id, "resource_path": resource_path})).await)
    }

    async fn validate_epub(&self, book_id: i64) -> Result<EpubValidation, String> {
        typed_result(&self.invoke("validate_epub", call_args!([book_id], {})).await)
    }

    async fn validate_pdf(&self, book_id: i64) -> Result<PdfValidation, String> {
        typed_result(&self.invoke("validate_pdf", call_args!([book_id], {})).await)
    }

    async fn get_books_in_category(&self, category: &str) -> Result<Vec<Book>, String> {
        let v = self.invoke("get_books_in_category", call_args!([category], {})).await;
        opt_string(arr(&v, 1)).map_or_else(|| Ok(books(arr(&v, 0))), Err)
    }

    async fn get_categories(&self) -> Result<Value, String> {
        let v = self.invoke("get_categories", call_args!([], {})).await;
        opt_string(arr(&v, 1)).map_or_else(|| Ok(arr(&v, 0)), Err)
    }

    async fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, exclude_categories: Option<&[String]>, category: Option<&str>) -> Result<BookPage, String> {
        let mut kwargs = json!({"size": size, "offset": offset, "exclude_categories": exclude_categories});
        if let Some(category) = category {
            kwargs["category"] = json!(category);
        }
        let v = self.invoke("search_by_keyword_paged", CallArgs::new(vec![json!(keyword)], kwargs)).await;
        page(&v)
    }

    async fn get_latest_books(&self, size: i64, exclude_categories: Option<&[String]>) -> Result<BookPage, String> {
        let v = self.invoke("get_latest_books", call_args!([], {"size": size, "exclude_categories": exclude_categories})).await;
        page(&v)
    }

    async fn search_similar_books_paged(&self, book_id: i64, size: i64, offset: i64) -> Result<BookPage, String> {
        let v = self.invoke("search_similar_books_paged", call_args!([book_id], {"size": size, "offset": offset})).await;
        page(&v)
    }

    async fn get_cover_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        let v = self.invoke("get_cover_books_paged", call_args!([], {"category": category, "size": size, "cursor": cursor, "exclude_categories": exclude_categories})).await;
        cover_page(&v)
    }

    async fn get_uncovered_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        let v = self.invoke("get_uncovered_books_paged", call_args!([], {"category": category, "size": size, "cursor": cursor, "exclude_categories": exclude_categories})).await;
        cover_page(&v)
    }

    #[allow(clippy::too_many_arguments)]
    async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> Result<Value, String> {
        let args = call_args!([request.category, request.mappings], {"content_type": request.content_type, "use_bookstore": request.use_bookstore, "use_content_meta": request.use_content_meta, "delay": request.delay})
            .with_callback("on_progress", progress_callback(request.on_progress));
        let v = self.invoke("propose_category_changes", args).await;
        value_result(&v)
    }

    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> Result<Value, String> {
        let mut allowed: Vec<&String> = request.allowed_file_paths.iter().collect();
        allowed.sort();
        let args = call_args!([request.items, allowed], {"content_type": request.content_type, "clean_existing": request.clean_existing})
            .with_callback("on_progress", progress_callback(request.on_progress))
            .with_callback("on_item_done", progress_callback(request.on_item_done))
            .with_callback("should_continue", continue_callback(request.should_continue));
        let v = self.invoke("apply_category_changes", args).await;
        value_result(&v)
    }

    async fn index_single_file(&self, file_path: &str) -> Result<i64, String> {
        let v = self.invoke("index_single_file", call_args!([file_path], {})).await;
        match (arr(&v, 0).as_i64(), opt_string(arr(&v, 1))) {
            (Some(book_id), None) => Ok(book_id),
            (_, error) => Err(error.unwrap_or_default()),
        }
    }

    /// `["Ok", null]`만 성공이다.
    async fn delete_file(&self, file_path: &str) -> Result<(), String> {
        let v = self.invoke("delete_file", call_args!([file_path], {})).await;
        if arr(&v, 0).as_str() == Some("Ok") { Ok(()) } else { Err(opt_string(arr(&v, 1)).unwrap_or_default()) }
    }

    async fn reload_category(&self, category: &str, content_type: &str) -> Result<ReloadedFiles, String> {
        match self.call("reload_category", call_args!([category, content_type], {})).await {
            Ok(v) => typed_result(&v),
            Err(e) => Err(e),
        }
    }

    async fn reload_category_mismatches(&self, content_type: &str, on_progress: Option<ReloadProgress>) -> Result<MismatchReload, String> {
        let args = call_args!([content_type], {}).with_callback(
            "on_progress",
            on_progress.map(|f| {
                Arc::new(move |event| -> Fut<'_, Value> {
                    let f = f.clone();
                    Box::pin(async move {
                        f(serde_json::from_value(event).unwrap_or_default());
                        Value::Null
                    })
                }) as Callback<'_>
            }),
        );
        match self.call("reload_category_mismatches", args).await {
            Ok(v) => typed_result(&v),
            Err(e) => Err(e),
        }
    }

    async fn reload_category_mismatch_files(&self, category: &str, content_type: &str, on_progress: Option<ReloadProgress>) -> Result<MismatchReload, String> {
        let args = call_args!([category, content_type], {}).with_callback(
            "on_progress",
            on_progress.map(|f| {
                Arc::new(move |event| -> Fut<'_, Value> {
                    let f = f.clone();
                    Box::pin(async move {
                        f(serde_json::from_value(event).unwrap_or_default());
                        Value::Null
                    })
                }) as Callback<'_>
            }),
        );
        match self.call("reload_category_mismatch_files", args).await {
            Ok(v) => typed_result(&v),
            Err(e) => Err(e),
        }
    }

    fn get_category_mismatch_details(&self, category: &str) -> Result<MismatchDetails, String> {
        self.invoke_sync_try("get_category_mismatch_details", call_args!([category], {})).and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
    }
}

/// category_mapping 대역. 이름 붙은 인자(`content_type=...`)로 호출을 기록한다.
impl CategoryMappingApi for Mock {
    fn get_all_mappings(&self, ct: &str) -> Result<Value, MappingError> {
        self.invoke_sync_try("get_all_mappings", call_args!([], {"content_type": ct})).map_err(MappingError::Other)
    }

    fn get_keywords(&self, category: &str, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_keywords", call_args!([category], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_keywords(&self, category: &str, keywords: &[String], ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_keywords", call_args!([category, keywords], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn add_keyword(&self, category: &str, keyword: &str, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("add_keyword", call_args!([category, keyword], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn remove_keyword(&self, category: &str, keyword: &str, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("remove_keyword", call_args!([category, keyword], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn delete_category(&self, category: &str, ct: &str, prefix: bool) -> Result<bool, MappingError> {
        self.invoke_sync_try("delete_category", call_args!([category], {"content_type": ct, "prefix": prefix})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn update_all_mappings(&self, mappings: &Value, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("update_all_mappings", call_args!([mappings], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn get_hidden_categories(&self, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_hidden_categories", call_args!([], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_hidden(&self, category: &str, hidden: bool, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_hidden", call_args!([category, hidden], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn get_latest_excluded_categories(&self, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_latest_excluded_categories", call_args!([], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_latest_excluded(&self, category: &str, excluded: bool, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_latest_excluded", call_args!([category, excluded], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn acquire_reload_lock(&self, ct: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        let v = self.invoke_sync("acquire_reload_lock", call_args!([ct, category, reload_source], {}));
        Ok((flag(&arr(&v, 0)), opt_string(arr(&v, 1)), opt_json(arr(&v, 2))))
    }

    fn heartbeat_reload_lock(&self, ct: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        self.invoke_sync_try("heartbeat_reload_lock", CallArgs::new(vec![json!(ct), json!(category)], counts.clone())).map(|_| ())
    }

    fn complete_reload_lock(&self, ct: &str, status: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let mut kwargs = counts.clone();
        if let Some(category) = category {
            kwargs["category"] = json!(category);
        }
        self.invoke_sync_try("complete_reload_lock", CallArgs::new(vec![json!(ct), json!(status), json!(error)], kwargs)).map(|_| ())
    }

    fn get_reload_status(&self, ct: &str, category: Option<&str>) -> Result<Option<Value>, MappingError> {
        Ok(opt_json(self.invoke_sync("get_reload_status", call_args!([ct, category], {}))))
    }

    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, MappingError> {
        self.invoke_sync_try("get_classify_proposal_status", call_args!([content_type, stale_seconds], {})).map_err(MappingError::Other)
    }

    fn set_classify_proposal_status(&self, status: Value, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("set_classify_proposal_status", call_args!([status, content_type], {})).map(|_| ()).map_err(MappingError::Other)
    }

    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("merge_classify_proposal_status", call_args!([fields, content_type], {})).map(|_| ()).map_err(MappingError::Other)
    }

    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let v = self.invoke_sync("try_start_classify_proposal", call_args!([status, content_type, stale_seconds], {}));
        Ok((flag(&arr(&v, 0)), arr(&v, 1)))
    }

    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let v = self.invoke_sync("try_begin_classify_apply", call_args!([apply_token, content_type, stale_seconds], {}));
        Ok((flag(&arr(&v, 0)), arr(&v, 1)))
    }

    fn clear_classify_proposal_items(&self, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync("clear_classify_proposal_items", call_args!([], {"content_type": content_type}));
        Ok(())
    }

    fn get_classify_proposal_items(&self, content_type: &str) -> Result<Vec<Value>, MappingError> {
        Ok(self.invoke_sync("get_classify_proposal_items", call_args!([], {"content_type": content_type})).as_array().cloned().unwrap_or_default())
    }

    fn add_classify_proposal_items(&self, items: &[Value], source_category: &str, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("add_classify_proposal_items", call_args!([items, source_category], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn update_classify_proposal_item_payloads(&self, items: &[Value], content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("update_classify_proposal_item_payloads", call_args!([items], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("update_classify_proposal_item_status", call_args!([file_path, apply_status, apply_error], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn delete_applied_classify_proposal_items(&self, content_type: &str) -> Result<u64, MappingError> {
        Ok(count(&self.invoke_sync("delete_applied_classify_proposal_items", call_args!([], {"content_type": content_type}))).max(0) as u64)
    }

    fn delete_classify_proposal_item(&self, file_path: &str, content_type: &str) -> Result<u64, MappingError> {
        Ok(count(&self.invoke_sync("delete_classify_proposal_item", call_args!([file_path], {"content_type": content_type}))).max(0) as u64)
    }
}

/// refresh token 저장소 대역. `rotate`/`revoke_family`만 쓴다.
/// TODO(common): `RefreshTokenStoreApi`의 시그니처는 설계 문서에 없다.
impl RefreshTokenStoreApi for Mock {
    fn store_issued(&self, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError> {
        let kwargs = json!({"token_id":token_id,"family_id":family_id,"email":email,"issued_at":issued_at,"expires_at":expires_at,"client_ip":client_ip,"user_agent":user_agent});
        self.invoke_sync_try("store_issued", CallArgs::new(vec![], kwargs)).map(|_| ()).map_err(StoreError::Unsupported)
    }
    #[allow(clippy::too_many_arguments)]
    fn rotate(
        &self,
        current_token_id: &str,
        new_token_id: &str,
        family_id: &str,
        email: &str,
        issued_at: i64,
        expires_at: i64,
        client_ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<Rotation, StoreError> {
        let kwargs = json!({"current_token_id": current_token_id, "new_token_id": new_token_id, "family_id": family_id, "email": email, "issued_at": issued_at, "expires_at": expires_at, "client_ip": client_ip, "user_agent": user_agent});
        self.invoke_sync_try("rotate", CallArgs::new(vec![], kwargs)).and_then(|v| v.as_str().unwrap_or("").parse()).map_err(StoreError::Unsupported)
    }

    fn revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError> {
        self.invoke_sync_try("revoke_family", call_args!([family_id, reason], {})).map(|_| ()).map_err(StoreError::Unsupported)
    }

    fn get_token_observation(&self, token_id: &str) -> Result<Option<Value>, StoreError> {
        self.invoke_sync_try("get_token_observation", call_args!([token_id], {})).map(opt_json).map_err(StoreError::Unsupported)
    }

    fn session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
        let value = self.invoke_sync_try("session_rows", call_args!([email, family_id], {})).map_err(StoreError::Unsupported)?;
        let rows = value
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                row.as_object()
                    .into_iter()
                    .flat_map(|obj| obj.iter())
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            match value {
                                Value::Null => DbValue::Null,
                                Value::Number(n) => DbValue::Int(n.as_i64().unwrap_or_default()),
                                Value::String(s) => DbValue::Text(s.clone()),
                                _ => DbValue::Text(value.to_string()),
                            },
                        )
                    })
                    .collect()
            })
            .collect();
        Ok(rows)
    }
}

/// `AsyncMock()` book manager.
pub struct MockBm {
    /// `m.es_manager = MagicMock()`. 결과는 `set_*_pdf_stats`/`fail`로 테스트가 정한다.
    pub es: Arc<FakeEsManager>,
    pub categories: Mutex<(Value, Option<String>)>,
    pub proposal_progress: Mutex<Vec<Value>>,
    pub proposal_result: Mutex<Option<Value>>,
    pub apply_result: Mutex<(Value, Option<String>)>,
    pub path_prefix: Mutex<PathBuf>,
    pub mismatch_details: Mutex<MismatchDetails>,
    pub mismatch_categories: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl BookManagerApi for MockBm {
    fn get_category_mismatch_details(&self, category: &str) -> Result<MismatchDetails, String> {
        self.mismatch_categories.lock().unwrap().push(category.to_string());
        Ok(self.mismatch_details.lock().unwrap().clone())
    }

    async fn reload_category(&self, category: &str, _content_type: &str) -> Result<ReloadedFiles, String> {
        Ok(ReloadedFiles { category: category.to_string(), processed_count: 3 })
    }

    async fn reload_category_mismatch_files(&self, category: &str, content_type: &str, on_progress: Option<ReloadProgress>) -> Result<MismatchReload, String> {
        if let Some(report) = on_progress {
            report(ReloadCounts { before_count: Some(2), indexed_count: Some(1), deleted_count: None });
        }
        Ok(MismatchReload { content_type: content_type.to_string(), category: Some(category.to_string()), indexed_count: 1, before_count: 2, after_count: 1, ..MismatchReload::default() })
    }

    async fn reload_category_mismatches(&self, content_type: &str, on_progress: Option<ReloadProgress>) -> Result<MismatchReload, String> {
        if let Some(report) = on_progress {
            report(ReloadCounts { before_count: Some(2), indexed_count: Some(1), deleted_count: None });
        }
        Ok(MismatchReload { content_type: content_type.to_string(), indexed_count: 1, before_count: 2, after_count: 1, ..MismatchReload::default() })
    }

    async fn index_single_file(&self, _file_path: &str) -> Result<i64, String> {
        Ok(17)
    }

    async fn delete_file(&self, _file_path: &str) -> Result<(), String> {
        Ok(())
    }

    async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> Result<Value, String> {
        if let Some(progress) = request.on_progress {
            let events = self.proposal_progress.lock().unwrap().clone();
            if events.is_empty() {
                progress(json!({"total_count":1,"processed_count":0,"new_items":[{"file_path":"0_inbox/a.epub","apply_status":"pending"}]}));
                progress(json!({"total_count":1,"processed_count":1,"updated_items":[{"file_path":"0_inbox/a.epub","target_category":"3_SF","apply_status":"pending"}]}));
            } else {
                for event in events {
                    progress(event);
                }
            }
        }
        let result = self
            .proposal_result
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| json!({"source_category":"0_inbox","total_count":1,"processed_count":1,"items":[{"file_path":"0_inbox/a.epub","target_category":"3_SF"}],"failures":[]}));
        Ok(result)
    }

    async fn get_categories(&self) -> Result<Value, String> {
        let (value, error) = self.categories.lock().unwrap().clone();
        error.map_or(Ok(value), Err)
    }

    async fn apply_category_changes(&self, _items: &[Value], _allowed: &HashSet<String>, _clean_existing: bool) -> Result<Value, String> {
        let (value, error) = self.apply_result.lock().unwrap().clone();
        error.map_or(Ok(value), Err)
    }

    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> Result<Value, String> {
        if let Some(progress) = request.on_progress {
            progress(json!({"processed_count":request.items.len()}));
        }
        if let Some(done) = request.on_item_done {
            for item in request.items {
                if request.allowed_file_paths.contains(item["file_path"].as_str().unwrap_or_default()) {
                    done(json!({"file_path":item["file_path"],"apply_status":"moved","apply_error":null}));
                }
            }
        }
        if request.should_continue.is_some_and(|check| !check()) {
            return Err("stopped".into());
        }
        self.apply_category_changes(request.items, request.allowed_file_paths, request.clean_existing).await
    }

    fn path_prefix(&self) -> PathBuf {
        self.path_prefix.lock().unwrap().clone()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        self.es.clone()
    }
}

/// 분류 제안 작업 상태를 메모리로 흉내 낸 category_mapping mock. 전이 규칙까지 흉내 낸다.
/// 항목에 `_age_seconds`를 넣으면 '갱신이 그만큼 끊긴 작업'이 된다.
pub struct MockCat {
    pub store: Mutex<HashMap<String, Value>>,
    pub clear_calls: AtomicUsize,
    pub proposal_items: Mutex<Vec<Value>>,
    pub proposal_add_batch_sizes: Mutex<Vec<usize>>,
    pub proposal_write_thread: Mutex<Option<std::thread::ThreadId>>,
    pub reload_locks: Mutex<HashMap<String, Value>>,
}

impl MockCat {
    /// `install_classify_status_store(mock_cat, initial)`에 대응한다.
    pub fn with_store(initial: Option<Value>) -> Arc<Self> {
        let mut store = HashMap::from([("book".to_string(), json!({"status": "idle", "content_type": "book"})), ("comic".to_string(), json!({"status": "idle", "content_type": "comic"}))]);
        if let Some(initial) = initial {
            let content_type = initial.get("content_type").cloned().unwrap_or(json!("book"));
            let mut state = initial.clone();
            state["content_type"] = content_type;
            store.insert("book".to_string(), state);
        }
        Arc::new(Self {
            store: Mutex::new(store),
            clear_calls: AtomicUsize::new(0),
            proposal_items: Mutex::new(Vec::new()),
            proposal_add_batch_sizes: Mutex::new(Vec::new()),
            proposal_write_thread: Mutex::new(None),
            reload_locks: Mutex::new(HashMap::new()),
        })
    }

    pub fn effective(&self, content_type: &str, stale_seconds: i64) -> Value {
        let mut current = self.store.lock().unwrap().get(content_type).cloned().unwrap_or(json!({"status": "idle", "content_type": content_type}));
        let obj = current.as_object_mut().unwrap();
        let running = matches!(obj.get("status").and_then(Value::as_str), Some("running" | "applying"));
        if running && obj.remove("_age_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0) > stale_seconds as f64 {
            obj.insert("status".into(), json!("failed"));
            let error = obj.get("error").filter(|e| !e.is_null() && *e != &json!("")).cloned().unwrap_or(json!("응답 없이 중단된 것으로 보입니다."));
            obj.insert("error".into(), error);
        }
        obj.remove("_age_seconds");
        current
    }
}

impl CategoryMappingApi for MockCat {
    fn acquire_reload_lock(&self, content_type: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        let lock_key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let blocker = if category.is_some() {
            locks.get("_bulk").filter(|s| s["status"] == "running").or_else(|| locks.get(lock_key).filter(|s| s["status"] == "running"))
        } else {
            locks.values().find(|s| s["status"] == "running")
        };
        if let Some(status) = blocker {
            return Ok((false, Some("already running".into()), Some(status.clone())));
        }
        let status = json!({"status":"running","category":category,"reload_source":reload_source.unwrap_or("bulk"),"content_type":content_type,"indexed_count":0,"deleted_count":0,"failed_count":0,"before_count":0,"after_count":0,"error":null});
        locks.insert(lock_key.to_string(), status);
        Ok((true, None, None))
    }

    fn heartbeat_reload_lock(&self, _content_type: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let status = locks.get_mut(key).ok_or_else(|| "missing reload lock".to_string())?;
        for (field, value) in counts.as_object().ok_or_else(|| "counts must be an object".to_string())? {
            status[field] = value.clone();
        }
        Ok(())
    }

    fn complete_reload_lock(&self, _content_type: &str, state: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let status = locks.get_mut(key).ok_or_else(|| "missing reload lock".to_string())?;
        status["status"] = json!(state);
        status["error"] = json!(error);
        for (field, value) in counts.as_object().ok_or_else(|| "counts must be an object".to_string())? {
            status[field] = value.clone();
        }
        Ok(())
    }

    fn get_all_mappings(&self, _content_type: &str) -> Result<Value, backend::main::MappingError> {
        Ok(json!({}))
    }

    fn get_reload_status(&self, _content_type: &str, category: Option<&str>) -> Result<Option<Value>, MappingError> {
        Ok(self.reload_locks.lock().unwrap().get(category.unwrap_or("_bulk")).cloned())
    }

    fn add_classify_proposal_items(&self, items: &[Value], _source_category: &str, _content_type: &str) -> Result<(), MappingError> {
        *self.proposal_write_thread.lock().unwrap() = Some(std::thread::current().id());
        self.proposal_add_batch_sizes.lock().unwrap().push(items.len());
        self.proposal_items.lock().unwrap().extend_from_slice(items);
        Ok(())
    }

    fn update_classify_proposal_item_payloads(&self, items: &[Value], _content_type: &str) -> Result<(), MappingError> {
        let mut current = self.proposal_items.lock().unwrap();
        for item in items {
            let path = item["file_path"].as_str();
            if let Some(existing) = current.iter_mut().find(|old| old["file_path"].as_str() == path) {
                *existing = item.clone();
            }
        }
        Ok(())
    }

    fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, _content_type: &str) -> Result<(), MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let item = items.iter_mut().find(|item| item["file_path"] == file_path).ok_or_else(|| MappingError::Other("proposal item not found".into()))?;
        item["apply_status"] = json!(apply_status);
        item["apply_error"] = apply_error.map_or(Value::Null, |error| json!(error));
        Ok(())
    }

    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, MappingError> {
        Ok(self.effective(content_type, stale_seconds))
    }

    fn set_classify_proposal_status(&self, mut status: Value, content_type: &str) -> Result<(), MappingError> {
        status["content_type"] = json!(content_type);
        self.store.lock().unwrap().insert(content_type.to_string(), status);
        Ok(())
    }

    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) -> Result<(), MappingError> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(content_type.to_string()).or_insert(json!({"status": "idle", "content_type": content_type}));
        for (key, value) in fields.as_object().unwrap() {
            if key != "status" {
                entry[key] = value.clone();
            }
        }
        Ok(())
    }

    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let current = self.effective(content_type, stale_seconds);
        if matches!(current["status"].as_str(), Some("running" | "applying")) {
            return Ok((false, current));
        }
        self.set_classify_proposal_status(status, content_type).unwrap();
        Ok((true, self.effective(content_type, stale_seconds)))
    }

    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let current = self.effective(content_type, stale_seconds);
        if !matches!(current["status"].as_str(), Some("ready" | "done" | "failed")) {
            return Ok((false, current));
        }
        let mut next = current;
        next["status"] = json!("applying");
        next["apply_token"] = json!(apply_token);
        self.set_classify_proposal_status(next, content_type).unwrap();
        Ok((true, self.effective(content_type, stale_seconds)))
    }

    fn clear_classify_proposal_items(&self, _content_type: &str) -> Result<(), MappingError> {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        self.proposal_items.lock().unwrap().clear();
        Ok(())
    }

    fn get_classify_proposal_items(&self, _content_type: &str) -> Result<Vec<Value>, MappingError> {
        Ok(self.proposal_items.lock().unwrap().clone())
    }

    fn delete_classify_proposal_item(&self, file_path: &str, _content_type: &str) -> Result<u64, MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let before = items.len();
        items.retain(|item| item["file_path"] != file_path);
        Ok((before - items.len()) as u64)
    }

    fn delete_applied_classify_proposal_items(&self, _content_type: &str) -> Result<u64, MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let before = items.len();
        items.retain(|item| item["apply_status"] != "moved");
        Ok((before - items.len()) as u64)
    }
}

#[derive(Default)]
pub struct MockState {
    pub returns: HashMap<String, Value>,
    pub side_effects: HashMap<String, SideEffect>,
    pub calls: Vec<(String, RecordedArgs)>,
    pub path_prefix: Option<PathBuf>,
}

// --- 누락된 spine 문서: 총 챕터 수와 NCX 목차 ---

pub const NCX_NS_STR: &str = "http://www.daisy.org/z3986/2005/ncx/";

// ---- seam 어댑터 (공용 모듈에 아직 없는 것만 이 파일에 둔다) ----

/// 구 `utils.parser_timeout.ParserTimeout`. `FileReader`는 오류를 돌려주지 못해(빈 묶음이 곧 실패다) 이 값은 어댑터가 빈 묶음으로 바꾼다.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ParserTimeout {
    pub message: String,
}

pub type ReadFileFn = dyn Fn(&Path) -> Result<Value, LoaderReadError> + Send + Sync;

#[derive(Clone)]
pub struct RecordedArgs {
    pub args: Vec<Value>,
    pub kwargs: Value,
    pub callback_names: HashSet<String>,
    pub thread_id: std::thread::ThreadId,
}

// ---- 환경 변수 / 토큰 / 요청 헬퍼 ----

pub const SECRET: &str = "testsecret123";

/// 책 루트(`books`)와 서비스 파일(`meta`)을 나눠 둔다. 캐시 파일이 책으로 스캔되면 안 된다.
pub struct Setup {
    pub _tmp: TempDir,
    pub books: PathBuf,
    pub meta: PathBuf,
}

pub type SideEffect = Arc<dyn for<'a> Fn(CallArgs<'a>) -> Fut<'a, Result<Value, String>> + Send + Sync>;

// ── 분류 제안 작업 상태 저장소 흉내 ──────────────────────────────────────────

pub type Store = Arc<Mutex<HashMap<String, Value>>>;

/// 임시 파일 정리(`finally: path.unlink(missing_ok=True)`). panic이 나도 Drop에서 지운다.
pub struct TempFile(pub PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        unlink(&self.0);
    }
}

pub const URL: &str = "https://img.example.com/cover.jpg";

/// 위치 인자 `idx` 또는 키워드 인자 `name`에서 값을 꺼낸다.
pub fn arg(c: &CallArgs, idx: usize, name: &str) -> Value {
    c.args.get(idx).cloned().or_else(|| c.kwargs.get(name).cloned()).unwrap_or(Value::Null)
}

pub fn arg_or(c: &CallArgs, idx: usize, name: &str, default: Value) -> Value {
    c.args.get(idx).cloned().or_else(|| c.kwargs.get(name).cloned()).unwrap_or(default)
}

/// 구 `manager.override_add_book(closure)`.
pub fn book_adder(f: impl Fn() -> Result<i64, AddBookError> + Send + Sync + 'static) -> Arc<dyn BookAdder> {
    Arc::new(FnBookAdder(Box::new(f)))
}

pub fn continue_callback<'a>(f: Option<&'a (dyn Fn() -> bool + Send + Sync)>) -> Option<Callback<'a>> {
    f.map(|f| -> Callback {
        Arc::new(move |_: Value| -> Fut<'a, Value> {
            let value = f();
            Box::pin(async move { json!(value) })
        })
    })
}

pub fn decode(token: &str) -> Value {
    jsonwebtoken::decode::<Value>(token, &DecodingKey::from_secret(SECRET.as_bytes()), &Validation::new(Algorithm::HS256)).unwrap().claims
}

pub fn effective(store: &Store, content_type: &str, stale_seconds: f64) -> Value {
    let mut current = store.lock().unwrap().get(content_type).cloned().unwrap_or_else(|| json!({"status": "idle", "content_type": content_type}));
    let obj = current.as_object_mut().unwrap();
    let age = obj.remove("_age_seconds").and_then(|a| a.as_f64()).unwrap_or(0.0);
    let status = obj.get("status").and_then(Value::as_str).unwrap_or("").to_string();
    if (status == "running" || status == "applying") && age > stale_seconds {
        obj.insert("status".into(), json!("failed"));
        let error = obj.get("error").filter(|e| is_truthy(e)).cloned().unwrap_or_else(|| json!("응답 없이 중단된 것으로 보입니다."));
        obj.insert("error".into(), error);
    }
    current
}

pub fn encode_hs256(payload: &Value) -> String {
    jsonwebtoken::encode(&Header::new(Algorithm::HS256), payload, &EncodingKey::from_secret(SECRET.as_bytes())).unwrap()
}

pub fn fetcher<F: Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync + 'static>(f: F) -> Arc<dyn ImageFetcher> {
    Arc::new(FnFetcher(f))
}

/// 구 `loader_read_file_override(closure)`.
pub fn file_reader(f: impl Fn(&Path) -> Result<Value, LoaderReadError> + Send + Sync + 'static) -> Arc<dyn FileReader> {
    Arc::new(FnFileReader(Box::new(f)))
}

pub fn fixture() -> EnvGuard {
    let mut env = EnvGuard::new();
    env.set("TM_JWT_SECRET", SECRET);
    env.set("TM_ADMIN_EMAIL", "admin@example.com");
    env.set("TM_ALLOWED_EMAILS", "viewer1@example.com,viewer2@example.com");
    env
}

/// `install_classify_status_store`에 대응한다. 단순히 반환값을 박지 않고 전이 규칙을 흉내 낸다.
/// 항목에 `_age_seconds`를 넣으면 '갱신이 그만큼 끊긴 작업'이 된다.
pub fn install_classify_status_store(mock_cat: &Mock, initial: Option<Value>) -> Store {
    let store: Store = Arc::new(Mutex::new(HashMap::new()));
    {
        let mut s = store.lock().unwrap();
        s.insert("book".into(), json!({"status": "idle", "content_type": "book"}));
        s.insert("comic".into(), json!({"status": "idle", "content_type": "comic"}));
        if let Some(init) = initial {
            let mut m = init.as_object().cloned().unwrap_or_default();
            let ct = m.get("content_type").cloned().unwrap_or(json!("book"));
            m.insert("content_type".into(), ct);
            s.insert("book".into(), Value::Object(m));
        }
    }

    let ct_of = |c: &CallArgs, idx: usize| arg_or(c, idx, "content_type", json!("book")).as_str().unwrap().to_string();
    let stale_of = |c: &CallArgs, idx: usize| arg_or(c, idx, "stale_seconds", json!(300)).as_f64().unwrap();

    let st = store.clone();
    mock_cat.side_effect("get_classify_proposal_status", move |c| ready(Ok(effective(&st, &ct_of(&c, 0), stale_of(&c, 1)))));

    let st = store.clone();
    mock_cat.side_effect("set_classify_proposal_status", move |c| {
        store_set(&st, &arg(&c, 0, "status"), &ct_of(&c, 1));
        ready(Ok(Value::Null))
    });

    let st = store.clone();
    mock_cat.side_effect("merge_classify_proposal_status", move |c| {
        let ct = ct_of(&c, 1);
        let mut s = st.lock().unwrap();
        let entry = s.entry(ct.clone()).or_insert_with(|| json!({"status": "idle", "content_type": ct}));
        for (k, v) in arg(&c, 0, "fields").as_object().cloned().unwrap_or_default() {
            if k != "status" {
                entry[k] = v;
            }
        }
        ready(Ok(Value::Null))
    });

    let st = store.clone();
    mock_cat.side_effect("try_start_classify_proposal", move |c| {
        let (ct, stale) = (ct_of(&c, 1), stale_of(&c, 2));
        let current = effective(&st, &ct, stale);
        if matches!(current["status"].as_str(), Some("running" | "applying")) {
            return ready(Ok(json!([false, current])));
        }
        store_set(&st, &arg(&c, 0, "status"), &ct);
        ready(Ok(json!([true, effective(&st, &ct, stale)])))
    });

    let st = store.clone();
    mock_cat.side_effect("try_begin_classify_apply", move |c| {
        let (ct, stale) = (ct_of(&c, 1), stale_of(&c, 2));
        let current = effective(&st, &ct, stale);
        if !matches!(current["status"].as_str(), Some("ready" | "done" | "failed")) {
            return ready(Ok(json!([false, current])));
        }
        let mut next = current.clone();
        next["status"] = json!("applying");
        next["apply_token"] = arg(&c, 0, "apply_token");
        store_set(&st, &next, &ct);
        ready(Ok(json!([true, effective(&st, &ct, stale)])))
    });

    store
}

pub fn make_request(token: Option<&str>, cookie_token: Option<&str>) -> FakeRequest {
    let mut headers = HashMap::new();
    let mut cookies = HashMap::new();
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        headers.insert("Authorization".to_string(), format!("Bearer {t}"));
    }
    if let Some(c) = cookie_token.filter(|c| !c.is_empty()) {
        cookies.insert("tm_access_token".to_string(), c.to_string());
    }
    FakeRequest { headers, cookies }
}

pub fn mock_bm() -> Arc<MockBm> {
    let es = FakeEsManager::new();
    Arc::new(MockBm {
        es,
        categories: Mutex::new((json!({}), None)),
        proposal_progress: Mutex::new(Vec::new()),
        proposal_result: Mutex::new(None),
        apply_result: Mutex::new((json!({}), None)),
        path_prefix: Mutex::new(PathBuf::new()),
        mismatch_details: Mutex::new(MismatchDetails { fs_only: vec![FsOnlyFile { file_path: "A/a.txt".into(), ..FsOnlyFile::default() }], ..MismatchDetails::default() }),
        mismatch_categories: Mutex::new(Vec::new()),
    })
}

/// 이미 끝난 future를 한 번 폴링해서 값을 꺼낸다. 동기 trait 메서드(`CategoryMappingApi` 등)가 쓴다.
pub fn poll_ready<T>(mut fut: Fut<'_, T>) -> T {
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("동기 trait 메서드의 side effect는 바로 끝나는 future여야 한다"),
    }
}

pub fn progress_callback<'a>(f: Option<&'a (dyn Fn(Value) + Send + Sync)>) -> Option<Callback<'a>> {
    f.map(|f| -> Callback {
        Arc::new(move |event: Value| -> Fut<'a, Value> {
            f(event);
            Box::pin(async { Value::Null })
        })
    })
}

pub fn ready(v: Result<Value, String>) -> Fut<'static, Result<Value, String>> {
    Box::pin(async move { v })
}

pub fn setup(files: &[&str]) -> Setup {
    let tmp = TempDir::new().unwrap();
    let books = tmp.path().join("books");
    let meta = tmp.path().join("meta");
    fs::create_dir_all(books.join("0_inbox")).unwrap();
    fs::create_dir_all(&meta).unwrap();
    for name in files {
        fs::write(books.join("0_inbox").join(name), "본문 내용 ".repeat(20)).unwrap();
    }
    Setup { books, meta, _tmp: tmp }
}

pub fn store_set(store: &Store, status: &Value, content_type: &str) {
    let mut m: Map<String, Value> = status.as_object().cloned().unwrap_or_default();
    m.insert("content_type".into(), json!(content_type));
    store.lock().unwrap().insert(content_type.to_string(), Value::Object(m));
}

pub fn thumbnailer<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync + 'static>(f: F) -> Arc<dyn Thumbnailer> {
    Arc::new(FnThumbnailer(f))
}

pub fn manager(s: &Setup, deps: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> BookManager {
    super::setup_env();
    BookManager::with_deps(deps(BookManagerDeps::new(s.books.clone(), FakeEsManager::new())))
}

pub fn epub_with_missing_page(path: &Path, include_page: bool) -> PathBuf {
    let container = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let opf = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="a" href="Text/a.xhtml" media-type="application/xhtml+xml"/><item id="colophon" href="Text/colophon%20page.xhtml" media-type="application/xhtml+xml"/></manifest><spine toc="ncx"><itemref idref="a"/><itemref idref="colophon"/><itemref idref="ghost"/></spine></package>"#;
    let ncx = format!(
        r#"<?xml version="1.0"?><ncx xmlns="{NCX_NS_STR}"><navMap><navPoint id="n1"><navLabel><text>A</text></navLabel><content src="Text/a.xhtml#top"/></navPoint><navPoint id="n2"><navLabel><text>판권</text></navLabel><content src="Text/colophon%20page.xhtml"/><navPoint id="n3"><navLabel><text>자식</text></navLabel><content src="Text/a.xhtml#kid"/></navPoint></navPoint></navMap></ncx>"#
    );
    let a = "<html xmlns='http://www.w3.org/1999/xhtml'><body><p>본문</p></body></html>";
    let colophon = "<html xmlns='http://www.w3.org/1999/xhtml'><body><p>판권</p></body></html>";
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/Text/a.xhtml", a.as_bytes()),
    ];
    if include_page {
        entries.push(("OEBPS/Text/colophon page.xhtml", colophon.as_bytes()));
    }
    write_zip(path, &entries);
    path.to_path_buf()
}

/// `os.listdir` 대역. 열거 결과(또는 오류)를 테스트가 정한다.
/// TODO(common): `FaultyFs`에는 읽기 결과를 바꾸는 훅이 없어 따로 둔다. 나머지 `FsOps` 메서드는 trait 기본 구현(설계 3.0 규칙 4)에 맡긴다.
pub struct ListDirFs(pub Result<Vec<PathBuf>, String>);

impl ListDirFs {
    pub fn ok(names: &[&str]) -> Arc<Self> {
        Arc::new(Self(Ok(names.iter().map(PathBuf::from).collect())))
    }

    pub fn err(message: &str) -> Arc<Self> {
        Arc::new(Self(Err(message.to_string())))
    }
}

impl sysdeps::FsOps for ListDirFs {
    fn read_dir(&self, _p: &Path) -> std::io::Result<Vec<PathBuf>> {
        self.0.clone().map_err(std::io::Error::other)
    }
}
