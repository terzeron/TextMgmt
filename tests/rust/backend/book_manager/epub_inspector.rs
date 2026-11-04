//! `backend::book_manager::epub_inspector` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use backend::epub_cover::{CoverError, edit_images, edit_images_with, EditImages, EpubCoverHooks, Limits, OPF_NS, revision};
use common::zipx::{write_zip_with, ZipMode};
use serde_json::{json, Value};
use tempfile::TempDir;
use common::epub::{EpubSpec, make_epub};
use doc_reader::TEXT_SIZE;
use doc_reader::epub::{EpubHooks, EpubPath, EpubRead, read_from_epub, read_from_epub_with, read_from_epub_with_extracting_zip};
use std::fs::File;
use std::time::Duration;
use backend::book_manager::BookManager;
use common::faulty_fs::FaultyFs;
use common::fsx::set_mtime_ago;
use crate::common::shared_fixtures::{assert_refused, edit, full, gif, jpeg, png};

fn page(images: &[&str], text: &str) -> String {
    let body: String = images.iter().map(|src| format!(r#"<div><img src="{src}"/></div>"#)).collect();
    format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body>{text}{body}</body></html>"#)
}

type Members = Vec<(String, Vec<u8>)>;

/// mimetype을 맨 앞에 두고 나머지 순서는 그대로 둔다. 멤버는 모두 무압축으로 쓴다.
fn write_zip(path: &Path, mut members: Members) {
    let pos = members.iter().position(|(n, _)| n == "mimetype").expect("mimetype");
    let mimetype = members.remove(pos);
    members.insert(0, mimetype);
    let entries: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    write_zip_with(path, &entries, ZipMode::Stored);
}

/// p1·p2·p3 는 이미지만 있는 쪽, text 는 글만 있는 쪽, two 는 이미지 둘, nav 는 p3 를 가리키는 목차.
fn make_book(path: &Path, extra: &[(&str, Vec<u8>)]) -> PathBuf {
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

/// `book` fixture: (책 경로, 휴지통 경로)
struct Book {
    path: PathBuf,
    trash: PathBuf,
    _tmp: TempDir,
}

fn book() -> Book {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_book(&tmp.path().join("책").join("book.epub"), &[]);
    let trash = tmp.path().join("0_trash");
    Book { path, trash, _tmp: tmp }
}

impl Book {
    fn edit(&self, args: EditImages) -> Result<Value, CoverError> {
        edit(&self.path, &self.trash, args)
    }
}

#[test]
fn test_empty_and_duplicate_requests_are_rejected() {
    let b = book();
    assert_refused(b.edit(EditImages::default()), "편집이 없습니다", "빈 요청");
    assert_refused(b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], deletes: vec![json!("p1")], ..EditImages::default() }), "두 번", "중복 요청");
}

#[test]
fn test_stale_revision_is_rejected_and_backup_is_kept_once() {
    let b = book();
    let stale = edit_images(&b.path, "stale", &b.trash, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() });
    assert_refused(stale, "변경", "stale");
    let original = fs::read(&b.path).unwrap();
    b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
    b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
    assert_eq!(fs::read(b.trash.join("book.epub")).unwrap(), original);
}

// ── 입력 검증과 상한(Python보다 엄격한 부분) ───────────────────────────────────

fn edit_with(b: &Book, hooks: &EpubCoverHooks, args: EditImages) -> Result<Value, CoverError> {
    edit_images_with(&b.path, &revision(&b.path), &b.trash, EditImages { backup_name: Some("book.epub".to_string()), ..args }, hooks)
}

#[test]
fn test_oversized_image_is_refused_before_decoding() {
    let b = book();
    let original = fs::read(&b.path).unwrap();
    let observer = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = observer.clone();
    let observe = move || {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    };
    // 바이트 크기: 읽기 전에 거절한다.
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_image_bytes: 10, ..Limits::default() }).with_pixel_decode_observer(Arc::new(observe.clone()));
    assert_refused(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }), "너무 큼", "bytes");
    // 화소 수: 디코딩 전에 거절한다.
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() }).with_pixel_decode_observer(Arc::new(observe));
    assert_refused(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }), "해상도가 너무 큼", "pixels");
    assert_eq!(observer.load(std::sync::atomic::Ordering::SeqCst), 0, "상한을 넘는 이미지를 디코딩했다");
    assert_eq!(fs::read(&b.path).unwrap(), original);
}

#[test]
fn test_oversized_page_document_is_refused() {
    let b = book();
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_xml_bytes: 50, ..Limits::default() });
    assert!(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }).is_err());
}

