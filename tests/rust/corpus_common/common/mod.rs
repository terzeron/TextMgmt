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
