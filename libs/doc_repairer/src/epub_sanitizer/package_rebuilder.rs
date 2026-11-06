//! EPUB 멤버를 읽고 허용 목록으로 다시 쓴다: 멤버 인코딩, 패키지 문서(OPF)와 선언 형식, 멤버 분류, manifest 정리.

use super::*;
use common::xml_parser::parse_xml;

/// 멤버 하나와 전체를 읽는 상한. 압축 폭탄이 메모리를 채우지 못하게 한다.
const MAX_MEMBER: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;

const XML_EXTS: &[&str] = &["xml", "opf", "ncx", "smil"];
const PASSIVE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "avif", "ico", "jp2", "jxl", "ttf", "otf", "woff", "woff2", "eot", "ttc", "mp3", "m4a", "aac", "ogg", "oga", "opus", "wav", "mp4",
    "m4v", "ogv", "webm",
];

static OPF_ITEM: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)[ \t]*<(?:\w+:)?item\b[^>]*>(?:\s*</(?:\w+:)?item>)?[ \t]*\r?\n?"));
static ATTR_HREF: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
static ATTR_PROPERTIES: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)\s+properties\s*=\s*(?:"([^"]*)"|'([^']*)')"#));

#[derive(Debug)]
pub struct Plan {
    /// 재구성한 EPUB 바이트.
    pub bytes: Vec<u8>,
    /// 지운 멤버와 이유.
    pub removed: Vec<(String, &'static str)>,
    /// 내용을 고친 멤버와 고친 종류.
    pub edited: Vec<(String, String)>,
}

pub(crate) fn ext_of(name: &str) -> String {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
}

pub(crate) struct Member {
    pub(crate) name: String,
    pub(crate) data: Vec<u8>,
    pub(crate) method: CompressionMethod,
    pub(crate) modified: Option<zip::DateTime>,
}

/// 멤버 텍스트의 인코딩. 정리한 글을 같은 인코딩으로 되써서 원본의 인코딩 선언과 어긋나지 않게 한다.
#[derive(Clone, Copy)]
enum Enc {
    Utf8,
    Utf16 { le: bool },
}

/// UTF-8(BOM 유무)과 BOM이 있는 UTF-16을 읽는다. 그 밖(UTF-32, 다른 코드페이지)은 안전하게 정리할 수 없어 오류다.
fn decode_enc(name: &str, data: &[u8]) -> Result<(String, Enc), String> {
    let utf16 = |bytes: &[u8], le: bool| -> Result<(String, Enc), String> {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16(&units).map(|t| (t, Enc::Utf16 { le })).map_err(|_| format!("{name}: UTF-16이 올바르지 않아 안전하게 정리할 수 없다"))
    };
    if data.starts_with(b"\xFF\xFE\x00\x00") || data.starts_with(b"\x00\x00\xFE\xFF") {
        return Err(format!("{name}: UTF-32라 안전하게 정리할 수 없다"));
    }
    if let Some(rest) = data.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, true);
    }
    if let Some(rest) = data.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, false);
    }
    String::from_utf8(data.to_vec()).map(|t| (t, Enc::Utf8)).map_err(|_| format!("{name}: UTF-8이 아니라 안전하게 정리할 수 없다"))
}