/// 본문 조각을 `<head>`(제목, 스타일)가 붙은 XHTML 챕터로 감싸 `c0.xhtml`, `c1.xhtml`, ...로 쓴다. 헤드는 행수에 들어가면 안 된다.
fn write_epub(path: &Path, bodies: &[&str]) {
    let pages: Vec<String> = bodies
        .iter()
        .map(|body| format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>본문 제목</title><style>p {{ color: black; }}</style></head><body>{body}</body></html>"#))
        .collect();
    let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
    make_epub(path, &EpubSpec::chapters(&refs));
}

/// Python `reader` fixture의 params: (이름, 읽기 함수).
type Reader = (&'static str, fn(&Path) -> EpubRead);

const READERS: [Reader; 2] = [("read_from_epub", read_from_epub), ("read_from_epub_with_extracting_zip", read_from_epub_with_extracting_zip)];

#[test]
fn test_line_count_ignores_source_formatting() {
    for (reader_name, reader) in READERS {
        let tmp = tempfile::tempdir().unwrap();
        let bodies = ["<p>첫 문단의 <span>강조</span> 내용</p><p>둘째 문단</p>", "\n  <p>\n    첫 문단의\n    <span>강조</span>\n    내용\n  </p>\n\n  <p>둘째 문단</p>\n"];
        let mut counts = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            let path = tmp.path().join(format!("format-{i}.epub"));
            write_epub(&path, &[*body]);
            // 일반 경로도 fallback 없이 읽을 수 있는 fixture다.
            assert_eq!(read_from_epub(&path).path, EpubPath::Normal, "reader={reader_name}, body={body:?}");
            counts.push(reader(&path).line_count);
        }
        assert_eq!(counts, [2, 2], "reader={reader_name}");
    }
}

#[test]
fn test_logical_line_count() {
    let cases: [(&str, usize); 10] = [
        ("<p>첫 줄<br/>둘째 줄<br/>셋째 줄</p>", 3),
        ("<p>첫 줄<br/><br/>둘째 줄</p><p> </p><p>&#160;</p>", 2),
        ("<div><h1>제목</h1><p>본문 <em>강조</em></p></div><ul><li>하나</li><li>둘</li></ul>", 4),
        ("<div>앞부분<p>문단</p>뒷부분</div>", 3),
        ("<div><div><p>중첩 문단</p></div></div>", 1),
        ("<table><tr><td>칸 하나</td><td>칸 둘</td></tr></table>", 2),
        ("<pre>첫 줄\n둘째 줄\n\n셋째 줄</pre>", 3),
        ("태그 없는\n본문", 1),
        ("<p>본문</p><!-- 주석 --><script>ignored();</script><style>p {color:red}</style>", 1),
        ("\n<p> </p><p>&#160;</p><br/><img src='cover.jpg'/><!-- 주석 -->", 0),
    ];
    for (reader_name, reader) in READERS {
        for (body, expected) in cases {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("lines.epub");
            write_epub(&path, &[body]);
            assert_eq!(reader(&path).line_count, expected, "reader={reader_name}, body={body:?}");
        }
    }
}

#[test]
fn test_counts_all_chapters_beyond_summary_limit() {
    for (reader_name, reader) in READERS {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("chapters.epub");
        let long = format!("<p>{}</p>", "가".repeat(TEXT_SIZE + 1));
        write_epub(&path, &[long.as_str(), "<p>둘째 장</p>", "<p>셋째 장</p>"]);
        assert_eq!(reader(&path).line_count, 3, "reader={reader_name}");
    }
}

#[test]
fn test_forced_zip_fallback_uses_same_line_count() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("fallback.epub");
    write_epub(&path, &["<h1>제목</h1><p>첫 줄<br/>둘째 줄</p>"]);
    let expected = read_from_epub(&path).line_count;

    let fail_read = |_path: &Path| -> Result<(String, usize), String> { Err("force ZIP fallback".to_string()) };
    let hooks = EpubHooks { read_epub: Some(&fail_read) };
    let forced = read_from_epub_with(&path, &hooks);
    assert_eq!(forced.path, EpubPath::Zipped);
    assert_eq!(forced.line_count, 3);
    assert_eq!(expected, 3);
    assert_eq!(forced.line_count, expected);
}

#[test]
fn libreoffice_without_an_output_file_returns_empty_text_and_logs_the_directory() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("unrelated.log"), "x").unwrap();
    let runner = common::runner::FakeRunner::returning(Ok(common::runner::output(0, "", "warn: nothing written")));
    let logs = common::log_capture::LogCapture::start(log::Level::Error);

    let text = BookManager::_convert_with_libreoffice_with(&*runner, "soffice", Some(dir.path()), Path::new("/books/book.hwp"), "txt:Text");

    assert_eq!(text, "");
    let logged = logs.text();
    assert!(logged.contains("LibreOffice produced no output") && logged.contains("format='txt:Text'"), "{logged}");
    assert!(logged.contains("unrelated.log") && logged.contains("warn: nothing written"), "{logged}");
}

mod xml_safety_and_cursors {
    use crate::common::shared_fixtures::{tmp_dir, verdict};
    use super::common;
    use std::borrow::Cow;

    use std::fs;

    use std::path::{Path, PathBuf};

    use backend::book_manager::{_declare_unbound_prefixes, _safe_xml_parser, BookManager, decode_category_cursor, encode_category_cursor};

