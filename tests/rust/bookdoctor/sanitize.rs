//! `--sanitize`: 허용 목록으로 EPUB을 재구성한다. 마크업·CSS 정리, 멤버 제거, 검증, 휴지통 보관과 제자리 덮어쓰기.

mod support;

use std::fs;
use std::io::{Cursor, Read};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bookdoctor::cli::{Options, execute, parse};
use bookdoctor::sanitize::{rebuild, strip_css, strip_markup, verify};
use support::*;

fn markup(s: &str) -> String {
    strip_markup(s, false).0
}

fn names(bytes: &[u8]) -> Vec<String> {
    zip::ZipArchive::new(Cursor::new(bytes)).unwrap().file_names().map(|n| n.unwrap().into_owned()).collect()
}

fn member(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn epub_bytes(chapter: &str, opf: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, opf, chapter, extra);
    fs::read(p).unwrap()
}

// ---- 마크업 ----

#[test]
fn active_elements_are_removed_with_their_content() {
    assert_eq!(markup("<p>a</p><script>alert(1)</script><p>b</p>"), "<p>a</p><p>b</p>");
    assert_eq!(markup("<p>a</p><SCRIPT type='x'>if (a<b) {}</SCRIPT>b"), "<p>a</p>b");
    assert_eq!(markup("a<script src='x.js'/>b"), "ab");
    assert_eq!(markup("a<iframe src='http://x'>대체<iframe>중첩</iframe></iframe>b"), "ab");
    assert_eq!(markup("a<object data='x'><param name='p'/>글</object>b"), "ab");
    assert_eq!(markup("a<embed src='x'>b"), "ab");
    assert_eq!(markup("a<applet code='x'></applet>b"), "ab");
    assert_eq!(markup("a<svg><foreignObject><div>x</div></foreignObject><circle/></svg>b"), "a<svg><circle/></svg>b");
    assert_eq!(markup("a<svg><animate attributeName='href' to='javascript:x'/></svg>b"), "a<svg></svg>b");
    // 닫히지 않은 script는 문서 끝까지 실행되므로 끝까지 지운다.
    assert_eq!(markup("a<script>alert(1)"), "a");
}

