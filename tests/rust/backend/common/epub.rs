//! 임시 EPUB 만들기와 OPF/XML 편집 도구. cover_fixtures, epub_cover_set, classifier, classifier_reader, epub_line_count에
//! 따로 있던 `make_epub` 5개를 `EpubSpec` 하나로 합쳤다. 설계: docs/rust-test-seams.md 5장.
//!
//! 사용: `use common::epub::{EpubSpec, make_epub};`
//! - cover_fixtures `make_epub(path, version, cover)`  -> `make_epub(path, &EpubSpec::cover(version, cover))`
//! - classifier `make_epub(path, publisher)`            -> `make_epub(path, &EpubSpec::publisher(publisher))`
//! - classifier_reader `make_epub(path, docs, opf)`     -> `make_epub(path, &EpubSpec::docs(docs, opf))`
//! - epub_line_count `write_epub(path, bodies)`         -> `make_epub(path, &EpubSpec::chapters(bodies))`
//! - bm2 `_make_minimal_epub` 같은 뼈대                 -> `EpubSpec::minimal()` 뒤에 `without_*`, `with_extra`

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use super::image::red_jpeg;
use super::zipx::{write_zip_with, ZipMode};

pub const CONTAINER_XML: &str = r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#;

pub const BODY: &str = "<html><body>본문을 보존한다.</body></html>";

pub const CSS: &str = "body { color: red; }";

