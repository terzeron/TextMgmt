//! 새 EPUB 파일을 임시 경로에 만드는 단계(OPF·XHTML 편집과 ZIP 쓰기). 원본은 건드리지 않는다.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use super::pathx::{dirname, ext, join, member_path};
use super::xml::{self, Element, Node, XHTML_NS};
use super::{entry_names, entry_size, find_cover_member_in, has_entry, inspect_archive, read_entry, read_package_with, CoverError, EpubCoverHooks, FResult, Fail, OPF_NS};

/// 추가·교체할 멤버. 넣은 순서를 지킨다(Python `dict`).
#[derive(Default, Debug)]
pub struct Added {
    items: Vec<(String, Vec<u8>)>,
}

impl Added {
    pub fn insert(&mut self, name: &str, data: Vec<u8>) {
        match self.items.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = data,
            None => self.items.push((name.to_string(), data)),
        }
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.items.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice())
    }

    pub fn names(&self) -> BTreeSet<String> {
        self.items.iter().map(|(n, _)| n.clone()).collect()
    }
}

/// `package`의 OPF 구역. 처음 나온 요소를 쓴다(`package.find`).
pub struct Sections<'a> {
    pub manifest: Option<&'a mut Element>,
    pub metadata: Option<&'a mut Element>,
    pub spine: Option<&'a mut Element>,
    pub guide: Option<&'a mut Element>,
}

pub fn sections(package: &mut Element) -> Sections<'_> {
    let mut found = Sections { manifest: None, metadata: None, spine: None, guide: None };
    for child in package.child_elements_mut() {
        if child.ns.as_deref() != Some(OPF_NS) {
            continue;
        }
        match child.name.as_str() {
            "manifest" if found.manifest.is_none() => found.manifest = Some(child),
            "metadata" if found.metadata.is_none() => found.metadata = Some(child),
            "spine" if found.spine.is_none() => found.spine = Some(child),
            "guide" if found.guide.is_none() => found.guide = Some(child),
            _ => {}
        }
    }
    found
}

fn new_item(manifest: &Element, attrs: &[(&str, &str)]) -> Element {
    let mut item = manifest.sibling("item");
    for (name, value) in attrs {
        item.set_attr(name, value);
    }
    item
}

pub fn has_token(value: &str, token: &str) -> bool {
    value.split_whitespace().any(|t| t == token)
}

fn digest16(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect::<String>()[..16].to_string()
}

static TOOL_COVER_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^tm-cover-[a-f0-9]{16}$").expect("정규식"));

/// 이 도구가 넣은 표지(`tm-cover-*`)의 manifest 항목과 spine 참조를 지우고, 파일에서 뺄 멤버 경로를 돌려준다.
///
/// 출판사 원본 표지는 건드리지 않는다. 그 구조는 책마다 달라 임의로 지우면 본문이 깨질 수 있다.
pub fn remove_tool_covers(opf_path: &str, manifest: &mut Element, metadata: &Element, spine: &mut Element) -> BTreeSet<String> {
    let mut obsolete = BTreeSet::new();
    let mut old_ids: BTreeSet<String> = metadata.child_elements().filter(|m| m.attr("name") == Some("cover")).map(|m| m.attr_or("content", "").to_string()).collect();
    old_ids.extend(manifest.child_elements().filter(|i| has_token(i.attr_or("properties", ""), "cover-image")).map(|i| i.attr_or("id", "").to_string()));
    for old_id in old_ids {
        if !TOOL_COVER_ID.is_match(&old_id) {
            continue;
        }
        let page_id = old_id.replacen("tm-cover-", "tm-cover-page-", 1);
        for (identifier, suffix) in [(old_id.clone(), ".jpg"), (page_id, ".xhtml")] {
            let href = format!("{identifier}{suffix}");
            let matches = |e: &Element| e.attr("id") == Some(identifier.as_str()) && e.attr("href") == Some(href.as_str());
            if manifest.child_elements().any(matches) {
                obsolete.insert(join(&dirname(opf_path), &href));
                manifest.remove_children_where(matches);
                spine.remove_children_where(|r| r.attr("idref") == Some(identifier.as_str()));
            }
        }
    }
    obsolete
}

/// 떼어 낸 멤버 경로와 고쳐 쓸 문서(이름, 내용).
pub type Detached = (BTreeSet<String>, Vec<(String, Vec<u8>)>);

/// 교체 전 표지: (멤버 경로, 바이트, media-type, 확장자).
pub type PreviousCover = (String, Vec<u8>, String, String);

