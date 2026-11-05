//! `common::url_parser` 단위 테스트.

use common::url_parser::{Netloc, quote, quote_component, unquote, urlencode, urljoin, urlsplit};

#[test]
fn urljoin_handles_absolute_protocol_relative_root_and_relative_refs() {
    let base = "https://example.com/books/list";
    assert_eq!(urljoin(base, "https://cdn.example.com/book"), "https://cdn.example.com/book");
    assert_eq!(urljoin(base, "//cdn.example.com/book"), "https://cdn.example.com/book");
    assert_eq!(urljoin(base, "/book"), "https://example.com/book");
    assert_eq!(urljoin(base, "book"), "https://example.com/books/book");
    assert_eq!(urljoin("https://example.com", "book"), "https://example.com/book");
}

#[test]
fn urljoin_resolves_dot_segments_query_and_fragment() {
    assert_eq!(urljoin("https://example.com/a/b/c", "../d"), "https://example.com/a/d");
    assert_eq!(urljoin("https://example.com/a/b/c", "./d/."), "https://example.com/a/b/d/");
    assert_eq!(urljoin("https://example.com/a?x=1", "?y=2"), "https://example.com/a?y=2");
    assert_eq!(urljoin("https://example.com/a?x=1", "#f"), "https://example.com/a?x=1#f");
}

#[test]
fn urljoin_handles_empty_inputs_and_scheme_change() {
    assert_eq!(urljoin("", "/x"), "/x");
    assert_eq!(urljoin("https://example.com/a", ""), "https://example.com/a");
    assert_eq!(urljoin("http://a.com/x", "ftp://b.com/y"), "ftp://b.com/y");
}

#[test]
fn urlsplit_and_netloc_parse_userinfo_ipv6_and_port() {
    let s = urlsplit("HTTPS://user:pw@[::1]:8080/p/q?k=v#frag");
    assert_eq!(s.scheme, "https");
    assert_eq!(s.netloc.as_deref(), Some("user:pw@[::1]:8080"));
    assert_eq!((s.path.as_str(), s.query.as_str(), s.fragment.as_str()), ("/p/q", "k=v", "frag"));
    let n = Netloc::parse(s.netloc.as_deref().unwrap());
    assert_eq!((n.username.as_deref(), n.password.as_deref(), n.hostname.as_deref()), (Some("user"), Some("pw"), Some("::1")));
    assert_eq!(n.port(), Ok(Some(8080)));
    assert!(Netloc::parse("h:99999").port().is_err());
    assert!(Netloc::parse("h:x1").port().is_err());
    assert_eq!(Netloc::parse("h:").port(), Ok(None));
}

#[test]
fn quote_and_urlencode_match_reference() {
    assert_eq!(quote("가 b/c~"), "%EA%B0%80%20b/c~");
    assert_eq!(quote_component("가 b/c~"), "%EA%B0%80%20b%2Fc~");
    assert_eq!(urlencode(&[("q", "가 나&"), ("a b", "1")]), "q=%EA%B0%80+%EB%82%98%26&a+b=1");
}

#[test]
fn unquote_decodes_percent_escapes_and_keeps_broken_ones() {
    assert_eq!(unquote("a%20b%zz%2"), "a b%zz%2");
    assert_eq!(unquote("%ED%95%9C"), "한");
    assert_eq!(unquote("100%"), "100%");
    assert_eq!(unquote("%28a%29"), "(a)");
    assert_eq!(unquote("%FF"), "\u{fffd}");
}
