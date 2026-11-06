//! 미리보기가 쓰는 EPUB 패키지 도구: OPF 찾기, 원문 구간 편집, 챕터 수, 이미지 축소, 미리보기 EPUB 검증, 캐시 정리.
//! 파일 하나를 다루는 형식 수준 도구라 `BookManager` 상태를 쓰지 않는다.
//!
//! XML은 `roxmltree`로 읽는다. 깨진 XML을 복구하지는 않는다. 대신 선언 없이 쓰인 접두사는
//! `common::xml_parser::declare_unbound_prefixes`로 먼저 선언한다. 문서를 고쳐 쓸 때는 DOM을 직렬화하지 않고 원문에서 해당 구간만 잘라내
//! (`Edit`) 원문 서식을 보존한다.

use common::xml_parser::parse_xml;
use common::zip_reader::{self, MemberError};
use std::io::{Read, Seek, Write};
use std::ops::Range;
use std::path::Path;
use std::time::{Duration, SystemTime};

use common::xml_parser::declare_unbound_prefixes;
use image::{ImageFormat, imageops::FilterType};
use regex::Regex;
use roxmltree::{Document, Node};
use sysdeps::{FsOps, RealFs};
use zip::ZipArchive;
use zip::result::ZipError;

use crate::path_validator::{clean_href, zip_path};
use common::url_parser::unquote;
use common::zip_reader::zip_names;

pub const OPF_NS: &str = "http://www.idpf.org/2007/opf";
pub const CONTAINER_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
pub const NCX_NS: &str = "http://www.daisy.org/z3986/2005/ncx/";

/// 이미지 축소 때 JPEG 품질.
const EPUB_IMAGE_JPEG_QUALITY: u8 = 80;

/// XML 바이트를 문자열로 바꾼다(UTF-8 BOM 제거, 잘못된 바이트는 대체 문자).
pub fn xml_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_string()
}

/// 네임스페이스와 이름으로 가장 먼저 나오는 자손 요소.
pub fn find_el<'a, 'i>(doc: &'a Document<'i>, ns: &str, local: &str) -> Option<Node<'a, 'i>> {
    doc.descendants().find(|n| n.is_element() && n.tag_name().namespace() == Some(ns) && n.tag_name().name() == local)
}

/// 직계 자식 요소 중 네임스페이스와 이름이 맞는 것.
pub fn children_named<'a, 'i>(node: Node<'a, 'i>, ns: &'static str, local: &'static str) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(move |n| n.is_element() && n.tag_name().namespace() == Some(ns) && n.tag_name().name() == local)
}

/// 원문 구간 하나를 바꾸거나(빈 문자열이면 지운다) 지우는 편집.
pub struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
}

impl Edit {
    pub fn remove(range: Range<usize>) -> Self {
        Self { range, replacement: String::new() }
    }
}

/// `root` 구간만 잘라 편집을 적용한다. 겹치거나 같은 구간은 한 번만 적용한다.
pub fn apply_edits(text: &str, root: Range<usize>, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    let mut out = String::new();
    let mut pos = root.start;
    for edit in edits {
        if edit.range.start < pos || edit.range.end > root.end {
            continue;
        }
        out.push_str(&text[pos..edit.range.start]);
        out.push_str(&edit.replacement);
        pos = edit.range.end;
    }
    out.push_str(&text[pos..root.end]);
    out
}

/// 새로 쓰는 OPF·NCX 앞에 붙이는 XML 선언.
pub const XML_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

