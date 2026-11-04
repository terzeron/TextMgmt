//! EPUB 미리보기 검증 테스트의 정상·손상 EPUB 준비물. 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use std::path::Path;
use super::shared_fixtures::{chapter, CONTAINER_XML, entry, make_chapter_xhtml, make_cover_xhtml_with_svg, make_opf, write_epub};

pub const OPF_NS: &str = "http://www.idpf.org/2007/opf";

/// spine에 챕터가 등록되어 있지만 실제 ZIP에는 파일이 없는 손상 EPUB.
///
/// `missing_all`이면 모든 챕터 파일을 제거한다.
pub fn create_corrupted_epub(path: &Path, chapter_count: usize, missing_all: bool) {
    let mut entries = vec![entry("META-INF/container.xml", CONTAINER_XML), entry("OEBPS/content.opf", make_opf(chapter_count, false, false)), entry("OEBPS/toc.ncx", "<ncx/>")];
    if !missing_all {
        // ch1만 누락, 나머지는 포함
        for i in 2..=chapter_count {
            entries.push(entry(&format!("OEBPS/ch{i}.xhtml"), chapter(i)));
        }
    }
    write_epub(path, entries);
}

/// manifest에 없는 idref(coverpage)를 spine에 포함하는 EPUB.
pub fn create_epub_with_invalid_spine(path: &Path) {
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
pub fn create_epub_without_spine(path: &Path) {
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

/// 테스트용 미니 EPUB을 생성.
pub fn create_test_epub(path: &Path, chapter_count: usize, include_css: bool, include_svg_cover: bool) {
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