/// manifest 항목의 읽기용 사본.
struct ItemInfo {
    id: Option<String>,
    href: String,
    media_type: Option<String>,
    properties: String,
}

impl ItemInfo {
    fn of(e: &Element) -> Self {
        ItemInfo {
            id: e.attr("id").map(str::to_string),
            href: e.attr_or("href", "").to_string(),
            media_type: e.attr("media-type").map(str::to_string),
            properties: e.attr_or("properties", "").to_string(),
        }
    }
}

struct Doc {
    name: String,
    item: ItemInfo,
    document: Option<Element>,
    data: Vec<u8>,
}

static CSS_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"url\(\s*['"]?([^\s)'"]+)"#).expect("정규식"));

/// 문서가 가리키는 멤버 경로(`references`). 모든 속성값과 CSS `url()`을 본다.
fn references(name: &str, document: Option<&Element>, data: &[u8]) -> BTreeSet<String> {
    let base = dirname(name);
    let mut hrefs: Vec<String> = Vec::new();
    if let Some(document) = document {
        for element in document.descendants() {
            hrefs.extend(element.attrs.iter().map(|a| a.value.clone()));
        }
    }
    let text = String::from_utf8_lossy(data);
    hrefs.extend(CSS_URL.captures_iter(&text).map(|c| c[1].to_string()));
    hrefs.iter().map(|href| member_path(&base, href)).collect()
}

fn local_is(e: &Element, name: &str) -> bool {
    e.name == name
}

fn normalize_play_order(document: &mut Element) -> FResult<()> {
    let mut values: BTreeSet<i64> = BTreeSet::new();
    for element in document.descendants() {
        if let Some(value) = element.attr("playOrder") {
            values.insert(value.trim().parse::<i64>().map_err(|e| Fail::Lib(e.to_string()))?);
        }
    }
    let numbering: Vec<i64> = values.into_iter().collect();
    document.for_each_mut(&mut |element| {
        if let Some(value) = element.attr("playOrder").and_then(|v| v.trim().parse::<i64>().ok()) {
            let index = numbering.iter().position(|v| *v == value).map_or(1, |p| p + 1);
            element.set_attr("playOrder", &index.to_string());
        }
    });
    Ok(())
}

const SHARED_PAGE: &str = "표지 페이지가 본문에서도 쓰여 자동으로 삭제할 수 없습니다";

/// NCX: 표지를 가리키는 `navPoint`를 지우고 하위 `navPoint`를 그 자리로 올린다.
fn clean_ncx(element: &mut Element, base: &str, detached: &BTreeSet<String>, changed: &mut bool) -> Result<(), CoverError> {
    let children = std::mem::take(&mut element.children);
    element.children = ncx_nodes(children, base, detached, changed)?;
    Ok(())
}

fn ncx_nodes(nodes: Vec<Node>, base: &str, detached: &BTreeSet<String>, changed: &mut bool) -> Result<Vec<Node>, CoverError> {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut iter = nodes.into_iter().peekable();
    while let Some(node) = iter.next() {
        let Node::Element(mut element) = node else {
            out.push(node);
            continue;
        };
        let points = element.child_elements().any(|c| local_is(c, "content") && detached.contains(&member_path(base, c.attr_or("src", ""))));
        if !points {
            clean_ncx(&mut element, base, detached, changed)?;
            out.push(Node::Element(element));
            continue;
        }
        if !local_is(&element, "navPoint") {
            return Err(CoverError::new("표지의 목차 참조를 안전하게 정리할 수 없습니다"));
        }
        *changed = true;
        // 하위 목차가 있으면 그 항목을 부모로 올려 본문 탐색 경로를 보존한다. 바로 뒤의 글(tail)도 함께 올린다.
        let mut promoted: Vec<Node> = Vec::new();
        let mut inner = std::mem::take(&mut element.children).into_iter().peekable();
        while let Some(inner_node) = inner.next() {
            if let Node::Element(sub) = inner_node
                && local_is(&sub, "navPoint")
            {
                promoted.push(Node::Element(sub));
                if let Some(Node::Text(_)) = inner.peek() {
                    promoted.extend(inner.next());
                }
            }
        }
        // 올린 항목도 같은 규칙으로 정리한다. 지우는 항목의 tail은 항목과 함께 사라진다.
        out.extend(ncx_nodes(promoted, base, detached, changed)?);
        if let Some(Node::Text(_)) = iter.peek() {
            iter.next();
        }
    }
    Ok(out)
}

/// nav 문서: 표지를 가리키는 링크를 지운다. 하위 목록이 없는 `li`는 통째로, 나머지는 `<span>`으로 바꾼다.
fn clean_nav(element: &mut Element, base: &str, detached: &BTreeSet<String>, changed: &mut bool) {
    let children = std::mem::take(&mut element.children);
    let mut out: Vec<Node> = Vec::with_capacity(children.len());
    let mut iter = children.into_iter().peekable();
    while let Some(node) = iter.next() {
        let Node::Element(mut child) = node else {
            out.push(node);
            continue;
        };
        let points = |e: &Element| local_is(e, "a") && detached.contains(&member_path(base, e.attr_or("href", "")));
        if child.child_elements().any(&points) {
            *changed = true;
            if local_is(&child, "li") && !child.child_elements().any(|c| local_is(c, "ol")) {
                if let Some(Node::Text(_)) = iter.peek() {
                    iter.next();
                }
                continue;
            }
            for link in child.child_elements_mut().filter(|c| points(c)) {
                rename_to_xhtml_span(link);
            }
        }
        clean_nav(&mut child, base, detached, changed);
        out.push(Node::Element(child));
    }
    element.children = out;
}

fn rename_to_xhtml_span(element: &mut Element) {
    element.name = "span".to_string();
    element.remove_attr("href");
    if element.ns.as_deref() != Some(XHTML_NS) {
        element.ns = Some(XHTML_NS.to_string());
        element.prefix = Some("ns0".to_string());
        element.decls.push((Some("ns0".to_string()), XHTML_NS.to_string()));
    }
}

/// 표지 전용 항목을 읽기 순서와 탐색에서 뺀다. 본문과 공유하는 파일은 보존한다.
///
/// 본문 페이지의 목차 링크(`<a href>`)가 빠지는 표지 페이지를 가리키면 링크만 걷어 낸 텍스트로 바꾼다.
/// 새 표지 페이지로 다시 잇지 않는다. 도구가 넣은 표지는 삭제·교체로 사라지므로 다시 끊어진다.
/// 링크 말고 다른 방식의 참조는 지울 수 없어 거부한다.
pub fn detach_publisher_cover<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    opf_path: &str,
    package: &mut Element,
    removing: bool,
    hooks: &EpubCoverHooks,
) -> FResult<Detached> {
    let empty = || Ok((BTreeSet::new(), Vec::new()));
    let Some(member) = find_cover_member_in(archive, hooks, &|_| None).filter(|m| !m.is_empty()) else { return empty() };
    let base = dirname(opf_path);
    let (items, cover_ids, first_id, cover_pages) = {
        let manifest = package.find(OPF_NS, "manifest").ok_or_else(|| Fail::lib("manifest 없음"))?;
        let metadata = package.find(OPF_NS, "metadata").ok_or_else(|| Fail::lib("metadata 없음"))?;
        let spine = package.find(OPF_NS, "spine").ok_or_else(|| Fail::lib("spine 없음"))?;
        let items: Vec<ItemInfo> = manifest.child_elements().map(ItemInfo::of).collect();
        let cover_ids: Vec<Option<String>> = metadata.child_elements().filter(|m| m.attr("name") == Some("cover")).map(|m| m.attr("content").map(str::to_string)).collect();
        // `spine[0]`은 첫 노드다. 주석이면 `get`이 None이다.
        let first_id = spine.children.iter().find(|n| !matches!(n, Node::Text(_))).and_then(|n| if let Node::Element(e) = n { e.attr("idref").map(str::to_string) } else { None });
        let cover_pages: Option<BTreeSet<String>> =
            package.find(OPF_NS, "guide").map(|g| g.child_elements().filter(|r| has_token(r.attr_or("type", ""), "cover")).map(|r| member_path(&base, r.attr_or("href", ""))).collect());
        (items, cover_ids, first_id, cover_pages)
    };
    if !items.iter().any(|i| member_path(&base, &i.href) == member) {
        return empty(); // 도구 표지는 remove_tool_covers가 이미 처리했다.
    }

    let mut documents: Vec<Doc> = Vec::new();
    for item in &items {
        let media = item.media_type.as_deref().unwrap_or("");
        if !matches!(media, "application/xhtml+xml" | "image/svg+xml" | "application/x-dtbncx+xml" | "text/css") {
            continue;
        }
        let name = member_path(&base, &item.href);
        if !has_entry(archive, &name) {
            continue;
        }
        if entry_size(archive, &name)? > hooks.limits().max_xml_bytes as u64 {
            return Err(CoverError::new("표지와 본문 참조를 검사할 파일이 너무 큽니다").into());
        }
        let data = read_entry(archive, &name)?;
        let document = if media == "text/css" { None } else { Some(xml::parse(&data)?) };
        let doc = Doc { name: name.clone(), item: ItemInfo { id: item.id.clone(), href: item.href.clone(), media_type: item.media_type.clone(), properties: item.properties.clone() }, document, data };
        match documents.iter_mut().find(|d| d.name == name) {
            Some(slot) => *slot = doc,
            None => documents.push(doc),
        }
    }

    let cover_pages = cover_pages.unwrap_or_default();
    let mut detached: BTreeSet<String> = BTreeSet::new();
    for doc in &documents {
        let Some(document) = doc.document.as_ref().filter(|_| doc.item.media_type.as_deref() == Some("application/xhtml+xml")) else { continue };
        let item_id = doc.item.id.as_deref();
        let is_cover_id = cover_ids.iter().any(|c| c.as_deref() == item_id) || first_id.as_deref() == item_id;
        let named_cover = format!("{} {}", doc.item.id.as_deref().unwrap_or(""), doc.item.href).to_lowercase().contains("cover");
        if !cover_pages.contains(&doc.name) && !is_cover_id && !named_cover {
            continue;
        }
        let all = document.descendants();
        let body = all.iter().copied().find(|e| local_is(e, "body")).unwrap_or(document);
        let page_dir = dirname(&doc.name);
        let image_refs: Vec<String> = body
            .descendants()
            .into_iter()
            .filter(|e| matches!(e.name.as_str(), "img" | "image"))
            .map(|e| [e.attr("src"), e.attr("href"), e.attr_ns(xml::XLINK_NS, "href")].into_iter().flatten().find(|v| !v.is_empty()).unwrap_or("").to_string())
            .collect();
        if !image_refs.is_empty() && image_refs.iter().all(|href| member_path(&page_dir, href) == member) && body.text_content().trim().is_empty() {
            detached.insert(doc.name.clone());
        }
    }

    let shared = documents.iter().filter(|d| !detached.contains(&d.name) && d.name != member).any(|d| references(&d.name, d.document.as_ref(), &d.data).contains(&member));
    if shared && removing {
        return Err(CoverError::new("표지 이미지가 본문에서도 쓰여 자동으로 삭제할 수 없습니다").into());
    }

    // 일반 본문의 목차 링크는 걷어 내고, 링크가 아닌 참조는 깨지므로 저장 전에 거부한다.
    let mut updates: Vec<(String, Vec<u8>)> = Vec::new();
    for doc in documents.iter_mut() {
        let is_toc = doc.item.media_type.as_deref() == Some("application/x-dtbncx+xml") || has_token(&doc.item.properties, "nav");
        if detached.contains(&doc.name) || is_toc {
            continue;
        }
        if detached.is_disjoint(&references(&doc.name, doc.document.as_ref(), &doc.data)) {
            continue;
        }
        let Some(document) = doc.document.as_mut() else { return Err(CoverError::new(SHARED_PAGE).into()) };
        let page_dir = dirname(&doc.name);
        let mut has_links = false;
        for element in document.descendants() {
            for attr in &element.attrs {
                if detached.contains(&member_path(&page_dir, &attr.value)) {
                    if element.name != "a" || attr.name != "href" || attr.ns.is_some() {
                        return Err(CoverError::new(SHARED_PAGE).into());
                    }
                    has_links = true;
                }
            }
        }
        if has_links {
            document.for_each_mut(&mut |element| {
                if element.name == "a" && element.attr("href").is_some_and(|href| detached.contains(&member_path(&page_dir, href))) {
                    element.name = "span".to_string();
                    element.remove_attr("href");
                }
            });
            updates.push((doc.name.clone(), xml::to_bytes_with_declaration(document)));
        }
    }

    {
        let mut parts = sections(package);
        let (Some(manifest), Some(spine), Some(metadata)) = (parts.manifest.take(), parts.spine.take(), parts.metadata.take()) else { return Err(Fail::lib("OPF 구역 없음")) };
        let removable = |i: &Element| {
            let path = member_path(&base, i.attr_or("href", ""));
            detached.contains(&path) || (path == member && !shared)
        };
        let removed_ids: Vec<Option<String>> = manifest.child_elements().filter(|i| removable(i)).map(|i| i.attr("id").map(str::to_string)).collect();
        manifest.remove_children_where(removable);
        spine.remove_children_where(|r| removed_ids.iter().any(|id| id.as_deref() == r.attr("idref")));
        metadata.remove_children_where(|m| m.attr("name") == Some("cover"));
        if let Some(guide) = parts.guide.take() {
            guide.remove_children_where(|r| has_token(r.attr_or("type", ""), "cover"));
        }
    }

    for doc in documents.iter_mut() {
        let is_ncx = doc.item.media_type.as_deref() == Some("application/x-dtbncx+xml");
        let is_nav = has_token(&doc.item.properties, "nav");
        let Some(document) = doc.document.as_mut() else { continue };
        if !is_ncx && !is_nav {
            continue;
        }
        let page_dir = dirname(&doc.name);
        let mut changed = false;
        if is_ncx {
            // 문서 요소 아래로 내려가며 정리한다. 루트 자신은 목차 항목이 아니다.
            clean_ncx(document, &page_dir, &detached, &mut changed)?;
        } else {
            clean_nav(document, &page_dir, &detached, &mut changed);
        }
        if changed {
            normalize_play_order(document)?;
            updates.retain(|(n, _)| n != &doc.name);
            updates.push((doc.name.clone(), xml::to_bytes_with_declaration(document)));
        }
    }
    let mut obsolete = detached;
    if !shared {
        obsolete.insert(member);
    }
    Ok((obsolete, updates))
}