    use base64::Engine;

    use common::zipx::{write_zip, zip_header_offset};
    use serde_json::json;

    // --- 미선언 네임스페이스 접두사 보정 ---

    const OPF_NS_STR: &str = "http://www.idpf.org/2007/opf";

    fn opf(sep: &str, attr: &str, extra_decl: &str) -> Vec<u8> {
        format!(r#"<?xml version="1.0"?>{}<package{sep}xmlns="{OPF_NS_STR}"{extra_decl} version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator {attr}>x</dc:creator></metadata><manifest/><spine/></package>"#, "\n").into_bytes()
    }

    fn strict_parses(data: &[u8]) -> bool {
        match std::str::from_utf8(data) {
            Ok(text) => roxmltree::Document::parse(text).is_ok(),
            Err(_) => false,
        }
    }

    // --- 누락된 spine 문서: 총 챕터 수와 NCX 목차 ---

    const NCX_NS_STR: &str = "http://www.daisy.org/z3986/2005/ncx/";

    fn epub_with_missing_page(path: &Path, include_page: bool) -> PathBuf {
        let container = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        let opf = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="a" href="Text/a.xhtml" media-type="application/xhtml+xml"/><item id="colophon" href="Text/colophon%20page.xhtml" media-type="application/xhtml+xml"/></manifest><spine toc="ncx"><itemref idref="a"/><itemref idref="colophon"/><itemref idref="ghost"/></spine></package>"#;
        let ncx = format!(
            r#"<?xml version="1.0"?><ncx xmlns="{NCX_NS_STR}"><navMap><navPoint id="n1"><navLabel><text>A</text></navLabel><content src="Text/a.xhtml#top"/></navPoint><navPoint id="n2"><navLabel><text>판권</text></navLabel><content src="Text/colophon%20page.xhtml"/><navPoint id="n3"><navLabel><text>자식</text></navLabel><content src="Text/a.xhtml#kid"/></navPoint></navPoint></navMap></ncx>"#
        );
        let a = "<html xmlns='http://www.w3.org/1999/xhtml'><body><p>본문</p></body></html>";
        let colophon = "<html xmlns='http://www.w3.org/1999/xhtml'><body><p>판권</p></body></html>";
        let mut entries: Vec<(&str, &[u8])> = vec![
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container.as_bytes()),
            ("OEBPS/content.opf", opf.as_bytes()),
            ("OEBPS/toc.ncx", ncx.as_bytes()),
            ("OEBPS/Text/a.xhtml", a.as_bytes()),
        ];
        if include_page {
            entries.push(("OEBPS/Text/colophon page.xhtml", colophon.as_bytes()));
        }
        write_zip(path, &entries);
        path.to_path_buf()
    }

    // --- M1: lxml XXE 방지 (CWE-611) ---