fn encode_enc(text: &str, enc: Enc) -> Vec<u8> {
    match enc {
        Enc::Utf8 => text.as_bytes().to_vec(),
        Enc::Utf16 { le } => {
            let mut out = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
            for unit in text.encode_utf16() {
                out.extend(if le { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            out
        }
    }
}

pub(crate) fn decode(name: &str, data: &[u8]) -> Result<String, String> {
    decode_enc(name, data).map(|(t, _)| t)
}

pub(crate) fn read_members(bytes: &[u8]) -> Result<Vec<Member>, String> {
    let (members, broken) = read_members_lenient(bytes)?;
    match broken.into_iter().next() {
        Some((_, message)) => Err(message),
        None => Ok(members),
    }
}

/// 읽을 수 없는 멤버: `(이름, 오류 메시지)`.
pub(crate) type BrokenMembers = Vec<(String, String)>;

/// 읽을 수 없는 멤버를 오류로 올리지 않고 `(이름, 오류 메시지)`로 모아 돌려준다. 이름을 알 수 없는 멤버나 크기 한도 초과는 여전히 오류다.
pub(crate) fn read_members_lenient(bytes: &[u8]) -> Result<(Vec<Member>, BrokenMembers), String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("ZIP을 열 수 없다: {e}"))?;
    let mut members = Vec::new();
    let mut broken = Vec::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let known = zip.name_for_index(i).and_then(Result::ok).map(std::borrow::Cow::into_owned);
        let mut entry = match zip.by_index(i) {
            Ok(entry) => entry,
            Err(e) => match known {
                Some(name) => {
                    broken.push((name, format!("멤버 {i}를 읽을 수 없다: {e}")));
                    continue;
                }
                None => return Err(format!("멤버 {i}를 읽을 수 없다: {e}")),
            },
        };
        let raw = entry.name_raw().to_vec();
        let name = String::from_utf8_lossy(&raw).into_owned();
        if entry.is_dir() {
            continue;
        }
        if entry.size() > MAX_MEMBER {
            return Err(format!("{name}: 멤버가 너무 크다"));
        }
        let mut data = Vec::with_capacity(entry.size().min(MAX_MEMBER) as usize);
        if let Err(e) = (&mut entry).take(MAX_MEMBER + 1).read_to_end(&mut data) {
            broken.push((name.clone(), format!("{name}: 읽을 수 없다: {e}")));
            continue;
        }
        total += data.len() as u64;
        if data.len() as u64 > MAX_MEMBER || total > MAX_TOTAL {
            return Err(format!("{name}: 압축을 푼 크기가 한도를 넘는다"));
        }
        let kept_utf8 = std::str::from_utf8(&raw).is_ok();
        members.push(Member { name: if kept_utf8 { name } else { format!("\u{0}{name}") }, data, method: entry.compression(), modified: entry.last_modified() });
    }
    Ok((members, broken))
}

/// 이름이 UTF-8이 아닌 멤버는 이름을 그대로 다시 쓸 수 없다. `read_members`가 앞에 NUL을 붙여 표시한다.
fn undecodable(name: &str) -> bool {
    name.starts_with('\0')
}

const MARKUP_MEDIA: &[&str] =
    &["application/xhtml+xml", "text/html", "image/svg+xml", "application/xml", "text/xml", "application/x-dtbncx+xml", "application/oebps-package+xml", "application/smil+xml", "application/smil"];

/// `media-type` 값을 비교할 수 있게 정규화한다: `;` 뒤의 매개변수를 떼고 앞뒤 공백을 지우고 소문자로 만든다.
fn base_media(raw: &str) -> String {
    raw.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// 리더가 마크업으로 여는 형식: 알려진 형식, `+xml` 접미사, `text/plain`·`text/css`가 아닌 `text/*`.
fn is_markup_media(media: &str) -> bool {
    MARKUP_MEDIA.contains(&media) || media.ends_with("+xml") || (media.starts_with("text/") && !matches!(media, "text/plain" | "text/css") && !is_script_media(media))
}

fn is_script_media(media: &str) -> bool {
    media.contains("javascript")
        || media.contains("ecmascript")
        || matches!(
            media,
            "application/java-archive" | "application/x-sh" | "application/x-msdownload" | "application/x-msdos-program" | "application/x-executable" | "application/x-bat" | "application/x-dosexec"
        )
}

/// 내용이 마크업처럼 시작하는가(BOM·앞 공백을 건너뛴다). 이미지·글꼴·미디어는 `<`로 시작하지 않는다.
/// UTF-16·UTF-32(BOM이 있든 없든)로 인코딩한 마크업도 알아본다.
fn looks_like_markup(data: &[u8]) -> bool {
    let head = &data[..data.len().min(256)];
    let first_char = |units: Vec<u32>| units.into_iter().filter_map(char::from_u32).find(|c| !c.is_whitespace());
    let utf16 = |bytes: &[u8], le: bool| -> Vec<u32> { bytes.chunks_exact(2).map(|c| u32::from(if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })).collect() };
    let utf32 =
        |bytes: &[u8], le: bool| -> Vec<u32> { bytes.chunks_exact(4).map(|c| if le { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) } else { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) }).collect() };
    let decoded = if let Some(rest) = head.strip_prefix(b"\xFF\xFE\x00\x00") {
        first_char(utf32(rest, true))
    } else if let Some(rest) = head.strip_prefix(b"\x00\x00\xFE\xFF") {
        first_char(utf32(rest, false))
    } else if let Some(rest) = head.strip_prefix(b"\xFF\xFE") {
        first_char(utf16(rest, true))
    } else if let Some(rest) = head.strip_prefix(b"\xFE\xFF") {
        first_char(utf16(rest, false))
    } else if head.starts_with(b"<\x00\x00\x00") {
        first_char(utf32(head, true))
    } else if head.starts_with(b"\x00\x00\x00<") {
        first_char(utf32(head, false))
    } else if head.starts_with(b"<\x00") {
        first_char(utf16(head, true))
    } else if head.starts_with(b"\x00<") {
        first_char(utf16(head, false))
    } else {
        let d = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
        return d.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<');
    };
    decoded == Some('<')
}