fn now_zip_time() -> DateTime {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    DateTime::from_date_and_time(now.year().clamp(1980, 2107) as u16, now.month() as u8, now.day() as u8, now.hour() as u8, now.minute() as u8, now.second().min(59) as u8).unwrap_or_default()
}

fn new_member_options(method: CompressionMethod) -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(method).last_modified_time(now_zip_time())
}

/// OPF와 `added`의 멤버를 갱신하고, `obsolete`를 제외한 나머지 멤버는 그대로 복사한다.
pub fn write_epub<R: Read + Seek>(original: &mut ZipArchive<R>, destination: &Path, opf_path: &str, package: &Element, obsolete: &BTreeSet<String>, added: &Added) -> FResult<()> {
    let output = OpenOptions::new().write(true).create(true).truncate(true).open(destination)?;
    let mut writer = ZipWriter::new(output);
    writer.start_file("mimetype", new_member_options(CompressionMethod::Stored))?;
    writer.write_all(b"application/epub+zip")?;
    writer.set_raw_comment(original.comment().to_vec().into_boxed_slice())?;
    let opf_bytes = xml::to_bytes_with_declaration(package);
    for index in 0..original.len() {
        let name = match original.name_for_index(index) {
            Some(Ok(name)) => name.into_owned(),
            _ => return Err(Fail::lib("ZIP 항목 이름을 읽을 수 없습니다")),
        };
        if name == "mimetype" || obsolete.contains(&name) {
            continue;
        }
        let replacement: Option<&[u8]> = if name == opf_path { Some(&opf_bytes) } else { added.get(&name) };
        let entry = original.by_index_raw(index)?;
        match replacement {
            Some(data) => {
                let method = match entry.compression() {
                    method @ (CompressionMethod::Stored | CompressionMethod::Deflated) => method,
                    _ => CompressionMethod::Deflated,
                };
                let mut options = SimpleFileOptions::default().compression_method(method).last_modified_time(entry.last_modified().unwrap_or_default());
                if let Some(mode) = entry.unix_mode() {
                    options = options.unix_permissions(mode);
                }
                drop(entry);
                writer.start_file(&name, options)?;
                writer.write_all(data)?;
            }
            None => writer.raw_copy_file(entry)?,
        }
    }
    let originals: BTreeSet<String> = entry_names(original).into_iter().collect();
    for (member, data) in &added.items {
        if !originals.contains(member) || obsolete.contains(member) {
            writer.start_file(member, new_member_options(CompressionMethod::Deflated))?;
            writer.write_all(data)?;
        }
    }
    writer.finish()?;
    Ok(())
}

