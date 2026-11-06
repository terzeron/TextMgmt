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

#[cfg(test)]
mod tests {
    use super::*;

    const OPF_NS: &str = "http://www.idpf.org/2007/opf";

    fn member(name: &str, data: &[u8]) -> Member {
        Member { name: name.to_string(), data: data.to_vec(), method: CompressionMethod::Deflated, modified: None }
    }

    /// `Result::unwrap_err`는 `Ok` 쪽에도 `Debug`를 요구하는데 `Enc`와 `Member`에는 없다.
    fn err<T>(result: Result<T, String>) -> String {
        match result {
            Ok(_) => panic!("오류여야 한다"),
            Err(e) => e,
        }
    }

    fn opf(manifest: &str, spine: &str) -> String {
        format!(r#"<?xml version="1.0"?><package xmlns="{OPF_NS}" version="3.0"><manifest>{manifest}</manifest><spine>{spine}</spine></package>"#)
    }

    fn container(full_path: &str) -> Vec<u8> {
        format!(r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="{full_path}" media-type="application/oebps-package+xml"/></rootfiles></container>"#).into_bytes()
    }

    // ---- 인코딩 ----

    #[test]
    fn ext_of_lowercases_the_last_extension_of_the_file_name() {
        assert_eq!(ext_of("OEBPS/Text/Ch1.XHTML"), "xhtml");
        assert_eq!(ext_of("a.tar.gz"), "gz");
        assert_eq!(ext_of("dir.d/noext"), "");
        assert_eq!(ext_of("mimetype"), "");
    }

    #[test]
    fn decode_enc_reads_utf8_and_bom_utf16() {
        let (text, enc) = decode_enc("a", "가나".as_bytes()).unwrap();
        assert!(text == "가나" && matches!(enc, Enc::Utf8));
        let (text, enc) = decode_enc("a", b"\xEF\xBB\xBFx").unwrap();
        assert!(text == "\u{feff}x" && matches!(enc, Enc::Utf8), "UTF-8 BOM은 글에 남는다");
        let (text, enc) = decode_enc("a", b"\xFF\xFEa\x00").unwrap();
        assert!(text == "a" && matches!(enc, Enc::Utf16 { le: true }));
        let (text, enc) = decode_enc("a", b"\xFE\xFF\x00a").unwrap();
        assert!(text == "a" && matches!(enc, Enc::Utf16 { le: false }));
    }

    #[test]
    fn decode_enc_refuses_what_it_cannot_write_back_safely() {
        assert!(err(decode_enc("a", b"\xFF\xFE\x00\x00x\x00\x00\x00")).contains("UTF-32"));
        assert!(err(decode_enc("a", b"\x00\x00\xFE\xFF")).contains("UTF-32"));
        assert!(err(decode_enc("a", b"\xB0\xA1")).contains("UTF-8이 아니라"));
        assert!(err(decode_enc("a", b"\xFF\xFE\x00\xD8")).contains("UTF-16이 올바르지 않아"), "짝 없는 서로게이트");
        assert!(decode("a.xhtml", b"\xB0\xA1").unwrap_err().starts_with("a.xhtml:"));
    }

    #[test]
    fn encode_enc_round_trips_each_encoding() {
        for (enc, expect_prefix) in [(Enc::Utf8, &b""[..]), (Enc::Utf16 { le: true }, b"\xFF\xFE"), (Enc::Utf16 { le: false }, b"\xFE\xFF")] {
            let bytes = encode_enc("가a나", enc);
            assert!(bytes.starts_with(expect_prefix));
            assert_eq!(decode("a", &bytes).unwrap(), "가a나");
        }
    }

    // ---- 멤버 입출력 ----

    #[test]
    fn write_members_puts_mimetype_first_stored_and_keeps_the_rest() {
        let mut members = vec![member("OEBPS/a.xhtml", &b"<p>x</p>".repeat(50)), member("mimetype", b"application/epub+zip")];
        let bytes = write_members(&mut members).unwrap();
        let read = read_members(&bytes).unwrap();
        assert_eq!(read.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["mimetype", "OEBPS/a.xhtml"]);
        assert_eq!(read[0].method, CompressionMethod::Stored);
        assert_eq!(read[1].method, CompressionMethod::Deflated);
        assert_eq!(read[1].data, b"<p>x</p>".repeat(50));
    }

    #[test]
    fn write_members_keeps_a_stored_member_stored() {
        let mut stored = member("a.bin", b"abc");
        stored.method = CompressionMethod::Stored;
        let read = read_members(&write_members(&mut [stored]).unwrap()).unwrap();
        assert_eq!(read[0].method, CompressionMethod::Stored);
    }

    #[test]
    fn write_members_preserves_the_modified_time() {
        let mut m = member("a.txt", b"x");
        m.modified = Some(zip::DateTime::from_date_and_time(2020, 5, 17, 10, 20, 30).unwrap());
        let read = read_members(&write_members(&mut [m]).unwrap()).unwrap();
        assert_eq!(read[0].modified, Some(zip::DateTime::from_date_and_time(2020, 5, 17, 10, 20, 30).unwrap()));
    }

    #[test]
    fn read_members_rejects_bytes_that_are_not_a_zip() {
        assert!(err(read_members(b"not a zip")).starts_with("ZIP을 열 수 없다"));
    }

    #[test]
    fn a_member_with_a_bad_checksum_is_broken_and_the_others_survive() {
        let mut stored = member("a.txt", b"hello world");
        stored.method = CompressionMethod::Stored;
        let mut bytes = write_members(&mut [stored, member("b.txt", b"fine")]).unwrap();
        let at = bytes.windows(11).position(|w| w == b"hello world").unwrap();
        bytes[at] = b'j';
        let (members, broken) = read_members_lenient(&bytes).unwrap();
        assert_eq!(broken.len(), 1, "{broken:?}");
        assert_eq!(broken[0].0, "a.txt");
        assert!(members.iter().any(|m| m.name == "b.txt"));
        assert!(read_members(&bytes).is_err(), "read_members는 깨진 멤버를 오류로 올린다");
    }

    #[test]
    fn undecodable_marks_the_nul_prefix_only() {
        assert!(undecodable("\0이름.xhtml"));
        assert!(!undecodable("a.xhtml"));
    }

    // ---- 패키지 문서 ----

    #[test]
    fn package_names_finds_opf_container_target_and_package_root_documents() {
        let members = [
            member("META-INF/container.xml", &container("OEBPS/main.xml")),
            member("OEBPS/main.xml", opf("", "").as_bytes()),
            member("OEBPS/extra.opf", opf("", "").as_bytes()),
            member("OEBPS/hidden", opf("", "").as_bytes()),
            member("OEBPS/style.xml", b"<style/>"),
            member("OEBPS/ch1.xhtml", b"<package/>"),
        ];
        let names: Vec<_> = package_names(&members).unwrap().into_iter().collect();
        assert_eq!(names, ["OEBPS/extra.opf", "OEBPS/hidden", "OEBPS/main.xml"]);
    }

    #[test]
    fn package_names_accepts_a_percent_encoded_full_path() {
        let members = [member("META-INF/container.xml", &container("OEBPS/my%20book.opf")), member("OEBPS/my book.opf", opf("", "").as_bytes())];
        assert!(package_names(&members).unwrap().contains("OEBPS/my book.opf"));
    }

    #[test]
    fn package_names_fails_when_container_xml_cannot_be_trusted() {
        let missing = [member("META-INF/container.xml", &container("OEBPS/none.opf"))];
        assert!(package_names(&missing).unwrap_err().contains("가리키는 패키지 문서가 없다"));
        let broken = [member("META-INF/container.xml", b"<container>")];
        assert!(package_names(&broken).unwrap_err().contains("해석할 수 없다"));
        let encoding = [member("META-INF/container.xml", b"\xB0\xA1")];
        assert!(package_names(&encoding).unwrap_err().contains("인코딩을 읽을 수 없다"));
    }

    #[test]
    fn package_text_returns_the_text_without_a_leading_bom() {
        let mut data = b"\xEF\xBB\xBF".to_vec();
        data.extend(opf("", "").as_bytes());
        assert_eq!(package_text(&member("a.opf", &data)).unwrap(), opf("", ""));
    }

    #[test]
    fn package_text_rejects_anything_that_is_not_a_readable_package_document() {
        let exec = package_text(&member("p.bin", opf("", "").as_bytes())).unwrap_err();
        assert!(exec.contains("실행 파일 확장자"), "{exec}");
        assert!(package_text(&member("p.opf", b"\xB0\xA1")).unwrap_err().contains("UTF-8이 아니라"));
        assert!(package_text(&member("p.opf", b"<html/>")).unwrap_err().contains("`<package>` 문서가 아니다"));
        assert!(package_text(&member("p.opf", b"<package>")).unwrap_err().contains("패키지 문서를 해석할 수 없다"));
        assert!(package_text(&member("\0p.opf", b"<html/>")).unwrap_err().starts_with("p.opf:"), "NUL 표지는 오류 이름에서 뗀다");
    }

    // ---- URL ----

    #[test]
    fn normalize_url_reads_like_a_browser() {
        assert_eq!(normalize_url("  JavaScript:\talert(1)"), "javascript:alert(1)");
        assert_eq!(normalize_url("jav&#97;script:x"), "javascript:x");
        assert_eq!(normalize_url("/\\evil"), "//evil");
        assert_eq!(normalize_url("\\/evil"), "//evil");
    }

    #[test]
    fn scheme_of_accepts_only_rfc_shaped_schemes() {
        assert_eq!(scheme_of("http://x"), Some("http"));
        assert_eq!(scheme_of("svn+ssh://x"), Some("svn+ssh"));
        for n in ["no colon", ":x", "1a:x", "a b:c", "a_b:c"] {
            assert_eq!(scheme_of(n), None, "{n}");
        }
    }

    #[test]
    fn local_refs_have_no_scheme_and_are_not_network_paths() {
        for href in ["ch1.xhtml", "../img/a.png", "ch1.xhtml#top", "sub/a b.png"] {
            assert!(is_local_ref(href), "{href}");
        }
        for href in ["http://x/a", "//host/x", "/\\evil", "\\\\host\\share", "\\\\host", "javascript:1", "&#92;&#92;host"] {
            assert!(!is_local_ref(href), "{href}");
        }
    }

    // ---- 재구성 검증 ----

    #[test]
    fn visible_text_drops_markup_scripts_and_whitespace() {
        assert_eq!(visible_text("<p>a b</p>\n<script>x()</script><p>c</p>"), "abc");
    }

    #[test]
    fn visible_text_keeps_the_body_after_a_self_closing_script() {
        assert_eq!(visible_text(r#"<p>a</p><script src="x.js"/><p>b</p><style/><p>c</p>"#), "abc");
    }

    #[test]
    fn is_subsequence_keeps_order_but_allows_gaps() {
        assert!(is_subsequence("ac", "abc"));
        assert!(is_subsequence("", "x"));
        assert!(!is_subsequence("ca", "abc"));
        assert!(!is_subsequence("abcd", "abc"));
    }

    #[test]
    fn dangling_counts_missing_manifest_targets_and_unknown_spine_ids() {
        let manifest = r#"<item id="ch1" href="ch1.xhtml#a" media-type="application/xhtml+xml"/><item id="gone" href="gone.xhtml" media-type="application/xhtml+xml"/><item id="web" href="http://example.com/x.css" media-type="text/css"/>"#;
        let spine = r#"<itemref idref="ch1"/><itemref idref="ghost"/>"#;
        let members = [member("OEBPS/content.opf", opf(manifest, spine).as_bytes()), member("OEBPS/ch1.xhtml", b"<p>x</p>")];
        assert_eq!(dangling(&members).unwrap(), (1, 1));
    }

    #[test]
    fn dangling_is_zero_for_a_consistent_package_and_fails_on_a_broken_one() {
        let ok = [member("content.opf", opf(r#"<item id="a" href="a.xhtml"/>"#, r#"<itemref idref="a"/>"#).as_bytes()), member("a.xhtml", b"x")];
        assert_eq!(dangling(&ok).unwrap(), (0, 0));
        let bad = [member("content.opf", b"<package><manifest>")];
        assert!(dangling(&bad).is_err());
    }
}
