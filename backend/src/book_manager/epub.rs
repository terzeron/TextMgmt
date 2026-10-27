//! EPUB 구조 검사·보조. Python `_find_opf_path`, `_validate_preview_epub`, `_get_epub_total_chapters`, `_downscale_epub_image`,
//! `_evict_old_cache`, `_find_libreoffice`, `_convert_with_libreoffice`에 대응한다.
//!
//! XML은 `roxmltree`로 읽는다. lxml의 `recover=True`처럼 깨진 XML을 복구하지는 않는다. 대신 선언 없이 쓰인 접두사는
//! `declare_unbound_prefixes`로 먼저 선언한다. 문서를 고쳐 쓸 때는 DOM을 직렬화하지 않고 원문에서 해당 구간만 잘라내
//! (`Edit`) 원문 서식을 보존한다.

use std::io::{Read, Seek, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use image::{ImageFormat, imageops::FilterType};
use regex::Regex;
use roxmltree::{Document, Node};
use sysdeps::{CommandError, CommandRunner, FsOps, RealFs};
use zip::ZipArchive;
use zip::result::ZipError;

use super::BookManager;
use super::util::{TempDir, clean_href, posix_dirname, unquote, zip_path};

pub const OPF_NS: &str = "http://www.idpf.org/2007/opf";
pub const CONTAINER_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
pub const NCX_NS: &str = "http://www.daisy.org/z3986/2005/ncx/";

/// 이미지 축소 때 JPEG 품질.
const EPUB_IMAGE_JPEG_QUALITY: u8 = 80;
/// LibreOffice 변환 시간 상한.
const LIBREOFFICE_TIMEOUT: Duration = Duration::from_secs(60);

/// OPF에서 선언 없이 자주 쓰이는 접두사. 모르는 접두사는 자리표시 URN으로 선언한다(값은 읽지 않는다).
fn known_prefix_uri(prefix: &str) -> String {
    match prefix {
        "opf" => "http://www.idpf.org/2007/opf",
        "dc" => "http://purl.org/dc/elements/1.1/",
        "dcterms" => "http://purl.org/dc/terms/",
        "xsi" => "http://www.w3.org/2001/XMLSchema-instance",
        "xlink" => "http://www.w3.org/1999/xlink",
        "epub" => "http://www.idpf.org/2007/ops",
        other => return format!("urn:unbound-prefix:{other}"),
    }
    .to_string()
}

fn regexes() -> &'static (Regex, Regex, Regex, Regex, Regex) {
    static RE: OnceLock<(Regex, Regex, Regex, Regex, Regex)> = OnceLock::new();
    RE.get_or_init(|| {
        (
            Regex::new(r"(?s)<!--.*?-->|<\?.*?\?>|<!DOCTYPE[^>]*>").expect("정규식"),
            Regex::new(r"(?s)<(/?)([^\s/>!?][^>]*)>").expect("정규식"),
            Regex::new(r"^([A-Za-z_][\w.-]*):").expect("정규식"),
            Regex::new(r"\s([A-Za-z_][\w.-]*):[\w.-]+\s*=").expect("정규식"),
            Regex::new(r"xmlns:([A-Za-z_][\w.-]*)\s*=").expect("정규식"),
        )
    })
}

/// 요소·속성 이름에 쓰였지만 선언되지 않은 접두사를 루트 요소에 선언한다. 빠진 선언이 없으면 입력을 그대로 돌려준다.
pub fn declare_unbound_prefixes(xml: &str) -> String {
    let (skip_re, tag_re, element_re, attr_re, declared_re) = regexes();
    let masked = skip_re.replace_all(xml, |m: &regex::Captures| " ".repeat(m[0].len())).into_owned();
    let Some(root) = tag_re.captures(&masked) else { return xml.to_string() };
    let declared: Vec<String> = declared_re.captures_iter(&masked).map(|c| c[1].to_string()).collect();
    let mut used: Vec<String> = Vec::new();
    for tag in tag_re.captures_iter(&masked) {
        let body = &tag[2];
        let mut names: Vec<String> = Vec::new();
        if let Some(c) = element_re.captures(body) {
            names.push(c[1].to_string());
        }
        names.extend(attr_re.captures_iter(body).map(|c| c[1].to_string()));
        for name in names {
            if !declared.contains(&name) && name != "xml" && name != "xmlns" && !used.contains(&name) {
                used.push(name);
            }
        }
    }
    if used.is_empty() {
        return xml.to_string();
    }
    let declarations: String = used.iter().map(|n| format!(" xmlns:{n}=\"{}\"", known_prefix_uri(n))).collect();
    let body = root.get(2).expect("그룹 2");
    let name_len = body.as_str().find(|c: char| c.is_whitespace() || c == '/' || c == '>').unwrap_or(body.as_str().len());
    let name_end = body.start() + name_len;
    format!("{}{declarations}{}", &xml[..name_end], &xml[name_end..])
}