/// 교체 전 표지 이미지를 (멤버 경로, 바이트, media-type, 확장자)로 읽는다. 도판으로 남길 수 없으면 `None`.
///
/// manifest 항목을 지우기 전에 불러야 한다. 이미지가 아니거나 너무 크면 남기지 않는다.
pub fn read_previous_cover<R: Read + Seek>(archive: &mut ZipArchive<R>, opf_path: &str, manifest: &Element, hooks: &EpubCoverHooks) -> FResult<Option<PreviousCover>> {
    let Some(member) = find_cover_member_in(archive, hooks, &|_| None).filter(|m| has_entry(archive, m)) else { return Ok(None) };
    if entry_size(archive, &member)? > hooks.limits().max_image_bytes as u64 {
        return Ok(None);
    }
    let base = dirname(opf_path);
    let media_type = manifest.child_elements().find(|e| member_path(&base, e.attr_or("href", "")) == member).map(|e| e.attr_or("media-type", "").to_string()).unwrap_or_default();
    if !media_type.starts_with("image/") {
        return Ok(None);
    }
    let suffix = ext(&member).to_lowercase();
    static SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\.[a-z0-9]{1,5}$").expect("정규식"));
    let suffix = if SUFFIX.is_match(&suffix) { suffix } else { ".jpg".to_string() };
    let data = read_entry(archive, &member)?;
    Ok(Some((member, data, media_type, suffix)))
}