    #[test]
    fn test_safe_xml_parser_blocks_external_entity() {
        // _safe_xml_parser 는 외부 엔티티(file://)를 해석하지 않아야 한다.
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let secret = tmp_path.join("canary.txt");
        fs::write(&secret, "XXE_CANARY_VALUE").unwrap();
        let payload = format!(r#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY xxe SYSTEM "file://{}">]><r>&xxe;</r>"#, secret.display());

        // recover=True (book_manager의 4개 호출부 모드): 엔티티 미해석 → 파일 내용 미노출
        let tree = _safe_xml_parser(true).fromstring(payload.as_bytes()).expect("recover 모드는 파싱을 포기하지 않는다");
        assert!(!tree.tostring().contains("XXE_CANARY_VALUE"));

        // 정상 XML 은 그대로 파싱되어야 한다 (기능 회귀 방지)
        let ok = _safe_xml_parser(false).fromstring(b"<r><a>hi</a></r>").unwrap();
        assert_eq!(ok.find("a").unwrap().text().as_deref(), Some("hi"));
    }

    // ── Additional edge case tests to reach >99% coverage on backend/book_manager.py ──

    #[test]
    fn test_empty_epub_spine_href_is_rejected() {
        let tmp = tmp_dir();
        let epub_path = tmp.path().join("empty_path.epub");
        write_zip(
            &epub_path,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#),
                ("content.opf", br#"<?xml version="1.0"?><package version="2.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="BookId"><manifest><item id="item1" href="" media-type="text/html"/></manifest><spine toc="ncx"><itemref idref="item1"/></spine></package>"#),
            ],
        );
        let (valid, _msg) = verdict(BookManager::_validate_preview_epub(&epub_path));
        assert!(!valid);
    }

    #[test]
    fn test_encode_decode_category_cursor() {
        // Lines 55, 60-64
        let cur = encode_category_cursor(&json!([1, "val"]));
        assert_eq!(decode_category_cursor(&cur), Some(json!([1, "val"])));
        assert_eq!(decode_category_cursor("invalid_base64!!!"), None);
        let not_list = base64::engine::general_purpose::URL_SAFE.encode(br#"{"a": 1}"#);
        assert_eq!(decode_category_cursor(&not_list), None);
    }

    #[test]
    fn test_declare_unbound_prefixes_handles_any_whitespace_after_package() {
        for sep in [" ", "\n  ", "\t"] {
            let data = opf(sep, r#"opf:role="aut""#, "");
            let fixed = _declare_unbound_prefixes(&data);
            assert!(strict_parses(&fixed), "sep={sep:?}");
            assert!(String::from_utf8_lossy(&fixed).contains(&format!(r#"xmlns:opf="{OPF_NS_STR}""#)), "sep={sep:?}");
        }
    }

    #[test]
    fn test_declare_unbound_prefixes_handles_prefixes_other_than_opf() {
        for (attr, prefix) in [(r#"xsi:type="x""#, "xsi"), (r#"calibre:series="s""#, "calibre")] {
            let data = opf(" ", attr, "");
            let fixed = _declare_unbound_prefixes(&data);
            assert!(strict_parses(&fixed), "attr={attr}");
            assert!(String::from_utf8_lossy(&fixed).contains(&format!("xmlns:{prefix}=")), "attr={attr}, prefix={prefix}");
        }
    }

    #[test]
    fn test_declare_unbound_prefixes_returns_input_unchanged_when_nothing_is_missing() {
        let clean = opf(" ", r#"id="a""#, "");
        let out = _declare_unbound_prefixes(&clean);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert!(std::ptr::eq(out.as_ptr(), clean.as_ptr()));
        let declared = opf(" ", r#"opf:role="aut""#, &format!(r#" xmlns:opf="{OPF_NS_STR}""#));
        let out = _declare_unbound_prefixes(&declared);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert!(std::ptr::eq(out.as_ptr(), declared.as_ptr()));
    }

    #[test]
    fn test_declare_unbound_prefixes_ignores_prefixes_that_only_appear_in_values_and_text() {
        let data = br#"<?xml version="1.0"?><!-- <fake:tag a:b="1"/> --><package xmlns="http://www.idpf.org/2007/opf"><meta property="dcterms:modified">urn:uuid:x</meta></package>"#;
        let out = _declare_unbound_prefixes(data);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert!(std::ptr::eq(out.as_ptr(), data.as_ptr()));
    }

    #[test]
    fn test_declare_unbound_prefixes_keeps_xml_prefix_and_declares_each_prefix_once() {
        let data = br#"<package xmlns="http://www.idpf.org/2007/opf" xml:lang="ko"><a opf:x="1" opf:y="2"/><b xsi:z="1"/></package>"#;
        let fixed = String::from_utf8(_declare_unbound_prefixes(data).into_owned()).unwrap();
        assert!(fixed.matches("xmlns:opf=").count() == 1 && !fixed.contains("xmlns:xml="));
        assert!(strict_parses(fixed.as_bytes()));
    }

    #[test]
    fn test_declare_unbound_prefixes_without_root_element_is_unchanged() {
        assert_eq!(_declare_unbound_prefixes(b"not xml").as_ref(), b"not xml");
    }

    #[test]
    fn test_total_chapters_counts_only_spine_items_that_exist_in_the_zip() {
        // manifest 에 없는 idref 와 ZIP 에 없는 문서는 뷰어가 보여 줄 수 없으므로 세지 않는다.
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        assert_eq!(BookManager::_get_epub_total_chapters(&epub_with_missing_page(&tmp_path.join("missing.epub"), false)), 1);
        assert_eq!(BookManager::_get_epub_total_chapters(&epub_with_missing_page(&tmp_path.join("full.epub"), true)), 2);
    }

    #[test]
    fn test_total_chapters_counts_percent_encoded_zip_entry_names() {
        // ZIP 항목 이름이 퍼센트 인코딩된 채 저장돼 있어도(href 는 풀어서 비교) 세어야 한다.
        let tmp = tmp_dir();
        let epub = tmp.path().join("pct.epub");
        write_zip(
            &epub,
            &[
                ("META-INF/container.xml", br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#),
                ("content.opf", br#"<package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="a" href="a%20b.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/></spine></package>"#),
                ("a%20b.xhtml", b"<html/>"),
            ],
        );
        assert_eq!(BookManager::_get_epub_total_chapters(&epub), 1);
    }

    #[test]
    fn test_total_chapters_does_not_count_chapter_with_broken_zip_entry() {
        let tmp = tmp_dir();
        let epub = epub_with_missing_page(&tmp.path().join("broken.epub"), true);
        let offset = zip_header_offset(&epub, "OEBPS/Text/colophon page.xhtml") as usize;
        let mut data = fs::read(&epub).unwrap();
        data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]);
        fs::write(&epub, &data).unwrap();
        assert_eq!(BookManager::_get_epub_total_chapters(&epub), 1);
    }
}

mod image_downscaling {
    use crate::common::shared_fixtures::{image_bytes};

    use backend::book_manager::BookManager;

    // ---- 이미지/EPUB 미리보기 ----

    #[test]
    fn test_downscale_epub_image_keeps_non_jpeg_png_format() {
        // GIF 같은 형식은 확장자와 실제 형식이 어긋나지 않게 그대로 돌려준다.
        let data = image_bytes(image::ImageFormat::Gif, (3000, 3000));
        assert_eq!(BookManager::_downscale_epub_image(&data), data);
    }
}

mod find_opf_path {
    use crate::common::shared_fixtures::{write_zip_stored};

    use std::fs::File;

    use std::path::Path;

    use backend::book_manager::BookManager;

    use zip::ZipArchive;

    // ── EPUB 생성 헬퍼 ────────────────────────────────────────

    const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    fn find_opf_path(epub_path: &Path) -> String {
        let mut zin = ZipArchive::new(File::open(epub_path).unwrap()).unwrap();
        BookManager::_find_opf_path(&mut zin).unwrap_or_default()
    }

    /// 정상 container.xml에서 OPF 경로를 추출한다.
    #[test]
    fn test_normal_container_xml() {
        let tmp = tempfile::tempdir().unwrap();
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("OEBPS/content.opf", b"<package/>")]);
        assert_eq!(find_opf_path(&epub_path), "OEBPS/content.opf");
    }

    /// container.xml이 없으면 ZIP 내 .opf 파일을 직접 찾는다.
    #[test]
    fn test_missing_container_xml_finds_opf() {
        let tmp = tempfile::tempdir().unwrap();
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("content.opf", b"<package/>")]);
        assert_eq!(find_opf_path(&epub_path), "content.opf");
    }

    /// container.xml이 깨진 XML이면 regex로 full-path를 추출한다.
    #[test]
    fn test_corrupted_container_xml_regex_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let bad_container = br#"<container><rootfiles><rootfile full-path="OEBPS/pkg.opf" broken:attr="x"/></rootfiles></container>"#;
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("META-INF/container.xml", bad_container), ("OEBPS/pkg.opf", b"<package/>")]);
        let result = find_opf_path(&epub_path);
        assert_eq!(result, "OEBPS/pkg.opf", "Regex fallback should find OPF, got '{result}'");
    }

    /// container.xml도 .opf 파일도 없으면 빈 문자열을 반환한다.
    #[test]
    fn test_no_opf_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("mimetype", b"application/epub+zip")]);
        assert_eq!(find_opf_path(&epub_path), "");
    }

    /// container.xml에 rootfile이 있지만 full-path가 비어있으면 .opf 직접 탐색으로 폴백.
    #[test]
    fn test_container_xml_empty_rootfile() {
        let tmp = tempfile::tempdir().unwrap();
        let container = r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("META-INF/container.xml", container.as_bytes()), ("OPS/book.opf", b"<package/>")]);
        let result = find_opf_path(&epub_path);
        assert_eq!(result, "OPS/book.opf", "Should fallback to direct scan, got '{result}'");
    }

    /// container.xml이 완전한 바이너리 쓰레기여도 regex로 full-path를 추출한다.
    #[test]
    fn test_binary_garbage_container_regex_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let garbage = b"\x89PNG\r\n not XML at all full-path=\"OPS/pkg.opf\" end";
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("META-INF/container.xml", garbage), ("OPS/pkg.opf", b"<package/>")]);
        let result = find_opf_path(&epub_path);
        assert_eq!(result, "OPS/pkg.opf", "Binary garbage regex fallback should find OPF, got '{result}'");
    }
}