/// XML 바이트를 문자열로 바꾼다(UTF-8 BOM 제거, 잘못된 바이트는 대체 문자).
pub fn xml_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_string()
}

/// 외부 엔티티·네트워크 접근 없이 파싱한다(`resolve_entities=False, no_network=True, load_dtd=False`와 같다).
pub fn parse_xml(text: &str) -> Result<Document<'_>, roxmltree::Error> {
    Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
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

/// ZIP 멤버 이름을 저장 순서대로 돌려준다. Python `namelist()`.
pub fn zip_names<R: Read + Seek>(zin: &mut ZipArchive<R>) -> Vec<String> {
    (0..zin.len()).filter_map(|i| zin.by_index_raw(i).ok().and_then(|f| f.name().ok().map(|n| n.into_owned()))).collect()
}

/// ZIP 멤버를 읽는다. 이름이 정확히 없으면 퍼센트 인코딩을 푼 이름이 같은 멤버를 찾는다.
pub fn read_member<R: Read + Seek>(zin: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>, ZipError> {
    let actual = if zin.index_for_name(name).is_some() { name.to_string() } else { zip_names(zin).into_iter().find(|n| unquote(n) == name).unwrap_or_else(|| name.to_string()) };
    let mut file = zin.by_name(&actual)?;
    let mut buf = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut buf).map_err(ZipError::Io)?;
    Ok(buf)
}