fn xhtml_page(title: &str, alt: &str, href: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{title}</title></head><body style=\"margin:0;text-align:center\"><div><img src=\"{href}\" alt=\"{alt}\" style=\"max-width:100%;max-height:100vh\"/></div></body></html>"
    )
    .into_bytes()
}

/// 이미지 한 장짜리 도판 페이지.
pub fn illustration_page(image_href: &str) -> Vec<u8> {
    xhtml_page("도판", "도판", image_href)
}

/// 새 파일을 `fsync`하고 ZIP 무결성을 확인한다.
fn sync_staged(destination: &Path, hooks: &EpubCoverHooks) -> FResult<()> {
    let output = File::open(destination)?;
    hooks.sync(&output)?;
    Ok(())
}

fn testzip_clean(destination: &Path, hooks: &EpubCoverHooks) -> FResult<bool> {
    Ok(hooks.testzip(destination)?.is_none())
}

pub fn stage_cover(source: &File, destination: &Path, image: &[u8], hooks: &EpubCoverHooks) -> FResult<()> {
    {
        let mut original = ZipArchive::new(source)?;
        let (opf_path, mut package) = read_package_with(&mut original, hooks)?;
        let base = dirname(&opf_path);
        let missing = || Fail::from(CoverError::new("EPUB manifest, metadata 또는 spine이 없습니다"));
        let version3 = package.attr_or("version", "").starts_with('3');
        let previous = {
            let manifest = package.find(OPF_NS, "manifest").ok_or_else(missing)?;
            read_previous_cover(&mut original, &opf_path, manifest, hooks)?
        };
        let mut obsolete = {
            let parts = sections(&mut package);
            let (Some(manifest), Some(metadata), Some(spine)) = (parts.manifest, parts.metadata, parts.spine) else { return Err(missing()) };
            remove_tool_covers(&opf_path, manifest, metadata, spine)
        };
        let (detached, updates) = detach_publisher_cover(&mut original, &opf_path, &mut package, false, hooks)?;
        obsolete.extend(detached);
        let digest = digest16(image);
        let (cover_id, page_id) = (format!("tm-cover-{digest}"), format!("tm-cover-page-{digest}"));
        let (cover_href, page_href) = (format!("{cover_id}.jpg"), format!("{page_id}.xhtml"));
        let (cover_member, page_member) = (join(&base, &cover_href), join(&base, &page_href));
        let existing_names: BTreeSet<String> = entry_names(&original).into_iter().filter(|n| !obsolete.contains(n)).collect();
        let parts = sections(&mut package);
        let (Some(manifest), Some(metadata), Some(spine)) = (parts.manifest, parts.metadata, parts.spine) else { return Err(missing()) };
        let existing_ids: BTreeSet<String> = manifest.child_elements().filter_map(|i| i.attr("id").map(str::to_string)).collect();
        if existing_ids.contains(&cover_id) || existing_ids.contains(&page_id) {
            return Err(CoverError::new("추가할 표지 ID가 이미 존재합니다").into());
        }
        if existing_names.contains(&cover_member) || existing_names.contains(&page_member) {
            return Err(CoverError::new("추가할 표지 경로가 이미 존재합니다").into());
        }
        for item in manifest.child_elements_mut() {
            let properties = item.attr_or("properties", "").to_string();
            if has_token(&properties, "cover-image") {
                let rest: Vec<&str> = properties.split_whitespace().filter(|p| *p != "cover-image").collect();
                if rest.is_empty() {
                    item.remove_attr("properties");
                } else {
                    item.set_attr("properties", &rest.join(" "));
                }
            }
        }
        metadata.remove_children_where(|m| m.is(OPF_NS, "meta") && m.attr("name") == Some("cover"));
        metadata.push_child(metadata.sibling("meta").with_attr("name", "cover").with_attr("content", &cover_id));
        let mut cover_item = new_item(manifest, &[("id", &cover_id), ("href", &cover_href), ("media-type", "image/jpeg")]);
        if version3 {
            cover_item.set_attr("properties", "cover-image");
        }
        manifest.push_child(cover_item);
        manifest.push_child(new_item(manifest, &[("id", &page_id), ("href", &page_href), ("media-type", "application/xhtml+xml")]));
        spine.insert_child(0, spine.sibling("itemref").with_attr("idref", &page_id));
        let mut added = Added::default();
        for (name, data) in updates {
            added.insert(&name, data);
        }
        added.insert(&cover_member, image.to_vec());
        added.insert(&page_member, xhtml_page("표지", "표지", &cover_href));
        // 교체로 빠지는 기존 표지는 새 표지 바로 뒤의 도판으로 남긴다. 본문에서도 쓰여 파일이 그대로 남는 표지는 이미 본문에 있다.
        // 이름에 cover를 넣지 않는다. 표지 탐색이 id·href의 cover로 이미지를 고르므로 표지로 오인된다.
        if let Some((_, previous_data, media_type, suffix)) = previous.filter(|p| obsolete.contains(&p.0)) {
            let plate_digest = digest16(&previous_data);
            let (plate_id, plate_page_id) = (format!("tm-plate-{plate_digest}"), format!("tm-plate-page-{plate_digest}"));
            let (plate_href, plate_page_href) = (format!("{plate_id}{suffix}"), format!("{plate_page_id}.xhtml"));
            let (plate_member, plate_page_member) = (join(&base, &plate_href), join(&base, &plate_page_href));
            let mut taken: BTreeSet<String> = existing_ids;
            taken.extend(existing_names);
            taken.extend([cover_id.clone(), page_id.clone(), cover_member.clone(), page_member.clone()]);
            if plate_digest != digest && ![&plate_id, &plate_page_id, &plate_member, &plate_page_member].iter().any(|c| taken.contains(*c)) {
                manifest.push_child(new_item(manifest, &[("id", &plate_id), ("href", &plate_href), ("media-type", &media_type)]));
                manifest.push_child(new_item(manifest, &[("id", &plate_page_id), ("href", &plate_page_href), ("media-type", "application/xhtml+xml")]));
                spine.insert_child(1, spine.sibling("itemref").with_attr("idref", &plate_page_id));
                added.insert(&plate_member, previous_data);
                added.insert(&plate_page_member, illustration_page(&plate_href));
            }
        }
        write_epub(&mut original, destination, &opf_path, &package, &obsolete, &added)?;
    }
    sync_staged(destination, hooks)?;
    if !testzip_clean(destination, hooks)? || !staged_has_cover(destination, hooks)? {
        return Err(CoverError::new("저장할 EPUB의 표지 검증에 실패했습니다").into());
    }
    Ok(())
}