const DAY: u64 = 86400;

/// 1일 이상 된 파일이 삭제된다.
#[test]
fn test_old_files_are_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path();

    let old_file = tmp_path.join("old.epub");
    fs::write(&old_file, "old").unwrap();
    // mtime을 2일 전으로 설정
    set_mtime_ago(&old_file, 2 * DAY);

    let new_file = tmp_path.join("new.epub");
    fs::write(&new_file, "new").unwrap();

    BookManager::_evict_old_cache(tmp_path);

    assert!(!old_file.exists());
    assert!(new_file.exists());
}

/// 1일 미만 파일은 보존된다.
#[test]
fn test_recent_files_are_kept() {
    let tmp = tempfile::tempdir().unwrap();

    let recent = tmp.path().join("recent.pdf");
    fs::write(&recent, "recent").unwrap();

    BookManager::_evict_old_cache(tmp.path());

    assert!(recent.exists());
}

/// 정확히 1일 된 파일도 삭제된다.
#[test]
fn test_exactly_one_day_old_is_deleted() {
    let tmp = tempfile::tempdir().unwrap();

    let edge_file = tmp.path().join("edge.html");
    fs::write(&edge_file, "edge").unwrap();
    set_mtime_ago(&edge_file, DAY + 1); // 1일 + 1초

    BookManager::_evict_old_cache(tmp.path());

    assert!(!edge_file.exists());
}

