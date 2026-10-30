//! 테스트 공용 fixture. Python 테스트의 make_epub / _corrupt_local_header 대응.
#![allow(dead_code)]

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub const CLEAN_KOREAN_UNIT: &str = "소림사 장로가 강호에서 내공을 운기조식하며 화산파 검법을 수련했다. 그는 무림맹의 장문인으로서 제자들을 이끌었다. ";

pub fn clean_korean() -> String {
    CLEAN_KOREAN_UNIT.repeat(6)
}

/// 정상 한국어를 UTF-8로 저장한 뒤 CP949로 잘못 읽으면 나오는 바로 그 문자열.
pub fn corrupted() -> String {
    let utf8 = clean_korean().repeat(4).into_bytes();
    let (text, _, _) = encoding_rs::EUC_KR.decode(&utf8);
    // Python의 errors="ignore"처럼 읽을 수 없는 바이트는 버린다.
    text.chars().filter(|&c| c != '\u{FFFD}').collect()
}

pub const HANJA_CLASSIC_UNIT: &str = "孟子曰 仁義禮智 信也 天下之達道也 君子之道 費而隱 夫婦之愚 可以與知焉 ";
pub const ENGLISH_UNIT: &str = "The quick brown fox jumps over the lazy dog. ";
pub const KOREAN_WORD_LIST_UNIT: &str = "천상 지하 만물 유전 성쇠 흥망 강산 세월 인생 무상 일장 춘몽 기고 만장 ";

pub const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
pub const CONTAINER_XML: &str = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
pub const OPF_XML: &str = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><manifest><item id="c1" href="ch%201.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;

pub fn plain_chapter() -> String {
    format!("<?xml version=\"1.0\"?>\n<!DOCTYPE html [<!ELEMENT html ANY>]>\n<!-- <fake> -->\n<html xmlns=\"{XHTML_NS}\"><head><title/></head><body><p>본문</p></body></html>")
}

pub fn prefixed_chapter() -> String {
    format!("<?xml version=\"1.0\"?>\n<html:html xmlns:html=\"{XHTML_NS}\"><html:head><html:title/></html:head><html:body><html:p>본문</html:p></html:body></html:html>")
}

pub struct EpubSpec<'a> {
    pub chapter: String,
    pub mimetype_first: bool,
    pub container: Option<&'a str>,
    pub opf: Option<String>,
    pub chapter_name: Option<&'a str>,
}

impl Default for EpubSpec<'_> {
    fn default() -> Self {
        EpubSpec { chapter: plain_chapter(), mimetype_first: true, container: Some(CONTAINER_XML), opf: Some(OPF_XML.to_string()), chapter_name: Some("OEBPS/ch 1.xhtml") }
    }
}

fn stored() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
}

fn deflated() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

pub fn make_epub_with(path: &Path, spec: EpubSpec) {
    let mut z = ZipWriter::new(File::create(path).unwrap());
    if spec.mimetype_first {
        z.start_file("mimetype", stored()).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
    }
    if let Some(c) = spec.container {
        z.start_file("META-INF/container.xml", deflated()).unwrap();
        z.write_all(c.as_bytes()).unwrap();
    }
    if let Some(o) = &spec.opf {
        z.start_file("OEBPS/content.opf", deflated()).unwrap();
        z.write_all(o.as_bytes()).unwrap();
    }
    if let Some(n) = spec.chapter_name {
        z.start_file(n, deflated()).unwrap();
        z.write_all(spec.chapter.as_bytes()).unwrap();
    }
    if !spec.mimetype_first {
        z.start_file("mimetype", deflated()).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
    }
    z.finish().unwrap();
}

pub fn make_epub(path: &Path) {
    make_epub_with(path, EpubSpec::default());
}

/// (이름, 본문) 목록으로 단순 ZIP을 만든다. reader 테스트용.
pub fn make_zip(path: &Path, docs: &[(&str, &str)]) {
    let mut z = ZipWriter::new(File::create(path).unwrap());
    for (name, body) in docs {
        z.start_file(*name, deflated()).unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

pub fn make_zip_with_media(path: &Path, media_type: &str, member_name: &str, payload: &[u8]) {
    let opf = format!(
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest><item id="a" href="{member_name}" media-type="{media_type}"/></manifest><spine><itemref idref="a"/></spine></package>"#
    );
    let mut z = ZipWriter::new(File::create(path).unwrap());
    z.start_file("mimetype", stored()).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    z.start_file("META-INF/container.xml", deflated()).unwrap();
    z.write_all(CONTAINER_XML.as_bytes()).unwrap();
    z.start_file("OEBPS/content.opf", deflated()).unwrap();
    z.write_all(opf.as_bytes()).unwrap();
    z.start_file(format!("OEBPS/{member_name}"), deflated()).unwrap();
    z.write_all(payload).unwrap();
    z.finish().unwrap();
}

/// ZIP 중앙 디렉토리는 그대로 두고 멤버의 로컬 헤더 시그니처만 망가뜨린다(Bad magic number 재현).
pub fn corrupt_local_header(path: &Path, member: &str) {
    let mut data = fs::read(path).unwrap();
    let name = member.as_bytes();
    let sig = b"PK\x03\x04";
    let offset = (0..data.len().saturating_sub(30 + name.len())).find(|&i| &data[i..i + 4] == sig && &data[i + 30..i + 30 + name.len()] == name).expect("local header not found");
    data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]);
    fs::write(path, data).unwrap();
}

