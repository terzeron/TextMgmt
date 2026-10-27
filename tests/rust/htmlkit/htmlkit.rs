//! htmlkit 테스트. 기대값은 BeautifulSoup 4.12.3 `html.parser`의 실제 출력에서 가져왔다.

use htmlkit::*;

fn text(html: &str) -> String {
    text_of(html)
}

// ---- text_of: BeautifulSoup(...).get_text() ----

#[test]
fn script_and_style_bodies_are_excluded() {
    assert_eq!(text("<p>a</p><script>1234</script><style>x{}</style>b"), "ab");
}

#[test]
fn comments_doctype_and_processing_instructions_are_excluded_but_cdata_is_kept() {
    assert_eq!(text("a<!-- c -->b<!DOCTYPE x>c<![CDATA[d]]>e"), "abcde");
    assert_eq!(text("<![CDATA[x]]>y"), "xy");
    assert_eq!(text("a<?pi x?>b"), "ab");
}

#[test]
fn entities_follow_beautifulsoup_including_the_dropped_semicolon() {
    assert_eq!(text("&nbsp;x&#8211;y&amp;z&bogus;&#x41;&hellip;"), "\u{a0}x–y&z&bogusA…");
    assert_eq!(text("&lt;b&gt; &quot;q&quot; &#X42; &#99999999;"), "<b> \"q\" B \u{fffd}");
    // 종결 문자가 없으면 엔티티가 아니라 글자 그대로다. 종결 문자는 `;`일 때만 먹는다.
    assert_eq!(text("a&ampb"), "a&ampb");
    assert_eq!(text("a&amp b"), "a& b");
    assert_eq!(text("x&amp"), "x&amp");
    assert_eq!(text("x&#65"), "x&#65");
    assert_eq!(text("&amp;amp;"), "&amp;");
    assert_eq!(text("&NewLine;|&Tab;|&colon;|&lpar;|&check;|&hyphen;|&dash;"), "\n|\t|:|(|✓|‐|‐");
    assert_eq!(text("&euro;&Alpha;&alpha;"), "€Αα");
    // 숫자 참조는 0x80~0x9f를 windows-1252로 읽는다. 정의되지 않은 번호는 그대로 코드포인트다.
    assert_eq!(text("&#150;&#128;&#x96;&#65 x"), "–€–A x");
    assert_eq!(text("&#129;&#0;"), "\u{81}\0");
    assert_eq!(text("&#65a;"), "&#65a;");
    assert_eq!(text("tom & jerry &"), "tom & jerry &");
}

#[test]
fn tags_vanish_and_whitespace_is_preserved() {
    assert_eq!(text("<p>one<br/>two</p>"), "onetwo");
    assert_eq!(text("<a href='x'>l</a>tail"), "ltail");
    assert_eq!(text("<P CLASS=a>Up</P>"), "Up");
    assert_eq!(text(" <b>x</b> \n y "), " x \n y ");
}

#[test]
fn an_unfinished_tag_at_the_end_is_dropped_and_stray_angle_brackets_are_text() {
    assert_eq!(text("text<unclosed"), "text");
    assert_eq!(text("1 < 2 and 3 > 2"), "1 < 2 and 3 > 2");
    assert_eq!(text("a<!-- never closed"), "a");
    // 닫히지 않은 CDATA는 마커만 버리고 나머지를 본문으로 본다(html.parser의 실제 동작).
    assert_eq!(text("a<![CDATA[never closed"), "anever closed");
    assert_eq!(text("a<!never"), "a");
    assert_eq!(text("a<?never"), "a");
    assert_eq!(text("<"), "<");
}

#[test]
fn script_bodies_stop_at_the_closing_tag_case_insensitively() {
    assert_eq!(text("a<SCRIPT>x</p></SCRIPT>b"), "ab");
    assert_eq!(text("a<script>never closed"), "a");
}

// ---- tokenize ----

