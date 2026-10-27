use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use doc_reader::epub::{EpubPath, pretty_text, read_from_epub};
use zip::write::SimpleFileOptions;

#[path = "corpus.rs"]
mod corpus;

/// 이름→내용 목록으로 EPUB(ZIP)을 만든다.
fn build(dir: &Path, name: &str, members: &[(&str, &str)]) -> PathBuf {
    let path = dir.join(name);
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (member, content) in members {
        zip.start_file(*member, SimpleFileOptions::default()).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

const CONTAINER: &str = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;

fn opf(manifest: &str, spine_attrs: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>제목 책</dc:title><dc:creator>저자</dc:creator><dc:creator></dc:creator></metadata><manifest>{manifest}</manifest><spine {spine_attrs}><itemref idref="c1"/></spine></package>"#
    )
}

#[test]
fn ebooklib_path_collects_metadata_then_chapter_text() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest =
        r#"<item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/><item id="img" href="a.png" media-type="image/png"/>"#;
    let path = build(
        tmp.path(),
        "a.epub",
        &[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", &opf(manifest, "")),
            ("OEBPS/ch1.xhtml", "<html><body><p>첫째 문단.</p><p>둘째 문단.</p></body></html>"),
            ("OEBPS/ch2.xhtml", "<html><body><div>다음 장 내용</div></body></html>"),
            ("OEBPS/a.png", "x"),
        ],
    );
    let read = read_from_epub(&path);
    assert_eq!(read.path, EpubPath::Normal);
    assert!(read.errors.is_empty(), "{:?}", read.errors);
    // 제목, 저자 순서. 저자와 본문 사이에는 구분 공백이 없다(Python `result += text`). 자식이 요소뿐인 body는 줄바꿈이 끼어 문단이 붙지 않는다.
    assert_eq!(read.summary, " 제목 책 저자첫째 문단 \n둘째 문단 \n다음 장 내용\n");
    assert_eq!(read.line_count, 3);
}

#[test]
fn a_missing_manifest_member_switches_to_the_regex_path() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = r#"<item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="img" href="missing.png" media-type="image/png"/>"#;
    let path = build(
        tmp.path(),
        "b.epub",
        &[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", &opf(manifest, "")), ("OEBPS/ch1.xhtml", "<html><head><title>머리</title></head><body><p>본문</p></body></html>")],
    );
    let read = read_from_epub(&path);
    assert_eq!(read.path, EpubPath::Zipped);
    // 대체 경로는 메타데이터를 읽지 않고 문서 전체(머리 제목 포함)의 글을 쓴다.
    assert_eq!(read.summary, "머리본문");
    assert_eq!(read.line_count, 1);
}

#[test]
fn spine_toc_must_point_to_a_parsable_ncx() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = r#"<item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#;
    let ncx_ok = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="p"><navLabel><text>t</text></navLabel><content src="ch1.xhtml"/></navPoint></navMap></ncx>"#;
    let members = |ncx: &'static str| vec![("META-INF/container.xml", CONTAINER), ("OEBPS/ch1.xhtml", "<html><body><p>본문</p></body></html>"), ("OEBPS/toc.ncx", ncx)];
    let mut ok = members(ncx_ok);
    let opf_text = opf(manifest, r#"toc="ncx""#);
    ok.push(("OEBPS/content.opf", &opf_text));
    assert_eq!(read_from_epub(&build(tmp.path(), "ok.epub", &ok)).path, EpubPath::Normal);

    for bad in ["<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\"><navMap><navPoint><navLabel/></navPoint></navMap></ncx>", "<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\"></ncx>", "not xml"]
    {
        let mut m = members(bad);
        m.push(("OEBPS/content.opf", &opf_text));
        assert_eq!(read_from_epub(&build(tmp.path(), "bad.epub", &m)).path, EpubPath::Zipped, "{bad}");
    }
}

#[test]
fn a_nav_document_without_a_toc_nav_switches_to_the_regex_path() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = r#"<item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#;
    let good_nav = r#"<html xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="ch1.xhtml">1</a></li></ol></nav></body></html>"#;
    let opf_text = opf(manifest, "");
    let base = |nav: &'static str| {
        vec![("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", opf_text.as_str()), ("OEBPS/ch1.xhtml", "<html><body><p>본문</p></body></html>"), ("OEBPS/nav.xhtml", nav)]
    };
    assert_eq!(read_from_epub(&build(tmp.path(), "n1.epub", &base(good_nav))).path, EpubPath::Normal);
    let no_toc = r#"<html><body><nav><ol><li><a href="x">1</a></li></ol></nav></body></html>"#;
    assert_eq!(read_from_epub(&build(tmp.path(), "n2.epub", &base(no_toc))).path, EpubPath::Zipped);
    let no_href = r#"<html><body><nav epub:type="toc"><ol><li><a>1</a></li></ol></nav></body></html>"#;
    assert_eq!(read_from_epub(&build(tmp.path(), "n3.epub", &base(no_href))).path, EpubPath::Zipped);
}

#[test]
fn broken_zips_and_missing_containers_report_errors_and_return_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let not_zip = tmp.path().join("x.epub");
    fs::write(&not_zip, b"not a zip").unwrap();
    let read = read_from_epub(&not_zip);
    assert_eq!((read.summary.as_str(), read.line_count, read.path), ("", 0, EpubPath::Zipped));
    assert!(!read.errors.is_empty());

    let no_container = build(tmp.path(), "nc.epub", &[("mimetype", "application/epub+zip")]);
    let read = read_from_epub(&no_container);
    assert_eq!(read.summary, "");
    assert!(read.errors.iter().any(|e| e.contains("container.xml")), "{:?}", read.errors);
}

#[test]
fn pretty_text_adds_newlines_only_in_element_only_containers() {
    // 자식이 요소뿐인 body: 여는 태그 뒤·자식 사이·마지막 자식 뒤에 줄바꿈. 문서 앞 공백은 버린다.
    assert_eq!(pretty_text("<html><body><p>a</p><p>b</p></body></html>").as_deref(), Some("a\nb\n"));
    // 텍스트 자식이 있으면 안쪽은 들여쓰기하지 않는다.
    assert_eq!(pretty_text("<html><body>t<p>a</p><p>b</p></body></html>").as_deref(), Some("t\n".trim_end_matches('\n')).map(|_| "tab").or(Some("tab")));
    // 속성이 있는 body는 태그를 떼지 않으므로 앞 줄바꿈이 남는다.
    assert_eq!(pretty_text("<html><body class='c'><p>a</p></body></html>").as_deref(), Some("\na\n"));
    // 중첩된 요소뿐인 컨테이너: 안쪽 div의 여는 태그 뒤 줄바꿈은 앞 공백이 아니므로 남는다(Python lxml 기준).
    assert_eq!(pretty_text("<body><div><span>a</span><span>b</span></div></body>").as_deref(), Some("\na\nb\n\n"));
    // 본문이 텍스트뿐이면 None(원본 내용을 쓴다)
    assert_eq!(pretty_text("<body>only text</body>"), None);
}

#[test]
fn corpus_epub_matches_python() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_epub.jsonl")) {
        if rec.get("error").is_some() {
            continue;
        }
        let path = rec["path"].as_str().unwrap();
        let read = read_from_epub(&root.join(path));
        assert_eq!(corpus::fnv1a64(&read.summary), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(read.line_count as i64, rec["line_count"].as_i64().unwrap(), "line count for {path}");
    }
}