fn staged_has_cover(destination: &Path, hooks: &EpubCoverHooks) -> FResult<bool> {
    let mut archive = ZipArchive::new(File::open(destination)?)?;
    Ok(inspect_archive(&mut archive, hooks)?["has_cover"].as_bool().unwrap_or(false))
}

/// 이미지 한 장짜리 쪽을 anchor(spine itemref) 앞(`after`면 뒤)에 끼우고 새 쪽의 idref를 돌려준다.
#[allow(clippy::too_many_arguments)]
pub fn add_image_page(opf_path: &str, manifest: &mut Element, spine: &mut Element, original_names: &BTreeSet<String>, added: &mut Added, image: &[u8], anchor_idref: &str, prefix: &str, suffix: &str, media_type: &str, after: bool) -> FResult<String> {
    let digest = digest16(image);
    let existing_ids: BTreeSet<String> = manifest.child_elements().filter_map(|i| i.attr("id").map(str::to_string)).collect();
    let mut existing_names = original_names.clone();
    existing_names.extend(added.names());
    let base = dirname(opf_path);
    // 같은 이미지를 여러 곳에 넣을 수 있도록 겹치면 번호를 붙인다.
    let mut chosen: Option<(String, String, String, String, String, String)> = None;
    for serial in 1..1000 {
        let token = if serial == 1 { digest.clone() } else { format!("{digest}-{serial}") };
        let (image_id, page_id) = (format!("{prefix}-{token}"), format!("{prefix}-page-{token}"));
        let (image_href, page_href) = (format!("{image_id}{suffix}"), format!("{page_id}.xhtml"));
        let (image_member, page_member) = (join(&base, &image_href), join(&base, &page_href));
        if !existing_ids.contains(&image_id) && !existing_ids.contains(&page_id) && !existing_names.contains(&image_member) && !existing_names.contains(&page_member) {
            chosen = Some((image_id, page_id, image_href, page_href, image_member, page_member));
            break;
        }
    }
    let Some((image_id, page_id, image_href, page_href, image_member, page_member)) = chosen else { return Err(CoverError::new("추가할 삽화 ID를 만들 수 없습니다").into()) };
    manifest.push_child(new_item(manifest, &[("id", &image_id), ("href", &image_href), ("media-type", media_type)]));
    manifest.push_child(new_item(manifest, &[("id", &page_id), ("href", &page_href), ("media-type", "application/xhtml+xml")]));
    let position = spine.element_position(|e| e.attr("idref") == Some(anchor_idref)).ok_or_else(|| Fail::lib("anchor 없음"))?;
    spine.insert_child(position + usize::from(after), spine.sibling("itemref").with_attr("idref", &page_id));
    added.insert(&image_member, image.to_vec());
    added.insert(&page_member, illustration_page(&image_href));
    Ok(page_id)
}