/// NCX에서 ZIP에 없는 문서를 가리키는 navPoint를 지운다. 자식 navPoint는 부모 자리로 올린다.
/// 바뀐 것이 없거나 파싱할 수 없으면 `None`.
pub fn prune_dead_ncx_navpoints(ncx_bytes: &[u8], ncx_dir: &str, exists: &dyn Fn(&str) -> bool) -> Option<Vec<u8>> {
    let text = xml_text(ncx_bytes);
    let doc = parse_xml(&text).ok()?;
    let is_nav = |n: &Node| n.is_element() && n.tag_name().namespace() == Some(NCX_NS) && n.tag_name().name() == "navPoint";
    let is_dead = |nav: &Node| {
        let content = children_named(*nav, NCX_NS, "content").next();
        let src = content.and_then(|c| c.attribute("src")).map(clean_href).unwrap_or_default();
        !src.is_empty() && !exists(&zip_path(ncx_dir, &src))
    };
    let mut changed = false;

    fn emit(node: Node, text: &str, out: &mut String, is_nav: &dyn Fn(&Node) -> bool, is_dead: &dyn Fn(&Node) -> bool, changed: &mut bool) {
        let range = node.range();
        let mut pos = range.start;
        for child in node.children().filter(Node::is_element) {
            out.push_str(&text[pos..child.range().start]);
            if is_nav(&child) {
                emit_nav(child, text, out, is_nav, is_dead, changed);
            } else {
                emit(child, text, out, is_nav, is_dead, changed);
            }
            pos = child.range().end;
        }
        out.push_str(&text[pos..range.end]);
    }

    fn emit_nav(nav: Node, text: &str, out: &mut String, is_nav: &dyn Fn(&Node) -> bool, is_dead: &dyn Fn(&Node) -> bool, changed: &mut bool) {
        if is_dead(&nav) {
            *changed = true;
            for child in nav.children().filter(|c| is_nav(c)) {
                emit_nav(child, text, out, is_nav, is_dead, changed);
            }
        } else {
            emit(nav, text, out, is_nav, is_dead, changed);
        }
    }

    let root = doc.root_element();
    let mut body = String::new();
    emit(root, &text, &mut body, &is_nav, &is_dead, &mut changed);
    changed.then(|| format!("<?xml version='1.0' encoding='utf-8'?>\n{body}").into_bytes())
}

/// EPUB 멤버 하나를 풀 때의 상한. 헤더의 선언 크기는 믿지 않는다(zip bomb).
pub const MAX_MEMBER_BYTES: u64 = 64 * 1024 * 1024;

