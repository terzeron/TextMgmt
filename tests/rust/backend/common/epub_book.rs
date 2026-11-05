//! 표지·이미지 편집 테스트의 EPUB 책 준비물(`Book`, `make_book`, `multi` 등). 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use super::shared_fixtures::{MULTI_PAGE, edit, gif, jpeg, png};
use super::zipx::{Members as ZipMembers, ZipMode, write_zip_with};
use backend::epub_cover::{CoverError, EditImages, EpubRewrite, OPF_NS};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// `book` fixture: (책 경로, 휴지통 경로)
pub struct Book {
    pub path: PathBuf,
    pub trash: PathBuf,
    pub _tmp: TempDir,
}

impl Book {
    pub fn edit(&self, args: EditImages) -> Result<EpubRewrite, CoverError> {
        edit(&self.path, &self.trash, args)
    }
}

pub type Members = Vec<(String, Vec<u8>)>;

pub fn book() -> Book {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("책").join("book.epub"), &[]);
    let trash = tmp.path().join("0_trash");
    Book { path, trash, _tmp: tmp }
}

/// p1·p2·p3 는 이미지만 있는 쪽, text 는 글만 있는 쪽, two 는 이미지 둘, nav 는 p3 를 가리키는 목차.
pub fn make_book(path: &Path, extra: &[(&str, Vec<u8>)]) -> PathBuf {
    let opf = format!(
        r##"<package xmlns="{OPF_NS}" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>책</dc:title></metadata><manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="p1" href="p1.xhtml" media-type="application/xhtml+xml"/><item id="p2" href="p2.xhtml" media-type="application/xhtml+xml"/>
<item id="p3" href="p3.xhtml" media-type="application/xhtml+xml"/><item id="text" href="text.xhtml" media-type="application/xhtml+xml"/>
<item id="two" href="two.xhtml" media-type="application/xhtml+xml"/><item id="shared" href="shared.xhtml" media-type="application/xhtml+xml"/>
<item id="gif" href="gif.xhtml" media-type="application/xhtml+xml"/><item id="mixed" href="mixed.xhtml" media-type="application/xhtml+xml"/>
<item id="i1" href="img/1.jpg" media-type="image/jpeg"/><item id="i2" href="img/2.png" media-type="image/png"/>
<item id="i3" href="img/3.jpg" media-type="image/jpeg"/><item id="i4" href="img/4.jpg" media-type="image/jpeg"/>
<item id="ig" href="img/g.gif" media-type="image/gif"/></manifest>
<spine><itemref idref="p1"/><itemref idref="p2"/><itemref idref="p3"/><itemref idref="text"/><itemref idref="two"/><itemref idref="shared"/><itemref idref="gif"/><itemref idref="mixed"/></spine></package>"##
    );
    let mut members: Members = vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        ("META-INF/container.xml".into(), br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#.to_vec()),
        ("OPS/book.opf".into(), opf.into_bytes()),
        (
            "OPS/nav.xhtml".into(),
            br#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="p3.xhtml">3</a></li></ol></nav></body></html>"#
                .to_vec(),
        ),
        ("OPS/p1.xhtml".into(), page(&["img/1.jpg"], "").into_bytes()),
        ("OPS/p2.xhtml".into(), page(&["img/2.png"], "").into_bytes()),
        ("OPS/p3.xhtml".into(), page(&["img/3.jpg"], "").into_bytes()),
        ("OPS/text.xhtml".into(), page(&[], "본문 글").into_bytes()),
        ("OPS/two.xhtml".into(), page(&["img/4.jpg", "img/5.jpg"], "").into_bytes()),
        ("OPS/shared.xhtml".into(), page(&["img/4.jpg"], "").into_bytes()),
        ("OPS/gif.xhtml".into(), page(&["img/g.gif"], "").into_bytes()),
        ("OPS/mixed.xhtml".into(), page(&["img/5.jpg"], "설명 글").into_bytes()),
        ("OPS/img/1.jpg".into(), jpeg("red")),
        ("OPS/img/2.png".into(), png("green")),
        ("OPS/img/3.jpg".into(), jpeg("blue")),
        ("OPS/img/4.jpg".into(), jpeg("yellow")),
        ("OPS/img/5.jpg".into(), jpeg("pink")),
        ("OPS/img/g.gif".into(), gif()),
    ];
    for (name, data) in extra {
        match members.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = data.clone(),
            None => members.push((name.to_string(), data.clone())),
        }
    }
    write_zip(path, members);
    path.to_path_buf()
}

pub fn make_multi(path: &Path, page: &[u8]) -> PathBuf {
    let opf = format!(
        concat!(
            r#"<package xmlns="{ns}" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">x</dc:identifier><dc:title>t</dc:title></metadata>"#,
            r#"<manifest><item id="multi" href="Text/multi.xhtml" media-type="application/xhtml+xml"/><item id="t" href="Text/t.xhtml" media-type="application/xhtml+xml"/>"#,
            r#"<item id="m1" href="Images/m1.jpg" media-type="image/jpeg"/><item id="m2" href="Images/m2.jpg" media-type="image/jpeg"/></manifest>"#,
            r#"<spine><itemref idref="multi"/><itemref idref="t"/></spine></package>"#,
        ),
        ns = OPF_NS
    );
    let members: Members = vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        (
            "META-INF/container.xml".into(),
            br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#.to_vec(),
        ),
        ("OEBPS/content.opf".into(), opf.into_bytes()),
        ("OEBPS/Text/multi.xhtml".into(), page.to_vec()),
        ("OEBPS/Text/t.xhtml".into(), "<html><body><p>다른 쪽</p></body></html>".as_bytes().to_vec()),
        ("OEBPS/Images/m1.jpg".into(), jpeg("red")),
        ("OEBPS/Images/m2.jpg".into(), jpeg("green")),
    ];
    write_zip(path, members);
    path.to_path_buf()
}

/// `multi` fixture
pub fn multi() -> Book {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_multi(&tmp.path().join("multi.epub"), MULTI_PAGE.as_bytes());
    let trash = tmp.path().join("0_trash");
    Book { path, trash, _tmp: tmp }
}

pub fn page(images: &[&str], text: &str) -> String {
    let body: String = images.iter().map(|src| format!(r#"<div><img src="{src}"/></div>"#)).collect();
    format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body>{text}{body}</body></html>"#)
}

/// mimetype을 맨 앞에 두고 나머지 순서는 그대로 둔다. 멤버는 모두 무압축으로 쓴다.
pub fn write_zip(path: &Path, mut members: Members) {
    let pos = members.iter().position(|(n, _)| n == "mimetype").expect("mimetype");
    let mimetype = members.remove(pos);
    members.insert(0, mimetype);
    let entries: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    write_zip_with(path, &entries, ZipMode::Stored);
}

pub fn read(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let members = ZipMembers::read(path);
    members.names().into_iter().map(|name| (name.clone(), members.get(&name).to_vec())).collect()
}