/// 하위 디렉토리는 삭제하지 않는다.
#[test]
fn test_subdirectories_are_not_deleted() {
    let tmp = tempfile::tempdir().unwrap();

    let sub_dir = tmp.path().join("subdir");
    fs::create_dir(&sub_dir).unwrap();
    File::open(&sub_dir).unwrap().set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * DAY))).unwrap();

    BookManager::_evict_old_cache(tmp.path());

    assert!(sub_dir.exists());
}

/// 빈 디렉토리에서 에러 없이 동작한다.
#[test]
fn test_empty_directory_no_error() {
    let tmp = tempfile::tempdir().unwrap();

    BookManager::_evict_old_cache(tmp.path()); // 에러 없이 완료
}

/// 존재하지 않는 디렉토리에서 에러 없이 동작한다.
#[test]
fn test_nonexistent_directory_no_error() {
    let tmp = tempfile::tempdir().unwrap();

    let fake_dir = tmp.path().join("nonexistent");
    BookManager::_evict_old_cache(&fake_dir); // 에러 없이 완료
}

/// 오래된 파일이 여러 개일 때 모두 삭제된다.
#[test]
fn test_multiple_old_files_all_deleted() {
    let tmp = tempfile::tempdir().unwrap();

    let mut old_files = Vec::new();
    for i in 0..5 {
        let f = tmp.path().join(format!("old_{i}.epub"));
        fs::write(&f, format!("old_{i}")).unwrap();
        set_mtime_ago(&f, 2 * DAY);
        old_files.push(f);
    }

    let new_file = tmp.path().join("new.pdf");
    fs::write(&new_file, "new").unwrap();

    BookManager::_evict_old_cache(tmp.path());

    for f in &old_files {
        assert!(!f.exists(), "{} should have been evicted", f.display());
    }
    assert!(new_file.exists());
}

/// 개별 파일 삭제 실패 시 나머지 파일 처리가 계속된다.
#[test]
fn test_deletion_failure_continues_processing() {
    let tmp = tempfile::tempdir().unwrap();

    let mut files = Vec::new();
    for i in 0..3 {
        let f = tmp.path().join(format!("old_{i}.epub"));
        fs::write(&f, format!("old_{i}")).unwrap();
        set_mtime_ago(&f, 2 * DAY);
        files.push(f);
    }

    // 첫 `remove_file`만 PermissionError로 실패한다. 나머지는 안쪽 RealFs가 실제로 지운다.
    let first_call = Arc::new(Mutex::new(true));
    let faulty = FaultyFs::new();
    faulty.set_remove_file(move |_path: &Path| {
        let mut first = first_call.lock().unwrap();
        if *first {
            *first = false;
            return Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "mocked deletion failure"));
        }
        None
    });
    BookManager::_evict_old_cache_with(faulty.as_ref(), tmp.path());

    // 1개는 실패로 남고, 나머지 2개는 삭제됨
    let remaining: Vec<&PathBuf> = files.iter().filter(|f| f.exists()).collect();
    assert_eq!(remaining.len(), 1);
}

/// 방금 쓴 캐시 파일과 오래된 파일이 공존할 때, 새 파일만 보존된다.
#[test]
fn test_just_written_cache_file_is_not_evicted() {
    let tmp = tempfile::tempdir().unwrap();

    let mut old_files = Vec::new();
    for name in ["1.pdf", "2_ch5.epub", "3.html", "4_p1-3.pdf"] {
        let f = tmp.path().join(name);
        fs::write(&f, "old").unwrap();
        set_mtime_ago(&f, 3 * DAY);
        old_files.push(f);
    }

    // 방금 생성된 파일
    let just_written = tmp.path().join("5_ch10.epub");
    fs::write(&just_written, "fresh").unwrap();

    BookManager::_evict_old_cache(tmp.path());

    for f in &old_files {
        assert!(!f.exists(), "{} should have been evicted", f.file_name().unwrap().to_string_lossy());
    }
    assert!(just_written.exists());
}

mod validate_preview_epub {
    use crate::common::shared_fixtures::{chapter, entry, err_has, make_chapter_xhtml, make_cover_xhtml_with_svg, make_opf, ns_children, ns_nodes, verdict, write_epub, write_zip_stored, zip_read_string};
    use std::fs;
    use std::fs::File;
    use std::path::Path;

    use backend::book_manager::BookManager;

    use zip::ZipArchive;

    // ── EPUB 생성 헬퍼 ────────────────────────────────────────

    const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    /// 테스트용 미니 EPUB을 생성.
    fn create_test_epub(path: &Path, chapter_count: usize, include_css: bool, include_svg_cover: bool) {
        let mut entries = vec![entry("META-INF/container.xml", CONTAINER_XML), entry("OEBPS/content.opf", make_opf(chapter_count, include_css, include_svg_cover)), entry("OEBPS/toc.ncx", "<ncx/>")];
        for i in 1..=chapter_count {
            if include_svg_cover && i == 1 {
                entries.push(entry(&format!("OEBPS/ch{i}.xhtml"), make_cover_xhtml_with_svg()));
            } else {
                entries.push(entry(&format!("OEBPS/ch{i}.xhtml"), make_chapter_xhtml(i, include_css)));
            }
        }
        if include_css {
            entries.push(entry("OEBPS/style.css", "body { margin: 1em; font-family: serif; }"));
        }
        if include_svg_cover {
            // 1x1 JPEG placeholder
            entries.push(entry("media/cover.jpg", b"\xff\xd8\xff\xe0\x00\x10JFIF\xff\xd9".to_vec()));
        }
        write_epub(path, entries);
    }