/// ZIP 멤버를 읽는다. 이름이 정확히 없으면 퍼센트 인코딩을 푼 이름이 같은 멤버를 찾는다.
pub fn read_member<R: Read + Seek>(zin: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>, ZipError> {
    read_member_limited(zin, name, MAX_MEMBER_BYTES)
}

/// `read_member`와 같고, 선언 크기나 실제로 푼 크기가 `limit`를 넘으면 오류다.
fn read_member_limited<R: Read + Seek>(zin: &mut ZipArchive<R>, name: &str, limit: u64) -> Result<Vec<u8>, ZipError> {
    let actual = if zin.index_for_name(name).is_some() { name.to_string() } else { zip_names(zin).into_iter().find(|n| unquote(n) == name).unwrap_or_else(|| name.to_string()) };
    zip_reader::read_member(zin, &actual, limit).map_err(|error| match error {
        MemberError::Open(error) => error,
        MemberError::Io(error) => ZipError::Io(error),
        MemberError::TooLarge => ZipError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{actual}: 압축을 푼 크기가 상한 {limit}바이트를 넘습니다"))),
    })
}

/// ZIP 안 OPF 경로를 찾는다. container.xml, 정규식 폴백, `.opf` 직접 탐색 순으로 시도한다.
pub fn find_opf_path<R: Read + Seek>(zin: &mut ZipArchive<R>) -> Option<String> {
    if let Ok(container_xml) = read_member(zin, "META-INF/container.xml") {
        let text = xml_text(&container_xml);
        if let Ok(doc) = parse_xml(&text)
            && let Some(rootfile) = find_el(&doc, CONTAINER_NS, "rootfile")
            && let Some(path) = rootfile.attribute("full-path").filter(|p| !p.is_empty())
        {
            return Some(path.to_string());
        }
        // XML 파싱 실패 시 정규식 폴백
        let re = Regex::new(r#"full-path=["']([^"']+)"#).ok()?;
        if let Some(m) = re.captures(&text) {
            return Some(m[1].to_string());
        }
    }
    // container.xml이 없거나 파싱 실패 시 ZIP 안 .opf 파일을 직접 찾는다.
    zip_names(zin).into_iter().find(|n| n.to_lowercase().ends_with(".opf"))
}

/// EPUB 총 챕터 수. manifest에 없거나 ZIP에 없는 spine 문서는 뷰어가 보여 줄 수 없어 세지 않는다.
pub fn get_epub_total_chapters(file_path: &Path) -> i64 {
    let count = || -> Result<i64, String> {
        let file = std::fs::File::open(file_path).map_err(|e| e.to_string())?;
        let mut zin = ZipArchive::new(file).map_err(|e| e.to_string())?;
        let Some(opf_path) = find_opf_path(&mut zin) else { return Ok(0) };
        let opf_bytes = read_member(&mut zin, &opf_path).map_err(|e| e.to_string())?;
        let text = declare_unbound_prefixes(&xml_text(&opf_bytes));
        let doc = parse_xml(&text).map_err(|e| e.to_string())?;
        let Some(spine) = find_el(&doc, OPF_NS, "spine") else { return Ok(0) };
        let manifest: std::collections::HashMap<&str, &str> = doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().namespace() == Some(OPF_NS) && n.tag_name().name() == "item")
            .filter_map(|n| Some((n.attribute("id")?, n.attribute("href").unwrap_or(""))))
            .collect();
        let base = Path::new(&opf_path).parent().and_then(Path::to_str).unwrap_or_default().to_owned();
        let names = zip_names(&mut zin);
        let mut count = 0;
        for itemref in children_named(spine, OPF_NS, "itemref") {
            let href = clean_href(itemref.attribute("idref").and_then(|id| manifest.get(id)).copied().unwrap_or(""));
            if href.is_empty() {
                continue;
            }
            let member = zip_path(&base, &href);
            // ZIP 항목 헤더가 손상되지 않았는지 본다(로컬 헤더만 읽는 가벼운 검사).
            let actual = if names.contains(&member) { Some(member.clone()) } else { names.iter().find(|n| unquote(n) == member).cloned() };
            if let Some(actual) = actual
                && zin.by_name(&actual).is_ok()
            {
                count += 1;
            }
        }
        Ok(count)
    };
    count().unwrap_or_else(|e| {
        log::error!("Failed to get EPUB total chapters for '{}': {e}", file_path.display());
        0
    })
}

/// EPUB 뷰어용으로 큰 이미지를 줄여 재압축한다. 실패하면 원본 바이트를 돌려준다. 형식은 유지한다(JPEG는 JPEG, PNG는 PNG).
pub fn downscale_epub_image(data: &[u8]) -> Vec<u8> {
    try_downscale_epub_image(data).unwrap_or_else(|| data.to_vec())
}

fn try_downscale_epub_image(data: &[u8]) -> Option<Vec<u8>> {
    let format = image::guess_format(data).ok().filter(|f| matches!(f, ImageFormat::Jpeg | ImageFormat::Png))?;
    let img = image::load_from_memory_with_format(data, format).ok()?;
    let (width, height) = (img.width(), img.height());
    let max_edge = width.max(height);
    if max_edge <= crate::EPUB_IMAGE_MAX_EDGE {
        return None;
    }
    let scale = f64::from(crate::EPUB_IMAGE_MAX_EDGE) / f64::from(max_edge);
    let resized = img.resize_exact(((f64::from(width) * scale).round() as u32).max(1), ((f64::from(height) * scale).round() as u32).max(1), FilterType::Lanczos3);
    let mut out = Vec::new();
    match format {
        ImageFormat::Png => {
            let encoder = image::codecs::png::PngEncoder::new_with_quality(&mut out, image::codecs::png::CompressionType::Best, image::codecs::png::FilterType::Adaptive);
            resized.write_with_encoder(encoder).ok()?;
        }
        _ => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, EPUB_IMAGE_JPEG_QUALITY);
            image::DynamicImage::ImageRgb8(resized.to_rgb8()).write_with_encoder(encoder).ok()?;
        }
    }
    (out.len() < data.len()).then_some(out)
}

/// 생성된 미리보기 EPUB의 구조적 유효성을 검증하고 경미한 문제는 자동으로 고친다.
pub fn validate_preview_epub(cache_file: &Path) -> Result<(), String> {
    check_preview_epub(&RealFs, cache_file)
}

pub fn validate_preview_epub_with(fs: &dyn FsOps, cache_file: &Path) -> Result<(), String> {
    check_preview_epub(fs, cache_file)
}

/// 1일 넘은 캐시 파일을 지운다.
pub fn evict_old_cache(cache_dir: &Path) {
    evict_old_cache_with(&RealFs, cache_dir);
}

