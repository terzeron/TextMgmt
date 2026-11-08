//! 파일 이름과 ISBN으로 서점 검색어를 만든다.

use book_manager::bookstore_query_builder::{BookQuery, normalize_isbn13, query_from_file};

fn query(author: &str, title: &str) -> BookQuery {
    BookQuery { isbn13: None, title: title.to_string(), author: author.to_string() }
}

#[test]
fn leading_bracket_author_and_volume_suffix() {
    assert_eq!(query_from_file("[홍길동] 검과 마법 1권", None), query("홍길동", "검과 마법"));
}

#[test]
fn completion_marker_in_trailing_bracket_is_not_an_author() {
    assert_eq!(query_from_file("개구리 인간 1~47화[완결]", None), query("", "개구리 인간"));
}

#[test]
fn trailing_at_author() {
    assert_eq!(query_from_file("제목 @ 저자", None), query("저자", "제목"));
}

#[test]
fn leading_paren_author_and_paren_episode_range() {
    assert_eq!(query_from_file("(저자) 제목 (1-34화)", None), query("저자", "제목"));
}

#[test]
fn trailing_bracket_author() {
    assert_eq!(query_from_file("검과 마법 [홍길동]", None), query("홍길동", "검과 마법"));
}

#[test]
fn isbn10_becomes_isbn13_and_invalid_is_none() {
    assert_eq!(normalize_isbn13("0-306-40615-2").as_deref(), Some("9780306406157"));
    assert_eq!(normalize_isbn13("978-0-306-40615-7").as_deref(), Some("9780306406157"));
    assert_eq!(normalize_isbn13("1234567890"), None);
    assert_eq!(normalize_isbn13(""), None);
}

#[test]
fn query_carries_normalized_isbn() {
    assert_eq!(query_from_file("[홍길동] 검과 마법", Some("0-306-40615-2")).isbn13.as_deref(), Some("9780306406157"));
    assert_eq!(query_from_file("[홍길동] 검과 마법", Some("garbage")).isbn13, None);
}

#[test]
fn title_key_joins_author_and_title() {
    assert_eq!(query("홍길동", "검과 마법").title_key(), "홍길동\u{1f}검과 마법");
}
