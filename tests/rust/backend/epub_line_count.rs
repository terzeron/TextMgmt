//! 원본: tests/test_epub_line_count.py
//!
//! EPUB 행수는 소스 포맷과 무관하게 본문의 논리적 행을 센다. Python의 `reader` fixture(두 읽기 함수에 대한 params)는
//! 각 테스트 안에서 두 읽기 함수를 차례로 돌리는 루프로 옮긴다.
//!
//! 가정한 API (아직 구현 전이다. `Loader.read_from_epub*`는 `doc_reader` crate로 옮겨져 있다):
//! - `doc_reader::epub::read_from_epub(path: &Path) -> EpubRead` (이미 있음. `line_count`, `path: EpubPath`)
//! - `doc_reader::epub::read_from_epub_with_extracting_zip(path: &Path) -> EpubRead` (정규식 대체 경로를 직접 부르는 공개 함수. 현재는 비공개 `read_with_zip_regex`뿐이다)
//! - `doc_reader::epub::read_from_epub_with(path: &Path, hooks: &EpubHooks) -> EpubRead` (구 `read_from_epub_using`. docs/rust-test-seams.md 3.12)
//!   - `EpubHooks { read_epub: Option<&dyn Fn(&Path) -> Result<(String, usize), String>> }` (`Default`). loader_2의 `*Hooks`와 같은 모양이다.
//!     ebooklib 경로를 주입하는 지점이며 Python `monkeypatch.setattr("utils.loader.epub.read_epub", ...)`에 대응한다.
//! - 테스트 EPUB은 `common::epub::{EpubSpec, make_epub}`로 만든다.
//! - `doc_reader::TEXT_SIZE: usize` (이미 있음)

mod common;

use std::path::Path;

use common::epub::{EpubSpec, make_epub};
use doc_reader::TEXT_SIZE;
use doc_reader::epub::{EpubHooks, EpubPath, EpubRead, read_from_epub, read_from_epub_with, read_from_epub_with_extracting_zip};

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
const READERS: [(&str, fn(&Path) -> EpubRead); 2] = [("read_from_epub", read_from_epub), ("read_from_epub_with_extracting_zip", read_from_epub_with_extracting_zip)];

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
    let hooks = EpubHooks { read_epub: Some(&fail_read), ..Default::default() };
    let forced = read_from_epub_with(&path, &hooks);
    assert_eq!(forced.path, EpubPath::Zipped);
    assert_eq!(forced.line_count, 3);
    assert_eq!(expected, 3);
    assert_eq!(forced.line_count, expected);
}