pub fn verify_staged(destination: &Path, hooks: &EpubCoverHooks) -> FResult<()> {
    sync_staged(destination, hooks)?;
    if !testzip_clean(destination, hooks)? {
        return Err(CoverError::new("저장할 EPUB의 검증에 실패했습니다").into());
    }
    let mut archive = ZipArchive::new(File::open(destination)?)?;
    read_package_with(&mut archive, hooks)?;
    Ok(())
}

/// `before_idref` 항목 바로 앞에 이미지 한 장짜리 도판 페이지를 끼운다. 새 페이지의 idref를 돌려준다.
pub fn stage_illustration(source: &File, destination: &Path, image: &[u8], before_idref: &str, hooks: &EpubCoverHooks) -> FResult<String> {
    let created = {
        let mut original = ZipArchive::new(source)?;
        let (opf_path, mut package) = read_package_with(&mut original, hooks)?;
        let parts = sections(&mut package);
        let (Some(manifest), Some(spine)) = (parts.manifest, parts.spine) else { return Err(CoverError::new("EPUB manifest 또는 spine이 없습니다").into()) };
        if spine.element_position(|e| e.attr("idref") == Some(before_idref)).is_none() {
            return Err(CoverError::new("삽화를 넣을 위치를 찾을 수 없습니다. 책을 다시 열어 보세요").into());
        }
        if before_idref.starts_with("tm-cover-page-") {
            return Err(CoverError::new("표지 앞에는 삽화를 넣을 수 없습니다").into());
        }
        let mut added = Added::default();
        let names: BTreeSet<String> = entry_names(&original).into_iter().collect();
        let created = add_image_page(&opf_path, manifest, spine, &names, &mut added, image, before_idref, "tm-plate", ".jpg", "image/jpeg", false)?;
        write_epub(&mut original, destination, &opf_path, &package, &BTreeSet::new(), &added)?;
        created
    };
    verify_staged(destination, hooks)?;
    Ok(created)
}