static PACKAGE_ROOT: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)\A\s*(?:<\?xml[^>]*\?>\s*)?(?:<!--.*?-->\s*)*(?:<!DOCTYPE[^>]*>\s*)?<(?:\w+:)?package\b"));

/// 패키지 문서(OPF) 이름들: `.opf` 확장자, `META-INF/container.xml`의 rootfile `full-path`(그대로와 퍼센트 디코딩한 두 형태 모두),
/// 루트 요소가 `<package>`인 XML. 확장자만 믿으면 다른 이름의 패키지 문서가 manifest 정리와 검증에서 빠진다.
/// `container.xml`이 있는데 해석할 수 없거나 가리키는 멤버가 없으면 오류다: 무엇이 패키지인지 모르면 고치지 않는다.
pub(crate) fn package_names(members: &[Member]) -> Result<BTreeSet<String>, String> {
    let existing: BTreeSet<&str> = members.iter().map(|m| m.name.as_str()).collect();
    let mut set = BTreeSet::new();
    for m in members {
        let ext = ext_of(&m.name);
        if ext == "opf" {
            set.insert(m.name.clone());
        }
        if m.name == "META-INF/container.xml" {
            let text = decode(&m.name, &m.data).map_err(|e| format!("container.xml의 인코딩을 읽을 수 없다: {e}"))?;
            let doc = parse_xml(&text).map_err(|e| format!("META-INF/container.xml을 해석할 수 없다: {e}"))?;
            for root in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "rootfile") {
                let raw = root.attribute("full-path").unwrap_or("");
                let found: Vec<String> = [zip_entry_name(raw), zip_entry_name(unquote(raw))].into_iter().flatten().filter(|p| existing.contains(p.as_str())).collect();
                if found.is_empty() {
                    return Err(format!("container.xml이 가리키는 패키지 문서가 없다: {raw}"));
                }
                set.extend(found);
            }
        } else if (XML_EXTS.contains(&ext.as_str()) || ext.is_empty() || ext == "txt") && PACKAGE_ROOT.is_match(&String::from_utf8_lossy(&m.data[..m.data.len().min(4096)])) {
            set.insert(m.name.clone());
        }
    }
    Ok(set)
}

/// 패키지 문서로 쓸 수 있는지 확인하고 해석에 쓴 글을 돌려준다. 실행 파일 확장자가 아니고, 읽을 수 있는 인코딩이고,
/// 내용이 정말 `<package>` 루트의 XML이어야 한다. 하나라도 어긋나면 오류다: 패키지 문서를 읽지 못하고 넘어가면 선언 형식을 놓친다.
pub(crate) fn package_text(m: &Member) -> Result<String, String> {
    let name = m.name.trim_start_matches('\0');
    if doc_checker::exec_extension_table::EPUB_MEMBER.contains(&ext_of(name)) {
        return Err(format!("{name}: 실행 파일 확장자인 패키지 문서"));
    }
    let (text, _) = decode_enc(name, &m.data)?;
    let text = text.trim_start_matches('\u{feff}').to_owned();
    if !PACKAGE_ROOT.is_match(&text) {
        return Err(format!("{name}: 컨테이너가 가리키는 패키지 문서가 `<package>` 문서가 아니다"));
    }
    let doc = parse_xml(&text).map_err(|e| format!("{name}: 패키지 문서를 해석할 수 없다: {e}"))?;
    if doc.root_element().tag_name().name() != "package" {
        return Err(format!("{name}: 루트 요소가 `<package>`가 아니다"));
    }
    drop(doc);
    Ok(text)
}

