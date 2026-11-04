//! `backend::epub_cover::epub_writer` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::fs;
use std::path::{Path, PathBuf};
use backend::epub_cover::{CoverError, EditImages, OPF_NS};
use common::zipx::{write_zip_with, ZipMode};
use serde_json::{json, Value};
use tempfile::TempDir;
use crate::common::shared_fixtures::{assert_refused, edit, full, jpeg};

type Members = Vec<(String, Vec<u8>)>;

/// mimetype을 맨 앞에 두고 나머지 순서는 그대로 둔다. 멤버는 모두 무압축으로 쓴다.
fn write_zip(path: &Path, mut members: Members) {
    let pos = members.iter().position(|(n, _)| n == "mimetype").expect("mimetype");
    let mimetype = members.remove(pos);
    members.insert(0, mimetype);
    let entries: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    write_zip_with(path, &entries, ZipMode::Stored);
}

/// `book` fixture: (책 경로, 휴지통 경로)
struct Book {
    path: PathBuf,
    trash: PathBuf,
    _tmp: TempDir,
}

impl Book {
    fn edit(&self, args: EditImages) -> Result<Value, CoverError> {
        edit(&self.path, &self.trash, args)
    }
}

// ── 한 쪽에 이미지가 여러 장인 책(실제 책처럼 xmlns 없는 비정형 HTML) ─────────────────────

const MULTI_PAGE: &str = concat!(
    "<html>\n  <head>\n    <title>엄마의 말뚝</title>\n  </head>\n    <body><li class=\"chapter\">\n",
    "<p><img alt=\"image\" height=\"597\" src=\"../Images/m1.jpg\" width=\"412\"/></p>\n",
    "<p><img alt=\"image\" height=\"597\" src=\"../Images/m2.jpg\" width=\"412\"/></p>\n",
    "</li></body></html>"
);

fn make_multi(path: &Path, page: &[u8]) -> PathBuf {
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
fn multi() -> Book {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_multi(&tmp.path().join("multi.epub"), MULTI_PAGE.as_bytes());
    let trash = tmp.path().join("0_trash");
    Book { path, trash, _tmp: tmp }
}

#[test]
fn test_index_out_of_range_is_refused() {
    let b = multi();
    let original = fs::read(&b.path).unwrap();
    for kind in ["crops", "splits", "deletes"] {
        let mut entry = json!({"idref": "multi", "index": 2});
        match kind {
            "crops" => entry["rect"] = full(),
            "splits" => {
                entry["first"] = full();
                entry["second"] = full();
            }
            _ => {}
        }
        let mut args = EditImages::default();
        match kind {
            "crops" => args.crops = vec![entry],
            "splits" => args.splits = vec![entry],
            _ => args.deletes = vec![entry],
        }
        assert_refused(b.edit(args), "이미지를 찾을 수 없음", kind);
    }
    assert_eq!(fs::read(&b.path).unwrap(), original);
}