fn container_xml(opf_path: &str) -> String {
    format!(r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="{opf_path}" media-type="application/oebps-package+xml"/></rootfiles></container>"#)
}

/// 만들 EPUB의 모양. 생성자로 시작해 `with_*`/`without_*`로 고친다.
#[derive(Clone, Debug)]
pub struct EpubSpec {
    pub version: String,
    /// `cover.jpg`를 `cover-image`로 선언하고 멤버로 넣는다.
    pub cover: bool,
    pub title: String,
    pub author: Option<String>,
    pub publisher: Option<String>,
    /// 있으면 `dc:identifier`와 `dc:language`를 넣고 `unique-identifier`를 선언한다.
    pub identifier: Option<String>,
    pub language: Option<String>,
    /// OPF 멤버 경로. 챕터와 표지 경로는 OPF 디렉터리 기준이다.
    pub opf_path: String,
    /// 있으면 OPF를 생성하지 않고 이 문자열을 그대로 쓴다.
    pub opf_override: Option<String>,
    /// (OPF 디렉터리 기준 파일 이름, 본문). manifest id는 확장자를 뺀 이름이다.
    pub chapters: Vec<(String, String)>,
    pub include_mimetype: bool,
    pub include_container: bool,
    pub include_style: bool,
    /// 마지막에 덧붙이는 멤버(전체 경로).
    pub extra: Vec<(String, Vec<u8>)>,
    pub mode: ZipMode,
}

impl EpubSpec {
    /// 표지 유무와 EPUB 버전만 정하는 기본 구성. cover_fixtures의 `make_epub`과 같다(모든 멤버 STORED).
    pub fn cover(version: &str, cover: bool) -> Self {
        Self {
            version: version.to_string(),
            cover,
            title: "테스트 책".to_string(),
            author: Some("저자".to_string()),
            publisher: None,
            identifier: None,
            language: None,
            opf_path: "OPS/book.opf".to_string(),
            opf_override: None,
            chapters: vec![("chapter.xhtml".to_string(), BODY.to_string())],
            include_mimetype: true,
            include_container: true,
            include_style: true,
            extra: Vec::new(),
            mode: ZipMode::Stored,
        }
    }

    /// 출판사만 있는 OPF 하나짜리 ZIP. classifier의 `make_epub(path, publisher)`와 같다.
    pub fn publisher(publisher: &str) -> Self {
        let opf = format!(r#"<?xml version="1.0"?><package><metadata><dc:publisher>{publisher}</dc:publisher></metadata></package>"#);
        Self {
            opf_path: "content.opf".to_string(),
            opf_override: Some(opf),
            chapters: Vec::new(),
            include_mimetype: false,
            include_container: false,
            include_style: false,
            mode: ZipMode::Deflated,
            ..Self::cover("3.0", false)
        }
    }

    /// 이름과 본문이 정해진 파일들과 선택적 `content.opf`. classifier_reader의 `make_epub(path, docs, opf)`와 같다.
    pub fn docs(docs: &[(&str, &str)], opf: Option<&str>) -> Self {
        let mut spec = Self {
            opf_path: "content.opf".to_string(),
            opf_override: opf.map(str::to_string),
            chapters: Vec::new(),
            include_mimetype: false,
            include_container: false,
            include_style: false,
            mode: ZipMode::Deflated,
            ..Self::cover("3.0", false)
        };
        spec.extra = docs.iter().map(|(n, b)| (n.to_string(), b.as_bytes().to_vec())).collect();
        if opf.is_none() {
            // OPF를 아예 넣지 않는 경우다. 빈 override와 구분하려고 경로를 비운다.
            spec.opf_path = String::new();
        }
        spec
    }

    /// 본문 파일 `c0.xhtml`, `c1.xhtml`, ...을 spine에 늘어놓은 EPUB. epub_line_count의 `write_epub`과 같다.
    pub fn chapters(bodies: &[&str]) -> Self {
        Self {
            version: "3.0".to_string(),
            title: "행수 테스트".to_string(),
            author: None,
            identifier: Some("lines-test".to_string()),
            language: Some("ko".to_string()),
            opf_path: "content.opf".to_string(),
            chapters: bodies.iter().enumerate().map(|(i, b)| (format!("c{i}.xhtml"), b.to_string())).collect(),
            include_style: false,
            ..Self::cover("3.0", false)
        }
    }

    /// OPF가 루트에 있고 챕터 하나만 있는 가장 작은 유효 EPUB. 멤버를 빼고 더해서 손상 변형을 만든다.
    pub fn minimal() -> Self {
        Self { opf_path: "content.opf".to_string(), include_style: false, mode: ZipMode::EpubStyle, ..Self::cover("3.0", false) }
    }

    pub fn with_version(mut self, version: &str) -> Self {
        self.version = version.to_string();
        self
    }

    pub fn with_cover(mut self, cover: bool) -> Self {
        self.cover = cover;
        self
    }

    pub fn with_title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    pub fn with_publisher(mut self, publisher: &str) -> Self {
        self.publisher = Some(publisher.to_string());
        self
    }

    pub fn with_opf(mut self, opf: &str) -> Self {
        self.opf_override = Some(opf.to_string());
        self
    }

    pub fn with_opf_path(mut self, opf_path: &str) -> Self {
        self.opf_path = opf_path.to_string();
        self
    }

    pub fn with_chapter(mut self, name: &str, body: &str) -> Self {
        self.chapters.push((name.to_string(), body.to_string()));
        self
    }

    pub fn with_extra(mut self, name: &str, data: impl Into<Vec<u8>>) -> Self {
        self.extra.push((name.to_string(), data.into()));
        self
    }

    pub fn with_mode(mut self, mode: ZipMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn without_mimetype(mut self) -> Self {
        self.include_mimetype = false;
        self
    }

    pub fn without_container(mut self) -> Self {
        self.include_container = false;
        self
    }

    /// OPF 디렉터리. 루트면 빈 문자열, 아니면 끝에 `/`가 붙는다.
    fn base(&self) -> String {
        match self.opf_path.rfind('/') {
            Some(at) => self.opf_path[..=at].to_string(),
            None => String::new(),
        }
    }

    fn opf(&self) -> String {
        if let Some(opf) = &self.opf_override {
            return opf.clone();
        }
        let version = &self.version;
        let unique = if self.identifier.is_some() { r#" unique-identifier="id""# } else { "" };
        let mut metadata = String::new();
        if let Some(id) = &self.identifier {
            metadata.push_str(&format!(r#"<dc:identifier id="id">{id}</dc:identifier>"#));
        }
        metadata.push_str(&format!("<dc:title>{}</dc:title>", self.title));
        if let Some(author) = &self.author {
            metadata.push_str(&format!("<dc:creator>{author}</dc:creator>"));
        }
        if let Some(language) = &self.language {
            metadata.push_str(&format!("<dc:language>{language}</dc:language>"));
        }
        if let Some(publisher) = &self.publisher {
            metadata.push_str(&format!("<dc:publisher>{publisher}</dc:publisher>"));
        }
        let stem = |name: &str| name.rsplit_once('.').map_or(name.to_string(), |(s, _)| s.to_string());
        let mut manifest: String = self.chapters.iter().map(|(name, _)| format!(r#"<item id="{}" href="{name}" media-type="application/xhtml+xml"/>"#, stem(name))).collect();
        if self.cover {
            manifest.push_str(r#"<item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>"#);
        }
        let spine: String = self.chapters.iter().map(|(name, _)| format!(r#"<itemref idref="{}"/>"#, stem(name))).collect();
        format!(
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="{version}"{unique}><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">{metadata}</metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>"#
        )
    }

    /// 쓸 멤버를 순서대로 만든다. 저장하지 않고 내용만 보고 싶을 때도 쓴다.
    pub fn entries(&self) -> Vec<(String, Vec<u8>)> {
        let base = self.base();
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        if self.include_mimetype {
            entries.push(("mimetype".to_string(), b"application/epub+zip".to_vec()));
        }
        if self.include_container {
            let container = if self.opf_path == "OPS/book.opf" { CONTAINER_XML.to_string() } else { container_xml(&self.opf_path) };
            entries.push(("META-INF/container.xml".to_string(), container.into_bytes()));
        }
        if !self.opf_path.is_empty() {
            entries.push((self.opf_path.clone(), self.opf().into_bytes()));
        }
        for (name, body) in &self.chapters {
            entries.push((format!("{base}{name}"), body.clone().into_bytes()));
        }
        if self.include_style {
            entries.push((format!("{base}style.css"), CSS.as_bytes().to_vec()));
        }
        if self.cover {
            entries.push((format!("{base}cover.jpg"), red_jpeg()));
        }
        entries.extend(self.extra.iter().cloned());
        entries
    }
}

/// 임시 EPUB을 만들고 경로를 돌려준다. 부모 디렉터리가 없으면 만든다.
pub fn make_epub(path: &Path, spec: &EpubSpec) -> PathBuf {
    write_zip_with(path, &spec.entries().iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect::<Vec<_>>(), spec.mode);
    path.to_path_buf()
}

// ---- OPF/XML 편집 ----------------------------------------------------------------------------

/// `//*[local-name()=local]` 중 `attr`(이름, 값)을 만족하는 요소 수. Python `package.xpath('count(...)')`.
pub fn count_elements(xml: &[u8], local: &str, attr: Option<(&str, &str)>) -> usize {
    let text = std::str::from_utf8(xml).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == local).filter(|n| attr.is_none_or(|(k, v)| n.attribute(k) == Some(v))).count()
}

/// `//*[local-name()=local and @id=id]/@attr`. 없으면 빈 문자열.
pub fn attr_of_element_with_id(xml: &[u8], local: &str, id: &str, attr: &str) -> String {
    let text = std::str::from_utf8(xml).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.descendants().find(|n| n.is_element() && n.tag_name().name() == local && n.attribute("id") == Some(id)).and_then(|n| n.attribute(attr)).unwrap_or("").to_string()
}

/// `<opf:manifest>` 같은 요소 하나를 통째로 지운다. Python `package.remove(package.find(...))`.
pub fn remove_element(xml: &[u8], local: &str) -> Vec<u8> {
    let text = std::str::from_utf8(xml).unwrap();
    let open = format!("<{local}");
    let close = format!("</{local}>");
    let start = text.find(&open).unwrap_or_else(|| panic!("요소 없음: {local}"));
    // 자기 닫힘 요소는 `/>`로 끝난다.
    let end = text[start..].find(&close).map(|e| start + e + close.len()).unwrap_or_else(|| start + text[start..].find("/>").unwrap() + 2);
    format!("{}{}", &text[..start], &text[end..]).into_bytes()
}

/// `</parent>` 앞에 조각을 넣는다. Python `etree.SubElement(parent, ...)`.
pub fn append_child(xml: &[u8], parent: &str, fragment: &str) -> Vec<u8> {
    let text = std::str::from_utf8(xml).unwrap();
    let close = format!("</{parent}>");
    let at = text.find(&close).unwrap_or_else(|| panic!("요소 없음: {parent}"));
    format!("{}{}{}", &text[..at], fragment, &text[at..]).into_bytes()
}
