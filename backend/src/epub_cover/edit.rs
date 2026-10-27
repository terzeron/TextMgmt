//! EPUB 쪽 이미지의 크롭·분할·삭제·삽입. Python `epub_cover.edit_images`와 그 도우미에 대응한다.
//!
//! 입력 검증은 Python보다 엄격하다.
//! - 편집 항목 수(크롭·분할·삭제 각 500, 삽입 100)와 쪽 안 이미지 번호(0~10000)에 상한이 있다.
//! - 크롭 영역은 숫자 5개(x0, y0, x1, y1, angle)여야 하고 유한해야 한다. 범위는 `image_edit::crop_pil`이 확인한다.
//! - 읽는 이미지는 읽기 전에 바이트 크기(`max_image_bytes`)를, 디코딩 전에 화소 수(`max_image_pixels`)를 상한과 비교한다.
//!   크롭 결과는 원본보다 커지지 않는다.
//! - 편집으로 쌓는 멤버의 합이 `MAX_EDIT_OUTPUT_BYTES`를 넘으면 거절한다.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;
use std::sync::LazyLock;

use image::{DynamicImage, ImageFormat};
use regex::Regex;
use serde_json::{Value, json};
use zip::ZipArchive;

use super::pathx::{dirname, member_path, relpath};
use super::rewrite::rewrite_epub;
use super::stage::{Added, add_image_page, add_split_image, read_previous_cover, sections, verify_staged, write_epub};
use super::xml::{self, Element, Node};
use super::{CoverError, EpubCoverHooks, FResult, Fail, MAX_XML_BYTES, entry_names, entry_size, has_entry, normalize_image_with, read_dimensions, read_entry, read_package_with};
use crate::cover_sources::urlx::quote;
use crate::image_edit::{Rect, crop_pil};

/// crops 의 idref 에 이 값을 주면 쪽이 아니라 표지 이미지를 자른다(표지 화면은 spine 항목이 아닐 수 있다).
pub const COVER_TARGET: &str = "@cover";
const TOOL_COVER_PREFIX: &str = "tm-cover-page-";
/// 요청 하나의 크롭·분할·삭제 항목 수 상한(종류별).
pub const MAX_EDITS: usize = 500;
/// 요청 하나의 삽입 항목 수 상한.
pub const MAX_INSERTS: usize = 100;
/// 쪽 안 이미지 번호 상한.
pub const MAX_IMAGE_INDEX: i64 = 10_000;
/// 편집 결과로 쌓아 두는 멤버 내용의 합 상한.
pub const MAX_EDIT_OUTPUT_BYTES: usize = 512 * 1024 * 1024;
/// 쪽 문서를 캐시하는 합 상한.
const MAX_PAGE_CACHE_BYTES: usize = 256 * 1024 * 1024;

/// 한 번에 저장할 이미지 편집.
///
/// crops 원소: `{"idref", "index"?, "rect": [x0, y0, x1, y1, angle]}`, splits 원소: `{"idref", "index"?, "first": rect, "second": rect}`,
/// deletes 원소: `{"idref", "index"?}` 또는 idref 문자열, inserts 원소: `(before_idref, 이미지 바이트)`.
#[derive(Default, Debug, Clone)]
pub struct EditImages {
    pub crops: Vec<Value>,
    pub splits: Vec<Value>,
    pub deletes: Vec<Value>,
    pub inserts: Vec<(String, Vec<u8>)>,
    pub backup_name: Option<String>,
}

fn err(message: &str) -> CoverError {
    CoverError::new(message)
}

fn crop_err(detail: &str) -> CoverError {
    CoverError::new(&format!("크롭할 수 없습니다 ({detail})"))
}

// ---------------------------------------------------------------------------
// 쪽 문서 해석
// ---------------------------------------------------------------------------

/// 이미지 편집 대상 이미지(쪽 spine 항목의 `index`번째)의 구조.
#[derive(Debug, Clone)]
struct PageInfo {
    page_member: String,
    image_member: String,
    /// 쪽 안 이미지 수
    count: usize,
    index: usize,
    /// 쪽 안 모든 이미지가 가리키는 멤버(문서 순서, 못 찾으면 "")
    image_refs: Vec<String>,
    /// 보이는 글이 없는가
    image_only: bool,
    /// `<img>`가 아니라 `<svg><image>`로 이미지를 넣은 쪽
    svg_image: bool,
}

impl PageInfo {
    /// 이미지 한 장뿐이고 글이 없어 쪽 단위(새 쪽 추가·쪽 삭제)로 분할·삭제할 수 있는 쪽인가.
    fn page_level(&self) -> bool {
        self.count == 1 && self.image_only
    }
}