// ---- EPUB 보안 검사용 fixture ----

pub const XHTML_HEAD: &str = r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body>"#;

pub fn xhtml(body: &str) -> String {
    format!("{XHTML_HEAD}{body}</body></html>")
}

pub fn opf(items: &str) -> String {
    format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest>{items}</manifest><spine><itemref idref="c1"/></spine></package>"#)
}

pub fn opf_item(id: &str, href: &str, media: &str, props: &str) -> String {
    let p = if props.is_empty() { String::new() } else { format!(r#" properties="{props}""#) };
    format!(r#"<item id="{id}" href="{href}" media-type="{media}"{p}/>"#)
}

/// 표준 뼈대(mimetype, container, opf, 본문 한 장)에 파일을 더해 EPUB을 만든다. `files`는 (이름, 내용).
pub fn epub_with_files(path: &Path, opf_xml: &str, chapter: &str, extra: &[(&str, &[u8])]) {
    let mut z = ZipWriter::new(File::create(path).unwrap());
    z.start_file("mimetype", stored()).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    for (name, data) in [("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("OEBPS/content.opf", opf_xml.as_bytes()), ("OEBPS/ch1.xhtml", chapter.as_bytes())] {
        z.start_file(name, deflated()).unwrap();
        z.write_all(data).unwrap();
    }
    for (name, data) in extra {
        z.start_file(*name, deflated()).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap();
}

pub fn clean_opf() -> String {
    opf(&opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""))
}

pub fn scripted_opf() -> String {
    opf(&opf_item("c1", "ch1.xhtml", "application/xhtml+xml", "scripted"))
}

pub fn epub_with_chapter(path: &Path, chapter: &str) {
    epub_with_files(path, &clean_opf(), chapter, &[]);
}

/// 로컬 헤더의 파일명 첫 글자를 바꿔 중앙 디렉토리 이름과 어긋나게 만든다.
pub fn mangle_local_name(path: &Path, member: &str) {
    let mut data = fs::read(path).unwrap();
    let name = member.as_bytes();
    let off = (0..data.len() - 30 - name.len()).find(|&i| &data[i..i + 4] == b"PK\x03\x04" && &data[i + 30..i + 30 + name.len()] == name).expect("local header");
    data[off + 30] ^= 0x01;
    fs::write(path, data).unwrap();
}

// ---- 공통 검사용 fixture ----

/// 임시 디렉토리가 살아 있는 동안만 파일이 존재한다.
pub struct NamedFile {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
}

impl NamedFile {
    /// 이미 만든 임시 디렉토리와 그 안의 파일 경로를 묶는다.
    pub fn keep(dir: tempfile::TempDir, path: std::path::PathBuf) -> Self {
        NamedFile { _dir: dir, path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// 정상 EPUB 내용을 `name`이라는 이름으로 만든다. 확장자를 일부러 틀리게 줄 때 쓴다.
pub fn epub_named(name: &str) -> NamedFile {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    make_epub(&path);
    NamedFile { _dir: dir, path }
}

/// `name`이라는 이름으로 `bytes`를 담은 파일을 만든다.
pub fn file_with(name: &str, bytes: &[u8]) -> NamedFile {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, bytes).unwrap();
    NamedFile { _dir: dir, path }
}

/// MZ + e_lfanew(0x3C) -> "PE\0\0"인 최소 PE 머리.
pub fn minimal_pe() -> Vec<u8> {
    let mut v = vec![0u8; 0x44];
    v[..2].copy_from_slice(b"MZ");
    v[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    v[0x40..0x44].copy_from_slice(b"PE\0\0");
    v
}

// ---- 렌더링 검사용 fixture ----

/// mimetype(stored), container.xml을 앞에 두고 `members`(이름, 내용)를 deflate로 더한다.
/// `stored_names`에 든 이름은 압축하지 않고 그대로 쓴다.
pub fn epub_members(path: &Path, members: &[(&str, &[u8])], stored_names: &[&str]) {
    let mut z = ZipWriter::new(File::create(path).unwrap());
    z.start_file("mimetype", stored()).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    z.start_file("META-INF/container.xml", deflated()).unwrap();
    z.write_all(CONTAINER_XML.as_bytes()).unwrap();
    for (name, data) in members {
        z.start_file(*name, if stored_names.contains(name) { stored() } else { deflated() }).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap();
}

// ---------------------------------------------------------------------------
// Poppler 가짜 실행기
// ---------------------------------------------------------------------------

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use sysdeps::{CommandError, CommandOutput, CommandRunner};

enum Info {
    Pages(usize),
    NotFound,
    Fails(String),
    Timeout,
    Spawn(String),
}

/// 미리 정한 `pdfinfo`·`pdftotext` 결과를 돌려주고 호출을 기록한다. 실제 Poppler를 부르지 않는다.
pub struct FakeRunner {
    info: Info,
    text: String,
    calls: AtomicUsize,
    programs: Mutex<Vec<String>>,
}

impl FakeRunner {
    fn new(info: Info, text: &str) -> Self {
        FakeRunner { info, text: text.to_owned(), calls: AtomicUsize::new(0), programs: Mutex::new(Vec::new()) }
    }

    /// `pdfinfo`가 `n`쪽이라고 답하고 `pdftotext`는 본문이 있다고 답한다.
    pub fn pages(n: usize) -> Self {
        Self::new(Info::Pages(n), "본문 텍스트")
    }

    /// Poppler 실행 파일이 없다.
    pub fn not_found() -> Self {
        Self::new(Info::NotFound, "")
    }

    /// `pdfinfo`가 0이 아닌 종료 코드로 끝난다.
    pub fn fails(msg: &str) -> Self {
        Self::new(Info::Fails(msg.to_owned()), "")
    }

    /// 시간 상한을 넘겼다.
    pub fn timeout() -> Self {
        Self::new(Info::Timeout, "")
    }

    /// 프로세스를 띄우지 못했다(권한 등).
    pub fn spawn_error(msg: &str) -> Self {
        Self::new(Info::Spawn(msg.to_owned()), "")
    }

    /// 1쪽이고 `pdftotext`가 `s`를 돌려준다.
    pub fn text(s: &str) -> Self {
        Self::new(Info::Pages(1), s)
    }

    /// `n`쪽이고 `pdftotext`가 `s`를 돌려준다.
    pub fn pages_text(n: usize, s: &str) -> Self {
        Self::new(Info::Pages(n), s)
    }

    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn programs(&self) -> Vec<String> {
        self.programs.lock().unwrap().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, argv: &[String], _timeout: Option<Duration>) -> Result<CommandOutput, CommandError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let program = argv.first().cloned().unwrap_or_default();
        self.programs.lock().unwrap().push(program.clone());
        if program == "pdftotext" {
            return Ok(CommandOutput { returncode: 0, stdout: self.text.clone().into_bytes(), stderr: Vec::new() });
        }
        match &self.info {
            Info::Pages(n) => Ok(CommandOutput { returncode: 0, stdout: format!("Title: x\nPages:          {n}\n").into_bytes(), stderr: Vec::new() }),
            Info::NotFound => Err(CommandError::NotFound),
            Info::Timeout => Err(CommandError::Timeout),
            Info::Spawn(msg) => Err(CommandError::Failed(msg.clone())),
            Info::Fails(msg) => Ok(CommandOutput { returncode: 1, stdout: Vec::new(), stderr: msg.clone().into_bytes() }),
        }
    }

    fn which(&self, _program: &str) -> Option<PathBuf> {
        None
    }
}

// ---- ZIP 점검용 fixture ----

/// (이름, 바이트) 목록으로 deflate ZIP을 만든다.
pub fn zip_of(path: &Path, entries: &[(&str, &[u8])]) {
    let mut z = ZipWriter::new(File::create(path).unwrap());
    for (name, body) in entries {
        z.start_file(*name, deflated()).unwrap();
        z.write_all(body).unwrap();
    }
    z.finish().unwrap();
}

/// (이름, 바이트) 목록으로 무압축(stored) ZIP을 만든다. 바이트를 깨뜨려도 압축 오류가 나지 않아 CRC 시험에 쓴다.
pub fn zip_stored(path: &Path, entries: &[(&str, &[u8])]) {
    let mut z = ZipWriter::new(File::create(path).unwrap());
    for (name, body) in entries {
        z.start_file(*name, stored()).unwrap();
        z.write_all(body).unwrap();
    }
    z.finish().unwrap();
}

/// 멤버 본문(로컬 헤더 뒤)이 시작하는 파일 오프셋.
pub fn data_offset(bytes: &[u8], member: &str) -> usize {
    let name = member.as_bytes();
    let at = (0..bytes.len().saturating_sub(30 + name.len())).find(|&i| &bytes[i..i + 4] == b"PK\x03\x04" && &bytes[i + 30..i + 30 + name.len()] == name).expect("local header not found");
    let name_len = usize::from(u16::from_le_bytes([bytes[at + 26], bytes[at + 27]]));
    let extra_len = usize::from(u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]));
    at + 30 + name_len + extra_len
}

/// 멤버를 암호화된 것으로 표시한다(로컬·중앙 헤더의 일반 목적 플래그 bit 0). 비밀번호 API 없이 `encrypted()`를 참으로 만든다.
pub fn mark_encrypted(path: &Path, member: &str) {
    let mut b = fs::read(path).unwrap();
    let name = member.as_bytes();
    let local = (0..b.len() - 30 - name.len()).find(|&i| &b[i..i + 4] == b"PK\x03\x04" && &b[i + 30..i + 30 + name.len()] == name).unwrap();
    b[local + 6] |= 1;
    let central = (0..b.len() - 46 - name.len()).find(|&i| &b[i..i + 4] == b"PK\x01\x02" && &b[i + 46..i + 46 + name.len()] == name).unwrap();
    b[central + 8] |= 1;
    fs::write(path, b).unwrap();
}

// ---- DOCX 점검용 fixture ----

pub const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

pub const DOCX_CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
pub const DOCX_ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
pub const DOCX_DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>본문</w:t></w:r></w:p></w:body></w:document>"#;

/// 최소 유효 DOCX의 부분 목록.
pub fn minimal_docx_parts() -> Vec<(String, Vec<u8>)> {
    vec![
        ("[Content_Types].xml".into(), DOCX_CONTENT_TYPES.as_bytes().to_vec()),
        ("_rels/.rels".into(), DOCX_ROOT_RELS.as_bytes().to_vec()),
        ("word/document.xml".into(), DOCX_DOCUMENT.as_bytes().to_vec()),
    ]
}

/// 최소 DOCX에서 `remove`한 부분을 빼고 `add`한 부분을 더하거나(같은 이름이면 바꾸고) 만든다.
pub fn docx_with(path: &Path, remove: &[&str], add: &[(&str, &[u8])]) {
    let mut parts: Vec<(String, Vec<u8>)> = minimal_docx_parts().into_iter().filter(|(n, _)| !remove.contains(&n.as_str())).collect();
    for (name, body) in add {
        parts.retain(|(n, _)| n != name);
        parts.push((name.to_string(), body.to_vec()));
    }
    let refs: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    zip_of(path, &refs);
}

// ---- HWPX 점검용 fixture (합성: docs/bookdoctor-hwpx-notes.md의 설명에서 만들었고 실제 한글 파일로 검증하지 못했다) ----

pub fn minimal_hwpx_parts() -> Vec<(String, Vec<u8>)> {
    let part = |n: &str, b: &str| (n.to_string(), b.as_bytes().to_vec());
    vec![
        part("mimetype", "application/hwp+zip"),
        part("version.xml", r#"<?xml version="1.0" encoding="UTF-8"?><hv:HCFVersion xmlns:hv="http://www.hancom.co.kr/hwpml/2011/version" tagetApplication="WORDPROCESSOR"/>"#),
        part("META-INF/container.xml", r#"<?xml version="1.0" encoding="UTF-8"?><ocf:container xmlns:ocf="urn:oasis:names:tc:opendocument:xmlns:container"><ocf:rootfiles><ocf:rootfile full-path="Contents/content.hpf" media-type="application/hwpml-package+xml"/></ocf:rootfiles></ocf:container>"#),
        part("Contents/content.hpf", r#"<?xml version="1.0" encoding="UTF-8"?><opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="section0" href="Contents/section0.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="header"/><opf:itemref idref="section0"/></opf:spine></opf:package>"#),
        part("Contents/header.xml", r#"<?xml version="1.0" encoding="UTF-8"?><hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#),
        part("Contents/section0.xml", r#"<?xml version="1.0" encoding="UTF-8"?><hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><hp:p xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"/></hs:sec>"#),
    ]
}

/// 최소 HWPX에서 `remove`한 부분을 빼고 `add`한 부분을 더하거나 바꿔 만든다.
pub fn hwpx_with(path: &Path, remove: &[&str], add: &[(&str, &[u8])]) {
    let mut parts: Vec<(String, Vec<u8>)> = minimal_hwpx_parts().into_iter().filter(|(n, _)| !remove.contains(&n.as_str())).collect();
    for (name, body) in add {
        parts.retain(|(n, _)| n != name);
        parts.push((name.to_string(), body.to_vec()));
    }
    let refs: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    zip_of(path, &refs);
}