/// manifest가 선언한 형식. 멤버 경로(OPF 기준으로 푼 것) → 소문자 media-type. 패키지 문서를 읽지 못하면 오류다.
fn declared_media(members: &[Member], packages: &BTreeSet<String>) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for m in members.iter().filter(|m| packages.contains(&m.name)) {
        let text = package_text(m)?;
        let doc = parse_xml(&text).map_err(|e| format!("{}: {e}", m.name))?;
        let base = Path::new(&m.name).parent().and_then(Path::to_str).unwrap_or_default().to_owned();
        for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "item") {
            let (Some(href), Some(media)) = (item.attribute("href"), item.attribute("media-type")) else { continue };
            map.insert(zip_entry_name(Path::new(&base).join(unquote(href).split('#').next().unwrap_or(""))).unwrap_or_default(), base_media(media));
        }
    }
    Ok(map)
}

fn classify(name: &str, data: &[u8], media: Option<&str>, is_package: bool) -> Option<&'static str> {
    if doc_checker::zip_name_rules::check(name.trim_start_matches('\0')).iter().any(|f| f.severity >= Severity::High) {
        return Some("위험한 멤버 이름");
    }
    if media.is_some_and(is_script_media) {
        return Some("스크립트 형식으로 선언됨");
    }
    // 컨테이너가 가리키는 패키지 문서는 확장자와 무관하게 남긴다(마크업으로 정리한다).
    if is_package {
        return None;
    }
    let ext = ext_of(name);
    let passive = PASSIVE_EXTS.contains(&ext.as_str());
    // 리더는 확장자가 아니라 선언된 형식으로 연다. 이미지·글꼴 확장자가 마크업·CSS라고 선언했거나 내용이 마크업이면 지운다.
    if passive && (media.is_some_and(|m| is_markup_media(m) || m == "text/css") || looks_like_markup(data)) {
        return Some("확장자와 선언 형식 불일치(마크업 위장)");
    }
    let textual = HTML_EXTS.contains(&ext.as_str()) || XML_EXTS.contains(&ext.as_str()) || matches!(ext.as_str(), "css" | "svg" | "txt");
    // 스캐너(`check_magic`)와 같은 기준이다: `MZ`로 시작하면 헤더가 온전하지 않아도 실행 파일 후보로 본다.
    if !textual && (data.starts_with(b"MZ") || matches!(sniff(data), Sniffed::Pe | Sniffed::Elf | Sniffed::MachO)) {
        return Some("실행 코드 내용");
    }
    if name == "mimetype" || name.starts_with("META-INF/") || textual || PASSIVE_EXTS.contains(&ext.as_str()) {
        return None;
    }
    Some(if doc_checker::exec_extension_table::EPUB_MEMBER.contains(&ext) { "실행 파일 형식" } else { "허용 목록 밖 형식" })
}

/// OPF에서 지운 멤버의 manifest 항목, 원격·위험 항목, 낡은 `scripted`·`remote-resources` 선언을 지운다.
fn rewrite_opf(text: &str, opf_name: &str, removed: &BTreeSet<String>, changes: &mut Changes) -> String {
    let base = Path::new(opf_name).parent().and_then(Path::to_str).unwrap_or_default().to_owned();
    let pruned = OPF_ITEM.replace_all(text, |c: &regex::Captures| {
        let item = &c[0];
        let Some(h) = ATTR_HREF.captures(item) else { return item.to_owned() };
        let href = h.get(1).or_else(|| h.get(2)).map_or("", |m| m.as_str());
        let resolved = zip_entry_name(Path::new(&base).join(unquote(href).split('#').next().unwrap_or(""))).unwrap_or_default();
        if removed.contains(&resolved) || !keep_url(href, true) {
            changes.add("manifest 항목");
            String::new()
        } else {
            item.to_owned()
        }
    });
    ATTR_PROPERTIES
        .replace_all(&pruned, |c: &regex::Captures| {
            let value = c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str());
            let kept: Vec<&str> = value.split_whitespace().filter(|p| !matches!(*p, "scripted" | "remote-resources")).collect();
            if kept.len() == value.split_whitespace().count() {
                c[0].to_owned()
            } else if kept.is_empty() {
                String::new()
            } else {
                format!(" properties=\"{}\"", kept.join(" "))
            }
        })
        .into_owned()
}