pub fn evict_old_cache_with(fs: &dyn FsOps, cache_dir: &Path) {
    let entries = match fs.read_dir(cache_dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("Cache eviction failed: {e}");
            return;
        }
    };
    let cutoff = SystemTime::now() - Duration::from_secs(crate::CACHE_MAX_AGE_SECONDS);
    for entry in entries {
        let name = entry.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let evict = || -> std::io::Result<bool> {
            if !fs.is_file(&entry)? || fs.metadata(&entry)?.modified()? >= cutoff {
                return Ok(false);
            }
            fs.remove_file(&entry)?;
            Ok(true)
        };
        match evict() {
            Ok(true) => log::debug!("Evicted old cache file: {name}"),
            Ok(false) => {}
            Err(e) => log::warn!("Failed to evict cache file {name}: {e}"),
        }
    }
}

/// `validate_preview_epub`의 본체. 문제가 있으면 사유를 `Err`로 돌려준다.
fn check_preview_epub(fs: &dyn FsOps, cache_file: &Path) -> Result<(), String> {
    let file = fs.open(cache_file).map_err(|e| format!("validation error: {e}"))?;
    let mut zin = match ZipArchive::new(file) {
        Ok(zin) => zin,
        Err(ZipError::Io(e)) if e.kind() != std::io::ErrorKind::UnexpectedEof => return Err(format!("validation error: {e}")),
        Err(_) => return Err("corrupted ZIP file".to_string()),
    };
    let validation = |e: ZipError| match e {
        ZipError::InvalidArchive(_) | ZipError::UnsupportedArchive(_) => "corrupted ZIP file".to_string(),
        other => format!("validation error: {other}"),
    };
    let names: std::collections::HashSet<String> = zip_names(&mut zin).into_iter().collect();
    let unquoted_names: std::collections::HashSet<String> = names.iter().map(|n| unquote(n)).collect();
    let in_names = |path: &str| -> bool {
        if path.is_empty() {
            return false;
        }
        let clean = clean_href(path);
        names.contains(path) || names.contains(&clean) || unquoted_names.contains(&clean)
    };

    // 1) mimetype 검증
    if !names.contains("mimetype") {
        return Err("mimetype file missing".to_string());
    }
    let mimetype = String::from_utf8_lossy(&read_member(&mut zin, "mimetype").map_err(validation)?).trim().to_string();
    if mimetype != "application/epub+zip" {
        return Err(format!("invalid mimetype: {mimetype}"));
    }

    // 2) OPF 찾기 및 파싱
    let Some(opf_path) = find_opf_path(&mut zin) else { return Err("OPF file not found".to_string()) };
    let opf_bytes = match read_member(&mut zin, &opf_path) {
        Ok(bytes) => bytes,
        Err(ZipError::FileNotFound) => return Err(format!("OPF file missing in archive: {opf_path}")),
        Err(e) => return Err(validation(e)),
    };
    let opf_text = declare_unbound_prefixes(&xml_text(&opf_bytes));
    let doc = parse_xml(&opf_text).map_err(|e| format!("OPF parse error: {e}"))?;
    let opf_dir = Path::new(&opf_path).parent().and_then(Path::to_str).unwrap_or_default().to_owned();

    // 3) manifest / spine 존재 확인
    let manifest_el = find_el(&doc, OPF_NS, "manifest");
    let spine_el = find_el(&doc, OPF_NS, "spine");
    let Some(manifest_el) = manifest_el else { return Err("manifest element missing".to_string()) };
    let Some(spine_el) = spine_el else { return Err("spine element missing".to_string()) };

    let mut manifest: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for item in children_named(manifest_el, OPF_NS, "item") {
        manifest.insert(item.attribute("id").unwrap_or("").to_string(), item.attribute("href").unwrap_or("").to_string());
    }

    let mut edits: Vec<Edit> = Vec::new();
    let mut ncx_replacement: Option<(String, Vec<u8>)> = None;

    // 4) spine itemref 검증: manifest에 없는 항목 제거
    let mut live_refs: Vec<Node> = Vec::new();
    for itemref in children_named(spine_el, OPF_NS, "itemref") {
        let idref = itemref.attribute("idref").unwrap_or("");
        if manifest.contains_key(idref) {
            live_refs.push(itemref);
        } else {
            log::warn!("EPUB validate: spine idref '{idref}' not in manifest, removing");
            edits.push(Edit::remove(itemref.range()));
        }
    }

    // 5) manifest 항목의 파일이 ZIP에 존재하는지 검증(spine 항목만)
    let mut remaining: Vec<Node> = Vec::new();
    for itemref in live_refs {
        let idref = itemref.attribute("idref").unwrap_or("").to_string();
        let href = clean_href(manifest.get(&idref).map_or("", String::as_str));
        let member = zip_path(&opf_dir, &href);
        if in_names(&member) {
            remaining.push(itemref);
            continue;
        }
        log::warn!("EPUB validate: spine item '{idref}' (href={href}) not in ZIP, removing");
        edits.push(Edit::remove(itemref.range()));
        if let Some(item) = children_named(manifest_el, OPF_NS, "item").find(|i| i.attribute("id") == Some(idref.as_str())) {
            edits.push(Edit::remove(item.range()));
        }
        manifest.remove(&idref);
    }

    // 6) 유효한 spine 챕터 수 확인
    if remaining.is_empty() {
        return Err("no valid spine chapters remain".to_string());
    }

    // 7) toc 속성이 참조하는 NCX 검증
    let toc_id = spine_el.attribute("toc").unwrap_or("");
    if !toc_id.is_empty() {
        let toc_attr_range = || spine_el.attributes().find(|a| a.name() == "toc").map(|a| a.range());
        match manifest.get(toc_id) {
            None => {
                log::warn!("EPUB validate: toc='{toc_id}' not in manifest, removing toc attribute");
                edits.extend(toc_attr_range().map(Edit::remove));
            }
            Some(href) => {
                let toc_zp = zip_path(&opf_dir, &clean_href(href));
                if !in_names(&toc_zp) {
                    log::warn!("EPUB validate: toc NCX '{toc_zp}' not in ZIP, removing toc attribute");
                    edits.extend(toc_attr_range().map(Edit::remove));
                } else {
                    let ncx = read_member(&mut zin, &toc_zp).map_err(validation)?;
                    if let Some(pruned) = prune_dead_ncx_navpoints(&ncx, Path::new(&toc_zp).parent().and_then(Path::to_str).unwrap_or_default(), &in_names) {
                        log::warn!("EPUB validate: removing NCX entries that point to missing documents in {toc_zp}");
                        ncx_replacement = Some((toc_zp, pruned));
                    }
                }
            }
        }
    }

    // 8) 필요 시 OPF 재작성
    if edits.is_empty() && ncx_replacement.is_none() {
        return Ok(());
    }
    log::info!("EPUB validate: rewriting OPF in {}", cache_file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let modified_opf = format!("{XML_DECLARATION}{}", apply_edits(&opf_text, doc.root_element().range(), edits));
    // 이름을 고정하면 같은 책의 동시 요청이 서로의 임시 파일을 덮어쓴다.
    let tmp_path = sysdeps::temp_file_maker::temp_sibling(cache_file);
    let rewrite = || -> Result<(), String> {
        let mut zin_r = ZipArchive::new(std::fs::File::open(cache_file).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut zout = zip::ZipWriter::new(std::fs::File::create(&tmp_path).map_err(|e| e.to_string())?);
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for name in zip_names(&mut zin_r) {
            if name.ends_with('/') {
                zout.add_directory(name.as_str(), deflated).map_err(|e| e.to_string())?;
                continue;
            }
            let data: Vec<u8> = if name == opf_path {
                modified_opf.clone().into_bytes()
            } else if ncx_replacement.as_ref().is_some_and(|(path, _)| *path == name) {
                ncx_replacement.as_ref().map(|(_, data)| data.clone()).unwrap_or_default()
            } else {
                read_member(&mut zin_r, &name).map_err(|e| e.to_string())?
            };
            zout.start_file(name.as_str(), if name == "mimetype" { stored } else { deflated }).map_err(|e| e.to_string())?;
            zout.write_all(&data).map_err(|e| e.to_string())?;
        }
        zout.finish().map_err(|e| e.to_string())?;
        std::fs::rename(&tmp_path, cache_file).map_err(|e| e.to_string())
    };
    rewrite().map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        format!("validation error: {e}")
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::ZipArchive;
    use zip::write::{SimpleFileOptions, ZipWriter};

    use super::read_member_limited;

    fn archive(data: &[u8]) -> ZipArchive<Cursor<Vec<u8>>> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("big.bin", SimpleFileOptions::default()).unwrap();
        writer.write_all(data).unwrap();
        ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap()
    }

    #[test]
    fn read_member_rejects_members_over_the_limit() {
        let mut zin = archive(&[b'a'; 100]);
        assert!(read_member_limited(&mut zin, "big.bin", 99).is_err());
        assert_eq!(read_member_limited(&mut zin, "big.bin", 100).unwrap().len(), 100);
    }
}
