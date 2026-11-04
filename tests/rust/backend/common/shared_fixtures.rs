//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use backend::book_metadata::{Book, BookPage};
use backend::book_manager::{BookManager, BookManagerDeps, BookResponse, CoverPage, ProposeOptions};
use backend::es_manager::{EsManagerApi, HttpTransport};
use super::{es_dummy, Response};
use serde_json::{json, Value};
use tempfile::TempDir;
use std::{fs};
use super::es_fake::{FakeEsManager};
use super::fsx::{dir_names};
use super::zipx::{Members, read_zip_entry, rewrite_epub_members, write_zip_with, ZipMode};
use super::image::{BLUE, color, encode, encode_image, image_size, jpeg_bytes};
use backend::{epub_cover};
use backend::epub_cover::{CoverError, edit_images, EditImages, EpubCoverHooks, Limits};
use super::epub::{EpubSpec, make_epub};
use image::{DynamicImage, ImageFormat, Rgb, RgbaImage, RgbImage};
use std::io::{Read};
use zip::ZipArchive;
use backend::cover_management::{CoverManagement};
use super::pdf::{make_blank_pdf};

pub fn admin_payload() -> Value {
    json!({"email": "admin@test.com", "role": "admin", "name": "Admin", "picture": "", "exp": now_plus_3600()})
}

pub fn allowed_set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

// JSON -> trait 반환형 변환.

pub fn arr(v: &Value, i: usize) -> Value {
    v.get(i).cloned().unwrap_or(Value::Null)
}

/// `Result<(), _>`를 Python 원본의 `("Ok" | "Error", 메시지)`로 펼친다.
pub fn as_status(result: Result<(), String>) -> (String, Option<String>) {
    match result {
        Ok(()) => ("Ok".to_string(), None),
        Err(error) => ("Error".to_string(), Some(error)),
    }
}

/// `pytest.raises(CoverError, match=needle)`
pub fn assert_cover_error<T: std::fmt::Debug>(result: Result<T, CoverError>, needle: &str) {
    match result {
        Err(e) if !e.is_io() => assert!(e.to_string().contains(needle), "CoverError 메시지에 {needle:?}가 없다: {e}"),
        other => panic!("CoverError({needle:?})를 기대했지만 {other:?}"),
    }
}

pub fn assert_refused(result: Result<Value, CoverError>, reason: &str, label: &str) {
    let text = err_of(result);
    assert!(text.contains(reason), "{label}: {text:?} 에 {reason:?}가 없다");
}

/// `asyncio.run(coro)`에 대응한다. 동기 테스트 안에서 future를 끝까지 돌린다.
pub fn asyncio_runner<F: std::future::Future>(fut: F) -> F::Output {
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

pub fn edit(path: &Path, trash: &Path, args: EditImages) -> Result<Value, CoverError> {
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

pub fn err_of(result: Result<Value, CoverError>) -> String {
    result.expect_err("CoverError가 나야 한다").to_string()
}

pub fn error_text(error: &Option<String>) -> &str {
    error.as_deref().unwrap_or("")
}

pub fn first_item(service: &CoverManagement, covered: bool) -> Value {
    service.missing("", "", 60, covered).unwrap()["items"][0].clone()
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
    epub_cover::inspect_epub(path).unwrap()["has_cover"].as_bool().unwrap()
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

/// Python `make_doc(path, file_type)`.
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

pub fn model_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/trained")
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

/// `Result<Value, _>`를 Python 원본의 `(결과, 오류)`로 펼친다. 실패하면 원본처럼 빈 객체다.
pub fn outcome(result: Result<serde_json::Value, String>) -> (serde_json::Value, Option<String>) {
    match result {
        Ok(value) => (value, None),
        Err(error) => (serde_json::json!({}), Some(error)),
    }
}

/// `[책 목록, 전체 수, 오류]`를 `Result<BookPage, _>`로 바꾼다.
pub fn page(v: &Value) -> Result<BookPage, String> {
    opt_string(arr(v, 2)).map_or_else(|| Ok(BookPage { books: books(arr(v, 0)), total: count(&arr(v, 1)), next_cursor: None }), Err)
}

// TODO(common): lopdf가 아니라 /Info 사전을 직접 쓰는 최소 PDF 생성기. super::pdf::PdfBuilder에 `info` 지원이 생기면 지운다.
/// pypdf `PdfWriter.add_blank_page`로 만든 PDF에 대응한다. `info`는 /Info 사전의 (키, 값)이다.
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

/// 현재 `revision`. Python `inspect_epub(path)["revision"]`.
pub fn rev(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap()["revision"].as_str().unwrap().to_string()
}

pub fn revision(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap()["revision"].as_str().unwrap().to_string()
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

/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
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

/// pytest `tmp_path`에 대응한다. 반환값이 drop되면 디렉토리가 지워진다.
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

/// 검증 결과를 Python 원본의 `(유효 여부, 사유)`로 펼친다.
pub fn verdict(result: Result<(), String>) -> (bool, Option<String>) {
    match result {
        Ok(()) => (true, None),
        Err(reason) => (false, Some(reason)),
    }
}

/// mimetype(무압축)을 맨 앞에 두고 나머지를 압축해서 쓴다. Python 생성 헬퍼가 모두 이 배치다.
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

// ── XML 확인 도구 (lxml xpath 대응) ────────────────────────

pub fn xml_text(data: &[u8]) -> String {
    String::from_utf8(data.to_vec()).unwrap()
}

pub fn zip_read<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>, name: &str) -> Vec<u8> {
    let mut data = Vec::new();
    zf.by_name(name).unwrap_or_else(|_| panic!("KeyError: {name}")).read_to_end(&mut data).unwrap();
    data
}

pub fn zip_read_string<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>, name: &str) -> String {
    String::from_utf8(zip_read(zf, name)).unwrap()
}
