mod corpus;

use doc_reader::markup::{count_lines, expand_self_closing, get_body_text, get_text, read_from_html, relocated_tail};
use serde_json::Value;
use std::fs;

fn cases() -> Value {
    serde_json::from_str(include_str!("markup_cases.json")).unwrap()
}

#[test]
fn get_text_and_line_counts_match_reference() {
    for case in cases().as_array().unwrap() {
        let html = case["html"].as_str().unwrap();
        assert_eq!(get_text(html), case["text"].as_str().unwrap(), "text of {html:?}");
        assert_eq!(get_body_text(html), case["body_text"].as_str().unwrap(), "body text of {html:?}");
        assert_eq!(count_lines(html) as i64, case["lines"].as_i64().unwrap(), "lines of {html:?}");
    }
}

#[test]
fn get_text_keeps_whitespace_before_an_implicit_head() {
    let html = "<!DOCTYPE html>\n<html>\n<body><p>x</p></body></html>";
    assert_eq!(get_text(html), "\nx");
}

#[test]
fn self_closing_expansion_skips_cdata_and_stops_at_an_unterminated_tag() {
    assert_eq!(expand_self_closing("<![CDATA[<a/>]]><b/>"), "<![CDATA[<a/>]]><b></b>");
    // 닫히지 않은 시작 태그를 만나면 거기서 멈추고 나머지는 그대로 둔다.
    assert_eq!(expand_self_closing("<b/><p class='x/>"), "<b></b><p class='x/>");
}

#[test]
fn body_tail_relocation_needs_only_blanks_after_the_body() {
    assert_eq!(relocated_tail("<body>a</body>x</html>"), "");
    assert_eq!(relocated_tail("<body>a</body>x"), "");
    // 꼬리 공백이 body의 마지막 텍스트 전부면 그 노드는 글에서 빠진다.
    assert_eq!(get_body_text("<html><body><p>a</p></body>\n"), "a");
}

#[test]
fn tail_blank_runs_are_kept_per_run_only_around_blank_body_and_html_ends() {
    // `</body>`·`</html>` 둘레가 공백뿐이면 덩어리마다 한 글자: 줄바꿈 없는 덩어리는 공백 하나다.
    assert_eq!(get_text("<html><body><p>a</p> </body> </html> "), "a   ");
    // `</html>` 뒤나 `</body>`와 `</html>` 사이에 글이 있으면 고치지 않는다.
    assert_eq!(get_text("<html><body>a</body></html>x"), "ax");
    assert_eq!(get_text("<html><body>a</body>x</html>"), "ax");
}

#[test]
fn read_from_html_cleans_symbols_and_counts_source_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.html");
    fs::write(&path, "<html><body><p>Hello, 세계!</p>\r\n<p>둘째 (줄)</p></body></html>").unwrap();
    let (summary, lines, pages) = read_from_html(&path).unwrap();
    assert_eq!(summary, "Hello  세계 \n둘째  줄 ");
    assert_eq!((lines, pages), (2, 0));
}

#[test]
fn read_from_html_truncates_to_4096_chars_and_reports_unreadable_files() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("long.html");
    fs::write(&path, format!("<p>{}</p>", "가".repeat(5000))).unwrap();
    assert_eq!(read_from_html(&path).unwrap().0.chars().count(), 4096);
    let bad = tmp.path().join("bad.html");
    fs::write(&bad, b"<p>\xff\xfe</p>").unwrap();
    assert!(read_from_html(&bad).unwrap_err().contains("utf-8"));
    assert!(read_from_html(&tmp.path().join("none.html")).is_err());
}

#[test]
fn corpus_html_matches_the_recorded_summaries() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_html.jsonl")) {
        let path = rec["path"].as_str().unwrap();
        let got = read_from_html(&root.join(path));
        if rec.get("error").is_some() {
            assert!(got.is_err(), "expected a reader error for {path}: {got:?}");
            continue;
        }
        let (summary, lines, pages) = got.unwrap_or_else(|error| panic!("Rust reader failed for {path}: {error}"));
        assert_eq!(corpus::fnv1a64(&summary), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(lines as i64, rec["line_count"].as_i64().unwrap(), "line count for {path}");
        assert_eq!(pages as i64, rec["page_count"].as_i64().unwrap(), "page count for {path}");
    }
}