impl BookManager {
    /// ZIP 안 OPF 경로를 찾는다. container.xml, 정규식 폴백, `.opf` 직접 탐색 순으로 시도한다.
    pub fn _find_opf_path<R: Read + Seek>(zin: &mut ZipArchive<R>) -> Option<String> {
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
    pub fn _get_epub_total_chapters(file_path: &Path) -> i64 {
        let count = || -> Result<i64, String> {
            let file = std::fs::File::open(file_path).map_err(|e| e.to_string())?;
            let mut zin = ZipArchive::new(file).map_err(|e| e.to_string())?;
            let Some(opf_path) = Self::_find_opf_path(&mut zin) else { return Ok(0) };
            let opf_bytes = read_member(&mut zin, &opf_path).map_err(|e| e.to_string())?;
            let text = declare_unbound_prefixes(&xml_text(&opf_bytes));
            let doc = parse_xml(&text).map_err(|e| e.to_string())?;
            let Some(spine) = find_el(&doc, OPF_NS, "spine") else { return Ok(0) };
            let manifest: std::collections::HashMap<&str, &str> = doc
                .descendants()
                .filter(|n| n.is_element() && n.tag_name().namespace() == Some(OPF_NS) && n.tag_name().name() == "item")
                .filter_map(|n| Some((n.attribute("id")?, n.attribute("href").unwrap_or(""))))
                .collect();
            let base = posix_dirname(&opf_path);
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
    pub fn _downscale_epub_image(data: &[u8]) -> Vec<u8> {
        Self::try_downscale_epub_image(data).unwrap_or_else(|| data.to_vec())
    }

    fn try_downscale_epub_image(data: &[u8]) -> Option<Vec<u8>> {
        let format = image::guess_format(data).ok().filter(|f| matches!(f, ImageFormat::Jpeg | ImageFormat::Png))?;
        let img = image::load_from_memory_with_format(data, format).ok()?;
        let (width, height) = (img.width(), img.height());
        let max_edge = width.max(height);
        if max_edge <= Self::EPUB_IMAGE_MAX_EDGE {
            return None;
        }
        let scale = f64::from(Self::EPUB_IMAGE_MAX_EDGE) / f64::from(max_edge);
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
    pub fn _validate_preview_epub(cache_file: &Path) -> (bool, Option<String>) {
        Self::_validate_preview_epub_with(&RealFs, cache_file)
    }

    pub fn _validate_preview_epub_with(fs: &dyn FsOps, cache_file: &Path) -> (bool, Option<String>) {
        match validate_preview_epub(fs, cache_file) {
            Ok(()) => (true, None),
            Err(message) => (false, Some(message)),
        }
    }

    /// 1일 넘은 캐시 파일을 지운다.
    pub fn _evict_old_cache(cache_dir: &Path) {
        Self::_evict_old_cache_with(&RealFs, cache_dir);
    }

    pub fn _evict_old_cache_with(fs: &dyn FsOps, cache_dir: &Path) {
        let entries = match fs.read_dir(cache_dir) {
            Ok(entries) => entries,
            Err(e) => {
                log::warn!("Cache eviction failed: {e}");
                return;
            }
        };
        let cutoff = SystemTime::now() - Duration::from_secs(Self::CACHE_MAX_AGE_SECONDS);
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

    /// LibreOffice 실행 파일 경로. `which`로 못 찾으면 맥 경로, 그것도 없으면 `libreoffice`다.
    pub fn _find_libreoffice_with(which: &dyn Fn(&str) -> Option<String>, exists: &dyn Fn(&Path) -> bool) -> String {
        for command in ["libreoffice", "soffice"] {
            if let Some(path) = which(command) {
                return path;
            }
        }
        let mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice";
        if exists(Path::new(mac_path)) {
            return mac_path.to_string();
        }
        "libreoffice".to_string()
    }

    /// 이 매니저의 외부 명령 대역으로 LibreOffice를 찾는다.
    pub(crate) fn find_libreoffice(&self) -> String {
        Self::_find_libreoffice_with(&|command| self.runner.which(command).map(|p| p.to_string_lossy().into_owned()), &|p| p.exists())
    }

    /// LibreOffice로 파일을 변환하고 결과 텍스트를 돌려준다. 변환 결과가 없으면 빈 문자열이다.
    /// `work_dir`가 없으면 임시 디렉터리를 만들어 쓰고 지운다.
    pub fn _convert_with_libreoffice_with(runner: &dyn CommandRunner, lo_bin: &str, work_dir: Option<&Path>, file_path: &Path, output_format: &str) -> String {
        Self::try_convert_with_libreoffice(runner, lo_bin, work_dir, file_path, output_format).unwrap_or_else(|e| {
            log::error!("LibreOffice 실행 실패: file='{}', format='{output_format}': {e}", file_path.display());
            String::new()
        })
    }

    /// `_convert_with_libreoffice_with`의 오류 판. 실행 실패(시간 초과 포함)는 `Err`다(Python은 예외가 올라간다).
    pub(crate) fn try_convert_with_libreoffice(runner: &dyn CommandRunner, lo_bin: &str, work_dir: Option<&Path>, file_path: &Path, output_format: &str) -> Result<String, CommandError> {
        let temp;
        let dir: PathBuf = match work_dir {
            Some(dir) => dir.to_path_buf(),
            None => {
                temp = TempDir::new("lo-convert-").map_err(|e| CommandError::Failed(e.to_string()))?;
                temp.path().to_path_buf()
            }
        };
        let argv = vec![
            lo_bin.to_string(),
            "--headless".to_string(),
            "--convert-to".to_string(),
            output_format.to_string(),
            "--outdir".to_string(),
            dir.to_string_lossy().into_owned(),
            file_path.to_string_lossy().into_owned(),
        ];
        let output = runner.run(&argv, Some(LIBREOFFICE_TIMEOUT))?;
        let ext = output_format.split(':').next().unwrap_or(output_format);
        let stem = file_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let read = |path: &Path| String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).into_owned();
        let out_file = dir.join(format!("{stem}.{ext}"));
        if out_file.exists() {
            return Ok(read(&out_file));
        }
        // stem 불일치 시 글로브로 검색
        let mut candidates: Vec<PathBuf> =
            std::fs::read_dir(&dir).map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.to_string_lossy() == ext)).collect()).unwrap_or_default();
        candidates.sort();
        if let Some(first) = candidates.first() {
            return Ok(read(first));
        }
        let names: Vec<String> = std::fs::read_dir(&dir).map(|rd| rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        log::error!(
            "LibreOffice produced no output: file='{}', format='{output_format}', returncode={}, stderr={}, tmpdir_files={names:?}",
            file_path.display(),
            output.returncode,
            String::from_utf8_lossy(&output.stderr).chars().take(500).collect::<String>()
        );
        Ok(String::new())
    }
}

/// `_validate_preview_epub`의 본체. 문제가 있으면 사유를 `Err`로 돌려준다.
fn validate_preview_epub(fs: &dyn FsOps, cache_file: &Path) -> Result<(), String> {
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
    let Some(opf_path) = BookManager::_find_opf_path(&mut zin) else { return Err("OPF file not found".to_string()) };
    let opf_bytes = match read_member(&mut zin, &opf_path) {
        Ok(bytes) => bytes,
        Err(ZipError::FileNotFound) => return Err(format!("OPF file missing in archive: {opf_path}")),
        Err(e) => return Err(validation(e)),
    };
    let opf_text = declare_unbound_prefixes(&xml_text(&opf_bytes));
    let doc = parse_xml(&opf_text).map_err(|e| format!("OPF parse error: {e}"))?;
    let opf_dir = posix_dirname(&opf_path);

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
                    if let Some(pruned) = prune_dead_ncx_navpoints(&ncx, &posix_dirname(&toc_zp), &in_names) {
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
    let tmp_path = cache_file.with_extension("tmp");
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