#[test]
fn tokenize_reads_tags_attributes_and_raw_text_bodies() {
    let (tags, malformed) = tokenize(r#"<A HREF="x" Hidden data-n=5 title='q'>t</a><script type=x>if (a<b) {}</script>"#);
    assert!(!malformed);
    let a = &tags[0];
    assert_eq!((a.name.as_str(), a.closing), ("a", false));
    assert_eq!(a.attr("href"), Some("x"));
    assert_eq!(a.attr("hidden"), Some(""));
    assert_eq!(a.attr("data-n"), Some("5"));
    assert_eq!(a.attr("title"), Some("q"));
    assert_eq!(a.attr("missing"), None);
    assert!(tags[1].closing);
    assert_eq!(tags[2].body.as_deref(), Some("if (a<b) {}"));
}

#[test]
fn tokenize_follows_html_comment_rules_and_flags_unclosed_ones() {
    for (src, expected) in [("<!--> <b>", 1), ("<!---> <b>", 1), ("<!-- x --> <b>", 1), ("<!-- x --!> <b>", 1), ("<!-- <i> --> <b>", 1)] {
        let (tags, malformed) = tokenize(src);
        assert_eq!((tags.len(), malformed), (expected, false), "{src}");
        assert_eq!(tags[0].name, "b");
    }
    let (tags, malformed) = tokenize("<p><!-- never closed <b>");
    assert!(malformed);
    assert_eq!(tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["p", "b"]);
}

#[test]
fn tokenize_scans_inside_cdata_and_skips_declarations() {
    let (tags, _) = tokenize("<![CDATA[ <script>x</script> ]]><!DOCTYPE html><?xml version='1'?><i>");
    assert_eq!(tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["script", "script", "i"]);
}

#[test]
fn tokenize_survives_truncated_markup() {
    for src in ["<a href=\"x", "<p class=a/>", "<img src=x onerror=1", "<", "<1>", "</", "<a", "<!doctype", "<?xml", "<p =x>", "<p a= >", "<p a='b>", "<p a=b c>"] {
        let _ = tokenize(src);
    }
    let (tags, _) = tokenize("<p a=b c>");
    assert_eq!(tags[0].attr("c"), Some(""));
    assert_eq!(tokenize("<script>alert(1)").0[0].body.as_deref(), Some("alert(1)"));
}

#[test]
fn namespace_prefixes_are_dropped_by_local_name() {
    assert_eq!(local_name("html:script"), "script");
    assert_eq!(local_name("svg"), "svg");
}

// ---- decode_entities (속성값용) ----

#[test]
fn attribute_entities_decode_numeric_and_common_named_references() {
    assert_eq!(decode_entities("javascript&colon;a&Tab;b&NewLine;c"), "javascript:a\tb\nc");
    assert_eq!(decode_entities("&#106;&#x6A;&amp;&lt;&gt;&quot;&apos;&lpar;&rpar;&sol;&bsol;&nbsp;"), "jj&<>\"'()/\\\u{a0}");
    assert_eq!(decode_entities("&bogus; & &# &#x &#xZZ; end"), "&bogus; & &# &#x &#xZZ; end");
    assert_eq!(decode_entities("&#99999999;"), "&#99999999;");
}

// ---- 공백만 있는 텍스트 노드는 한 글자로 줄어든다 (BeautifulSoup endData) ----

#[test]
fn whitespace_only_text_nodes_collapse_to_one_newline_or_space() {
    for (src, expected) in [
        ("<p>a</p>\r\n\t\r\n<p>b</p>", "a\nb"),
        ("<p>a</p>  \t <p>b</p>", "a b"),
        ("<p>a</p>&#32;<p>b</p>", "a b"),
        ("<p>a</p>&nbsp;<p>b</p>", "a\u{a0}b"),
        ("<p>a</p>\n&#10;\n<p>b</p>", "a\nb"),
        ("a\r\nb", "a\r\nb"),
        ("<p>a</p> <!-- c --> \n <p>b</p>", "a \nb"),
        ("<p>a</p>\u{c}<p>b</p>", "a b"),
        ("<![CDATA[\n\n]]>x", "\nx"),
        ("<script>\n</script>y\n\n<p>b</p>", "y\n\nb"),
        ("<p>a</p>\n<br/>\n<p>b</p>", "a\n\nb"),
    ] {
        assert_eq!(text(src), expected, "{src:?}");
    }
}

#[test]
fn pre_and_textarea_keep_their_whitespace() {
    for (src, expected) in [
        ("<pre>\n\n  </pre>x", "\n\n  x"),
        ("<pre>a</pre>\n\n<p>b</p>", "a\nb"),
        ("<textarea>\n \n</textarea>\n\n<i>x</i>", "\n \n\nx"),
        ("<pre>\n<b>x</b>\n  </pre>", "\nx\n  "),
        ("<PRE>\n\n</PRE>z", "\n\nz"),
    ] {
        assert_eq!(text(src), expected, "{src:?}");
    }
    // 짝이 없는 닫는 태그는 무시한다.
    assert_eq!(text("</pre>\n\n<p>b</p>"), "\nb");
}

#[test]
fn void_elements_close_at_their_start_tag_and_their_end_tag_is_not_a_boundary() {
    for (src, expected) in [
        ("<p>x</p><meta>Z</meta>\r\n\r\n<!-- c -->\r\n<p>b</p>", "xZ\r\n\r\n\nb"),
        ("<br>\n</br>\n<p>b</p>", "\nb"),
        ("<img src=a>\n\n</img>\n<p>b</p>", "\nb"),
        ("<br/>\n</br>\n<p>b</p>", "\n\nb"),
        ("<p>a</p></br>\n\n<p>b</p>", "a\nb"),
        ("<div/>\n\n<p>b</p>", "\nb"),
        ("<p>a</p><br>\n\n<p>b</p>", "a\nb"),
        ("<meta>1</meta></meta>\n\n<p>b</p>", "1\nb"),
        ("<link>a</link>  <link>b</link>  \n<i>c</i>", "a  b  \nc"),
        ("<hr></hr> \n x", " \n x"),
    ] {
        assert_eq!(text(src), expected, "{src:?}");
    }
}

#[test]
fn malformed_references_are_plain_text() {
    assert_eq!(text("&#; &#x; &1 &-x & "), "&#; &#x; &1 &-x & ");
}
