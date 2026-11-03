//! `bookdoctor::epub` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

mod support;

// `tests/rust/bookdoctor/epub_perf.rs`에서 옮긴 테스트.
mod epub_perf {
    use super::{support};

    use std::path::Path;

    use std::sync::atomic::Ordering;

    use bookdoctor::{FileReport, Format};

    fn binary(len: usize, seed: u8) -> Vec<u8> {
        // UTF-8로 유효하지 않은 바이트가 많이 든 데이터(이미지·폰트처럼).
        (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) | 0x80).collect()
    }

    // --- 렌더링 단계의 중복 압축 해제 제거 ---

    use bookdoctor::cli::Options;
    use bookdoctor::epub::render::FULL_READS;
    use bookdoctor::epub::scan_epub_file;

    fn epub_with_media(path: &Path) {
        let jpg = binary(300_000, 1);
        let png = binary(200_000, 2);
        support::epub_with_files(path, &support::clean_opf(), &support::xhtml("<p>본문</p>"), &[("OEBPS/a.jpg", &jpg), ("OEBPS/b.png", &png), ("OEBPS/c.ttf", &jpg)]);
    }

    #[test]
    fn render_skips_members_the_security_pass_already_read_to_the_end() {
        let d = tempfile::tempdir().unwrap();
        let plain = d.path().join("plain.epub");
        support::epub_with_chapter(&plain, &support::xhtml("<p>본문</p>"));
        let media = d.path().join("media.epub");
        epub_with_media(&media);
        let count = |p: &Path| {
            let before = FULL_READS.load(Ordering::SeqCst);
            scan_epub_file(p, &Options::default(), &mut FileReport::new(p, Some(Format::Epub)));
            FULL_READS.load(Ordering::SeqCst) - before
        };
        assert_eq!(count(&media), count(&plain), "이미지·폰트 멤버는 보안 단계가 이미 끝까지 읽었으므로 렌더링이 다시 읽지 않는다");
    }
}

// `tests/rust/bookdoctor/epub_gaps.rs`에서 옮긴 테스트.
mod epub_gaps {
    use super::{support};

    use std::path::Path;

    use support::*;
    use bookdoctor::epub::security::{Limits, scan as scan_into};
    use bookdoctor::{FileReport, Format, Severity};

    fn scan(path: &Path) -> FileReport {
        let mut r = FileReport::new(path, Some(Format::Epub));
        scan_into(path, &Limits::default(), &mut r);
        r
    }

    fn scan_chapter(body: &str) -> FileReport {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.epub");
        epub_with_chapter(&p, &xhtml(body));
        scan(&p)
    }

    fn has(r: &FileReport, code: &str) -> bool {
        r.findings.iter().any(|f| f.code == code)
    }

    // ---- 토크나이저 가장자리 ----

    #[test]
    fn truncated_and_odd_markup_never_panics_and_still_finds_what_is_there() {
        for body in [
            "<a href=\"x",
            "<p class=a/>",
            "<img src=x onerror=1",
            "<",
            "<1>",
            "x < y",
            "</",
            "<a",
            "<!doctype",
            "<?xml",
            "<p =x>",
            "<p a= >",
            "<p a='b>",
            "<p a=b c>",
            "&",
            "&#",
            "&#x",
            "&unknown; &#xZZ; &#99999999;",
        ] {
            let _ = scan_chapter(body);
        }
        // 따옴표 없는 속성값과 값 없는 속성이 섞여도 핸들러를 잡는다.
        assert!(has(&scan_chapter("<p hidden onclick=alert(1)>x</p>"), "EVENT_HANDLER_ATTR"));
        assert!(has(&scan_chapter("<script></script"), "SCRIPT_TAG"));
        assert!(has(&scan_chapter("<script>alert(1)"), "SCRIPT_TAG"));
    }

    #[test]
    fn named_and_numeric_entities_are_decoded_before_scheme_checks() {
        for href in ["javascript&colon;alert(1)", "java&Tab;script:alert(1)", "java&NewLine;script:alert(1)", "javascript&#58;alert(1)", "&#x6A;avascript:alert(1)", "&bsol;&bsol;host\\share"] {
            let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
            assert!(has(&r, "JS_URL") || has(&r, "UNC_PATH") || has(&r, "UNKNOWN_URI_SCHEME"), "{href}");
        }
        // 괄호 엔티티는 스킴으로 이어지지 않는다(`javascript(1):x`는 JS URL이 아니다).
        assert!(!has(&scan_chapter(r#"<a href="javascript&lpar;1&rpar;:x">x</a>"#), "JS_URL"));
        // 그 밖의 이름 있는 엔티티는 그대로 두되 스킴 판정에는 영향이 없다.
        for ent in ["amp", "lt", "gt", "quot", "apos", "sol", "nbsp"] {
            let r = scan_chapter(&format!(r#"<a href="a&{ent};b">x</a>"#));
            assert!(!has(&r, "JS_URL"), "{ent}");
        }
    }

    #[test]
    fn script_src_with_a_local_or_dangerous_url_is_classified() {
        assert!(has(&scan_chapter(r#"<script src="javascript:alert(1)"></script>"#), "JS_URL"));
        assert!(!has(&scan_chapter(r#"<script src="a.js"></script>"#), "REMOTE_SCRIPT"));
    }

    #[test]
    fn svg_animation_values_with_script_urls_are_flagged() {
        let r = scan_chapter(r#"<svg xmlns="http://www.w3.org/2000/svg"><animate attributeName="href" values="javascript:alert(1)"/><set attributeName="xlink:href" from="vbscript:x" to="ok"/></svg>"#);
        assert!(has(&r, "JS_URL") && has(&r, "SVG_ANIMATE_HREF"));
        // href가 아닌 속성을 바꾸는 애니메이션은 무시한다.
        assert!(!has(&scan_chapter(r#"<svg xmlns="http://www.w3.org/2000/svg"><animate attributeName="opacity" values="0;1"/></svg>"#), "SVG_ANIMATE_HREF"));
    }

    #[test]
    fn meta_without_refresh_and_base_without_href_are_ignored() {
        let r = scan_chapter(r#"<meta name="viewport" content="x"><base target="_blank"><meta http-equiv="refresh" content="5">"#);
        assert!(!has(&r, "BASE_HREF"));
        assert_eq!(r.findings.iter().find(|f| f.code == "META_REFRESH").map(|f| f.severity), Some(Severity::Medium));
    }
}