#[test]
fn dangerous_attributes_are_dropped_and_ordinary_ones_stay() {
    assert_eq!(markup(r#"<div id="x" onclick="evil()" class="c">t</div>"#), r#"<div id="x" class="c">t</div>"#);
    assert_eq!(markup(r#"<a href="javascript:eval(atob('x'))">t</a>"#), "<a>t</a>");
    assert_eq!(markup("<a href=\"jav&#x61;script:x\">t</a>"), "<a>t</a>");
    assert_eq!(markup("<a href=\"JaVa\tScRiPt:x\">t</a>"), "<a>t</a>");
    assert_eq!(markup(r#"<a href="data:text/html;base64,AAAA">t</a>"#), "<a>t</a>");
    assert_eq!(markup(r#"<a href="file://evil/share/x">t</a>"#), "<a>t</a>");
    assert_eq!(markup(r#"<a href="ms-msdt:/id x">t</a>"#), "<a>t</a>");
    assert_eq!(markup(r#"<a href="\\evil\share\x">t</a>"#), "<a>t</a>");
    assert_eq!(markup(r#"<a href="weird-app://x">t</a>"#), "<a>t</a>");
    // 본문 링크는 그대로 둔다.
    for keep in [r##"<a href="#top">t</a>"##, r#"<a href="ch2.xhtml#a">t</a>"#, r#"<a href="https://example.com/">t</a>"#, r#"<a href="mailto:a@b.c">t</a>"#, r#"<img src="images/a.png"/>"#, r#"<img src="data:image/png;base64,AAAA"/>"#] {
        assert_eq!(markup(keep), keep);
    }
}

#[test]
fn remote_resources_and_page_level_hazards_are_removed() {
    assert_eq!(markup(r#"<img src="http://evil/x.png" alt="a"/>"#), r#"<img alt="a"/>"#);
    assert_eq!(markup(r#"<img src="//evil/x.png"/>"#), "<img/>");
    assert_eq!(markup(r#"<img src="\\evil\share\x.png"/>"#), "<img/>");
    assert_eq!(markup(r#"<link rel="stylesheet" href="https://evil/a.css"/>"#), "");
    assert_eq!(markup(r#"<link rel="stylesheet" href="a.css"/>"#), r#"<link rel="stylesheet" href="a.css"/>"#);
    assert_eq!(markup(r#"<meta http-equiv="refresh" content="0;url=http://evil/"/>"#), "");
    assert_eq!(markup(r#"<meta charset="utf-8"/>"#), r#"<meta charset="utf-8"/>"#);
    assert_eq!(markup(r#"<base href="http://evil/"/>"#), "");
    assert_eq!(markup(r#"<p>a</p><form action="x"><input name="q"/><button>go</button>글</form>"#), "<p>a</p>go글");
    assert_eq!(markup(r#"<?xml-stylesheet href="http://evil/a.xsl"?><p>a</p>"#), "<p>a</p>");
    assert_eq!(markup(r#"<iframe srcdoc="x"></iframe>a"#), "a");
}

#[test]
fn entity_doctype_and_style_are_cleaned() {
    let out = markup("<?xml version=\"1.0\"?><!DOCTYPE html [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><html><body>t</body></html>");
    assert!(out.contains("<!DOCTYPE html>") && !out.contains("ENTITY") && !out.contains("passwd"), "{out}");
    let std_doctype = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd"><html/>"#;
    assert_eq!(markup(std_doctype), std_doctype);
    assert_eq!(markup(r#"<p style="color:red;width:expression(alert(1))">t</p>"#), r#"<p style="color:red;width:">t</p>"#);
    assert_eq!(markup("<style>p{color:red}@import url(http://evil/a.css);b{background:url(http://evil/i.png)}</style>"), "<style>p{color:red}b{background:url()}</style>");
}

#[test]
fn visible_text_survives_but_comments_and_processing_instructions_do_not() {
    let page = "<html><body><h1>제목</h1><p>본문 &amp; 글 <b>굵게</b></p></body></html>";
    assert_eq!(markup(page), page);
    assert_eq!(markup("<p>a</p><!-- 주석 --><p>b</p>"), "<p>a</p><p>b</p>");
    // CDATA는 마크업이 아니라 이스케이프한 글자로 남긴다.
    assert_eq!(markup("<p><![CDATA[a<b & c]]></p>"), "<p>a&lt;b &amp; c</p>");
    // XML 선언은 문서 맨 앞에서만 남긴다.
    assert_eq!(markup("<?xml version=\"1.0\" encoding=\"utf-8\"?><p>t</p>"), "<?xml version=\"1.0\" encoding=\"utf-8\"?><p>t</p>");
    assert_eq!(markup("<p>t</p><?xml version=\"1.0\"?>"), "<p>t</p>");
    // 속성값 안의 `>`나 따옴표가 태그 경계를 속이지 않는다.
    assert_eq!(markup(r#"<p title="a>b" onclick="x">t</p>"#), r#"<p title="a&gt;b">t</p>"#);
}

/// 보안 리뷰 지적: 브라우저는 주석·CDATA·처리 명령·선언을 규격대로 일찍 끝내는데 스캐너가 늦게 끝내면, 안에 숨긴 스크립트가 그대로 남는다.
#[test]
fn parser_differentials_cannot_smuggle_markup_through() {
    for attack in [
        "<!--><script>alert(1)</script>-->",
        "<!---><script>alert(1)</script>-->",
        "<![CDATA[ > <script>alert(1)</script> ]]>",
        "<? > <script>alert(1)</script> ?>",
        "<!DOCTYPE html [ > <script>alert(1)</script> ]>",
        "<!x [ > <script>alert(1)</script> ]>",
        "<!-- x --!><script>alert(1)</script>-->",
        "<!DOCTYPE html \"a>\"><script>alert(1)</script>",
    ] {
        let out = markup(attack);
        assert!(!out.to_ascii_lowercase().contains("<script"), "{attack} -> {out}");
    }
}

/// 보안 리뷰 지적: XHTML·SVG에서 `<style>` 안의 마크업은 실제로 파싱된다. raw text 요소를 속이는 속성값도 막는다.
#[test]
fn style_and_raw_text_contexts_cannot_hide_markup() {
    for attack in [
        "<svg><style><script>alert(1)</script></style></svg>",
        "<style>p{}<img src=x onerror=alert(1)></style>",
        "<noscript><p title=\"</noscript><script>alert(1)</script>\">x</p></noscript>",
        "<xmp><script>alert(1)</script></xmp>",
        "<noframes><script>alert(1)</script></noframes>",
        "<plaintext><script>alert(1)</script>",
        "<title><a title=\"</title><script>alert(1)</script>\">t</a></title>",
        "<p a='</style><script>alert(1)</script>'>t</p>",
    ] {
        let out = markup(attack);
        assert!(!out.to_ascii_lowercase().contains("<script") && !out.contains("onerror"), "{attack} -> {out}");
        assert_eq!(markup(&out), out, "한 번 더 정리해도 같아야 한다: {out}");
    }
    // 정상 CSS는 그대로 둔다. CDATA 표지가 있어도 마찬가지다.
    let css = "<style type=\"text/css\">/*<![CDATA[*/ p{color:red} a>b{margin:0} /*]]>*/</style>";
    assert_eq!(markup(css), css);
    // 속성값의 `<`·`>`는 항상 이스케이프하고, 이름에 `<`나 따옴표가 든 속성은 지운다.
    assert_eq!(markup(r#"<p title="a<b>c">t</p>"#), r#"<p title="a&lt;b&gt;c">t</p>"#);
    assert_eq!(markup(r#"<p a<b="1" id="x">t</p>"#), r#"<p id="x">t</p>"#);
}

/// 보안 리뷰 지적: 다시 쓴 `style` 값이 원래 따옴표 밖으로 나가 새 속성을 만들 수 있었다.
#[test]
fn rewritten_style_values_cannot_break_out_of_the_attribute() {
    for tag in [
        "<p style='a:b&#39; onmouseover=&#39;alert(1);width:expression(1)'>t</p>",
        "<p style=\"a:b&quot; onmouseover=&quot;alert(1);width:expression(1)\">t</p>",
        "<p style=a:b&#32;onmouseover=alert(1);width:expression(1)>t</p>",
        "<p style='a:b&gt;&lt;script&gt;alert(1)&lt;/script&gt;;width:expression(1)'>t</p>",
    ] {
        let out = markup(tag);
        assert!(!out.to_ascii_lowercase().contains("<script"), "{tag} -> {out}");
        // 값은 큰따옴표 한 쌍 안에 갇히고, 안쪽의 따옴표는 모두 이스케이프됐다: 새 속성이 생길 수 없다.
        let head = &out[..out.find("\">").map_or(out.len(), |k| k + 1)];
        assert_eq!(head.matches('"').count(), 2, "{tag} -> {out}");
        assert!(!head.contains('\''), "{tag} -> {out}");
        assert_eq!(markup(&out), out, "한 번 더 정리해도 같아야 한다: {out}");
    }
}

#[test]
fn css_files_drop_remote_loads_and_script_hooks() {
    let css = "p{color:red;behavior:url(x.htc)}a{-moz-binding:url(http://evil/x)}@import 'http://evil/a.css';b{width:expression(1+1)}i{background:url(img/a.png)}";
    let out = strip_css(css);
    assert!(!out.contains("behavior") && !out.contains("-moz-binding") && !out.contains("expression") && !out.contains("evil"), "{out}");
    assert!(out.contains("url(img/a.png)") && out.contains("color:red"), "{out}");
}

// ---- 재구성 ----

fn opf_with(items: &str) -> String {
    format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>{items}</manifest><spine><itemref idref="c1"/></spine></package>"#)
}

#[test]
fn rebuild_removes_non_rendering_members_and_their_manifest_items() {
    let opf = opf_with(r#"<item id="exe" href="Misc/setup.exe" media-type="application/x-dosexec"/><item id="js" href="js/a.js" media-type="text/javascript"/><item id="img" href="img/a.png" media-type="image/png"/>"#);
    let pe: &[u8] = b"MZ\x90\x00more";
    let png: &[u8] = b"\x89PNG\r\n\x1a\nrest";
    let src = epub_bytes(&xhtml("<p>본문</p>"), &opf, &[("OEBPS/Misc/setup.exe", pe), ("OEBPS/js/a.js", b"alert(1)"), ("OEBPS/run.sh", b"#!/bin/sh"), ("OEBPS/img/a.png", png), ("OEBPS/x.lnk", b"L\0\0\0")]);
    let plan = rebuild(&src).unwrap();
    let kept = names(&plan.bytes);
    for gone in ["OEBPS/Misc/setup.exe", "OEBPS/js/a.js", "OEBPS/run.sh", "OEBPS/x.lnk"] {
        assert!(!kept.contains(&gone.to_string()), "{gone}");
    }
    for stay in ["mimetype", "META-INF/container.xml", "OEBPS/content.opf", "OEBPS/ch1.xhtml", "OEBPS/img/a.png"] {
        assert!(kept.contains(&stay.to_string()), "{stay}");
    }
    assert_eq!(kept[0], "mimetype");
    let new_opf = member(&plan.bytes, "OEBPS/content.opf");
    assert!(!new_opf.contains("setup.exe") && !new_opf.contains("a.js") && new_opf.contains("a.png") && new_opf.contains("ch1.xhtml"), "{new_opf}");
    assert_eq!(plan.removed.len(), 4);
    verify(&src, &plan.bytes).unwrap();
}

#[test]
fn rebuild_removes_executable_content_under_a_harmless_name() {
    let src = epub_bytes(&xhtml("<p>본문</p>"), &clean_opf(), &[("OEBPS/a.jpg", b"MZ\x90\x00more"), ("OEBPS/b.png", b"\x7fELF\x02\x01x")]);
    let kept = names(&rebuild(&src).unwrap().bytes);
    assert!(!kept.iter().any(|n| n.ends_with("a.jpg") || n.ends_with("b.png")), "{kept:?}");
}

#[test]
fn rebuild_cleans_markup_inside_members_and_keeps_the_text() {
    let page = xhtml(r#"<p onclick="x()">앞</p><script>alert(1)</script><img src="http://evil/i.png"/><p>뒤</p>"#);
    let src = epub_bytes(&page, &clean_opf(), &[("OEBPS/style.css", b"p{width:expression(1)}")]);
    let plan = rebuild(&src).unwrap();
    let new_page = member(&plan.bytes, "OEBPS/ch1.xhtml");
    assert!(!new_page.contains("script") && !new_page.contains("onclick") && !new_page.contains("evil"), "{new_page}");
    assert!(new_page.contains("<p>앞</p>") && new_page.contains("<p>뒤</p>"), "{new_page}");
    assert!(!member(&plan.bytes, "OEBPS/style.css").contains("expression"));
    assert!(plan.edited.iter().any(|(n, _)| n == "OEBPS/ch1.xhtml"));
    verify(&src, &plan.bytes).unwrap();
}

#[test]
fn rebuild_refuses_what_it_cannot_rewrite_safely() {
    // 읽을 수 없는 인코딩의 마크업은 정리했다고 말할 수 없으므로 건너뛴다: 코드페이지 바이트, UTF-32, 짝이 없는 UTF-16 서로게이트.
    let cp949: &[u8] = &[b'<', b'p', b'>', 0xb0, 0xa1, 0xb0, 0xa2];
    let utf32: &[u8] = &[0xff, 0xfe, 0, 0, b'<', 0, 0, 0];
    let lone_surrogate: &[u8] = &[0xff, 0xfe, b'<', 0, 0x00, 0xd8, b'p', 0];
    for bad in [cp949, utf32, lone_surrogate] {
        let src = epub_bytes(&xhtml("<p>본문</p>"), &clean_opf(), &[("OEBPS/bad.xhtml", bad)]);
        let err = rebuild(&src).err().unwrap_or_else(|| panic!("{bad:?}는 거절해야 한다"));
        assert!(err.contains("bad.xhtml"), "{err}");
    }
    assert!(rebuild(b"not a zip").is_err());
}

#[test]
fn utf16_markup_is_cleaned_and_written_back_as_utf16() {
    let page: Vec<u8> = [&[0xFF, 0xFE][..], &xhtml("<p>앞</p><script>alert(1)</script><p>뒤</p>").encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>()].concat();
    let src = epub_bytes(&xhtml("<p>본문</p>"), &clean_opf(), &[("OEBPS/u.xhtml", &page)]);
    let plan = rebuild(&src).unwrap();
    let mut z = zip::ZipArchive::new(Cursor::new(&plan.bytes)).unwrap();
    let mut raw = Vec::new();
    z.by_name("OEBPS/u.xhtml").unwrap().read_to_end(&mut raw).unwrap();
    assert_eq!(&raw[..2], &[0xFF, 0xFE], "UTF-16LE BOM을 유지한다");
    let units: Vec<u16> = raw[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16(&units).unwrap();
    assert!(text.contains("<p>앞</p>") && text.contains("<p>뒤</p>") && !text.contains("script"), "{text}");
}

#[test]
fn verify_rejects_lost_or_invented_text() {
    let src = epub_bytes(&xhtml("<p>본문이 충분히 길어야 한다 ".repeat(40).as_str()), &clean_opf(), &[]);
    let cut = epub_bytes(&xhtml("<p>본문</p>"), &clean_opf(), &[]);
    assert!(verify(&src, &cut).unwrap_err().contains("텍스트"));
    let added = epub_bytes(&xhtml("<p>전혀 다른 글</p>"), &clean_opf(), &[]);
    let base = epub_bytes(&xhtml("<p>원래 글</p>"), &clean_opf(), &[]);
    assert!(verify(&base, &added).is_err());
}

#[test]
fn verify_ignores_whitespace_that_only_moves_between_text_nodes() {
    let src = epub_bytes(&xhtml("<p>가 나</p>\n<script>x</script>\n<p>다\n라</p>"), &clean_opf(), &[]);
    let new = epub_bytes(&xhtml("<p>가나</p><p>다라</p>"), &clean_opf(), &[]);
    verify(&src, &new).unwrap();
    // 글자가 바뀌면 여전히 거절한다.
    let changed = epub_bytes(&xhtml("<p>가나</p><p>다마</p>"), &clean_opf(), &[]);
    assert!(verify(&src, &changed).is_err());
}

/// XHTML의 자기 닫힘 `<script src=".."/>`는 HTML 규칙으로 읽으면 문서 끝까지 스크립트다. 검증용 텍스트 추출이 본문을 통째로 잃지 않아야 한다.
#[test]
fn self_closing_script_does_not_hide_the_chapter_from_verification() {
    let body = format!("<script src=\"js/book.js\"/><p>{}</p>", "긴 본문 ".repeat(200));
    let src = epub_bytes(&xhtml(&body), &clean_opf(), &[("OEBPS/js/book.js", b"alert(1)")]);
    let plan = rebuild(&src).unwrap();
    verify(&src, &plan.bytes).unwrap();
    assert!(member(&plan.bytes, "OEBPS/ch1.xhtml").contains("긴 본문"));
    // 본문이 정말로 사라지면 여전히 거절한다.
    let lost = epub_bytes(&xhtml("<p>짧음</p>"), &clean_opf(), &[]);
    assert!(verify(&src, &lost).is_err());
}

// ---- 보안 리뷰: 형식 판정, URL 슬래시, CSS 이스케이프, 출력 ----

fn opf_declaring(href: &str, media: &str) -> String {
    format!(r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="x" href="{href}" media-type="{media}"/></manifest><spine><itemref idref="c1"/></spine></package>"#)
}

#[test]
fn members_are_classified_by_declared_media_type_not_just_extension() {
    // `.css`라고 적혀 있어도 manifest가 XHTML이라 선언하면 리더는 마크업으로 연다.
    let src = epub_bytes(&xhtml("<p>본문</p>"), &opf_declaring("style.css", "application/xhtml+xml"), &[("OEBPS/style.css", b"<html><body><script>alert(1)</script><p>x</p></body></html>")]);
    let plan = rebuild(&src).unwrap();
    assert!(!member(&plan.bytes, "OEBPS/style.css").contains("<script"));
    // 이미지 확장자인데 SVG·XHTML이라고 선언했거나 내용이 마크업이면 지운다.
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'><script>alert(1)</script></svg>";
    let src = epub_bytes(&xhtml("<p>본문</p>"), &opf_declaring("a.png", "image/svg+xml"), &[("OEBPS/a.png", svg), ("OEBPS/b.jpg", b"  <html><script>alert(1)</script></html>")]);
    let kept = names(&rebuild(&src).unwrap().bytes);
    assert!(!kept.iter().any(|n| n.ends_with("a.png") || n.ends_with("b.jpg")), "{kept:?}");
    // 스크립트 형식으로 선언한 멤버는 확장자와 무관하게 지운다.
    let src = epub_bytes(&xhtml("<p>본문</p>"), &opf_declaring("data.txt", "text/javascript"), &[("OEBPS/data.txt", b"alert(1)")]);
    assert!(!names(&rebuild(&src).unwrap().bytes).iter().any(|n| n.ends_with("data.txt")));
    // 위험한 멤버 이름(경로 탈출, 절대 경로, Windows 장치 이름)은 허용 확장자여도 지운다.
    let src = epub_bytes(&xhtml("<p>본문</p>"), &clean_opf(), &[("/abs.txt", b"x"), ("OEBPS/../../evil.txt", b"x"), ("OEBPS/con.txt", b"x"), ("OEBPS/ok.txt", b"x")]);
    let kept = names(&rebuild(&src).unwrap().bytes);
    assert!(kept.contains(&"OEBPS/ok.txt".to_string()) && !kept.iter().any(|n| n.contains("abs.txt") || n.contains("evil.txt") || n.ends_with("con.txt")), "{kept:?}");
}

#[test]
fn backslashes_count_as_slashes_like_browsers_do() {
    for tag in [r#"<img src="/\evil/x.png"/>"#, r#"<img src="\/evil/x.png"/>"#, r#"<img src="http:\\evil\x.png"/>"#] {
        let out = markup(tag);
        assert!(!out.contains("evil"), "{tag} -> {out}");
    }
    assert_eq!(markup(r#"<img src="images\a.png"/>"#), r#"<img src="images\a.png"/>"#);
}

#[test]
fn css_escapes_cannot_hide_remote_loads() {
    for css in [
        r#"p{background:\75rl(http://evil/a.png)}"#,
        r#"p{background:u\72l("http://evil/i.png")}"#,
        r#"@\69mport "http://evil/a.css";"#,
        r#"@import "\68ttp://evil/a.css";"#,
        r#"p{background:url(\68 ttp://evil/a.png)}"#,
        r#"p{background:image-set("http://evil/a.png" 1x)}"#,
        r#"p{background:-webkit-image-set(url(http://evil/a.png) 1x)}"#,
        r#"p{background:src("http://evil/a.png")}"#,
        r#"p{background:url("\\evil\share\a.png")}"#,
        r#"p{wid\74h:expr\65ssion(alert(1))}"#,
    ] {
        let out = strip_css(css);
        assert!(!out.contains("evil") && !out.to_ascii_lowercase().contains("expression"), "{css} -> {out}");
    }
    // 정상 CSS의 이스케이프(한자 글꼴 이름)와 상대 경로는 그대로 둔다.
    for keep in [r#"p{font-family:"\5B8B\4F53",serif}"#, "p{background:url(img/a.png)}", r#"p{background:url("../img/a b.png")}"#] {
        assert_eq!(strip_css(keep), keep);
    }
}

#[test]
fn output_never_carries_terminal_escapes_from_member_names() {
    let d = tempfile::tempdir().unwrap();
    let opf = opf_with(r#"<item id="e" href="Misc/a%1B%5B31mred.exe" media-type="application/x-dosexec"/>"#);
    let p = d.path().join("a.epub");
    epub_with_files(&p, &opf, &xhtml("<script>x</script><p>t</p>"), &[("OEBPS/Misc/a\u{1b}[31mred.exe", b"MZ\x90\x00more")]);
    let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
    assert_eq!(code, 0, "{out:?}");
    assert!(!out.contains('\u{1b}'), "{out:?}");
}

#[test]
fn a_file_with_security_findings_left_after_rebuild_is_not_applied() {
    // manifest가 압축 밖을 가리키는 항목은 허용 목록으로 고치지 못한다. 남은 보안 지표가 있으면 적용하지 않는다.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.epub");
    epub_with_files(&p, &opf_with(r#"<item id="esc" href="../../outside.xhtml" media-type="application/xhtml+xml"/>"#), &xhtml("<p>본문</p><script>alert(1)</script>"), &[]);
    let before = fs::read(&p).unwrap();
    let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("SKIP"), "{out}");
    assert_eq!(fs::read(&p).unwrap(), before);
}

// ---- 보안 리뷰 3차: CSS 줄바꿈 전처리, media-type 정규화, 패키지 문서 찾기 ----

#[test]
fn css_escape_followed_by_any_line_break_is_still_decoded() {
    for css in ["p{background:\\75\r\nrl(http://evil/a.png)}", "p{background:\\75\x0crl(http://evil/a.png)}", "p{background:\\75\nrl(http://evil/a.png)}", "p{background:\\75\trl(http://evil/a.png)}"] {
        let out = strip_css(css);
        assert!(!out.contains("evil"), "{css:?} -> {out:?}");
    }
}

#[test]
fn declared_media_types_are_normalized_before_comparison() {
    // 매개변수·대소문자·`+xml` 접미사·`text/*`도 마크업으로 본다.
    for media in ["application/xhtml+xml; charset=utf-8", "APPLICATION/XHTML+XML", "application/vnd.foo+xml", "text/html;charset=utf-8", "Text/Html ; x=y"] {
        let src = epub_bytes(&xhtml("<p>본문</p>"), &opf_declaring("style.css", media), &[("OEBPS/style.css", b"<html><body><script>alert(1)</script><p>x</p></body></html>")]);
        let plan = rebuild(&src).unwrap();
        assert!(!member(&plan.bytes, "OEBPS/style.css").contains("<script"), "{media}");
    }
    for media in ["text/javascript; charset=utf-8", "Application/Javascript"] {
        let src = epub_bytes(&xhtml("<p>본문</p>"), &opf_declaring("data.txt", media), &[("OEBPS/data.txt", b"alert(1)")]);
        assert!(!names(&rebuild(&src).unwrap().bytes).iter().any(|n| n.ends_with("data.txt")), "{media}");
    }
    let src = epub_bytes(&xhtml("<p>본문</p>"), &opf_declaring("a.png", "text/html; charset=utf-8"), &[("OEBPS/a.png", b"\x89PNG\r\n\x1a\nrest")]);
    assert!(!names(&rebuild(&src).unwrap().bytes).iter().any(|n| n.ends_with("a.png")));
}

#[test]
fn markup_in_utf16_or_utf32_is_recognized_in_binary_extensions() {
    let utf16le: Vec<u8> = [&[0xFF, 0xFE][..], &"<html><script>alert(1)</script></html>".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>()].concat();
    let utf16be: Vec<u8> = [&[0xFE, 0xFF][..], &"  <svg onload='x'/>".encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>()].concat();
    let utf16_bomless: Vec<u8> = "<html/>".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let utf32le: Vec<u8> = [&[0xFF, 0xFE, 0, 0][..], &"<html/>".chars().flat_map(|c| (c as u32).to_le_bytes()).collect::<Vec<u8>>()].concat();
    let src = epub_bytes(&xhtml("<p>본문</p>"), &clean_opf(), &[("OEBPS/a.png", &utf16le), ("OEBPS/b.jpg", &utf16be), ("OEBPS/c.gif", &utf16_bomless), ("OEBPS/d.webp", &utf32le), ("OEBPS/e.png", b"\x89PNG\r\n\x1a\nrest")]);
    let kept = names(&rebuild(&src).unwrap().bytes);
    assert!(kept.contains(&"OEBPS/e.png".to_string()), "{kept:?}");
    for gone in ["a.png", "b.jpg", "c.gif", "d.webp"] {
        assert!(!kept.iter().any(|n| n.ends_with(gone)), "{gone}: {kept:?}");
    }
}

#[test]
fn the_package_document_is_found_through_container_xml_not_the_extension() {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let container = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/package.xml" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let package = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="exe" href="Misc/setup.exe" media-type="application/x-dosexec"/></manifest><spine><itemref idref="c1"/></spine></package>"#;
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("mimetype", stored).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    for (name, data) in [("META-INF/container.xml", container.as_bytes()), ("OEBPS/package.xml", package.as_bytes()), ("OEBPS/ch1.xhtml", xhtml("<p>본문</p>").as_bytes()), ("OEBPS/Misc/setup.exe", b"MZ\x90\x00more")] {
        z.start_file(name, deflated).unwrap();
        z.write_all(data).unwrap();
    }
    let src = z.finish().unwrap().into_inner();
    let plan = rebuild(&src).unwrap();
    assert!(!member(&plan.bytes, "OEBPS/package.xml").contains("setup.exe"), "{}", member(&plan.bytes, "OEBPS/package.xml"));
    verify(&src, &plan.bytes).unwrap();
}

// ---- 보안 리뷰 4차: 패키지 문서 찾기는 실패하면 닫는다 ----

/// 컨테이너와 패키지 문서를 직접 지정한 EPUB. 패키지 문서 안에 실행 파일 manifest 항목이 하나 있다.
fn epub_with_container(container: &[u8], package_name: &str) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let package = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="exe" href="Misc/setup.exe" media-type="application/x-dosexec"/></manifest><spine><itemref idref="c1"/></spine></package>"#;
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("mimetype", stored).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    for (name, data) in [("META-INF/container.xml", container), (package_name, package.as_bytes()), ("OEBPS/ch1.xhtml", xhtml("<p>본문</p>").as_bytes()), ("OEBPS/Misc/setup.exe", b"MZ\x90\x00more")] {
        z.start_file(name, deflated).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn container_for(full_path: &str) -> String {
    format!(r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="{full_path}" media-type="application/oebps-package+xml"/></rootfiles></container>"#)
}

#[test]
fn rootfile_paths_are_matched_both_raw_and_percent_decoded() {
    // 퍼센트 인코딩한 경로(`pack%20age.xml`)가 공백 있는 멤버를, 그대로의 경로가 `%`가 든 멤버를 가리킨다.
    for (full_path, member_name) in [("OEBPS/pack%20age.xml", "OEBPS/pack age.xml"), ("OEBPS/pack%20age.xml", "OEBPS/pack%20age.xml")] {
        let src = epub_with_container(container_for(full_path).as_bytes(), member_name);
        let plan = rebuild(&src).unwrap_or_else(|e| panic!("{full_path} {member_name}: {e}"));
        assert!(!member(&plan.bytes, member_name).contains("setup.exe"), "{full_path} {member_name}");
        verify(&src, &plan.bytes).unwrap();
    }
}

#[test]
fn a_utf16_container_is_decoded_and_an_unreadable_one_stops_the_rebuild() {
    let utf16: Vec<u8> = [&[0xFF, 0xFE][..], &container_for("OEBPS/package.xml").encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>()].concat();
    let src = epub_with_container(&utf16, "OEBPS/package.xml");
    let plan = rebuild(&src).unwrap();
    assert!(!member(&plan.bytes, "OEBPS/package.xml").contains("setup.exe"));
    // 해석할 수 없는 container.xml이나 가리키는 멤버가 없는 rootfile은 닫는다: 무엇이 패키지인지 모르면 고치지 않는다.
    for bad in [&b"<container><rootfiles>"[..], b"\xff\xfe\xfa not xml", container_for("OEBPS/missing.opf").as_bytes()] {
        let src = epub_with_container(bad, "OEBPS/package.xml");
        assert!(rebuild(&src).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn a_package_document_keeps_its_place_whatever_its_extension() {
    let src = epub_with_container(container_for("OEBPS/package.dat").as_bytes(), "OEBPS/package.dat");
    let plan = rebuild(&src).unwrap();
    assert!(names(&plan.bytes).contains(&"OEBPS/package.dat".to_string()));
    assert!(!member(&plan.bytes, "OEBPS/package.dat").contains("setup.exe"));
    verify(&src, &plan.bytes).unwrap();
}

// ---- 보안 리뷰 5차: 패키지 문서는 정말 패키지일 때만 예외, 읽지 못하면 닫는다 ----

fn epub_with_package(package_name: &str, package: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("mimetype", stored).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    for (name, data) in [
        ("META-INF/container.xml", container_for(package_name).as_bytes()),
        (package_name, package),
        ("OEBPS/ch1.xhtml", xhtml("<p>본문</p>").as_bytes()),
        ("OEBPS/style.css", b"<html><body><script>alert(1)</script><p>x</p></body></html>"),
    ] {
        z.start_file(name, deflated).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

const PACKAGE_DECLARING_CSS_AS_XHTML: &str = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="s" href="style.css" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;

#[test]
fn a_utf16_package_still_contributes_its_declared_media_types() {
    let utf16: Vec<u8> = [&[0xFF, 0xFE][..], &PACKAGE_DECLARING_CSS_AS_XHTML.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>()].concat();
    let src = epub_with_package("OEBPS/package.xml", &utf16);
    let plan = rebuild(&src).unwrap();
    // 선언(application/xhtml+xml)에 따라 style.css도 마크업으로 정리돼야 한다.
    assert!(!member(&plan.bytes, "OEBPS/style.css").contains("<script"));
}

#[test]
fn a_package_document_that_cannot_be_read_stops_the_rebuild() {
    for bad in [&b"<package xmlns='http://www.idpf.org/2007/opf'><manifest>"[..], b"\xff\xfe\x00\x00<", b"\xb0\xa1<package/>"] {
        let src = epub_with_package("OEBPS/package.xml", bad);
        assert!(rebuild(&src).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn only_a_real_package_document_is_exempt_from_the_extension_allowlist() {
    // 컨테이너가 가리키는 멤버가 패키지가 아니면(실행 파일, 다른 XML, 평문) 예외를 주지 않고 중단한다.
    for (name, data) in [
        ("OEBPS/package.dat", &b"MZ\x90\x00more"[..]),
        ("OEBPS/package.dat", b"just text"),
        ("OEBPS/package.dat", b"<?xml version=\"1.0\"?><html xmlns='http://www.w3.org/1999/xhtml'/>"),
        ("OEBPS/package.exe", PACKAGE_DECLARING_CSS_AS_XHTML.as_bytes()),
        ("OEBPS/package.sh", PACKAGE_DECLARING_CSS_AS_XHTML.as_bytes()),
    ] {
        let src = epub_with_package(name, data);
        assert!(rebuild(&src).is_err(), "{name}: {:?}", String::from_utf8_lossy(&data[..data.len().min(20)]));
    }
    // 진짜 패키지면 `.dat` 확장자여도 남긴다.
    let src = epub_with_package("OEBPS/package.dat", PACKAGE_DECLARING_CSS_AS_XHTML.as_bytes());
    assert!(names(&rebuild(&src).unwrap().bytes).contains(&"OEBPS/package.dat".to_string()));
}

// ---- 실행: 적용, 휴지통 ----

/// `root`를 코퍼스 루트로 삼아 명령을 실행한다. 휴지통은 `root/0_trash`다.
fn run_in(root: &Path, args: &[&str]) -> (i32, String) {
    let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let code = match parse(&argv) {
        Ok(o) => execute(&Options { library_root: root.to_path_buf(), ..o }, &mut out, false, true),
        Err(_) => 2,
    };
    (code, String::from_utf8_lossy(&out).into_owned())
}

fn infected(dir: &Path, name: &str) -> std::path::PathBuf {
    let opf = opf_with(r#"<item id="exe" href="Misc/setup.exe" media-type="application/x-dosexec"/>"#);
    let p = dir.join(name);
    epub_with_files(&p, &opf, &xhtml("<p>본문</p><script>alert(1)</script>"), &[("OEBPS/Misc/setup.exe", b"MZ\x90\x00more")]);
    p
}

#[test]
fn apply_moves_the_original_to_trash_and_overwrites_in_place() {
    let d = tempfile::tempdir().unwrap();
    let trash = d.path().join("0_trash");
    let p = infected(d.path(), "a.epub");
    let original = fs::read(&p).unwrap();
    let (ino, mtime) = {
        let m = fs::metadata(&p).unwrap();
        (m.ino(), m.modified().unwrap())
    };
    let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    let m = fs::metadata(&p).unwrap();
    assert_eq!((m.ino(), m.modified().unwrap()), (ino, mtime), "inode와 mtime을 유지한다");
    assert_eq!(fs::read(trash.join("a.epub")).unwrap(), original);
    let now = fs::read(&p).unwrap();
    assert!(!names(&now).iter().any(|n| n.ends_with("setup.exe")));
    assert!(!member(&now, "OEBPS/ch1.xhtml").contains("script"));
    // 다시 돌려도 더 바꿀 것이 없고 휴지통에 새 사본이 늘지 않는다.
    let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read_dir(&trash).unwrap().count(), 1);
}

#[test]
fn trash_name_collisions_get_a_suffix_and_clean_files_are_left_alone() {
    let d = tempfile::tempdir().unwrap();
    let trash = d.path().join("0_trash");
    fs::create_dir(&trash).unwrap();
    fs::write(trash.join("a.epub"), b"older").unwrap();
    let p = infected(d.path(), "a.epub");
    let clean = d.path().join("clean.epub");
    epub_with_chapter(&clean, &xhtml("<p>본문</p>"));
    let clean_before = fs::read(&clean).unwrap();
    let (code, out) = run_in(d.path(), &["sanitize", p.to_str().unwrap(), clean.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read(trash.join("a.epub")).unwrap(), b"older");
    assert_eq!(fs::read_dir(&trash).unwrap().count(), 2);
    assert_eq!(fs::read(&clean).unwrap(), clean_before);
}