/// element 안의 보이는 글. SVG의 title·desc는 화면에 그려지지 않으므로 뺀다(Python과 같게 주석 본문은 센다).
fn visible_text(element: &Element) -> String {
    const SVG_NS: &str = "http://www.w3.org/2000/svg";
    fn walk(element: &Element, out: &mut String) {
        for node in &element.children {
            match node {
                Node::Text(text) | Node::Comment(text) => out.push_str(text),
                Node::Element(child) => {
                    let hidden = matches!(child.name.as_str(), "title" | "desc") && child.ns.as_deref() == Some(SVG_NS);
                    if !hidden {
                        walk(child, out);
                    }
                }
                Node::Pi(..) => {}
            }
        }
    }
    let mut out = String::new();
    walk(element, &mut out);
    out.trim().to_string()
}

fn image_href(element: &Element) -> String {
    [element.attr("src"), element.attr("href"), element.attr_ns(xml::XLINK_NS, "href")].into_iter().flatten().find(|v| !v.is_empty()).unwrap_or("").to_string()
}

fn page_info<R: Read + Seek>(archive: &mut ZipArchive<R>, base: &str, items: &BTreeMap<String, String>, idref: &str, index: usize, hooks: &EpubCoverHooks) -> FResult<PageInfo> {
    let page_member = items.get(idref).map(|href| member_path(base, href)).unwrap_or_default();
    if page_member.is_empty() || !has_entry(archive, &page_member) {
        return Err(err("편집할 쪽을 찾을 수 없습니다. 책을 다시 열어 보세요").into());
    }
    if entry_size(archive, &page_member)? > hooks.limits().max_xml_bytes as u64 {
        return Err(crop_err("쪽 문서가 너무 큼").into());
    }
    let root = xml::parse_lenient(&read_entry(archive, &page_member)?).ok_or_else(|| crop_err("쪽 문서를 읽을 수 없음"))?;
    let all = root.descendants();
    let images: Vec<&Element> = all.iter().copied().filter(|e| matches!(e.name.as_str(), "img" | "image")).collect();
    if images.is_empty() {
        return Err(crop_err("이미지가 0개인 쪽").into());
    }
    if index >= images.len() {
        return Err(crop_err(&format!("이미지를 찾을 수 없음: 이 쪽의 {}번째 이미지, 전체 {}개", index + 1, images.len())).into());
    }
    let page_dir = dirname(&page_member);
    let refs: Vec<String> = images.iter().map(|e| member_path(&page_dir, &image_href(e))).collect();
    let image_member = refs[index].clone();
    if image_member.is_empty() || !has_entry(archive, &image_member) {
        return Err(crop_err("이미지를 찾을 수 없음").into());
    }
    let body = all.iter().copied().find(|e| e.name == "body").unwrap_or(&root);
    Ok(PageInfo { page_member, image_member, count: images.len(), index, image_refs: refs, image_only: visible_text(body).is_empty(), svg_image: images[index].name == "image" })
}

