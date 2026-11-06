mod common;
mod corpus;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use common::epub::{EpubSpec, make_epub};
use doc_reader::TEXT_SIZE;
use doc_reader::epub::{EpubHooks, EpubPath, EpubRead, pretty_text, read_from_epub, read_from_epub_with, read_from_epub_with_extracting_zip};
use zip::write::SimpleFileOptions;

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
fn package_path_collects_metadata_then_chapter_text() {
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
    // 제목, 저자 순서. 저자와 본문 사이에는 구분 공백이 없다. 자식이 요소뿐인 body는 줄바꿈이 끼어 문단이 붙지 않는다.
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
    // 중첩된 요소뿐인 컨테이너: 안쪽 div의 여는 태그 뒤 줄바꿈은 앞 공백이 아니므로 남는다.
    assert_eq!(pretty_text("<body><div><span>a</span><span>b</span></div></body>").as_deref(), Some("\na\nb\n\n"));
    // 본문이 텍스트뿐이면 None(원본 내용을 쓴다)
    assert_eq!(pretty_text("<body>only text</body>"), None);
}

#[test]
fn corpus_epub_matches_the_recorded_summaries() {
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

// ---- 줄 수와 대체 경로 (book_manager epub_inspector 테스트에서 옮김) ----

type Reader = (&'static str, fn(&Path) -> EpubRead);

const READERS: [Reader; 2] = [("read_from_epub", read_from_epub), ("read_from_epub_with_extracting_zip", read_from_epub_with_extracting_zip)];

/// 본문 조각을 `<head>`(제목, 스타일)가 붙은 XHTML 챕터로 감싸 `c0.xhtml`, `c1.xhtml`, ...로 쓴다. 헤드는 행수에 들어가면 안 된다.
fn write_epub(path: &Path, bodies: &[&str]) {
    let pages: Vec<String> = bodies
        .iter()
        .map(|body| format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>본문 제목</title><style>p {{ color: black; }}</style></head><body>{body}</body></html>"#))
        .collect();
    let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
    make_epub(path, &EpubSpec::chapters(&refs));
}

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

/// OPF 메타데이터(`inspect_epub_metadata`).
mod metadata {
    use crate::common::zipx::{write_zip, write_zip_text};
    use doc_reader::epub::{EpubMetadata, MAX_OPF_BYTES, inspect_epub_metadata};
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn empty_meta() -> EpubMetadata {
        EpubMetadata { title: String::new(), author: String::new(), subject: String::new(), description: String::new() }
    }

    #[test]
    fn test_inspect_epub_metadata_extracts_opf_fields() {
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("metadata.epub");
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
        <package xmlns:dc="http://purl.org/dc/elements/1.1/">
          <metadata>
            <dc:title>룬의 아이들</dc:title>
            <dc:creator>전민희</dc:creator>
            <dc:subject>판타지</dc:subject>
            <dc:subject>한국소설</dc:subject>
            <dc:description>마법과 모험 이야기</dc:description>
          </metadata>
        </package>
        "#;
        write_zip_text(&epub_path, &[("OPS/content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta, EpubMetadata { title: "룬의 아이들".into(), author: "전민희".into(), subject: "판타지 > 한국소설".into(), description: "마법과 모험 이야기".into() });
    }

    #[test]
    fn test_inspect_epub_metadata_returns_empty_metadata_without_opf_or_on_error() {
        let tmp = TempDir::new().unwrap();
        let no_opf = tmp.path().join("no-opf.epub");
        write_zip_text(&no_opf, &[("mimetype", "application/epub+zip")], false);
        let broken = tmp.path().join("broken.epub");
        fs::write(&broken, "not a zip").unwrap();

        assert_eq!(inspect_epub_metadata(&no_opf), empty_meta());
        assert_eq!(inspect_epub_metadata(&broken), empty_meta());
    }

    /// 중첩 엔티티(billion laughs)를 확장하지 않아야 한다.
    #[test]
    fn test_inspect_epub_metadata_does_not_expand_entity_bomb() {
        let mut decls = vec![r#"<!ENTITY a0 "AAAAAAAAAA">"#.to_string()];
        for i in 1..9 {
            let body: String = (0..10).map(|_| format!("&a{};", i - 1)).collect();
            decls.push(format!(r#"<!ENTITY a{i} "{body}">"#));
        }
        let bomb = format!(r#"<?xml version="1.0"?><!DOCTYPE p [{}]><package><title>&a8;</title></package>"#, decls.concat());

        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("bomb.epub");
        write_zip_text(&epub_path, &[("content.opf", &bomb)], false);

        // 539 바이트가 확장되면 약 100GB 다. 빈 meta 로 끝나야 한다.
        assert_eq!(inspect_epub_metadata(&epub_path), empty_meta());
    }

    /// 외부 엔티티(file://)로 로컬 파일을 읽어오지 않아야 한다.
    #[test]
    fn test_inspect_epub_metadata_does_not_resolve_external_entity() {
        let tmp = TempDir::new().unwrap();
        let secret = tmp.path().join("canary.txt");
        fs::write(&secret, "XXE_CANARY_VALUE").unwrap();
        let xxe = format!(r#"<?xml version="1.0"?><!DOCTYPE p [<!ENTITY x SYSTEM "file://{}">]><package><title>&x;</title></package>"#, secret.display());

        let epub_path = tmp.path().join("xxe.epub");
        write_zip_text(&epub_path, &[("content.opf", &xxe)], false);

        assert!(!format!("{:?}", inspect_epub_metadata(&epub_path)).contains("XXE_CANARY_VALUE"));
    }

    /// 압축을 풀면 거대한 OPF 는 읽기 전에 건너뛴다.
    #[test]
    fn test_inspect_epub_metadata_skips_oversized_opf() {
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("big.epub");
        let body = format!("<package>{}</package>", "x".repeat(MAX_OPF_BYTES + 1));
        write_zip_text(&epub_path, &[("content.opf", &body)], true);

        assert!(fs::metadata(&epub_path).unwrap().len() < 100 * 1024, "압축본은 작다");
        assert_eq!(inspect_epub_metadata(&epub_path), empty_meta());
    }

    /// 주석과 처리명령이 섞여도 메타데이터를 읽어야 한다.
    #[test]
    fn test_inspect_epub_metadata_reads_opf_with_comments() {
        let opf = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/">"#,
            "<!-- 주석 --><?pi 처리명령?>",
            "<metadata><dc:title>주석있음</dc:title><dc:creator>작가</dc:creator></metadata>",
            "</package>"
        );
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("comment.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);
        assert_eq!(meta.title, "주석있음");
        assert_eq!(meta.author, "작가");
    }

    /// 잘린 OPF에서도 이미 읽을 수 있는 메타데이터를 보존한다.
    #[test]
    fn test_inspect_epub_metadata_recovers_truncated_opf() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>recovered</dc:title><dc:creator>Author</dc:creator></metadata>"#;
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("truncated.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "recovered");
        assert_eq!(meta.author, "Author");
    }

    #[test]
    fn test_inspect_epub_metadata_recovers_before_partial_tag() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>kept</dc:title><dc:creator"#;
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("partial-tag.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "kept");
    }

    /// 복구 중 끝나지 않은 주석은 버리고 앞서 읽은 메타데이터를 보존한다.
    #[test]
    fn test_inspect_epub_metadata_recovers_before_unterminated_comment() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>kept</dc:title><!-- unfinished"#;
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("unterminated-comment.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "kept");
    }

    /// CDATA와 맞지 않는 닫는 태그가 있어도 XML 구조를 복구해 메타데이터를 읽는다.
    #[test]
    fn test_inspect_epub_metadata_recovers_cdata_and_mismatched_closing_tags() {
        let opf = concat!(r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title><![CDATA[kept]]></dc:title><?pi ok?>"#, "</package>");
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("mismatched-tags.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "kept");
    }

    /// 열린 태그와 대응하지 않는 종료 태그는 복구 과정에서 무시한다.
    #[test]
    fn test_inspect_epub_metadata_ignores_unmatched_closing_tag() {
        let opf = concat!(r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>kept</dc:title>"#, "</ghost></metadata></package>");
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("unmatched-tag.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "kept");
    }

    #[test]
    fn test_inspect_epub_metadata_returns_empty_meta_for_non_xml_opf() {
        let tmp = TempDir::new().unwrap();
        let epub = tmp.path().join("bad.epub");
        write_zip(&epub, &[("content.opf", "xml 이 아니다".as_bytes())]);
        assert_eq!(inspect_epub_metadata(&epub), json!({"title": "", "author": "", "subject": "", "description": ""}));
    }
}