/// EPUB 바이트를 허용 목록으로 다시 만든다.
pub fn rebuild(src: &[u8]) -> Result<Plan, String> {
    let members = read_members(src)?;
    let packages = package_names(&members)?;
    let media = declared_media(&members, &packages)?;
    let mut removed: Vec<(String, &'static str)> = Vec::new();
    let mut kept: Vec<Member> = Vec::new();
    for m in members {
        match classify(&m.name, &m.data, media.get(m.name.trim_start_matches('\0')).map(String::as_str), packages.contains(&m.name)) {
            Some(reason) => removed.push((m.name.trim_start_matches('\0').to_owned(), reason)),
            None if undecodable(&m.name) => return Err(format!("{}: 이름이 UTF-8이 아니라 다시 쓸 수 없다", m.name.trim_start_matches('\0'))),
            None => kept.push(m),
        }
    }
    let removed_names: BTreeSet<String> = removed.iter().map(|(n, _)| n.clone()).collect();
    let mut edited: Vec<(String, String)> = Vec::new();
    for m in &mut kept {
        let ext = ext_of(&m.name);
        let is_opf = packages.contains(&m.name);
        let declared = media.get(m.name.as_str()).map(String::as_str);
        // 확장자와 선언 형식 중 하나라도 마크업이면 마크업으로, CSS면 CSS로 정리한다(둘 다일 수도 있다).
        let markup = is_opf || HTML_EXTS.contains(&ext.as_str()) || ext == "svg" || XML_EXTS.contains(&ext.as_str()) || declared.is_some_and(is_markup_media);
        let css = ext == "css" || declared == Some("text/css");
        if !markup && !css {
            continue;
        }
        let (original, enc) = decode_enc(&m.name, &m.data)?;
        let (mut text, mut changes) = if markup { strip_markup(&original, ext == "svg") } else { (original.clone(), Changes::default()) };
        if css {
            let cleaned = strip_css(&text);
            if cleaned != text {
                changes.add("CSS 정리");
                text = cleaned;
            }
        }
        let text = if is_opf {
            let mut more = Changes::default();
            let t = rewrite_opf(&text, &m.name, &removed_names, &mut more);
            changes.merge(more);
            t
        } else {
            text
        };
        let encoded = encode_enc(&text, enc);
        if encoded != m.data {
            edited.push((m.name.clone(), changes.summary()));
            m.data = encoded;
        }
    }
    let bytes = write_members(&mut kept)?;
    Ok(Plan { bytes, removed, edited })
}

/// 멤버를 EPUB ZIP으로 쓴다. `mimetype`은 항상 첫 멤버이고 압축하지 않는다.
pub(crate) fn write_members(kept: &mut [Member]) -> Result<Vec<u8>, String> {
    kept.sort_by_key(|m| m.name != "mimetype");
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for m in kept.iter() {
        let method = if m.name == "mimetype" || m.method == CompressionMethod::Stored { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        let mut opts = SimpleFileOptions::default().compression_method(method);
        if let Some(t) = m.modified {
            opts = opts.last_modified_time(t);
        }
        zip.start_file(m.name.as_str(), opts).map_err(|e| format!("{}: 쓸 수 없다: {e}", m.name))?;
        zip.write_all(&m.data).map_err(|e| e.to_string())?;
    }
    let bytes = zip.finish().map_err(|e| e.to_string())?.into_inner();
    Ok(bytes)
}
