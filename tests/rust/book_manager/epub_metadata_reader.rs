//! `book_manager::epub_metadata_reader` 단위 테스트. 분류기 껍데기 정리 때 `classifier::book_classifier`에서 옮겼다.

mod common;
use book_manager::epub_metadata_reader::*;
use common::zipx::{write_zip, write_zip_text};
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