/// 도구가 넣은 표지와 출판사 표지를 EPUB에서 뺀다. 표지를 지워도 책 자체는 읽혀야 한다.
pub fn stage_cover_removal(source: &File, destination: &Path, hooks: &EpubCoverHooks) -> FResult<()> {
    {
        let mut original = ZipArchive::new(source)?;
        let (opf_path, mut package) = read_package_with(&mut original, hooks)?;
        let missing = || Fail::from(CoverError::new("EPUB manifest, metadata 또는 spine이 없습니다"));
        let mut obsolete = {
            let parts = sections(&mut package);
            let (Some(manifest), Some(metadata), Some(spine)) = (parts.manifest, parts.metadata, parts.spine) else { return Err(missing()) };
            remove_tool_covers(&opf_path, manifest, metadata, spine)
        };
        let (detached, updates) = detach_publisher_cover(&mut original, &opf_path, &mut package, true, hooks)?;
        obsolete.extend(detached);
        {
            let parts = sections(&mut package);
            let Some(metadata) = parts.metadata else { return Err(missing()) };
            metadata.remove_children_where(|m| m.is(OPF_NS, "meta") && m.attr("name") == Some("cover") && TOOL_COVER_ID.is_match(m.attr_or("content", "")));
        }
        let mut added = Added::default();
        for (name, data) in updates {
            added.insert(&name, data);
        }
        write_epub(&mut original, destination, &opf_path, &package, &obsolete, &added)?;
    }
    sync_staged(destination, hooks)?;
    if !testzip_clean(destination, hooks)? {
        return Err(CoverError::new("저장할 EPUB의 검증에 실패했습니다").into());
    }
    let mut archive = ZipArchive::new(File::open(destination)?)?;
    read_package_with(&mut archive, hooks)?;
    if inspect_archive(&mut archive, hooks)?["has_cover"].as_bool().unwrap_or(false) {
        return Err(CoverError::new("표지를 삭제한 EPUB의 검증에 실패했습니다").into());
    }
    Ok(())
}
