//! EPUB 패키지를 다시 쓰는 교정 기법이 함께 쓰는 공용 도구. 보안 정리(`epub_sanitizer`)와 결함 수리(`defect_repairer`)가 쓴다.
//!
//! - 멤버 입출력: 상한 안에서 멤버 읽기(`read_members`), 인코딩을 지키는 글 해석(`decode`), EPUB ZIP 쓰기(`write_members`)
//! - 패키지 문서(OPF) 찾기와 읽기(`package_names`, `package_text`)
//! - 브라우저 방식 URL 해석(`normalize_url`, `scheme_of`, `is_local_ref`)
//! - 재구성 검증: 보이는 글자 보존(`visible_text`, `is_subsequence`), manifest·spine 정합성(`dangling`)

use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::LazyLock;

use common::url_parser::unquote;
use common::xml_parser::parse_xml;
use common::zip_reader::zip_entry_name;
use htmlkit::{decode_entities, text_of};
use regex::Regex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

// ---- 멤버 입출력 ----

pub(crate) fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("잘못된 정규식 {p}: {e}"))
}

/// 멤버 하나와 전체를 읽는 상한. 압축 폭탄이 메모리를 채우지 못하게 한다.
pub(crate) const MAX_MEMBER: u64 = 256 * 1024 * 1024;

pub(crate) const MAX_TOTAL: u64 = 1024 * 1024 * 1024;

pub(crate) const HTML_EXTS: &[&str] = &["xhtml", "html", "htm"];

pub(crate) const XML_EXTS: &[&str] = &["xml", "opf", "ncx", "smil"];

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
pub(crate) enum Enc {
    Utf8,
    Utf16 { le: bool },
}

/// UTF-8(BOM 유무)과 BOM이 있는 UTF-16을 읽는다. 그 밖(UTF-32, 다른 코드페이지)은 안전하게 정리할 수 없어 오류다.
pub(crate) fn decode_enc(name: &str, data: &[u8]) -> Result<(String, Enc), String> {
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

pub(crate) fn encode_enc(text: &str, enc: Enc) -> Vec<u8> {
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
pub(crate) fn undecodable(name: &str) -> bool {
    name.starts_with('\0')
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

// ---- 패키지 문서 ----

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

// ---- URL 해석 ----

/// 브라우저가 URL을 읽는 방식으로 정규화한다: 엔티티를 풀고, 공백·제어문자를 지우고, 소문자로 만들고, `\`를 `/`로 본다
/// (`/\evil`과 `\/evil`은 프로토콜 상대 주소 `//evil`이다).
pub(crate) fn normalize_url(v: &str) -> String {
    decode_entities(v).chars().filter(|c| !c.is_control() && !c.is_whitespace()).map(|c| if c == '\\' { '/' } else { c }).collect::<String>().to_lowercase()
}

pub(crate) fn scheme_of(n: &str) -> Option<&str> {
    let (s, _) = n.split_once(':')?;
    (!s.is_empty() && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))).then_some(s)
}

/// 패키지 안 멤버를 가리키는 상대 참조인가: 스킴이 없고, 프로토콜 상대 주소(`//host`)나 UNC 경로(`\\\\host`)가 아니다.
pub(crate) fn is_local_ref(href: &str) -> bool {
    if decode_entities(href).trim().starts_with("\\\\") {
        return false;
    }
    let n = normalize_url(href);
    !n.starts_with("//") && scheme_of(&n).is_none()
}

// ---- 재구성 검증 ----

static SELF_CLOSING_RAW: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<(script|style)\b([^>]*?)/>"));

/// 눈에 보이는 글자만 공백 없이 모은다. `text_of`는 HTML 규칙으로 읽어서 XHTML의 자기 닫힘 `<script src=".."/>`를 문서 끝까지 이어지는
/// 스크립트로 보고 본문을 버리므로, 자기 닫힘 `script`·`style`을 먼저 펼친다. 스크립트나 주석을 지우면 이어 붙는 텍스트 노드의
/// 경계 공백도 달라지므로 공백은 글자로 세지 않는다.
pub(crate) fn visible_text(markup: &str) -> String {
    let expanded = SELF_CLOSING_RAW.replace_all(markup, "<$1$2></$1>");
    text_of(&expanded).chars().filter(|c| !c.is_whitespace()).collect()
}

pub(crate) fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// OPF manifest 항목 중 대상이 없는 수와 spine에서 manifest에 없는 idref 수.
pub(crate) fn dangling(members: &[Member]) -> Result<(usize, usize), String> {
    let names: BTreeSet<&str> = members.iter().map(|m| m.name.as_str()).collect();
    let (mut missing, mut bad_spine) = (0, 0);
    let packages = package_names(members)?;
    for m in members.iter().filter(|m| packages.contains(&m.name)) {
        let text = package_text(m)?;
        let doc = parse_xml(&text).map_err(|e| format!("{}: OPF를 해석할 수 없다: {e}", m.name))?;
        let base = Path::new(&m.name).parent().and_then(Path::to_str).unwrap_or_default().to_owned();
        let mut ids = BTreeSet::new();
        for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "item") {
            ids.insert(item.attribute("id").unwrap_or("").to_owned());
            let href = item.attribute("href").unwrap_or("");
            if is_local_ref(href) {
                let resolved = zip_entry_name(Path::new(&base).join(unquote(href).split('#').next().unwrap_or(""))).unwrap_or_default();
                if !names.contains(resolved.as_str()) {
                    missing += 1;
                }
            }
        }
        bad_spine += doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "itemref").filter(|n| !ids.contains(n.attribute("idref").unwrap_or(""))).count();
    }
    Ok((missing, bad_spine))
}