    /// spine에 챕터가 등록되어 있지만 실제 ZIP에는 파일이 없는 손상 EPUB.
    ///
    /// `missing_all`이면 모든 챕터 파일을 제거한다.
    fn create_corrupted_epub(path: &Path, chapter_count: usize, missing_all: bool) {
        let mut entries = vec![entry("META-INF/container.xml", CONTAINER_XML), entry("OEBPS/content.opf", make_opf(chapter_count, false, false)), entry("OEBPS/toc.ncx", "<ncx/>")];
        if !missing_all {
            // ch1만 누락, 나머지는 포함
            for i in 2..=chapter_count {
                entries.push(entry(&format!("OEBPS/ch{i}.xhtml"), chapter(i)));
            }
        }
        write_epub(path, entries);
    }

    const OPF_NS: &str = "http://www.idpf.org/2007/opf";

    /// 파일 안 OPF의 `(spine toc 속성, spine itemref idref 목록)`.
    fn spine_of(path: &Path, opf_member: &str) -> (Option<String>, Vec<String>) {
        let mut zf = ZipArchive::new(File::open(path).unwrap()).unwrap();
        let text = zip_read_string(&mut zf, opf_member);
        let opf = roxmltree::Document::parse(&text).unwrap();
        let spine_el = ns_nodes(&opf, OPF_NS, "spine")[0];
        let idrefs = ns_children(spine_el, OPF_NS, "itemref").iter().map(|r| r.attribute("idref").unwrap().to_string()).collect();
        (spine_el.attribute("toc").map(String::from), idrefs)
    }

    /// manifest에 없는 idref(coverpage)를 spine에 포함하는 EPUB.
    fn create_epub_with_invalid_spine(path: &Path) {
        let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Invalid Spine Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="coverpage"/>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#;
        write_epub(
            path,
            vec![
                entry("META-INF/container.xml", CONTAINER_XML),
                entry("OEBPS/content.opf", opf),
                entry("OEBPS/toc.ncx", "<ncx/>"),
                entry("OEBPS/ch1.xhtml", chapter(1)),
                entry("OEBPS/ch2.xhtml", chapter(2)),
            ],
        );
    }

    /// spine 요소가 없는 OPF를 가진 EPUB.
    fn create_epub_without_spine(path: &Path) {
        let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>No Spine Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
</package>"#;
        write_epub(
            path,
            vec![
                entry("META-INF/container.xml", CONTAINER_XML),
                entry("OEBPS/content.opf", opf),
                entry("OEBPS/toc.ncx", "<ncx/>"),
                entry("OEBPS/ch1.xhtml", chapter(1)),
                entry("OEBPS/ch2.xhtml", chapter(2)),
            ],
        );
    }

    /// 빈 spine 경로는 ZIP 항목 이름으로 인정하지 않는다(Python `test_in_names_empty_path`).
    #[test]
    fn test_empty_spine_href_is_not_present_in_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("empty_href.epub");
        let opf = make_opf(1, false, false).replace("href=\"ch1.xhtml\"", "href=\"\"");
        let container =
            r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        write_zip_stored(
            &epub,
            &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", container.as_bytes()), ("content.opf", opf.as_bytes()), ("toc.ncx", b"<ncx/>"), ("ch1.xhtml", chapter(1).as_bytes())],
        );

        let (valid, error) = verdict(BookManager::_validate_preview_epub(&epub));