static IMAGE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<(?:img|image)\b[^>]*>").expect("정규식"));
static MASKED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->|<!\[CDATA\[.*?\]\]>").expect("정규식"));
static URL_ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)(xlink:href|src|href)\s*=\s*(["'])"#).expect("정규식"));
static WRAPPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<(p|div)\b[^>]*>\s*\z").expect("정규식"));
static CLOSING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\A\s*</(\w+)\s*>").expect("정규식"));
static XML_DECLARATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<\?xml[^>]*encoding\s*=\s*["']([\w.-]+)["']"#).expect("정규식"));
static META_CHARSET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)<meta[^>]*charset\s*=\s*["']?([\w.-]+)"#).expect("정규식"));

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Utf8,
    Utf8Sig,
    Utf16,
    Latin1,
}

/// 쪽 XHTML 바이트를 (글, 인코딩)으로 푼다. BOM·xml 선언·meta charset을 보고 없으면 utf-8. 풀 수 없으면 `CoverError`.
fn decode_page(raw: &[u8]) -> Result<(String, Encoding), CoverError> {
    let unreadable = || err("쪽 구조를 해석할 수 없습니다 (문자 인코딩을 읽을 수 없음)");
    if let Some(rest) = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Ok((String::from_utf8(rest.to_vec()).map_err(|_| unreadable())?, Encoding::Utf8Sig));
    }
    if raw.starts_with(&[0xFF, 0xFE]) || raw.starts_with(&[0xFE, 0xFF]) {
        return Ok((xml::decode(raw).map_err(|_| unreadable())?, Encoding::Utf16));
    }
    let head: String = raw.iter().take(1024).filter(|b| b.is_ascii()).map(|b| *b as char).collect();
    let declared = XML_DECLARATION.captures(&head).or_else(|| META_CHARSET.captures(&head)).map(|c| c[1].to_lowercase());
    let encoding = match declared.as_deref() {
        None | Some("utf-8" | "utf8" | "ascii" | "us-ascii") => Encoding::Utf8,
        Some("iso-8859-1" | "latin1" | "latin-1" | "iso8859-1") => Encoding::Latin1,
        Some(_) => return Err(unreadable()),
    };
    let text = match encoding {
        Encoding::Latin1 => raw.iter().map(|b| *b as char).collect(),
        _ => String::from_utf8(raw.to_vec()).map_err(|_| unreadable())?,
    };
    Ok((text, encoding))
}

fn encode_page(text: &str, encoding: Encoding) -> Result<Vec<u8>, CoverError> {
    match encoding {
        Encoding::Utf8 => Ok(text.as_bytes().to_vec()),
        Encoding::Utf8Sig => Ok([&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat()),
        Encoding::Utf16 => Ok([0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect()),
        Encoding::Latin1 => text.chars().map(|c| u8::try_from(u32::from(c)).map_err(|_| err("쪽 구조를 해석할 수 없습니다 (문자 인코딩을 쓸 수 없음)"))).collect(),
    }
}

/// text 에서 `<img>`/`<image>` 시작 태그를 문서 순서로 찾는다. 주석·CDATA 안의 것은 뺀다.
fn image_tags(text: &str) -> Vec<(usize, usize)> {
    let masked: Vec<(usize, usize)> = MASKED.find_iter(text).map(|m| (m.start(), m.end())).collect();
    IMAGE_TAG.find_iter(text).filter(|m| !masked.iter().any(|(start, end)| *start <= m.start() && m.start() < *end)).map(|m| (m.start(), m.end())).collect()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 이미지 태그의 src/href/xlink:href 값을 `new_href`로 바꾼 복사본. 다른 속성은 그대로 둔다.
fn split_tag_copy(tag: &str, new_href: &str) -> String {
    let quoted = quote(new_href);
    let mut out = String::with_capacity(tag.len() + quoted.len());
    let mut at = 0;
    while let Some(caps) = URL_ATTRIBUTE.captures_at(tag, at) {
        let whole = caps.get(0).expect("전체 일치");
        let name = caps.get(1).expect("속성 이름");
        let previous = tag[..name.start()].chars().next_back();
        if previous.is_some_and(|c| is_word(c) || c == ':' || c == '-') {
            // `data-src=`처럼 다른 속성의 일부다. 한 글자 뒤에서 다시 찾는다.
            let step = tag[name.start()..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&tag[at..name.start() + step]);
            at = name.start() + step;
            continue;
        }
        let quote_char = caps.get(2).expect("따옴표").as_str();
        let value_start = whole.end();
        let Some(close) = tag[value_start..].find(quote_char) else { break };
        out.push_str(&tag[at..value_start]);
        out.push_str(&quoted);
        out.push_str(quote_char);
        at = value_start + close + quote_char.len();
    }
    out.push_str(&tag[at..]);
    out
}

/// 쪽 XHTML 원문에서 이미지 태그를 문자열로 바꾼다(다시 직렬화하지 않아 나머지는 바이트 그대로다).
///
/// edits: `[(index, "split" | "delete", split 이면 새 이미지 멤버 상대경로)]`. index 내림차순으로 적용해 번호가 밀리지 않는다.
pub fn edit_page_images(raw: &[u8], count: usize, edits: &[(usize, &str, Option<Value>)]) -> Result<Vec<u8>, CoverError> {
    let (mut text, encoding) = decode_page(raw)?;
    let tags = image_tags(&text);
    if tags.len() != count {
        return Err(err("쪽 구조를 해석할 수 없습니다 (이미지 태그 수가 맞지 않음)"));
    }
    let mut ordered: Vec<&(usize, &str, Option<Value>)> = edits.iter().collect();
    ordered.sort_by_key(|edit| std::cmp::Reverse(edit.0));
    for (index, kind, new_href) in ordered {
        let Some(&(mut start, mut end)) = tags.get(*index) else { return Err(err("쪽 구조를 해석할 수 없습니다 (이미지 번호가 범위 밖)")) };
        if *kind == "split" {
            let tag = text[start..end].to_string();
            // 영역 둘이 위아래로 이어서 보이도록 줄을 바꾼다(SVG 안에서는 쓸 수 없다).
            let separator = if tag.get(1..4).is_some_and(|t| t.eq_ignore_ascii_case("img")) { "<br/>" } else { "" };
            let href = new_href.as_ref().and_then(Value::as_str).unwrap_or("");
            text.insert_str(end, &format!("{separator}{}", split_tag_copy(&tag, href)));
        } else {
            let wrapper = WRAPPER.captures(&text[..start]);
            let closing = CLOSING.captures(&text[end..]);
            if let (Some(wrapper), Some(closing)) = (wrapper, closing)
                && closing[1].eq_ignore_ascii_case(&wrapper[1])
            {
                start = wrapper.get(0).expect("전체 일치").start();
                end += closing.get(0).expect("전체 일치").end();
            }
            text.replace_range(start..end, "");
        }
    }
    encode_page(&text, encoding)
}

// ---------------------------------------------------------------------------
// 이미지
// ---------------------------------------------------------------------------

/// 편집 영역(`[x0, y0, x1, y1, angle]`)을 읽는다. 숫자 다섯 개가 아니거나 유한하지 않으면 거절한다.
fn parse_rect(value: Option<&Value>) -> Result<Rect, CoverError> {
    let bad = || crop_err("크롭 영역이 올바르지 않음");
    let array = value.and_then(Value::as_array).ok_or_else(bad)?;
    let numbers: Vec<f64> = array.iter().map(|v| v.as_f64().filter(|f| f.is_finite()).ok_or_else(bad)).collect::<Result<_, _>>()?;
    match numbers[..] {
        [x0, y0, x1, y1] => Ok((x0, y0, x1, y1, 0.0)),
        [x0, y0, x1, y1, angle] => Ok((x0, y0, x1, y1, angle)),
        _ => Err(bad()),
    }
}

fn format_name(format: Option<ImageFormat>) -> String {
    match format {
        Some(ImageFormat::Gif) => "GIF".to_string(),
        Some(ImageFormat::WebP) => "WEBP".to_string(),
        Some(other) => format!("{other:?}").to_uppercase(),
        None => "알 수 없음".to_string(),
    }
}

/// 투명 부분을 흰색으로 깔아 알파를 없앤다.
fn flatten(source: DynamicImage) -> DynamicImage {
    if !source.color().has_alpha() {
        return match source {
            DynamicImage::ImageRgb8(_) | DynamicImage::ImageLuma8(_) => source,
            other => DynamicImage::ImageRgb8(other.to_rgb8()),
        };
    }
    let rgba = source.to_rgba8();
    let mut flat = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = u32::from(pixel[3]);
        let mix = |c: u8| ((u32::from(c) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        flat.put_pixel(x, y, image::Rgb([mix(pixel[0]), mix(pixel[1]), mix(pixel[2])]));
    }
    DynamicImage::ImageRgb8(flat)
}

/// JPEG/PNG 이미지를 `rect`로 잘라 같은 형식으로 다시 인코딩한다: (바이트, 형식).
fn crop_image_bytes(data: &[u8], rect: Rect, hooks: &EpubCoverHooks) -> Result<(Vec<u8>, ImageFormat), CoverError> {
    let unreadable = || crop_err("이미지를 읽을 수 없음");
    let limits = hooks.limits();
    if data.len() > limits.max_image_bytes {
        return Err(crop_err("이미지가 너무 큼"));
    }
    let reader = image::ImageReader::new(Cursor::new(data)).with_guessed_format().map_err(|_| unreadable())?;
    let format = reader.format();
    let Some(format @ (ImageFormat::Jpeg | ImageFormat::Png)) = format else {
        return Err(crop_err(&format!("지원하지 않는 이미지 형식 {}", format_name(format))));
    };
    let (width, height) = reader.into_dimensions().map_err(|_| unreadable())?;
    if (width as usize).saturating_mul(height as usize) > limits.max_image_pixels {
        return Err(crop_err("이미지 해상도가 너무 큼"));
    }
    hooks.observe_decode();
    let decoded = image::ImageReader::with_format(Cursor::new(data), format).decode().map_err(|_| unreadable())?;
    let source = flatten(decoded);
    let cropped = crop_pil(&source, rect).map_err(|e| crop_err(&e.to_string()))?;
    let mut output = Vec::new();
    match format {
        ImageFormat::Jpeg => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 95);
            cropped.write_with_encoder(encoder).map_err(|_| unreadable())?;
        }
        _ => cropped.write_to(&mut Cursor::new(&mut output), ImageFormat::Png).map_err(|_| unreadable())?,
    }
    Ok((output, format))
}

fn format_parts(format: ImageFormat) -> (&'static str, &'static str) {
    if format == ImageFormat::Png { (".png", "image/png") } else { (".jpg", "image/jpeg") }
}

fn image_size(data: &[u8]) -> Result<(u32, u32), CoverError> {
    read_dimensions(data).map_err(|_| crop_err("이미지를 읽을 수 없음"))
}

/// `<svg><image>` 쪽의 이미지 크기를 크롭한 이미지 크기에 맞춘다(찌그러짐 방지).
///
/// image 의 width/height 가 원래 픽셀 크기와 같으면 새 크기로 바꾸고, svg 의 viewBox 가 그 크기를 감쌌으면 같이 바꾼다.
/// 구조가 예상과 다르거나 문서를 읽을 수 없으면 쪽을 건드리지 않는다.
fn fit_svg_page(page_xml: &[u8], old_size: (u32, u32), new_size: (u32, u32)) -> Vec<u8> {
    let Ok(mut root) = xml::parse(page_xml) else { return page_xml.to_vec() };
    fn find_image(element: &Element, path: &mut Vec<usize>) -> bool {
        if element.name == "image" {
            return true;
        }
        for (index, node) in element.children.iter().enumerate() {
            if let Node::Element(child) = node {
                path.push(index);
                if find_image(child, path) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }
    fn at_mut<'a>(root: &'a mut Element, path: &[usize]) -> &'a mut Element {
        let mut current = root;
        for index in path {
            current = match &mut current.children[*index] {
                Node::Element(child) => child,
                _ => unreachable!("경로는 요소만 가리킨다"),
            };
        }
        current
    }
    let mut path = Vec::new();
    if !find_image(&root, &mut path) {
        return page_xml.to_vec();
    }
    let ((old_w, old_h), (new_w, new_h)) = (old_size, new_size);
    let mut changed = false;
    {
        let image = at_mut(&mut root, &path);
        if image.attr("width") == Some(&old_w.to_string()) && image.attr("height") == Some(&old_h.to_string()) {
            image.set_attr("width", &new_w.to_string());
            image.set_attr("height", &new_h.to_string());
            changed = true;
        }
    }
    for depth in (0..path.len()).rev() {
        let svg = at_mut(&mut root, &path[..depth]);
        let wanted = ["0".to_string(), "0".to_string(), old_w.to_string(), old_h.to_string()];
        if svg.name == "svg" && svg.attr_or("viewBox", "").split_whitespace().map(str::to_string).collect::<Vec<_>>() == wanted {
            svg.set_attr("viewBox", &format!("0 0 {new_w} {new_h}"));
            changed = true;
            break;
        }
    }
    if changed { xml::to_bytes_with_declaration(&root) } else { page_xml.to_vec() }
}

fn reference_variants(member: &str) -> Vec<Vec<u8>> {
    let name = member.rsplit('/').next().unwrap_or(member);
    let mut variants = vec![name.as_bytes().to_vec()];
    let quoted = quote(name).into_bytes();
    if !variants.contains(&quoted) {
        variants.push(quoted);
    }
    variants
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// 편집 단계
// ---------------------------------------------------------------------------

fn idref_of(entry: &Value) -> Result<&str, CoverError> {
    entry.get("idref").and_then(Value::as_str).ok_or_else(|| err("편집 항목에 idref가 없습니다"))
}

/// 쪽 안 이미지 번호. 없으면 0이다. 정수가 아니거나 상한을 넘으면 거절한다.
fn index_of(entry: &Value) -> Result<i64, CoverError> {
    match entry.get("index") {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value.as_i64().filter(|i| *i <= MAX_IMAGE_INDEX).ok_or_else(|| err("이미지 위치가 올바르지 않습니다")),
    }
}

/// 쪽 멤버, 쪽 안 이미지 수, [(번호, 종류, 새 이미지 상대경로)].
type PageEdits = (String, usize, Vec<(usize, &'static str, Option<Value>)>);

struct Stager<'a, R: Read + Seek> {
    original: &'a mut ZipArchive<R>,
    hooks: &'a EpubCoverHooks,
    page_cache: BTreeMap<String, Vec<u8>>,
    cached_bytes: usize,
}

impl<R: Read + Seek> Stager<'_, R> {
    fn read_page(&mut self, member: &str) -> FResult<Vec<u8>> {
        if !self.page_cache.contains_key(member) {
            let small = has_entry(self.original, member) && entry_size(self.original, member)? <= (MAX_XML_BYTES as u64) * 4;
            let data = if small { read_entry(self.original, member)? } else { Vec::new() };
            self.cached_bytes += data.len();
            if self.cached_bytes > MAX_PAGE_CACHE_BYTES {
                return Err(err("EPUB이 검사 크기 제한을 초과했습니다").into());
            }
            self.page_cache.insert(member.to_string(), data);
        }
        Ok(self.page_cache[member].clone())
    }

    fn used_elsewhere(&mut self, image_member: &str, own_pages: &BTreeSet<String>, spine_members: &BTreeMap<String, String>, css_members: &[String], include_styles: bool) -> FResult<bool> {
        let variants = reference_variants(image_member);
        let mut members: BTreeSet<String> = spine_members.values().filter(|m| !m.is_empty() && !own_pages.contains(*m)).cloned().collect();
        if include_styles {
            members.extend(css_members.iter().cloned());
        }
        for member in members {
            if !has_entry(self.original, &member) {
                continue;
            }
            let data = self.read_page(&member)?;
            if variants.iter().any(|v| contains_bytes(&data, v)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn read_image(&mut self, member: &str) -> FResult<Vec<u8>> {
        if entry_size(self.original, member)? > self.hooks.limits().max_image_bytes as u64 {
            return Err(crop_err("이미지가 너무 큼").into());
        }
        read_entry(self.original, member)
    }
}

fn put(added: &mut Added, name: &str, data: Vec<u8>) -> FResult<()> {
    added.insert(name, data);
    if added.total_bytes() > MAX_EDIT_OUTPUT_BYTES {
        return Err(err("편집 결과가 너무 커서 저장할 수 없습니다").into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn stage_image_edits(
    source: &File,
    destination: &Path,
    crops: &[Value],
    splits: &[Value],
    deletes: &[Value],
    inserts: &[(String, Vec<u8>)],
    created: &mut Value,
    hooks: &EpubCoverHooks,
) -> FResult<()> {
    {
        let mut original = ZipArchive::new(source)?;
        let (opf_path, mut package) = read_package_with(&mut original, hooks)?;
        let base = dirname(&opf_path);
        let missing = || Fail::from(err("EPUB manifest 또는 spine이 없습니다"));
        // 읽기 전용 사본: id → href, spine idref, spine 멤버, 목차·스타일 멤버
        let (items, spine_ids, css_members, toc_members) = {
            let parts = sections(&mut package);
            let (Some(manifest), Some(spine)) = (parts.manifest.as_deref(), parts.spine.as_deref()) else { return Err(missing()) };
            let items: BTreeMap<String, String> = manifest.child_elements().filter_map(|i| i.attr("id").map(|id| (id.to_string(), i.attr_or("href", "").to_string()))).collect();
            let spine_ids: Vec<String> = spine.child_elements().filter_map(|r| r.attr("idref").map(str::to_string)).collect();
            let css: Vec<String> = manifest.child_elements().filter(|i| i.attr("media-type") == Some("text/css")).map(|i| member_path(&base, i.attr_or("href", ""))).collect();
            let toc: Vec<String> = manifest
                .child_elements()
                .filter(|i| i.attr_or("properties", "").split_whitespace().any(|p| p == "nav") || i.attr("media-type") == Some("application/x-dtbncx+xml"))
                .map(|i| member_path(&base, i.attr_or("href", "")))
                .collect();
            (items, spine_ids, css, toc)
        };
        let in_spine = |idref: &str| spine_ids.iter().any(|s| s == idref);
        let spine_members: BTreeMap<String, String> = spine_ids.iter().filter_map(|id| items.get(id).map(|href| (id.clone(), member_path(&base, href)))).collect();
        let original_names: BTreeSet<String> = entry_names(&original).into_iter().collect();
        let mut added = Added::default();
        let mut obsolete: BTreeSet<String> = BTreeSet::new();
        // 쪽 안에서 처리하는 분할·삭제: 쪽 멤버 → (이미지 수, [(index, 종류, 새 이미지 상대경로)]). 모두 모은 뒤 쪽마다 한 번에 적용한다.
        let mut page_edits: Vec<PageEdits> = Vec::new();
        let mut stager = Stager { original: &mut original, hooks, page_cache: BTreeMap::new(), cached_bytes: 0 };

        let mut add_page_edit = |info: &PageInfo, kind: &'static str, new_href: Option<String>| {
            let entry = match page_edits.iter().position(|(member, _, _)| *member == info.page_member) {
                Some(at) => at,
                None => {
                    page_edits.push((info.page_member.clone(), info.count, Vec::new()));
                    page_edits.len() - 1
                }
            };
            page_edits[entry].2.push((info.index, kind, new_href.map(Value::String)));
        };
        let info_for = |stager: &mut Stager<'_, &File>, entry: &Value| -> FResult<PageInfo> {
            let index = usize::try_from(index_of(entry)?).map_err(|_| err("이미지 위치가 올바르지 않습니다"))?;
            page_info(&mut *stager.original, &base, &items, idref_of(entry)?, index, hooks)
        };
        let refuse_shared = |stager: &mut Stager<'_, &File>, info: &PageInfo| -> FResult<()> {
            // 대상 이미지 멤버를 다른 쪽이나 같은 쪽의 다른 이미지 태그도 쓰면 제자리 교체가 그쪽도 바꾸므로 거절한다.
            let own: BTreeSet<String> = BTreeSet::from([info.page_member.clone()]);
            if stager.used_elsewhere(&info.image_member, &own, &spine_members, &css_members, false)? || info.image_refs.iter().filter(|r| **r == info.image_member).count() > 1 {
                return Err(crop_err("이미지를 다른 쪽과 공유함").into());
            }
            Ok(())
        };

        for entry in crops {
            let rect = parse_rect(entry.get("rect"))?;
            if idref_of(entry)? == COVER_TARGET {
                // 표지 화면의 이미지: 쪽(spine 항목)이 아니라 표지 이미지 멤버를 제자리에서 자른다.
                let previous = {
                    let manifest = package.find(super::OPF_NS, "manifest").ok_or_else(missing)?;
                    read_previous_cover(&mut *stager.original, &opf_path, manifest, hooks)?
                };
                let Some((cover_member, cover_data, _, _)) = previous else { return Err(crop_err("표지 이미지를 찾을 수 없음").into()) };
                let (data, _) = crop_image_bytes(&cover_data, rect, hooks)?;
                put(&mut added, &cover_member, data.clone())?;
                // 표지 쪽이 <svg><image> 로 이 이미지를 그리면 크기를 새 이미지에 맞춘다.
                let pages: BTreeSet<String> = spine_members.values().filter(|m| has_entry(stager.original, m)).cloned().collect();
                for page_member in pages {
                    let page = stager.read_page(&page_member)?;
                    if !page.is_empty() && reference_variants(&cover_member).iter().any(|v| contains_bytes(&page, v)) {
                        put(&mut added, &page_member, fit_svg_page(&page, image_size(&cover_data)?, image_size(&data)?))?;
                    }
                }
                continue;
            }
            let info = info_for(&mut stager, entry)?;
            refuse_shared(&mut stager, &info)?;
            let image_data = stager.read_image(&info.image_member)?;
            let (data, _) = crop_image_bytes(&image_data, rect, hooks)?;
            if info.svg_image && info.count == 1 {
                let page = stager.read_page(&info.page_member)?;
                put(&mut added, &info.page_member, fit_svg_page(&page, image_size(&image_data)?, image_size(&data)?))?;
            }
            put(&mut added, &info.image_member, data)?;
        }

        for entry in splits {
            let idref = idref_of(entry)?;
            if idref.starts_with(TOOL_COVER_PREFIX) {
                return Err(err("표지 쪽은 분할할 수 없습니다").into());
            }
            let (first_rect, second_rect) = (parse_rect(entry.get("first"))?, parse_rect(entry.get("second"))?);
            let info = info_for(&mut stager, entry)?;
            refuse_shared(&mut stager, &info)?;
            let image_data = stager.read_image(&info.image_member)?;
            let (first, format) = crop_image_bytes(&image_data, first_rect, hooks)?;
            let (second, _) = crop_image_bytes(&image_data, second_rect, hooks)?;
            let (suffix, media_type) = format_parts(format);
            put(&mut added, &info.image_member, first.clone())?;
            if info.page_level() {
                if !in_spine(idref) {
                    return Err(err("편집할 쪽을 찾을 수 없습니다. 책을 다시 열어 보세요").into());
                }
                if info.svg_image {
                    let page = stager.read_page(&info.page_member)?;
                    put(&mut added, &info.page_member, fit_svg_page(&page, image_size(&image_data)?, image_size(&first)?))?;
                }
                let parts = sections(&mut package);
                let (Some(manifest), Some(spine)) = (parts.manifest, parts.spine) else { return Err(missing()) };
                let new_idref = add_image_page(&opf_path, manifest, spine, &original_names, &mut added, &second, idref, "tm-split", suffix, media_type, true)?;
                if added.total_bytes() > MAX_EDIT_OUTPUT_BYTES {
                    return Err(err("편집 결과가 너무 커서 저장할 수 없습니다").into());
                }
                created["split"][idref] = json!(new_idref);
            } else {
                let parts = sections(&mut package);
                let Some(manifest) = parts.manifest else { return Err(missing()) };
                let new_member = add_split_image(&opf_path, manifest, &original_names, &mut added, &second, suffix, media_type)?;
                add_page_edit(&info, "split", Some(relpath(&new_member, &dirname(&info.page_member))));
            }
        }

        for (before_idref, image) in inserts {
            if !in_spine(before_idref) {
                return Err(err("삽화를 넣을 위치를 찾을 수 없습니다. 책을 다시 열어 보세요").into());
            }
            if before_idref.starts_with(TOOL_COVER_PREFIX) {
                return Err(err("표지 앞에는 삽화를 넣을 수 없습니다").into());
            }
            let parts = sections(&mut package);
            let (Some(manifest), Some(spine)) = (parts.manifest, parts.spine) else { return Err(missing()) };
            let new_idref = add_image_page(&opf_path, manifest, spine, &original_names, &mut added, image, before_idref, "tm-plate", ".jpg", "image/jpeg", false)?;
            if added.total_bytes() > MAX_EDIT_OUTPUT_BYTES {
                return Err(err("편집 결과가 너무 커서 저장할 수 없습니다").into());
            }
            created["inserted"].as_array_mut().expect("inserted는 배열이다").push(json!(new_idref));
        }

        if !deletes.is_empty() {
            let mut toc_bytes: Vec<Vec<u8>> = Vec::new();
            for member in &toc_members {
                if has_entry(stager.original, member) && entry_size(stager.original, member)? <= hooks.limits().max_xml_bytes as u64 {
                    toc_bytes.push(read_entry(stager.original, member)?);
                }
            }
            let mut infos: Vec<(String, PageInfo)> = Vec::new();
            for entry in deletes {
                let idref = idref_of(entry)?;
                if idref.starts_with(TOOL_COVER_PREFIX) {
                    return Err(err("표지 쪽은 삭제할 수 없습니다").into());
                }
                infos.push((idref.to_string(), info_for(&mut stager, entry)?));
            }
            let doomed_pages: BTreeSet<String> = infos.iter().filter(|(_, info)| info.page_level()).map(|(_, info)| info.page_member.clone()).collect();
            let mut page_deleted = false;
            for (idref, info) in &infos {
                let own_pages: BTreeSet<String>;
                if info.page_level() {
                    if reference_variants(&info.page_member).iter().any(|v| toc_bytes.iter().any(|data| contains_bytes(data, v))) {
                        return Err(err("목차가 가리키는 쪽은 삭제할 수 없습니다").into());
                    }
                    let parts = sections(&mut package);
                    let (Some(manifest), Some(spine)) = (parts.manifest, parts.spine) else { return Err(missing()) };
                    spine.remove_children_where(|r| r.attr("idref") == Some(idref.as_str()));
                    manifest.remove_children_where(|i| i.attr("id") == Some(idref.as_str()));
                    obsolete.insert(info.page_member.clone());
                    page_deleted = true;
                    own_pages = doomed_pages.clone();
                } else {
                    add_page_edit(info, "delete", None);
                    // 같은 쪽의 다른 이미지 태그가 같은 멤버를 쓰면 이미지 파일은 남긴다.
                    if info.image_refs.iter().filter(|r| **r == info.image_member).count() > 1 {
                        continue;
                    }
                    own_pages = BTreeSet::from([info.page_member.clone()]);
                }
                if !stager.used_elsewhere(&info.image_member, &own_pages, &spine_members, &css_members, true)? {
                    let parts = sections(&mut package);
                    let Some(manifest) = parts.manifest else { return Err(missing()) };
                    let image_item = manifest.child_elements().find(|i| member_path(&base, i.attr_or("href", "")) == info.image_member);
                    let keep = image_item.is_some_and(|i| i.attr_or("properties", "").split_whitespace().any(|p| p == "cover-image"));
                    if keep {
                        continue;
                    }
                    if image_item.is_some() {
                        manifest.remove_children_where(|i| member_path(&base, i.attr_or("href", "")) == info.image_member);
                    }
                    obsolete.insert(info.image_member.clone());
                }
            }
            if page_deleted && package.find(super::OPF_NS, "spine").is_none_or(|s| s.child_elements().next().is_none()) {
                return Err(err("모든 쪽을 삭제할 수는 없습니다").into());
            }
        }

        for (page_member, count, edits) in &page_edits {
            let current = match added.get(page_member) {
                Some(data) if !data.is_empty() => data.to_vec(),
                _ => stager.read_page_original(page_member)?,
            };
            let borrowed: Vec<(usize, &str, Option<Value>)> = edits.iter().map(|(i, k, h)| (*i, *k, h.clone())).collect();
            put(&mut added, page_member, edit_page_images(&current, *count, &borrowed)?)?;
        }
        write_epub(&mut *stager.original, destination, &opf_path, &package, &obsolete, &added)?;
    }
    verify_staged(destination, hooks)
}

impl<R: Read + Seek> Stager<'_, R> {
    /// 쪽 문서 원본(편집 전). 크기 상한을 넘으면 거절한다.
    fn read_page_original(&mut self, member: &str) -> FResult<Vec<u8>> {
        if entry_size(self.original, member)? > (MAX_XML_BYTES as u64) * 4 {
            return Err(crop_err("쪽 문서가 너무 큼").into());
        }
        read_entry(self.original, member)
    }
}

/// EPUB 쪽의 이미지를 한 번에 크롭·분할·삭제·삽입한다(원자적).
pub fn edit_images(path: &Path, expected_revision: &str, backup_dir: &Path, args: EditImages) -> Result<Value, CoverError> {
    edit_images_with(path, expected_revision, backup_dir, args, &EpubCoverHooks::default())
}

pub fn edit_images_with(path: &Path, expected_revision: &str, backup_dir: &Path, args: EditImages, hooks: &EpubCoverHooks) -> Result<Value, CoverError> {
    let EditImages { crops, splits, deletes, inserts, backup_name } = args;
    if crops.len() > MAX_EDITS || splits.len() > MAX_EDITS || deletes.len() > MAX_EDITS || inserts.len() > MAX_INSERTS {
        return Err(err("한 번에 저장할 수 있는 편집 항목 수를 넘었습니다"));
    }
    // deletes 는 [{"idref", "index"}] 이거나(구형) idref 문자열 목록이다.
    let deletes: Vec<Value> = deletes.into_iter().map(|entry| if let Value::String(idref) = entry { json!({"idref": idref}) } else { entry }).collect();
    let mut targets: Vec<(String, i64)> = Vec::new();
    for entry in crops.iter().chain(&splits).chain(&deletes) {
        if !entry.is_object() {
            return Err(err("편집 항목이 올바르지 않습니다"));
        }
        let idref = idref_of(entry)?;
        targets.push((idref.to_string(), if idref == COVER_TARGET { 0 } else { index_of(entry)? }));
    }
    if targets.is_empty() && inserts.is_empty() {
        return Err(err("저장할 이미지 편집이 없습니다"));
    }
    let unique: BTreeSet<&(String, i64)> = targets.iter().collect();
    if unique.len() != targets.len() {
        return Err(err("같은 쪽에 편집을 두 번 지정할 수 없습니다"));
    }
    if targets.iter().any(|(_, index)| *index < 0) {
        return Err(err("이미지 위치가 올바르지 않습니다"));
    }
    let mut normalized: Vec<(String, Vec<u8>)> = Vec::with_capacity(inserts.len());
    for (before, image) in &inserts {
        normalized.push((before.clone(), normalize_image_with(image, Some(1200), hooks)?));
    }
    let mut created = json!({"inserted": [], "split": {}});
    let mut result = rewrite_epub(
        path,
        expected_revision,
        backup_dir,
        backup_name.as_deref(),
        &mut |source, staged| stage_image_edits(source, staged, &crops, &splits, &deletes, &normalized, &mut created, hooks),
        None,
        hooks,
    )?;
    result["cover_image"] = Value::Null;
    result["idrefs"] = created;
    Ok(result)
}

#[cfg(test)]
#[path = "../../../tests/rust/backend/epub_cover_edit_unit.rs"]
mod tests;