        assert!(!valid);
        assert!(err_has(&error, "no valid spine chapters"));
    }

    /// 정상 EPUB은 (True, None)을 반환한다.
    #[test]
    fn test_valid_epub_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("valid.epub");
        create_test_epub(&epub, 3, false, false);

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(valid);
        assert!(err.is_none());
    }

    /// mimetype 파일이 없으면 거부된다.
    #[test]
    fn test_missing_mimetype_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("no_mimetype.epub");
        write_zip_stored(
            &epub,
            &[
                ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
                ("OEBPS/content.opf", make_opf(1, false, false).as_bytes()),
                ("OEBPS/toc.ncx", b"<ncx/>"),
                ("OEBPS/ch1.xhtml", chapter(1).as_bytes()),
            ],
        );

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "mimetype"));
    }

    /// mimetype 내용이 잘못되면 거부된다.
    #[test]
    fn test_invalid_mimetype_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("bad_mimetype.epub");
        write_zip_stored(
            &epub,
            &[
                ("mimetype", b"text/plain"),
                ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
                ("OEBPS/content.opf", make_opf(1, false, false).as_bytes()),
                ("OEBPS/toc.ncx", b"<ncx/>"),
                ("OEBPS/ch1.xhtml", chapter(1).as_bytes()),
            ],
        );

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "invalid mimetype"));
    }

    /// OPF 파일이 없으면 거부된다.
    #[test]
    fn test_missing_opf_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("no_opf.epub");
        write_zip_stored(&epub, &[("mimetype", b"application/epub+zip"), ("OEBPS/ch1.xhtml", chapter(1).as_bytes())]);

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "OPF"));
    }

    /// 유효한 spine 챕터가 0개이면 거부된다.
    #[test]
    fn test_no_valid_spine_chapters_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("empty_spine.epub");
        create_corrupted_epub(&epub, 2, true);

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "no valid spine chapters"));
    }

    /// NCX 파일이 ZIP에 없으면 toc 속성이 제거되고 통과한다.
    #[test]
    fn test_missing_ncx_toc_attribute_removed() {
        let tmp = tempfile::tempdir().unwrap();
        // toc="toc"이 있지만 toc.ncx 파일 없는 EPUB
        let epub = tmp.path().join("no_ncx.epub");
        // toc.ncx 파일 생략
        write_zip_stored(
            &epub,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
                ("OEBPS/content.opf", make_opf(2, false, false).as_bytes()),
                ("OEBPS/ch1.xhtml", chapter(1).as_bytes()),
                ("OEBPS/ch2.xhtml", chapter(2).as_bytes()),
            ],
        );

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(valid, "Expected valid, got: {err:?}");

        // 수정된 EPUB에서 toc 속성이 제거되었는지 확인
        let (toc, _) = spine_of(&epub, "OEBPS/content.opf");
        assert!(toc.is_none(), "toc attribute should be removed");
    }

    /// manifest에 없는 spine idref가 자동 제거되고 통과한다.
    #[test]
    fn test_invalid_spine_idref_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("bad_idref.epub");
        create_epub_with_invalid_spine(&epub);

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(valid, "Expected valid, got: {err:?}");

        // 수정된 EPUB에서 coverpage idref가 제거되었는지 확인
        let (_, idrefs) = spine_of(&epub, "OEBPS/content.opf");
        assert!(!idrefs.contains(&"coverpage".to_string()), "invalid idref should be removed");
        assert!(idrefs.contains(&"ch1".to_string()));
        assert!(idrefs.contains(&"ch2".to_string()));
    }

    /// manifest에는 있지만 ZIP에 파일이 없는 spine 항목이 자동 제거된다.
    #[test]
    fn test_spine_item_missing_from_zip_removed() {
        let tmp = tempfile::tempdir().unwrap();
        // ch1이 누락된 EPUB (ch2, ch3은 존재)
        let epub = tmp.path().join("missing_ch1.epub");
        create_corrupted_epub(&epub, 3, false);

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(valid, "Expected valid, got: {err:?}");

        // ch1이 spine에서 제거되었는지 확인
        let (_, idrefs) = spine_of(&epub, "OEBPS/content.opf");
        assert!(!idrefs.contains(&"ch1".to_string()), "missing ch1 should be removed from spine");
        assert!(idrefs.contains(&"ch2".to_string()));
        assert!(idrefs.contains(&"ch3".to_string()));
    }

    /// 손상된 ZIP 파일은 거부된다.
    #[test]
    fn test_corrupted_zip_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("corrupted.epub");
        fs::write(&epub, b"this is not a zip file").unwrap();

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "corrupted ZIP"));
    }

    /// spine 요소가 없는 OPF는 거부된다.
    #[test]
    fn test_missing_spine_element_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("no_spine.epub");
        create_epub_without_spine(&epub);

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "spine"));
    }

    /// toc 속성이 manifest에 없는 ID를 참조하면 제거된다.
    #[test]
    fn test_toc_id_not_in_manifest_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let opf_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test</dc:title>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="nonexistent_toc">
    <itemref idref="ch1"/>
  </spine>
</package>"#;

        let epub = tmp.path().join("bad_toc_id.epub");
        write_zip_stored(
            &epub,
            &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("OEBPS/content.opf", opf_content.as_bytes()), ("OEBPS/ch1.xhtml", chapter(1).as_bytes())],
        );

        let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(valid, "Expected valid, got: {err:?}");

        let (toc, _) = spine_of(&epub, "OEBPS/content.opf");
        assert!(toc.is_none());
    }

    /// 문제가 없는 EPUB은 재작성하지 않는다 (mtime 불변).
    #[test]
    fn test_no_rewrite_when_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("clean.epub");
        create_test_epub(&epub, 2, false, false);
        let mtime_before = fs::metadata(&epub).unwrap().modified().unwrap();

        let (valid, _err) = verdict(BookManager::_validate_preview_epub(&epub));
        assert!(valid);
        let mtime_after = fs::metadata(&epub).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "clean EPUB should not be rewritten");
    }
}
