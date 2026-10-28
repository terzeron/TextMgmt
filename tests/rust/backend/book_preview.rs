//! 원본: tests/test_book_preview.py
//!
//! EPUB 미리보기(preview)와 다운로드(download) 엔드포인트 통합 테스트.
//! 테스트용 미니 EPUB을 직접 만들어서 실제 EPUB 파일이 없는 환경에서도 동작한다.
//! Python의 `backend_test_setup`/`test_book` fixture는 `Setup`/`TestBook`과 `register`로 옮겼다.
//! 모든 파일은 테스트별 TempDir에 만들고 FakeEsManager를 주입한다.
//! epubcheck는 CommandRunner 대역으로 출력·시간 상한을 검증한다.

mod common;

use std::collections::HashMap;
use std::fs::{self, File};
use std::future::Future;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend::book_manager::{BookManager, BookManagerDeps, BookRef};
use backend::main::{AppDeps, AuthMode, app_with};
use backend::pdf::{LopdfOpener, PdfDoc, PdfError, PdfOpener};
use common::TestClient;
use common::es_fake::FakeEsManager;
use common::faulty_fs::FaultyFs;
use common::fsx::set_mtime_ago;
use common::zipx::{ZipMode, write_zip_with};
use serde_json::{Value, json};
use sysdeps::{CommandError, CommandOutput, CommandRunner};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

const CATEGORY: &str = "_epub";
const OPF_NS: &str = "http://www.idpf.org/2007/opf";
const CNT_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
const NCX_NS: &str = "http://www.daisy.org/z3986/2005/ncx/";

#[tokio::test]
async fn test_html_routes_preserve_comics_prefix_and_download_attachment() {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("page.html"), r#"<html><body><script>alert(1)</script><img src="cover.png"><p>본문</p></body></html>"#).unwrap();
    let image = common::image::png_bytes(2, 2);
    fs::write(tmp.path().join("cover.png"), &image).unwrap();
    let es = FakeEsManager::with_doc(json!({"category":"test", "title":"HTML", "author":"", "file_path":"page.html", "file_type":"html", "file_size":100, "updated_time":"2024-01-01T00:00:00"}));
    let deps = BookManagerDeps::new(tmp.path().to_path_buf(), es);
    let app = app_with(
        AppDeps::new(AuthMode::Fixed(json!({"role":"admin"})))
            .with_book_manager(Arc::new(BookManager::with_deps(deps.clone())))
            .with_comics_manager(Arc::new(backend::comics_manager::ComicsManager::with_deps(deps))),
    );
    let client = TestClient::with_app(app);
    for prefix in ["", "/comics"] {
        let preview = client.get(&format!("{prefix}/preview/1")).await;
        assert_eq!(preview.status, 200);
        assert!(preview.text().contains(&format!("{prefix}/html-resource/1?path=cover.png")));
        assert!(!preview.text().contains("alert(1)"));
        let resource = client.get(&format!("{prefix}/html-resource/1?path=cover.png")).await;
        assert_eq!(resource.status, 200);
        assert_eq!(resource.body, image);
        assert_eq!(client.get(&format!("{prefix}/html-resource/1")).await.status, 422);
        assert_eq!(client.get(&format!("{prefix}/html-resource/1?path=../outside.png")).await.status, 400);
        let download = client.get(&format!("{prefix}/download/1")).await;
        assert_eq!(download.status, 200);
        assert_eq!(download.header("content-disposition"), Some("attachment; filename=\"page.html\""));
        assert_eq!(download.body, fs::read(tmp.path().join("page.html")).unwrap());
    }
}

// ── EPUB 생성 헬퍼 ────────────────────────────────────────

const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

/// 챕터 N개를 포함한 OPF XML을 생성.
fn make_opf(chapter_count: usize, include_css: bool, include_svg_cover: bool) -> String {
    let mut items = Vec::new();
    let mut spine_refs = Vec::new();
    for i in 1..=chapter_count {
        items.push(format!(r#"    <item id="ch{i}" href="ch{i}.xhtml" media-type="application/xhtml+xml"/>"#));
        spine_refs.push(format!(r#"    <itemref idref="ch{i}"/>"#));
    }
    if include_css {
        items.push(r#"    <item id="style" href="style.css" media-type="text/css"/>"#.to_string());
    }
    if include_svg_cover {
        items.push(r#"    <item id="cover-image" href="../media/cover.jpg" media-type="image/jpeg"/>"#.to_string());
    }
    let items = items.join("\n");
    let spine_refs = spine_refs.join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test Book</dc:title>
    <dc:creator>Test Author</dc:creator>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
{items}
  </manifest>
  <spine toc="toc">
{spine_refs}
  </spine>
</package>"#
    )
}

/// SVG `<image>` 태그로 커버 이미지를 참조하는 커버 페이지.
fn make_cover_xhtml_with_svg() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Cover</title></head>
<body>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
     width="100%" height="100%" viewBox="0 0 600 800">
  <image xlink:href="../media/cover.jpg" width="600" height="800"/>
</svg>
</body>
</html>"#
        .to_string()
}

fn make_chapter_xhtml(index: usize, link_css: bool) -> String {
    let css_link = if link_css { r#"<link rel="stylesheet" href="style.css"/>"# } else { "" };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter {index}</title>{css_link}</head>
<body><h1>Chapter {index}</h1><p>Content of chapter {index}.</p></body>
</html>"#
    )
}

fn chapter(index: usize) -> String {
    make_chapter_xhtml(index, false)
}

fn stored() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
}

fn deflated() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

/// mimetype(무압축)을 맨 앞에 두고 나머지를 압축해서 쓴다. Python 생성 헬퍼가 모두 이 배치다.
fn write_epub(path: &Path, entries: Vec<(String, Vec<u8>)>) {
    let mimetype: &[u8] = b"application/epub+zip";
    let mut all: Vec<(&str, &[u8])> = vec![("mimetype", mimetype)];
    all.extend(entries.iter().map(|(n, d)| (n.as_str(), d.as_slice())));
    write_zip_with(path, &all, ZipMode::EpubStyle);
}

/// `zipfile.ZipFile(path, "w")`: 압축 인자가 없으면 ZIP_STORED다.
fn write_zip_stored(path: &Path, entries: &[(&str, &[u8])]) {
    write_zip_with(path, entries, ZipMode::Stored);
}

fn entry(name: &str, data: impl Into<Vec<u8>>) -> (String, Vec<u8>) {
    (name.to_string(), data.into())
}

/// 테스트용 미니 EPUB을 생성.
fn create_test_epub(path: &Path, chapter_count: usize, include_css: bool, include_svg_cover: bool) {
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

/// 앞쪽에 비선형(linear="no") 항목(nav·표지)을 둔 EPUB.
///
/// build_epub.py 산출물처럼 스파인 첫머리가 nav·표지(linear="no")이고 그 뒤에
/// linear 본문이 이어진다. 미리보기가 비선형 항목을 세면 넘길 페이지가 거의
/// 남지 않으므로, 미리보기는 linear 본문만 골라야 한다.
fn create_epub_with_nonlinear_front(path: &Path, chapter_count: usize) {
    let mut items = vec![
        r#"    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#.to_string(),
        r#"    <item id="coverpage" href="cover.xhtml" media-type="application/xhtml+xml"/>"#.to_string(),
    ];
    let mut spine = vec![r#"    <itemref idref="nav" linear="no"/>"#.to_string(), r#"    <itemref idref="coverpage" linear="no"/>"#.to_string()];
    for i in 1..=chapter_count {
        items.push(format!(r#"    <item id="ch{i}" href="ch{i}.xhtml" media-type="application/xhtml+xml"/>"#));
        spine.push(format!(r#"    <itemref idref="ch{i}"/>"#));
    }
    let items = items.join("\n");
    let spine = spine.join("\n");
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Nonlinear Front Book</dc:title>
    <dc:creator>Test Author</dc:creator>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
{items}
  </manifest>
  <spine toc="toc">
{spine}
  </spine>
</package>"#
    );
    let mut entries = vec![
        entry("META-INF/container.xml", CONTAINER_XML),
        entry("OEBPS/content.opf", opf),
        entry("OEBPS/toc.ncx", "<ncx/>"),
        entry("OEBPS/nav.xhtml", chapter(0)),
        entry("OEBPS/cover.xhtml", chapter(0)),
    ];
    for i in 1..=chapter_count {
        entries.push(entry(&format!("OEBPS/ch{i}.xhtml"), chapter(i)));
    }
    write_epub(path, entries);
}

/// CSS `url()`로 폰트를 참조하는 EPUB. `font_size`로 폰트 크기를 제어.
fn create_epub_with_font(path: &Path, font_size: usize) {
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Font Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="style" href="style.css" media-type="text/css"/>
    <item id="testfont" href="fonts/test.ttf" media-type="font/ttf"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="ch1"/>
  </spine>
</package>"#;
    let chapter = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Ch1</title><link rel="stylesheet" href="style.css"/></head>
<body><p>Font test chapter</p></body>
</html>"#;
    let css = "@font-face { font-family: 'Test'; src: url('fonts/test.ttf'); }\nbody { font-family: 'Test', serif; }";
    write_epub(
        path,
        vec![
            entry("META-INF/container.xml", CONTAINER_XML),
            entry("OEBPS/content.opf", opf),
            entry("OEBPS/toc.ncx", "<ncx/>"),
            entry("OEBPS/ch1.xhtml", chapter),
            entry("OEBPS/style.css", css),
            entry("OEBPS/fonts/test.ttf", vec![0u8; font_size]),
        ],
    );
}

/// manifest에 없는 idref(coverpage)를 spine에 포함하는 EPUB.
fn create_epub_with_invalid_spine(path: &Path) {
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

/// 챕터에서 참조하지 않는 CSS(대용량 폰트 `url()` 포함)를 가진 EPUB.
fn create_epub_with_unreferenced_css(path: &Path) {
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Unreferenced CSS Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="used-style" href="used.css" media-type="text/css"/>
    <item id="unused-style" href="unused.css" media-type="text/css"/>
    <item id="bigfont" href="fonts/big.ttf" media-type="font/ttf"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="ch1"/>
  </spine>
</package>"#;
    let chapter = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Ch1</title><link rel="stylesheet" href="used.css"/></head>
<body><p>Content</p></body>
</html>"#;
    write_epub(
        path,
        vec![
            entry("META-INF/container.xml", CONTAINER_XML),
            entry("OEBPS/content.opf", opf),
            entry("OEBPS/toc.ncx", "<ncx/>"),
            entry("OEBPS/ch1.xhtml", chapter),
            entry("OEBPS/used.css", "body { margin: 1em; }"),
            entry("OEBPS/unused.css", "@font-face { font-family: 'Big'; src: url('fonts/big.ttf'); }"),
            entry("OEBPS/fonts/big.ttf", vec![0u8; 600 * 1024]),
        ],
    );
}

/// OPF에 선언되지 않은 네임스페이스 프리픽스(opf:role)가 있는 EPUB.
fn create_epub_with_unbound_prefix(path: &Path) {
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Unbound Prefix Test</dc:title>
    <dc:creator opf:role="aut" opf:file-as="Author, Test">Test Author</dc:creator>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="toc">
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

/// container.xml이 없지만 OPF는 존재하는 EPUB.
fn create_epub_without_container_xml(path: &Path) {
    // container.xml 없음
    write_epub(path, vec![entry("OEBPS/content.opf", make_opf(2, false, false)), entry("OEBPS/toc.ncx", "<ncx/>"), entry("OEBPS/ch1.xhtml", chapter(1)), entry("OEBPS/ch2.xhtml", chapter(2))]);
}

/// spine 요소가 없는 OPF를 가진 EPUB.
fn create_epub_without_spine(path: &Path) {
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

/// container.xml이 깨진 XML이지만 full-path 속성은 regex로 추출 가능한 EPUB.
fn create_epub_with_corrupted_container_xml(path: &Path) {
    let bad_container = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"
              calibre:unknown-attr="value"/>
  </rootfiles>
</container>"#;
    write_epub(
        path,
        vec![
            entry("META-INF/container.xml", bad_container),
            entry("OEBPS/content.opf", make_opf(2, false, false)),
            entry("OEBPS/toc.ncx", "<ncx/>"),
            entry("OEBPS/ch1.xhtml", chapter(1)),
            entry("OEBPS/ch2.xhtml", chapter(2)),
        ],
    );
}

/// container.xml이 존재하지 않는 OPF 경로를 가리키는 EPUB.
fn create_epub_with_missing_opf(path: &Path) {
    let container = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/nonexistent.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;
    // OPF 파일 없음
    write_epub(path, vec![entry("META-INF/container.xml", container)]);
}

/// container.xml도 없고 .opf 파일도 없는 EPUB.
fn create_epub_with_no_opf_at_all(path: &Path) {
    write_epub(path, vec![entry("OEBPS/ch1.xhtml", chapter(1))]);
}

/// container.xml이 완전한 바이너리 쓰레기이지만 regex로 full-path를 추출할 수 있는 EPUB.
fn create_epub_with_binary_garbage_container(path: &Path) {
    let garbage = b"\x89PNG\r\n not XML at all full-path=\"OEBPS/content.opf\" end of garbage".to_vec();
    write_epub(
        path,
        vec![
            entry("META-INF/container.xml", garbage),
            entry("OEBPS/content.opf", make_opf(2, false, false)),
            entry("OEBPS/toc.ncx", "<ncx/>"),
            entry("OEBPS/ch1.xhtml", chapter(1)),
            entry("OEBPS/ch2.xhtml", chapter(2)),
        ],
    );
}

/// NCX에 실제 챕터보다 많은 navPoint가 포함된 EPUB 생성.
///
/// `chapter_count`개의 챕터 파일만 포함하고, NCX에는 `total_ncx_points`개의 navPoint를 생성.
fn create_epub_with_ncx_extra_refs(path: &Path, chapter_count: usize, total_ncx_points: usize) {
    let nav_points: Vec<String> = (1..=total_ncx_points)
        .map(|i| {
            format!(
                r#"  <navPoint id="np-{i}" playOrder="{i}">
    <navLabel><text>Chapter {i}</text></navLabel>
    <content src="ch{i}.xhtml#frag{i}"/>
  </navPoint>"#
            )
        })
        .collect();
    let nav_points = nav_points.join("\n");
    let ncx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{NCX_NS}" version="2005-1">
  <navMap>
{nav_points}
  </navMap>
</ncx>"#
    );
    let mut entries = vec![entry("META-INF/container.xml", CONTAINER_XML), entry("OEBPS/content.opf", make_opf(chapter_count, false, false)), entry("OEBPS/toc.ncx", ncx)];
    for i in 1..=chapter_count {
        entries.push(entry(&format!("OEBPS/ch{i}.xhtml"), chapter(i)));
    }
    write_epub(path, entries);
}

/// spine에 챕터가 등록되어 있지만 실제 ZIP에는 파일이 없는 손상 EPUB.
///
/// `missing_all`이면 모든 챕터 파일을 제거한다.
fn create_corrupted_epub(path: &Path, chapter_count: usize, missing_all: bool) {
    let mut entries = vec![entry("META-INF/container.xml", CONTAINER_XML), entry("OEBPS/content.opf", make_opf(chapter_count, false, false)), entry("OEBPS/toc.ncx", "<ncx/>")];
    if !missing_all {
        // ch1만 누락, 나머지는 포함
        for i in 2..=chapter_count {
            entries.push(entry(&format!("OEBPS/ch{i}.xhtml"), chapter(i)));
        }
    }
    write_epub(path, entries);
}

// ── fixtures ──────────────────────────────────────────────

/// BookManager + TestClient (ES 공유).
struct Setup {
    bm: Arc<BookManager>,
    client: TestClient,
    es: Arc<FakeEsManager>,
    _tmp: tempfile::TempDir,
}

/// 테스트별 파일과 ES 상태를 분리한다.
async fn backend_test_setup() -> Setup {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let bm = Arc::new(BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone())));
    let token = backend::auth::create_jwt_token("admin@test.com", "admin", Some("Test Admin"), None);
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(bm.clone()))).cookie("tm_access_token", &token);
    Setup { bm, client, es, _tmp: tmp }
}

impl Setup {
    /// 같은 ES와 경로를 쓰되 epubcheck 대역을 꽂은 `BookManager`로 라우터를 만든 클라이언트. 구 `hooks::set_epubcheck_spawner`.
    fn client_with_spawner(&self, spawner: Arc<FakeSpawner>) -> TestClient {
        let deps = BookManagerDeps::new(self.bm.path_prefix(), self.es.clone()).with_runner(spawner);
        let token = backend::auth::create_jwt_token("admin@test.com", "admin", Some("Test Admin"), None);
        TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(Arc::new(BookManager::with_deps(deps))))).cookie("tm_access_token", &token)
    }
}

macro_rules! setup {
    () => {
        backend_test_setup().await
    };
}

fn unlink(path: &Path) {
    let _ = fs::remove_file(path);
}

/// 임시 파일 정리(`finally: path.unlink(missing_ok=True)`). panic이 나도 Drop에서 지운다.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        unlink(&self.0);
    }
}

/// EPUB 파일의 ES 등록용 데이터를 생성.
fn build_epub_data(bm: &BookManager, epub_path: &Path) -> Value {
    use std::os::unix::fs::MetadataExt;

    let meta = fs::metadata(epub_path).unwrap();
    let inode = meta.ino();
    let rel_path = epub_path.strip_prefix(bm.path_prefix()).unwrap();
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f").to_string();
    let mut data = serde_json::Map::new();
    data.insert(
        inode.to_string(),
        json!({"category": CATEGORY, "title": epub_path.file_stem().unwrap().to_string_lossy(), "author": "Test Author", "file_path": rel_path.to_string_lossy(), "file_type": "epub", "file_size": meta.len(), "line_count": 0, "page_count": 0, "isbn": "", "summary": "test epub", "updated_time": now}),
    );
    Value::Object(data)
}

/// 등록된 책. `run`이 Python의 `try: ... finally: _cleanup_book(...)`에 대응한다.
struct Registered<'a> {
    setup: &'a Setup,
    book_id: i64,
    path: PathBuf,
}

/// EPUB 파일을 ES에 등록하고 book_id를 돌려준다.
async fn register<'a>(setup: &'a Setup, epub_path: &Path) -> Registered<'a> {
    let data = build_epub_data(&setup.bm, epub_path);
    let docs = serde_json::from_value::<std::collections::BTreeMap<i64, Value>>(data.clone()).unwrap();
    setup.es.with(|s| s.docs.extend(docs));
    let (book_id, error) = split(setup.bm.add_book(&serde_json::from_value(data).unwrap()).await);
    assert!(book_id.is_some_and(|id| id != 0) && error.as_deref().is_none_or(str::is_empty), "Failed to register epub: {error:?}");
    Registered { setup, book_id: book_id.unwrap(), path: epub_path.to_path_buf() }
}

struct OfficeHtmlRunner;

impl CommandRunner for OfficeHtmlRunner {
    fn run(&self, args: &[String], _timeout: Option<Duration>) -> Result<CommandOutput, CommandError> {
        let outdir = args.iter().position(|arg| arg == "--outdir").and_then(|index| args.get(index + 1)).ok_or_else(|| CommandError::Failed("missing --outdir".into()))?;
        let input = args.last().ok_or_else(|| CommandError::Failed("missing input file".into()))?;
        let stem = Path::new(input).file_stem().unwrap().to_string_lossy();
        fs::write(Path::new(outdir).join(format!("{stem}.html")), "<html><body>office</body></html>").map_err(|e| CommandError::Failed(e.to_string()))?;
        Ok(CommandOutput { returncode: 0, ..CommandOutput::default() })
    }

    fn which(&self, _program: &str) -> Option<PathBuf> {
        Some(PathBuf::from("/fake/libreoffice"))
    }
}

/// 테스트 후 정리.
async fn cleanup_book(setup: &Setup, book_id: i64, epub_path: &Path) {
    let _ = setup.client.delete(&format!("/books/{book_id}")).await; // 실패해도 무시한다
    let cache_dir = setup.bm.path_prefix().join(".preview_cache");
    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&book_id.to_string()) {
                unlink(&e.path());
            }
        }
    }
    unlink(epub_path);
}

impl Registered<'_> {
    async fn run<F: Future<Output = ()>>(&self, body: F) {
        body.await;
        cleanup_book(self.setup, self.book_id, &self.path).await;
    }

    /// 이 책의 미리보기 캐시 경로(`{book_id}.epub`).
    fn cache_file(&self) -> PathBuf {
        self.setup.bm.path_prefix().join(".preview_cache").join(format!("{}.epub", self.book_id))
    }
}

/// 정상 EPUB을 생성·등록하고 테스트 후 정리하는 fixture `test_book`.
struct TestBook {
    setup: Setup,
    book_id: i64,
    epub_path: PathBuf,
}

async fn test_book() -> TestBook {
    let setup = backend_test_setup().await;
    let epub_dir = setup.bm.path_prefix().join(CATEGORY);
    let epub_path = epub_dir.join("[Test Author] Test Preview Book.epub");
    create_test_epub(&epub_path, 3, false, false);
    let book_id = register(&setup, &epub_path).await.book_id;
    TestBook { setup, book_id, epub_path }
}

macro_rules! book {
    () => {
        test_book().await
    };
}

impl TestBook {
    fn client(&self) -> &TestClient {
        &self.setup.client
    }

    fn cache_file(&self) -> PathBuf {
        self.setup.bm.path_prefix().join(".preview_cache").join(format!("{}.epub", self.book_id))
    }

    async fn run<F: Future<Output = ()>>(&self, body: F) {
        body.await;
        // cleanup
        let _ = self.setup.client.delete(&format!("/books/{}", self.book_id)).await;
        unlink(&self.cache_file());
        unlink(&self.epub_path);
    }
}

// ── 응답 파싱 헬퍼 ────────────────────────────────────────

type Zip = ZipArchive<Cursor<Vec<u8>>>;

fn parse_epub_zip(content: &[u8]) -> Zip {
    ZipArchive::new(Cursor::new(content.to_vec())).unwrap()
}

fn namelist<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>) -> Vec<String> {
    (0..zf.len()).map(|i| zf.by_index(i).unwrap().name().unwrap().into_owned()).collect()
}

fn zip_read<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>, name: &str) -> Vec<u8> {
    let mut data = Vec::new();
    zf.by_name(name).unwrap_or_else(|_| panic!("KeyError: {name}")).read_to_end(&mut data).unwrap();
    data
}

fn zip_read_string<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>, name: &str) -> String {
    String::from_utf8(zip_read(zf, name)).unwrap()
}

/// ElementTree의 `.//{ns}name`처럼 네임스페이스까지 맞는 요소를 문서 순서로 돌려준다.
fn ns_nodes<'a, 'i>(doc: &'a roxmltree::Document<'i>, ns: &str, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)).collect()
}

fn ns_children<'a, 'i>(node: roxmltree::Node<'a, 'i>, ns: &str, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    node.children().filter(|n| n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)).collect()
}

fn opf_path_of<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>) -> String {
    let text = zip_read_string(zf, "META-INF/container.xml");
    let container = roxmltree::Document::parse(&text).unwrap();
    ns_nodes(&container, CNT_NS, "rootfile")[0].attribute("full-path").unwrap_or("").to_string()
}

fn get_spine_idrefs<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>) -> Vec<String> {
    let opf_path = opf_path_of(zf);
    let text = zip_read_string(zf, &opf_path);
    let opf = roxmltree::Document::parse(&text).unwrap();
    let spine_el = ns_nodes(&opf, OPF_NS, "spine")[0];
    ns_children(spine_el, OPF_NS, "itemref").iter().map(|r| r.attribute("idref").unwrap().to_string()).collect()
}

/// EPUB ZIP에서 manifest의 `{id: {href, media-type}}`를 문서 순서로 추출.
fn get_manifest_items<R: Read + std::io::Seek>(zf: &mut ZipArchive<R>) -> Vec<(String, String, String)> {
    let opf_path = opf_path_of(zf);
    let text = zip_read_string(zf, &opf_path);
    let opf = roxmltree::Document::parse(&text).unwrap();
    ns_nodes(&opf, OPF_NS, "item")
        .iter()
        .map(|i| (i.attribute("id").unwrap_or("").to_string(), i.attribute("href").unwrap_or("").to_string(), i.attribute("media-type").unwrap_or("").to_string()))
        .collect()
}

/// 파일 안 OPF의 `(spine toc 속성, spine itemref idref 목록)`.
fn spine_of(path: &Path, opf_member: &str) -> (Option<String>, Vec<String>) {
    let mut zf = ZipArchive::new(File::open(path).unwrap()).unwrap();
    let text = zip_read_string(&mut zf, opf_member);
    let opf = roxmltree::Document::parse(&text).unwrap();
    let spine_el = ns_nodes(&opf, OPF_NS, "spine")[0];
    let idrefs = ns_children(spine_el, OPF_NS, "itemref").iter().map(|r| r.attribute("idref").unwrap().to_string()).collect();
    (spine_el.attribute("toc").map(String::from), idrefs)
}

fn err_has(error: &Option<String>, needle: &str) -> bool {
    error.as_deref().unwrap_or_else(|| panic!("error가 None이다 (기대: {needle:?} 포함)")).contains(needle)
}

// ── tests: preview 엔드포인트 ─────────────────────────────

mod test_book_preview {
    use super::*;

    /// 정상 EPUB → 200 + 유효한 ZIP (mimetype, container.xml 포함).
    #[tokio::test]
    async fn test_preview_returns_valid_epub() {
        let book = book!();
        book.run(async {
            let response = book.client().get(&format!("/preview/{}?chapters=3", book.book_id)).await;
            assert_eq!(response.status, 200);
            assert!(response.header("content-type").unwrap_or("").contains("epub"));

            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(names.contains(&"mimetype".to_string()));
            assert_eq!(zip_read(&mut zf, "mimetype"), b"application/epub+zip");
            assert!(names.contains(&"META-INF/container.xml".to_string()));
        })
        .await;
    }

    /// preview 응답에 Content-Encoding: identity와 Cache-Control: no-transform이 포함된다.
    #[tokio::test]
    async fn test_preview_response_headers() {
        let book = book!();
        book.run(async {
            let response = book.client().get(&format!("/preview/{}?chapters=3", book.book_id)).await;
            assert_eq!(response.status, 200);
            assert_eq!(response.header("content-encoding"), Some("identity"));
            assert_eq!(response.header("cache-control"), Some("no-transform"));
        })
        .await;
    }

    /// chapters=1 → spine의 itemref가 1개 이하.
    #[tokio::test]
    async fn test_preview_limits_chapters() {
        let book = book!();
        book.run(async {
            // 캐시 무효화
            unlink(&book.cache_file());

            let response = book.client().get(&format!("/preview/{}?chapters=1", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let idrefs = get_spine_idrefs(&mut zf);
            assert!(idrefs.len() <= 1);
        })
        .await;
    }

    /// 미리보기는 비선형(표지·목차) 항목을 세지 않고 linear 본문만 앞 N개 선택한다.
    ///
    /// 회귀: 스파인 첫머리가 nav·표지(linear="no")면 예전엔 그것들이 미리보기
    /// 정원을 차지해 넘길 페이지가 거의 남지 않았다. 이제 linear 항목만 골라야 한다.
    #[tokio::test]
    async fn test_preview_skips_nonlinear_front_matter() {
        let setup = setup!();
        let epub_path = setup.bm.path_prefix().join(CATEGORY).join("[Test Author] Nonlinear Front.epub");
        create_epub_with_nonlinear_front(&epub_path, 6);
        let book = register(&setup, &epub_path).await;
        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=3", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let idrefs = get_spine_idrefs(&mut zf);

            // 비선형 항목(nav·표지)은 미리보기 스파인에 없어야 한다.
            assert!(!idrefs.contains(&"nav".to_string()));
            assert!(!idrefs.contains(&"coverpage".to_string()));
            // linear 본문 3개(ch1~ch3)만 선택된다.
            assert_eq!(idrefs, ["ch1", "ch2", "ch3"]);
        })
        .await;
    }

    /// preview에서 사용되지 않는 manifest item이 제거된다.
    #[tokio::test]
    async fn test_preview_strips_unreferenced_manifest_items() {
        let book = book!();
        book.run(async {
            unlink(&book.cache_file());

            let response = book.client().get(&format!("/preview/{}?chapters=1", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let manifest = get_manifest_items(&mut zf);
            let spine = get_spine_idrefs(&mut zf);

            // spine에 없고 toc도 아닌 manifest item이 없어야 함 (CSS 제외)
            for (item_id, href, media_type) in &manifest {
                if !spine.contains(item_id) && item_id != "toc" && !media_type.contains("css") {
                    panic!("Unreferenced manifest item found: {item_id} ({href})");
                }
            }
        })
        .await;
    }

    /// 두 번째 요청 시 동일 응답 (캐시 히트).
    #[tokio::test]
    async fn test_preview_caches_result() {
        let book = book!();
        book.run(async {
            let cache_file = book.setup.bm.path_prefix().join(".preview_cache").join(format!("{}_ch3_v{}.epub", book.book_id, BookManager::EPUB_IMAGE_POLICY_VERSION));
            unlink(&cache_file);

            let resp1 = book.client().get(&format!("/preview/{}?chapters=3", book.book_id)).await;
            assert_eq!(resp1.status, 200);
            assert!(cache_file.exists());

            let resp2 = book.client().get(&format!("/preview/{}?chapters=3", book.book_id)).await;
            assert_eq!(resp2.status, 200);
            assert_eq!(resp1.body, resp2.body);
        })
        .await;
    }

    /// 없는 book_id → 404.
    #[tokio::test]
    async fn test_preview_nonexistent_book_returns_404() {
        let setup = setup!();
        let response = setup.client.get("/preview/999999999?chapters=3").await;
        assert_eq!(response.status, 404);
    }

    #[tokio::test]
    async fn test_preview_reports_book_lookup_failure() {
        let setup = setup!();
        setup.es.fail("search_by_id", "search unavailable");

        let response = setup.bm.get_book_preview(123, None, None).await;

        assert_eq!(response.status_code, 500);
        assert!(String::from_utf8(response.body).unwrap().contains("Book lookup failed:"));
    }

    #[tokio::test]
    async fn test_preview_reports_cache_directory_creation_failure() {
        let book = book!();
        let cache_dir = book.setup.bm.path_prefix().join(".preview_cache");
        fs::write(&cache_dir, b"file blocks cache directory").unwrap();

        let response = book.setup.bm.get_book_preview(book.book_id, None, Some(3)).await;

        assert_eq!(response.status_code, 500);
        assert!(String::from_utf8(response.body).unwrap().contains("Preview cache unavailable:"));
    }

    #[tokio::test]
    async fn test_office_preview_reports_cache_write_failure() {
        let setup = setup!();
        let path = setup.bm.path_prefix().join("office.doc");
        fs::write(&path, b"office source").unwrap();
        let mut data = build_epub_data(&setup.bm, &path);
        for doc in data.as_object_mut().unwrap().values_mut() {
            doc["file_type"] = json!("doc");
        }
        let (book_id, doc) = serde_json::from_value::<std::collections::BTreeMap<i64, Value>>(data).unwrap().into_iter().next().unwrap();
        setup.es.put_doc(book_id, doc);

        let faulty_fs = FaultyFs::new();
        faulty_fs.set_write(|_| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "cache is read-only")));
        let manager = BookManager::with_deps(BookManagerDeps::new(setup.bm.path_prefix(), setup.es.clone()).with_fs(faulty_fs).with_runner(Arc::new(OfficeHtmlRunner)));

        let response = manager.get_book_preview(book_id, None, None).await;

        assert_eq!(response.status_code, 500);
        assert!(String::from_utf8(response.body).unwrap().contains("DOC preview failed"));
    }

    #[tokio::test]
    async fn test_preview_continues_when_stale_cache_removal_fails() {
        let book = book!();
        let cache_dir = book.setup.bm.path_prefix().join(".preview_cache");
        fs::create_dir_all(&cache_dir).unwrap();
        let stale = [cache_dir.join(format!("{}_ch3_v2.epub", book.book_id)), cache_dir.join(format!("{}.epub", book.book_id)), cache_dir.join(format!("{}.html", book.book_id))];
        for path in &stale {
            fs::write(path, b"stale cache").unwrap();
        }

        let faulty_fs = FaultyFs::new();
        faulty_fs.set_remove_file(|_| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "cache is locked")));
        let manager = BookManager::with_deps(BookManagerDeps::new(book.setup.bm.path_prefix(), book.setup.es.clone()).with_fs(faulty_fs));
        let response = manager.get_book_preview(book.book_id, None, Some(3)).await;

        assert_eq!(response.status_code, 200);
        assert!(stale.iter().all(|path| path.exists()));
    }

    #[tokio::test]
    async fn test_preview_continues_when_cache_listing_fails() {
        let book = book!();
        let faulty_fs = FaultyFs::new();
        faulty_fs.set_read_dir(|_| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "cache listing denied")));
        let manager = BookManager::with_deps(BookManagerDeps::new(book.setup.bm.path_prefix(), book.setup.es.clone()).with_fs(faulty_fs));

        let response = manager.get_book_preview(book.book_id, None, Some(3)).await;

        assert_eq!(response.status_code, 200);
    }

    /// 버그 #2 회귀 테스트 — 챕터 파일 1개 누락된 EPUB → 500이 아닌 200.
    ///
    /// spine에 등록된 ch1이 실제 ZIP에 없을 때,
    /// KeyError로 인한 500 대신 graceful하게 처리되어야 한다.
    #[tokio::test]
    async fn test_preview_missing_chapter_graceful() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let corrupted_path = epub_dir.join("[Test Author] Corrupted Preview Book.epub");
        create_corrupted_epub(&corrupted_path, 3, false);

        let book = register(&setup, &corrupted_path).await;

        book.run(async {
            unlink(&book.cache_file());

            let response = setup.client.get(&format!("/preview/{}?chapters=3", book.book_id)).await;
            assert_eq!(response.status, 200, "Missing chapter should not cause 500, got {}", response.status);
        })
        .await;
    }

    /// 모든 챕터 파일이 누락된 EPUB → 유효성 검증에서 422 반환.
    #[tokio::test]
    async fn test_preview_all_chapters_missing_graceful() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let corrupted_path = epub_dir.join("[Test Author] All Missing Preview Book.epub");
        create_corrupted_epub(&corrupted_path, 3, true);

        let book = register(&setup, &corrupted_path).await;

        book.run(async {
            unlink(&book.cache_file());

            let response = setup.client.get(&format!("/preview/{}?chapters=3", book.book_id)).await;
            assert_eq!(response.status, 422, "All chapters missing should return 422, got {}", response.status);
            assert!(response.text().contains("no valid spine chapters"));
        })
        .await;
    }

    /// CSS를 참조하는 EPUB → preview에 CSS 파일이 포함된다.
    #[tokio::test]
    async fn test_preview_includes_css_resources() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let css_path = epub_dir.join("[Test Author] CSS Preview Book.epub");
        create_test_epub(&css_path, 2, true, false);

        let book = register(&setup, &css_path).await;

        book.run(async {
            unlink(&book.cache_file());

            let response = setup.client.get(&format!("/preview/{}?chapters=1", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            assert!(namelist(&mut zf).contains(&"OEBPS/style.css".to_string()), "CSS file should be included in preview");
        })
        .await;
    }

    /// SVG `<image>`로 커버를 참조하는 EPUB → preview에 커버 이미지가 포함된다.
    #[tokio::test]
    async fn test_preview_includes_svg_image_cover() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let svg_cover_path = epub_dir.join("[Test Author] SVG Cover Book.epub");
        create_test_epub(&svg_cover_path, 2, false, true);

        let book = register(&setup, &svg_cover_path).await;

        book.run(async {
            unlink(&book.cache_file());

            let response = setup.client.get(&format!("/preview/{}?chapters=1", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            assert!(namelist(&mut zf).contains(&"media/cover.jpg".to_string()), "SVG <image> referenced cover.jpg should be included in preview");
        })
        .await;
    }

    /// 500KB 초과 폰트 파일은 미리보기에서 제외된다.
    #[tokio::test]
    async fn test_preview_excludes_large_font() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Large Font Book.epub");
        create_epub_with_font(&epub_path, 600 * 1024);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=1", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(!names.contains(&"OEBPS/fonts/test.ttf".to_string()), "Large font (>500KB) should be excluded from preview");
            assert!(names.contains(&"OEBPS/style.css".to_string()), "CSS should still be included even when its referenced font is excluded");
        })
        .await;
    }

    /// 500KB 이하 폰트 파일은 미리보기에 포함된다.
    #[tokio::test]
    async fn test_preview_includes_small_font() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Small Font Book.epub");
        create_epub_with_font(&epub_path, 100 * 1024);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=1", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            assert!(namelist(&mut zf).contains(&"OEBPS/fonts/test.ttf".to_string()), "Small font (<=500KB) should be included in preview");
        })
        .await;
    }

    /// manifest에 없는 spine idref는 필터링되고 정상 동작한다.
    #[tokio::test]
    async fn test_preview_skips_invalid_spine_items() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Invalid Spine Book.epub");
        create_epub_with_invalid_spine(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=3", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let spine = get_spine_idrefs(&mut zf);
            assert!(!spine.contains(&"coverpage".to_string()), "Invalid spine idref (not in manifest) should be filtered out");
            assert!(spine.contains(&"ch1".to_string()));
            assert!(spine.contains(&"ch2".to_string()));
        })
        .await;
    }

    /// 챕터에서 참조하지 않는 CSS와 그 CSS가 참조하는 폰트가 미리보기에서 제외된다.
    #[tokio::test]
    async fn test_preview_excludes_unreferenced_css() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Unreferenced CSS Book.epub");
        create_epub_with_unreferenced_css(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=1", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(names.contains(&"OEBPS/used.css".to_string()), "Referenced CSS should be included");
            assert!(!names.contains(&"OEBPS/unused.css".to_string()), "Unreferenced CSS should NOT be included");
            assert!(!names.contains(&"OEBPS/fonts/big.ttf".to_string()), "Font referenced only by unreferenced CSS should NOT be included");
        })
        .await;
    }

    /// 제외된 대용량 폰트의 @font-face 선언이 CSS에서 제거된다.
    #[tokio::test]
    async fn test_preview_strips_font_face_for_excluded_font() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] FontFace Strip Book.epub");
        create_epub_with_font(&epub_path, 600 * 1024);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=1", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            for name in namelist(&mut zf) {
                if name.ends_with(".css") {
                    let css = String::from_utf8_lossy(&zip_read(&mut zf, &name)).into_owned();
                    assert!(!css.contains("@font-face"), "@font-face for excluded font should be stripped from {name}");
                }
            }
        })
        .await;
    }
}

// ── tests: EPUB 구조 오류 복원 ────────────────────────────

/// 다양한 EPUB 구조 오류에서 500 대신 정상 미리보기 또는 적절한 응답을 반환하는지 검증.
mod test_epub_error_recovery {
    use super::*;

    // ── unbound namespace prefix ─────────────────────────

    /// OPF에 선언되지 않은 네임스페이스 프리픽스(opf:role)가 있어도 미리보기가 생성된다.
    #[tokio::test]
    async fn test_preview_unbound_namespace_prefix() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Unbound Prefix Book.epub");
        create_epub_with_unbound_prefix(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200, "Unbound prefix should not cause 500, got {}: {}", response.status, response.text());

            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(names.contains(&"mimetype".to_string()));
            assert!(names.iter().any(|n| n.contains("ch1")), "ch1 should be in preview");
            assert!(names.iter().any(|n| n.contains("ch2")), "ch2 should be in preview");
            assert_eq!(response.header("x-total-chapters"), Some("2"));
        })
        .await;
    }

    /// `<package` 뒤가 개행이거나 opf: 외의 접두사(xsi:)가 미선언이어도 브라우저가 파싱할 수 있는 OPF를 낸다.
    #[tokio::test]
    async fn test_preview_opf_is_strictly_parsable_for_other_unbound_prefixes() {
        for package_open in ["<package\n    xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\">", "<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\">"] {
            let setup = setup!();
            let epub_path = setup.bm.path_prefix().join(CATEGORY).join("[Test Author] Other Unbound Prefix.epub");
            create_epub_with_unbound_prefix(&epub_path);
            let mut zf = ZipArchive::new(File::open(&epub_path).unwrap()).unwrap();
            let names = namelist(&mut zf);
            let mut members: Vec<(String, Vec<u8>)> = names.iter().map(|n| (n.clone(), zip_read(&mut zf, n))).collect();
            drop(zf);
            let opf = String::from_utf8(members.iter().find(|(n, _)| n == "OEBPS/content.opf").unwrap().1.clone())
                .unwrap()
                .replace("<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\">", package_open);
            let patched = opf.replace("opf:role=\"aut\"", "opf:role=\"aut\" xsi:type=\"x\"").into_bytes();
            members.iter_mut().find(|(n, _)| n == "OEBPS/content.opf").unwrap().1 = patched;
            // members.pop("mimetype")를 맨 앞에 무압축으로 쓰고 나머지는 순서대로 쓴다.
            let mimetype = members.remove(members.iter().position(|(n, _)| n == "mimetype").unwrap());
            let mut out = ZipWriter::new(File::create(&epub_path).unwrap());
            out.start_file("mimetype", stored()).unwrap();
            out.write_all(&mimetype.1).unwrap();
            for (name, data) in &members {
                out.start_file(name, deflated()).unwrap();
                out.write_all(data).unwrap();
            }
            out.finish().unwrap();

            let book = register(&setup, &epub_path).await;
            book.run(async {
                let response = setup.client.get(&format!("/preview/{}?chapters=0", book.book_id)).await;
                assert_eq!(response.status, 200, "package_open={package_open:?}: {}", response.text());
                let mut zf = parse_epub_zip(&response.body);
                // 엄격 XML 파서(브라우저와 같은 기준)
                let text = zip_read_string(&mut zf, "OEBPS/content.opf");
                roxmltree::Document::parse(&text).unwrap_or_else(|e| panic!("package_open={package_open:?}: OPF가 엄격 파싱되지 않는다: {e}"));
            })
            .await;
        }
    }

    /// mimetype 이 압축됐거나 맨 앞이 아닌 원본도 미리보기는 표준 배치(맨 앞·무압축)로 내보낸다.
    ///
    /// 코퍼스의 을유세계문학전집 86건이 이 형태였고, 뷰어는 이 정규화에 기대어 정상 동작한다.
    #[tokio::test]
    async fn test_preview_normalizes_nonstandard_mimetype_layout() {
        for layout in ["compressed", "directory_first", "compressed_not_first"] {
            let setup = setup!();
            let epub_path = setup.bm.path_prefix().join(CATEGORY).join(format!("[Test Author] Mimetype {layout}.epub"));
            fs::create_dir_all(epub_path.parent().unwrap()).unwrap();
            let mut zf = ZipWriter::new(File::create(&epub_path).unwrap());
            if layout == "directory_first" {
                zf.add_directory("META-INF/", deflated()).unwrap();
                zf.start_file("mimetype", stored()).unwrap();
                zf.write_all(b"application/epub+zip").unwrap();
            } else if layout == "compressed" {
                zf.start_file("mimetype", deflated()).unwrap();
                zf.write_all(b"application/epub+zip").unwrap();
            }
            for (name, data) in [
                ("META-INF/container.xml", CONTAINER_XML.to_string()),
                ("OEBPS/content.opf", make_opf(2, false, false)),
                ("OEBPS/toc.ncx", "<ncx/>".to_string()),
                ("OEBPS/ch1.xhtml", chapter(1)),
                ("OEBPS/ch2.xhtml", chapter(2)),
            ] {
                zf.start_file(name, deflated()).unwrap();
                zf.write_all(data.as_bytes()).unwrap();
            }
            if layout == "compressed_not_first" {
                zf.start_file("mimetype", deflated()).unwrap();
                zf.write_all(b"application/epub+zip").unwrap();
            }
            zf.finish().unwrap();

            let book = register(&setup, &epub_path).await;
            book.run(async {
                let response = setup.client.get(&format!("/preview/{}?chapters=0", book.book_id)).await;
                assert_eq!(response.status, 200, "layout={layout}: {}", response.text());
                let mut zf = parse_epub_zip(&response.body);
                {
                    let first = zf.by_index(0).unwrap();
                    assert_eq!((first.name().unwrap().into_owned(), first.compression()), ("mimetype".to_string(), CompressionMethod::Stored), "layout={layout}");
                }
                assert_eq!(zip_read(&mut zf, "mimetype"), b"application/epub+zip", "layout={layout}");
            })
            .await;
        }
    }

    /// spine·manifest·NCX 가 ZIP 에 없는 문서를 가리켜도 목차에 죽은 링크를 남기지 않고 챕터 수를 부풀리지 않는다.
    #[tokio::test]
    async fn test_preview_drops_dead_ncx_entries_and_counts_only_existing_chapters() {
        for chapters in [0, 10] {
            let setup = setup!();
            let epub_path = setup.bm.path_prefix().join(CATEGORY).join("[Test Author] Dead Spine Page.epub");
            fs::create_dir_all(epub_path.parent().unwrap()).unwrap();
            let ncx = concat!(
                r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>"#,
                r#"<navPoint id="n1"><navLabel><text>1</text></navLabel><content src="ch1.xhtml"/></navPoint>"#,
                r#"<navPoint id="n2"><navLabel><text>판권</text></navLabel><content src="colophon.xhtml"/></navPoint>"#,
                "</navMap></ncx>"
            );
            let opf = make_opf(1, false, false)
                .replace(r#"<itemref idref="ch1"/>"#, r#"<itemref idref="ch1"/><itemref idref="colophon"/>"#)
                .replace("</manifest>", r#"<item id="colophon" href="colophon.xhtml" media-type="application/xhtml+xml"/></manifest>"#);
            write_epub(&epub_path, vec![entry("META-INF/container.xml", CONTAINER_XML), entry("OEBPS/content.opf", opf), entry("OEBPS/toc.ncx", ncx), entry("OEBPS/ch1.xhtml", chapter(1))]);

            let book = register(&setup, &epub_path).await;
            book.run(async {
                let response = setup.client.get(&format!("/preview/{}?chapters={chapters}", book.book_id)).await;
                assert_eq!(response.status, 200, "chapters={chapters}: {}", response.text());
                assert_eq!(response.header("x-total-chapters"), Some("1"), "chapters={chapters}");
                let mut zf = parse_epub_zip(&response.body);
                let contains = |data: &[u8], needle: &str| String::from_utf8_lossy(data).contains(needle);
                assert!(!contains(&zip_read(&mut zf, "OEBPS/toc.ncx"), "colophon"), "chapters={chapters}");
                assert!(!contains(&zip_read(&mut zf, "OEBPS/content.opf"), "colophon"), "chapters={chapters}");
                assert!(contains(&zip_read(&mut zf, "OEBPS/toc.ncx"), "ch1.xhtml"), "chapters={chapters}");
            })
            .await;
        }
    }

    /// 다운로드가 중단돼 끝 디렉토리(EOCD)가 없는 ZIP 은 500 이 아니라 원인을 알 수 있는 422 로 답한다.
    ///
    /// 코퍼스의 죄와 벌 (하).epub 이 128KiB 에서 잘린 이 형태였다.
    #[tokio::test]
    async fn test_preview_of_truncated_download_is_a_clear_422() {
        for chapters in [0, 10] {
            let setup = setup!();
            let epub_path = setup.bm.path_prefix().join(CATEGORY).join("[Test Author] Truncated Download.epub");
            create_epub_with_unbound_prefix(&epub_path);
            let data = fs::read(&epub_path).unwrap();
            fs::write(&epub_path, &data[..data.len() - 120]).unwrap(); // EOCD 와 중앙 디렉토리를 잘라 낸다

            let book = register(&setup, &epub_path).await;
            book.run(async {
                let response = setup.client.get(&format!("/preview/{}?chapters={chapters}", book.book_id)).await;
                assert_eq!(response.status, 422, "chapters={chapters}: {}", response.text());
                assert!(response.text().contains("corrupted or not a valid ZIP"), "chapters={chapters}");
            })
            .await;
        }
    }

    #[tokio::test]
    async fn test_total_chapters_of_truncated_download_is_zero() {
        let setup = setup!();
        let epub_path = setup.bm.path_prefix().join(CATEGORY).join("[Test Author] Truncated Count.epub");
        create_epub_with_unbound_prefix(&epub_path);
        let data = fs::read(&epub_path).unwrap();
        fs::write(&epub_path, &data[..data.len() - 120]).unwrap();
        let _cleanup = TempFile(epub_path.clone());
        assert_eq!(BookManager::_get_epub_total_chapters(&epub_path), 0);
    }

    /// 챕터 하나의 ZIP 항목이 손상돼(Bad magic number / Overlapped entries) 읽히지 않아도 책 전체를 거부하지 않는다.
    ///
    /// 코퍼스의 9_BLGL [모아이] 사규연애 4권이 챕터 하나 때문에 통째로 422 였다.
    /// 읽을 수 없는 챕터만 누락 챕터처럼 건너뛰고 나머지는 열린다.
    #[tokio::test]
    async fn test_preview_skips_chapter_whose_zip_entry_is_unreadable() {
        for chapters in [0, 10] {
            let setup = setup!();
            let epub_path = setup.bm.path_prefix().join(CATEGORY).join("[Test Author] Unreadable Chapter.epub");
            create_epub_with_unbound_prefix(&epub_path);
            let offset = {
                let mut zf = ZipArchive::new(File::open(&epub_path).unwrap()).unwrap();
                zf.by_name("OEBPS/ch2.xhtml").unwrap().header_start() as usize
            };
            let mut data = fs::read(&epub_path).unwrap();
            data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]); // 로컬 헤더 시그니처를 망가뜨린다
            fs::write(&epub_path, &data).unwrap();

            let book = register(&setup, &epub_path).await;
            book.run(async {
                let response = setup.client.get(&format!("/preview/{}?chapters={chapters}", book.book_id)).await;
                assert_eq!(response.status, 200, "chapters={chapters}: {}", response.text());
                assert_eq!(response.header("x-total-chapters"), Some("1"), "chapters={chapters}");
                let mut zf = parse_epub_zip(&response.body);
                let names = namelist(&mut zf);
                assert!(names.contains(&"OEBPS/ch1.xhtml".to_string()), "chapters={chapters}");
                assert!(!names.contains(&"OEBPS/ch2.xhtml".to_string()), "chapters={chapters}");
                let opf = zip_read_string(&mut zf, "OEBPS/content.opf");
                assert!(opf.contains("idref=\"ch1\"") && !opf.contains("idref=\"ch2\""), "chapters={chapters}");
            })
            .await;
        }
    }

    /// OPF에 선언되지 않은 프리픽스가 있어도 총 챕터 수가 올바르게 반환된다.
    #[tokio::test]
    async fn test_total_chapters_unbound_prefix() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Unbound Prefix Chapters.epub");
        create_epub_with_unbound_prefix(&epub_path);
        let _cleanup = TempFile(epub_path.clone());

        let count = BookManager::_get_epub_total_chapters(&epub_path);
        assert_eq!(count, 2, "Expected 2 chapters, got {count}");
    }

    // ── missing container.xml ────────────────────────────

    /// container.xml이 없어도 OPF를 직접 탐색하여 미리보기가 생성된다.
    #[tokio::test]
    async fn test_preview_missing_container_xml() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] No Container Book.epub");
        create_epub_without_container_xml(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200, "Missing container.xml should not cause 500, got {}: {}", response.status, response.text());

            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(names.contains(&"mimetype".to_string()));
            assert!(names.iter().any(|n| n.contains("ch1")), "ch1 should be in preview");
            assert!(names.iter().any(|n| n.contains("ch2")), "ch2 should be in preview");
            // container.xml이 원본에 없으므로 미리보기에도 없어야 함
            assert!(!names.contains(&"META-INF/container.xml".to_string()));
        })
        .await;
    }

    /// container.xml이 없어도 총 챕터 수가 올바르게 반환된다.
    #[tokio::test]
    async fn test_total_chapters_missing_container_xml() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] No Container Chapters.epub");
        create_epub_without_container_xml(&epub_path);
        let _cleanup = TempFile(epub_path.clone());

        let count = BookManager::_get_epub_total_chapters(&epub_path);
        assert_eq!(count, 2, "Expected 2 chapters, got {count}");
    }

    // ── corrupted container.xml (lxml recover) ───────────

    /// container.xml에 선언되지 않은 네임스페이스 프리픽스가 있어도 lxml recover로 미리보기가 생성된다.
    #[tokio::test]
    async fn test_preview_corrupted_container_xml() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Bad Container Book.epub");
        create_epub_with_corrupted_container_xml(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200, "Corrupted container.xml should not cause 500, got {}: {}", response.status, response.text());

            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(names.iter().any(|n| n.contains("ch1")), "ch1 should be in preview");
            assert!(names.iter().any(|n| n.contains("ch2")), "ch2 should be in preview");
        })
        .await;
    }

    // ── missing spine ────────────────────────────────────

    /// spine이 없어도 manifest의 XHTML 문서 순서로 미리보기가 생성된다.
    #[tokio::test]
    async fn test_preview_missing_spine() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] No Spine Book.epub");
        create_epub_without_spine(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200, "Missing spine should not cause 500, got {}: {}", response.status, response.text());

            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(names.contains(&"mimetype".to_string()));
            assert!(names.iter().any(|n| n.contains("ch1")), "ch1 should be in preview");
            assert!(names.iter().any(|n| n.contains("ch2")), "ch2 should be in preview");
            // toc.ncx는 비문서 항목이므로 챕터로 선택되면 안 됨
            assert!(!names.iter().any(|n| n.contains("toc.ncx")), "toc.ncx (non-XHTML) should not be selected as chapter");
        })
        .await;
    }

    /// spine이 없으면 총 챕터 수가 0으로 반환된다.
    #[tokio::test]
    async fn test_total_chapters_missing_spine() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] No Spine Chapters.epub");
        create_epub_without_spine(&epub_path);
        let _cleanup = TempFile(epub_path.clone());

        let count = BookManager::_get_epub_total_chapters(&epub_path);
        assert_eq!(count, 0, "Expected 0 (no spine), got {count}");
    }

    // ── OPF missing in archive ───────────────────────────

    /// container.xml이 존재하지 않는 OPF를 가리키면 422를 반환한다.
    #[tokio::test]
    async fn test_preview_opf_missing_in_archive() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Missing OPF Book.epub");
        create_epub_with_missing_opf(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 422, "Missing OPF in archive should return 422, got {}", response.status);
            assert!(response.text().contains("OPF"));
        })
        .await;
    }

    // ── no OPF at all ────────────────────────────────────

    /// container.xml도 없고 .opf 파일도 없으면 422를 반환한다.
    #[tokio::test]
    async fn test_preview_no_opf_at_all() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] No OPF Book.epub");
        create_epub_with_no_opf_at_all(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 422, "No OPF at all should return 422, got {}", response.status);
        })
        .await;
    }

    // ── binary garbage container.xml (regex fallback) ───

    /// container.xml이 완전한 바이너리 쓰레기여도 regex 폴백으로 미리보기가 생성된다.
    #[tokio::test]
    async fn test_preview_binary_garbage_container_xml() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Binary Garbage Container Book.epub");
        create_epub_with_binary_garbage_container(&epub_path);

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200, "Binary garbage container.xml should not cause 500, got {}: {}", response.status, response.text());

            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(names.iter().any(|n| n.contains("ch1")), "ch1 should be in preview");
            assert!(names.iter().any(|n| n.contains("ch2")), "ch2 should be in preview");
        })
        .await;
    }

    /// container.xml이 깨진 XML이어도 총 챕터 수가 올바르게 반환된다.
    #[tokio::test]
    async fn test_total_chapters_corrupted_container_xml() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Corrupted Container Chapters.epub");
        create_epub_with_corrupted_container_xml(&epub_path);
        let _cleanup = TempFile(epub_path.clone());

        let count = BookManager::_get_epub_total_chapters(&epub_path);
        assert_eq!(count, 2, "Expected 2 chapters, got {count}");
    }

    /// container.xml이 바이너리 쓰레기여도 regex 폴백으로 총 챕터 수가 반환된다.
    #[tokio::test]
    async fn test_total_chapters_binary_garbage_container() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Garbage Container Chapters.epub");
        create_epub_with_binary_garbage_container(&epub_path);
        let _cleanup = TempFile(epub_path.clone());

        let count = BookManager::_get_epub_total_chapters(&epub_path);
        assert_eq!(count, 2, "Expected 2 chapters, got {count}");
    }

    // ── bad ZIP ──────────────────────────────────────────

    /// 손상된 ZIP 파일은 422를 반환한다.
    #[tokio::test]
    async fn test_preview_bad_zip_file() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Bad Zip Book.epub");
        fs::create_dir_all(epub_path.parent().unwrap()).unwrap();
        fs::write(&epub_path, b"this is not a zip file at all").unwrap();

        let book = register(&setup, &epub_path).await;

        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 422, "Bad ZIP should return 422, got {}", response.status);
        })
        .await;
    }

    /// 손상된 ZIP 파일에 대해 총 챕터 수가 0으로 반환된다.
    #[tokio::test]
    async fn test_total_chapters_bad_zip() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Bad Zip Chapters.epub");
        fs::create_dir_all(epub_path.parent().unwrap()).unwrap();
        fs::write(&epub_path, b"not a zip").unwrap();
        let _cleanup = TempFile(epub_path.clone());

        let count = BookManager::_get_epub_total_chapters(&epub_path);
        assert_eq!(count, 0);
    }
}

// ── tests: _find_opf_path 단위 테스트 ────────────────────

/// `BookManager::_find_opf_path` 헬퍼 단위 테스트.
mod test_find_opf_path {
    use super::*;

    fn find_opf_path(epub_path: &Path) -> String {
        let mut zin = ZipArchive::new(File::open(epub_path).unwrap()).unwrap();
        BookManager::_find_opf_path(&mut zin).unwrap_or_default()
    }

    /// 정상 container.xml에서 OPF 경로를 추출한다.
    #[test]
    fn test_normal_container_xml() {
        let tmp = tempfile::tempdir().unwrap();
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("OEBPS/content.opf", b"<package/>")]);
        assert_eq!(find_opf_path(&epub_path), "OEBPS/content.opf");
    }

    /// container.xml이 없으면 ZIP 내 .opf 파일을 직접 찾는다.
    #[test]
    fn test_missing_container_xml_finds_opf() {
        let tmp = tempfile::tempdir().unwrap();
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("content.opf", b"<package/>")]);
        assert_eq!(find_opf_path(&epub_path), "content.opf");
    }

    /// container.xml이 깨진 XML이면 regex로 full-path를 추출한다.
    #[test]
    fn test_corrupted_container_xml_regex_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let bad_container = br#"<container><rootfiles><rootfile full-path="OEBPS/pkg.opf" broken:attr="x"/></rootfiles></container>"#;
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("META-INF/container.xml", bad_container), ("OEBPS/pkg.opf", b"<package/>")]);
        let result = find_opf_path(&epub_path);
        assert_eq!(result, "OEBPS/pkg.opf", "Regex fallback should find OPF, got '{result}'");
    }

    /// container.xml도 .opf 파일도 없으면 빈 문자열을 반환한다.
    #[test]
    fn test_no_opf_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("mimetype", b"application/epub+zip")]);
        assert_eq!(find_opf_path(&epub_path), "");
    }

    /// container.xml에 rootfile이 있지만 full-path가 비어있으면 .opf 직접 탐색으로 폴백.
    #[test]
    fn test_container_xml_empty_rootfile() {
        let tmp = tempfile::tempdir().unwrap();
        let container = r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("META-INF/container.xml", container.as_bytes()), ("OPS/book.opf", b"<package/>")]);
        let result = find_opf_path(&epub_path);
        assert_eq!(result, "OPS/book.opf", "Should fallback to direct scan, got '{result}'");
    }

    /// container.xml이 완전한 바이너리 쓰레기여도 regex로 full-path를 추출한다.
    #[test]
    fn test_binary_garbage_container_regex_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let garbage = b"\x89PNG\r\n not XML at all full-path=\"OPS/pkg.opf\" end";
        let epub_path = tmp.path().join("test.epub");
        write_zip_stored(&epub_path, &[("META-INF/container.xml", garbage), ("OPS/pkg.opf", b"<package/>")]);
        let result = find_opf_path(&epub_path);
        assert_eq!(result, "OPS/pkg.opf", "Binary garbage regex fallback should find OPF, got '{result}'");
    }
}

// ── tests: download 엔드포인트 ────────────────────────────

mod test_book_download {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tokio_stream::StreamExt;
    use tower::ServiceExt;

    fn client_with_header(book: &TestBook, name: &str, value: &str) -> TestClient {
        let token = backend::auth::create_jwt_token("admin@test.com", "admin", Some("Test Admin"), None);
        TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(book.setup.bm.clone()))).cookie("tm_access_token", &token).header(name, value)
    }

    /// book_id만으로 download 요청 → 200 + EPUB 바이너리.
    #[tokio::test]
    async fn test_download_by_book_id_only() {
        let book = book!();
        book.run(async {
            let response = book.client().get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(response.status, 200);
            assert!(!response.body.is_empty());
            let content_type = response.header("content-type").unwrap_or("");
            assert!(content_type.contains("epub") || content_type.contains("octet-stream"), "content-type={content_type}");
        })
        .await;
    }

    /// download 응답에 Content-Encoding: identity와 Cache-Control: no-transform이 포함된다.
    #[tokio::test]
    async fn test_download_response_headers() {
        let book = book!();
        book.run(async {
            let response = book.client().get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(response.status, 200);
            assert_eq!(response.header("content-encoding"), Some("identity"));
            assert_eq!(response.header("cache-control"), Some("no-transform"));
            assert_eq!(response.header("content-length").unwrap().parse::<u64>().unwrap(), fs::metadata(&book.epub_path).unwrap().len());
            assert_eq!(response.header("accept-ranges"), Some("bytes"));
            assert!(response.header("last-modified").is_some());
            assert!(response.header("etag").is_some_and(|value| value.starts_with('"') && value.ends_with('"')));
        })
        .await;
    }

    #[tokio::test]
    async fn test_download_honors_single_byte_range() {
        let book = book!();
        book.run(async {
            let client = client_with_header(&book, "range", "bytes=0-3");
            let expected = fs::read(&book.epub_path).unwrap();
            let response = client.get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(response.status, 206);
            assert_eq!(response.header("content-range"), Some(format!("bytes 0-3/{}", expected.len()).as_str()));
            assert_eq!(response.body, expected[..4]);
        })
        .await;
    }

    #[tokio::test]
    async fn test_download_streams_file_in_bounded_chunks() {
        let book = book!();
        book.run(async {
            let content = vec![b'x'; 512 * 1024];
            fs::write(&book.epub_path, &content).unwrap();
            let token = backend::auth::create_jwt_token("admin@test.com", "admin", Some("Test Admin"), None);
            let app = app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(book.setup.bm.clone()));
            let request = Request::builder().uri(format!("/download/{}", book.book_id)).header("cookie", format!("tm_access_token={token}")).body(Body::empty()).unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), 200);
            let mut chunks = response.into_body().into_data_stream();
            let mut received = Vec::new();
            let mut largest_chunk = 0;
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk.unwrap();
                largest_chunk = largest_chunk.max(chunk.len());
                received.extend_from_slice(&chunk);
            }
            assert!(largest_chunk <= 64 * 1024, "largest chunk was {largest_chunk} bytes");
            assert_eq!(received, content);

            let range_request = Request::builder()
                .uri(format!("/download/{}", book.book_id))
                .header("cookie", format!("tm_access_token={token}"))
                .header("range", "bytes=0-299999,400000-499999")
                .body(Body::empty())
                .unwrap();
            let response = app.oneshot(range_request).await.unwrap();
            assert_eq!(response.status(), 206);
            let content_length = response.headers().get("content-length").unwrap().to_str().unwrap().parse::<usize>().unwrap();
            let mut chunks = response.into_body().into_data_stream();
            let mut largest_chunk = 0;
            let mut received = 0;
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk.unwrap();
                largest_chunk = largest_chunk.max(chunk.len());
                received += chunk.len();
            }
            assert!(largest_chunk <= 64 * 1024, "multi-range chunk was {largest_chunk} bytes");
            assert_eq!(received, content_length);
        })
        .await;
    }

    #[tokio::test]
    async fn test_download_supports_suffix_and_multiple_byte_ranges() {
        let book = book!();
        book.run(async {
            let expected = fs::read(&book.epub_path).unwrap();
            let suffix = client_with_header(&book, "range", "bytes=-8").get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(suffix.status, 206);
            assert_eq!(suffix.body, expected[expected.len() - 8..]);

            let multiple = client_with_header(&book, "range", "bytes=0-1,4-5").get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(multiple.status, 206);
            assert!(multiple.header("content-type").unwrap().starts_with("multipart/byteranges; boundary="));
            assert!(multiple.text().contains("Content-Range: bytes 0-1/"));
            assert!(multiple.text().contains("Content-Range: bytes 4-5/"));
            assert!(multiple.body.windows(2).any(|part| part == &expected[..2]));
            assert!(multiple.body.windows(2).any(|part| part == &expected[4..6]));
        })
        .await;
    }

    #[tokio::test]
    async fn test_download_range_errors_and_if_range_match_file_response() {
        let book = book!();
        book.run(async {
            let expected = fs::read(&book.epub_path).unwrap();
            let unsatisfiable = client_with_header(&book, "range", "bytes=999999-").get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(unsatisfiable.status, 416);
            assert_eq!(unsatisfiable.header("content-range"), Some(format!("bytes */{}", expected.len()).as_str()));
            let oversized_offset = client_with_header(&book, "range", "bytes=18446744073709551616-").get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(oversized_offset.status, 416);

            let malformed = client_with_header(&book, "range", "items=0-1").get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(malformed.status, 400);
            assert_eq!(malformed.text(), "Only support bytes range");

            let baseline = book.client().get(&format!("/download/{}", book.book_id)).await;
            let validator = baseline.header("etag").unwrap().to_string();
            let matched = client_with_header(&book, "range", "bytes=0-3").header("if-range", &validator).get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(matched.status, 206);

            let ignored = client_with_header(&book, "range", "bytes=0-3").header("if-range", "stale-validator").get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(ignored.status, 200);
            assert_eq!(ignored.body, expected);
        })
        .await;
    }

    /// 라우트 제거 후 /download/{id}/{path} → 404 (회귀 테스트).
    #[tokio::test]
    async fn test_download_with_path_returns_404() {
        let book = book!();
        book.run(async {
            let response = book.client().get(&format!("/download/{}/some/path.epub", book.book_id)).await;
            // {path:path} 라우트 제거 후, 이 URL은 매칭되는 라우트가 없으므로 404
            assert_eq!(response.status, 404);
        })
        .await;
    }

    /// 없는 book_id → 200 + 빈 문자열 응답.
    #[tokio::test]
    async fn test_download_nonexistent_book() {
        let setup = setup!();
        let response = setup.client.get("/download/999999999").await;
        // get_book_content는 doc 못 찾으면 빈 문자열 반환 (FastAPI가 JSON "\"\"" 또는 "" 직렬화)
        assert_eq!(response.status, 200);
        assert_eq!(response.text().trim_matches('"'), "");
    }

    /// download한 EPUB이 유효한 ZIP 파일이다.
    #[tokio::test]
    async fn test_download_returns_valid_epub_content() {
        let book = book!();
        book.run(async {
            let response = book.client().get(&format!("/download/{}", book.book_id)).await;
            assert_eq!(response.status, 200);

            // 유효한 ZIP/EPUB인지 확인
            let mut zf = parse_epub_zip(&response.body);
            let names = namelist(&mut zf);
            assert!(names.contains(&"mimetype".to_string()));
            assert!(names.contains(&"META-INF/container.xml".to_string()));
        })
        .await;
    }
}

// ── tests: 캐시 정리 (eviction) ────────────────────────────

/// `BookManager::_evict_old_cache` 단위 테스트.
mod test_cache_eviction {
    use super::*;

    const DAY: u64 = 86400;

    /// 1일 이상 된 파일이 삭제된다.
    #[test]
    fn test_old_files_are_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path();

        let old_file = tmp_path.join("old.epub");
        fs::write(&old_file, "old").unwrap();
        // mtime을 2일 전으로 설정
        set_mtime_ago(&old_file, 2 * DAY);

        let new_file = tmp_path.join("new.epub");
        fs::write(&new_file, "new").unwrap();

        BookManager::_evict_old_cache(tmp_path);

        assert!(!old_file.exists());
        assert!(new_file.exists());
    }

    /// 1일 미만 파일은 보존된다.
    #[test]
    fn test_recent_files_are_kept() {
        let tmp = tempfile::tempdir().unwrap();

        let recent = tmp.path().join("recent.pdf");
        fs::write(&recent, "recent").unwrap();

        BookManager::_evict_old_cache(tmp.path());

        assert!(recent.exists());
    }

    /// 정확히 1일 된 파일도 삭제된다.
    #[test]
    fn test_exactly_one_day_old_is_deleted() {
        let tmp = tempfile::tempdir().unwrap();

        let edge_file = tmp.path().join("edge.html");
        fs::write(&edge_file, "edge").unwrap();
        set_mtime_ago(&edge_file, DAY + 1); // 1일 + 1초

        BookManager::_evict_old_cache(tmp.path());

        assert!(!edge_file.exists());
    }

    /// 하위 디렉토리는 삭제하지 않는다.
    #[test]
    fn test_subdirectories_are_not_deleted() {
        let tmp = tempfile::tempdir().unwrap();

        let sub_dir = tmp.path().join("subdir");
        fs::create_dir(&sub_dir).unwrap();
        File::open(&sub_dir).unwrap().set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * DAY))).unwrap();

        BookManager::_evict_old_cache(tmp.path());

        assert!(sub_dir.exists());
    }

    /// 빈 디렉토리에서 에러 없이 동작한다.
    #[test]
    fn test_empty_directory_no_error() {
        let tmp = tempfile::tempdir().unwrap();

        BookManager::_evict_old_cache(tmp.path()); // 에러 없이 완료
    }

    /// 존재하지 않는 디렉토리에서 에러 없이 동작한다.
    #[test]
    fn test_nonexistent_directory_no_error() {
        let tmp = tempfile::tempdir().unwrap();

        let fake_dir = tmp.path().join("nonexistent");
        BookManager::_evict_old_cache(&fake_dir); // 에러 없이 완료
    }

    /// 오래된 파일이 여러 개일 때 모두 삭제된다.
    #[test]
    fn test_multiple_old_files_all_deleted() {
        let tmp = tempfile::tempdir().unwrap();

        let mut old_files = Vec::new();
        for i in 0..5 {
            let f = tmp.path().join(format!("old_{i}.epub"));
            fs::write(&f, format!("old_{i}")).unwrap();
            set_mtime_ago(&f, 2 * DAY);
            old_files.push(f);
        }

        let new_file = tmp.path().join("new.pdf");
        fs::write(&new_file, "new").unwrap();

        BookManager::_evict_old_cache(tmp.path());

        for f in &old_files {
            assert!(!f.exists(), "{} should have been evicted", f.display());
        }
        assert!(new_file.exists());
    }

    /// 개별 파일 삭제 실패 시 나머지 파일 처리가 계속된다.
    #[test]
    fn test_deletion_failure_continues_processing() {
        let tmp = tempfile::tempdir().unwrap();

        let mut files = Vec::new();
        for i in 0..3 {
            let f = tmp.path().join(format!("old_{i}.epub"));
            fs::write(&f, format!("old_{i}")).unwrap();
            set_mtime_ago(&f, 2 * DAY);
            files.push(f);
        }

        // 첫 `remove_file`만 PermissionError로 실패한다. 나머지는 안쪽 RealFs가 실제로 지운다.
        let first_call = Arc::new(Mutex::new(true));
        let faulty = FaultyFs::new();
        faulty.set_remove_file(move |_path: &Path| {
            let mut first = first_call.lock().unwrap();
            if *first {
                *first = false;
                return Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "mocked deletion failure"));
            }
            None
        });
        BookManager::_evict_old_cache_with(faulty.as_ref(), tmp.path());

        // 1개는 실패로 남고, 나머지 2개는 삭제됨
        let remaining: Vec<&PathBuf> = files.iter().filter(|f| f.exists()).collect();
        assert_eq!(remaining.len(), 1);
    }

    /// 방금 쓴 캐시 파일과 오래된 파일이 공존할 때, 새 파일만 보존된다.
    #[test]
    fn test_just_written_cache_file_is_not_evicted() {
        let tmp = tempfile::tempdir().unwrap();

        let mut old_files = Vec::new();
        for name in ["1.pdf", "2_ch5.epub", "3.html", "4_p1-3.pdf"] {
            let f = tmp.path().join(name);
            fs::write(&f, "old").unwrap();
            set_mtime_ago(&f, 3 * DAY);
            old_files.push(f);
        }

        // 방금 생성된 파일
        let just_written = tmp.path().join("5_ch10.epub");
        fs::write(&just_written, "fresh").unwrap();

        BookManager::_evict_old_cache(tmp.path());

        for f in &old_files {
            assert!(!f.exists(), "{} should have been evicted", f.file_name().unwrap().to_string_lossy());
        }
        assert!(just_written.exists());
    }
}

/// `BookManager::_validate_preview_epub` 단위 테스트.
mod test_validate_preview_epub {
    use super::*;

    /// 빈 spine 경로는 ZIP 항목 이름으로 인정하지 않는다(Python `test_in_names_empty_path`).
    #[test]
    fn test_empty_spine_href_is_not_present_in_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("empty_href.epub");
        let opf = make_opf(1, false, false).replace("href=\"ch1.xhtml\"", "href=\"\"");
        let container =
            r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        write_zip_stored(
            &epub,
            &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", container.as_bytes()), ("content.opf", opf.as_bytes()), ("toc.ncx", b"<ncx/>"), ("ch1.xhtml", chapter(1).as_bytes())],
        );

        let (valid, error) = BookManager::_validate_preview_epub(&epub);

        assert!(!valid);
        assert!(err_has(&error, "no valid spine chapters"));
    }

    /// 정상 EPUB은 (True, None)을 반환한다.
    #[test]
    fn test_valid_epub_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("valid.epub");
        create_test_epub(&epub, 3, false, false);

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(valid);
        assert!(err.is_none());
    }

    /// mimetype 파일이 없으면 거부된다.
    #[test]
    fn test_missing_mimetype_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("no_mimetype.epub");
        write_zip_stored(
            &epub,
            &[
                ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
                ("OEBPS/content.opf", make_opf(1, false, false).as_bytes()),
                ("OEBPS/toc.ncx", b"<ncx/>"),
                ("OEBPS/ch1.xhtml", chapter(1).as_bytes()),
            ],
        );

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(!valid);
        assert!(err_has(&err, "mimetype"));
    }

    /// mimetype 내용이 잘못되면 거부된다.
    #[test]
    fn test_invalid_mimetype_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("bad_mimetype.epub");
        write_zip_stored(
            &epub,
            &[
                ("mimetype", b"text/plain"),
                ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
                ("OEBPS/content.opf", make_opf(1, false, false).as_bytes()),
                ("OEBPS/toc.ncx", b"<ncx/>"),
                ("OEBPS/ch1.xhtml", chapter(1).as_bytes()),
            ],
        );

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(!valid);
        assert!(err_has(&err, "invalid mimetype"));
    }

    /// OPF 파일이 없으면 거부된다.
    #[test]
    fn test_missing_opf_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("no_opf.epub");
        write_zip_stored(&epub, &[("mimetype", b"application/epub+zip"), ("OEBPS/ch1.xhtml", chapter(1).as_bytes())]);

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(!valid);
        assert!(err_has(&err, "OPF"));
    }

    /// 유효한 spine 챕터가 0개이면 거부된다.
    #[test]
    fn test_no_valid_spine_chapters_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("empty_spine.epub");
        create_corrupted_epub(&epub, 2, true);

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(!valid);
        assert!(err_has(&err, "no valid spine chapters"));
    }

    /// NCX 파일이 ZIP에 없으면 toc 속성이 제거되고 통과한다.
    #[test]
    fn test_missing_ncx_toc_attribute_removed() {
        let tmp = tempfile::tempdir().unwrap();
        // toc="toc"이 있지만 toc.ncx 파일 없는 EPUB
        let epub = tmp.path().join("no_ncx.epub");
        // toc.ncx 파일 생략
        write_zip_stored(
            &epub,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
                ("OEBPS/content.opf", make_opf(2, false, false).as_bytes()),
                ("OEBPS/ch1.xhtml", chapter(1).as_bytes()),
                ("OEBPS/ch2.xhtml", chapter(2).as_bytes()),
            ],
        );

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(valid, "Expected valid, got: {err:?}");

        // 수정된 EPUB에서 toc 속성이 제거되었는지 확인
        let (toc, _) = spine_of(&epub, "OEBPS/content.opf");
        assert!(toc.is_none(), "toc attribute should be removed");
    }

    /// manifest에 없는 spine idref가 자동 제거되고 통과한다.
    #[test]
    fn test_invalid_spine_idref_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("bad_idref.epub");
        create_epub_with_invalid_spine(&epub);

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(valid, "Expected valid, got: {err:?}");

        // 수정된 EPUB에서 coverpage idref가 제거되었는지 확인
        let (_, idrefs) = spine_of(&epub, "OEBPS/content.opf");
        assert!(!idrefs.contains(&"coverpage".to_string()), "invalid idref should be removed");
        assert!(idrefs.contains(&"ch1".to_string()));
        assert!(idrefs.contains(&"ch2".to_string()));
    }

    /// manifest에는 있지만 ZIP에 파일이 없는 spine 항목이 자동 제거된다.
    #[test]
    fn test_spine_item_missing_from_zip_removed() {
        let tmp = tempfile::tempdir().unwrap();
        // ch1이 누락된 EPUB (ch2, ch3은 존재)
        let epub = tmp.path().join("missing_ch1.epub");
        create_corrupted_epub(&epub, 3, false);

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(valid, "Expected valid, got: {err:?}");

        // ch1이 spine에서 제거되었는지 확인
        let (_, idrefs) = spine_of(&epub, "OEBPS/content.opf");
        assert!(!idrefs.contains(&"ch1".to_string()), "missing ch1 should be removed from spine");
        assert!(idrefs.contains(&"ch2".to_string()));
        assert!(idrefs.contains(&"ch3".to_string()));
    }

    /// 손상된 ZIP 파일은 거부된다.
    #[test]
    fn test_corrupted_zip_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("corrupted.epub");
        fs::write(&epub, b"this is not a zip file").unwrap();

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(!valid);
        assert!(err_has(&err, "corrupted ZIP"));
    }

    /// spine 요소가 없는 OPF는 거부된다.
    #[test]
    fn test_missing_spine_element_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("no_spine.epub");
        create_epub_without_spine(&epub);

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(!valid);
        assert!(err_has(&err, "spine"));
    }

    /// toc 속성이 manifest에 없는 ID를 참조하면 제거된다.
    #[test]
    fn test_toc_id_not_in_manifest_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let opf_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test</dc:title>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="nonexistent_toc">
    <itemref idref="ch1"/>
  </spine>
</package>"#;

        let epub = tmp.path().join("bad_toc_id.epub");
        write_zip_stored(
            &epub,
            &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("OEBPS/content.opf", opf_content.as_bytes()), ("OEBPS/ch1.xhtml", chapter(1).as_bytes())],
        );

        let (valid, err) = BookManager::_validate_preview_epub(&epub);
        assert!(valid, "Expected valid, got: {err:?}");

        let (toc, _) = spine_of(&epub, "OEBPS/content.opf");
        assert!(toc.is_none());
    }

    /// 문제가 없는 EPUB은 재작성하지 않는다 (mtime 불변).
    #[test]
    fn test_no_rewrite_when_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("clean.epub");
        create_test_epub(&epub, 2, false, false);
        let mtime_before = fs::metadata(&epub).unwrap().modified().unwrap();

        let (valid, _err) = BookManager::_validate_preview_epub(&epub);
        assert!(valid);
        let mtime_after = fs::metadata(&epub).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "clean EPUB should not be rewritten");
    }
}

/// 미리보기 EPUB 생성 시 NCX navPoint 필터링 테스트.
mod test_ncx_filtering {
    use super::*;

    /// NCX에서 미리보기에 포함되지 않은 파일 참조 navPoint가 제거된다.
    #[tokio::test]
    async fn test_preview_strips_ncx_extra_navpoints() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] NCX Extra Refs.epub");
        create_epub_with_ncx_extra_refs(&epub_path, 5, 10);

        let book = register(&setup, &epub_path).await;
        book.run(async {
            unlink(&book.cache_file());

            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let ncx_data = zip_read_string(&mut zf, "OEBPS/toc.ncx");

            let ncx_tree = roxmltree::Document::parse(&ncx_data).unwrap();
            let nav_points = ns_nodes(&ncx_tree, NCX_NS, "navPoint");

            // 2챕터만 요청했으므로 ch1, ch2만 포함
            assert!(nav_points.len() <= 2, "NCX should have at most 2 navPoints, got {}", nav_points.len());

            // 남아있는 navPoint의 src가 모두 ch1 또는 ch2를 참조
            for np in &nav_points {
                let content = ns_children(*np, NCX_NS, "content")[0];
                let src = content.attribute("src").unwrap_or("");
                let src_file = src.split('#').next().unwrap();
                assert!(["ch1.xhtml", "ch2.xhtml"].contains(&src_file), "NCX navPoint references unexpected file: {src_file}");
            }
        })
        .await;
    }

    /// NCX에서 fragment-only src를 가진 navPoint는 유지된다.
    #[tokio::test]
    async fn test_preview_keeps_fragment_only_navpoints() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] NCX Fragment Only.epub");

        let ncx = format!(
            r##"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{NCX_NS}" version="2005-1">
  <navMap>
    <navPoint id="np-1" playOrder="1">
      <navLabel><text>Chapter 1</text></navLabel>
      <content src="ch1.xhtml"/>
    </navPoint>
    <navPoint id="np-frag" playOrder="2">
      <navLabel><text>Fragment Ref</text></navLabel>
      <content src="#bookmark"/>
    </navPoint>
    <navPoint id="np-missing" playOrder="3">
      <navLabel><text>Missing</text></navLabel>
      <content src="ch99.xhtml"/>
    </navPoint>
  </navMap>
</ncx>"##
        );
        write_epub(
            &epub_path,
            vec![
                entry("META-INF/container.xml", CONTAINER_XML),
                entry("OEBPS/content.opf", make_opf(2, false, false)),
                entry("OEBPS/toc.ncx", ncx),
                entry("OEBPS/ch1.xhtml", chapter(1)),
                entry("OEBPS/ch2.xhtml", chapter(2)),
            ],
        );

        let book = register(&setup, &epub_path).await;
        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let ncx_data = zip_read_string(&mut zf, "OEBPS/toc.ncx");

            let ncx_tree = roxmltree::Document::parse(&ncx_data).unwrap();
            let nav_ids: Vec<String> = ns_nodes(&ncx_tree, NCX_NS, "navPoint").iter().filter_map(|np| np.attribute("id").map(String::from)).collect();

            assert!(nav_ids.contains(&"np-1".to_string()), "ch1 navPoint should be kept");
            assert!(nav_ids.contains(&"np-frag".to_string()), "fragment-only navPoint should be kept");
            assert!(!nav_ids.contains(&"np-missing".to_string()), "missing file navPoint should be removed");
        })
        .await;
    }

    /// guide reference의 href에 fragment가 포함되어도 올바르게 처리된다.
    #[tokio::test]
    async fn test_preview_guide_href_with_fragment() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Guide Fragment.epub");

        let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Guide Fragment Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
  <guide>
    <reference type="text" title="Start" href="ch1.xhtml#start"/>
    <reference type="toc" title="TOC" href="missing.xhtml#toc"/>
  </guide>
</package>"#;
        write_epub(
            &epub_path,
            vec![
                entry("META-INF/container.xml", CONTAINER_XML),
                entry("OEBPS/content.opf", opf),
                entry("OEBPS/toc.ncx", "<ncx/>"),
                entry("OEBPS/ch1.xhtml", chapter(1)),
                entry("OEBPS/ch2.xhtml", chapter(2)),
            ],
        );

        let book = register(&setup, &epub_path).await;
        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let opf_data = zip_read_string(&mut zf, "OEBPS/content.opf");

            let opf_tree = roxmltree::Document::parse(&opf_data).unwrap();
            let guide_refs = ns_nodes(&opf_tree, OPF_NS, "reference");

            // ch1.xhtml#start → ch1.xhtml은 포함되어 있으므로 유지
            // missing.xhtml#toc → missing.xhtml은 없으므로 제거
            let hrefs: Vec<&str> = guide_refs.iter().map(|r| r.attribute("href").unwrap_or("")).collect();
            assert!(hrefs.iter().any(|h| h.contains("ch1.xhtml")), "guide reference to ch1.xhtml#start should be kept");
            assert!(!hrefs.iter().any(|h| h.contains("missing.xhtml")), "guide reference to missing.xhtml should be removed");
        })
        .await;
    }
}

/// OPF에 선언되지 않은 네임스페이스 프리픽스(opf:role 등)가 있을 때 수정 검증.
mod test_opf_namespace_fix {
    use super::*;

    /// opf:role이 xmlns:opf 없이 사용된 OPF가 유효한 XML로 출력된다.
    #[tokio::test]
    async fn test_preview_fixes_undeclared_opf_prefix() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] OPF Prefix.epub");

        // xmlns:opf 선언 없이 opf:role 사용 (실제 EPUB에서 흔한 패턴)
        let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:test</dc:identifier>
    <dc:title>OPF Prefix Test</dc:title>
    <dc:creator opf:role="aut">Author</dc:creator>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#;
        write_epub(
            &epub_path,
            vec![
                entry("META-INF/container.xml", CONTAINER_XML),
                entry("OEBPS/content.opf", opf),
                entry("OEBPS/toc.ncx", "<ncx/>"),
                entry("OEBPS/ch1.xhtml", chapter(1)),
                entry("OEBPS/ch2.xhtml", chapter(2)),
            ],
        );

        let book = register(&setup, &epub_path).await;
        book.run(async {
            let response = setup.client.get(&format!("/preview/{}?chapters=2", book.book_id)).await;
            assert_eq!(response.status, 200);

            let mut zf = parse_epub_zip(&response.body);
            let opf_data = zip_read_string(&mut zf, "OEBPS/content.opf");

            // 출력 OPF가 유효한 XML인지 확인 (strict 파싱). 파싱 실패 시 panic
            roxmltree::Document::parse(&opf_data).unwrap();

            // xmlns:opf 선언이 추가되었는지 확인
            assert!(opf_data.contains("xmlns:opf="), "xmlns:opf declaration should be added");
            assert!(opf_data.contains("opf:role=\"aut\""), "opf:role attribute should be preserved");
        })
        .await;
    }
}

// ── tests: epubcheck 검증 API ─────────────────────────────

/// epubcheck 프로세스 대역의 호출 기록.
struct ProcProbe {
    timeouts: Mutex<Vec<Option<Duration>>>,
}

enum Behavior {
    /// json_path(args[3])에 내용을 기록한다.
    WritesJson { content: String, returncode: i32 },
    /// json_path에 아무것도 기록하지 않는다.
    Noop { returncode: i32 },
    /// `FileNotFoundError`
    NotInstalled,
    /// `communicate`가 시간 초과
    TimesOut,
}

/// `patch("asyncio.create_subprocess_exec", side_effect=...)` 대역.
struct FakeSpawner {
    behavior: Behavior,
    probe: Arc<ProcProbe>,
}

impl FakeSpawner {
    fn new(behavior: Behavior) -> (Arc<Self>, Arc<ProcProbe>) {
        let probe = Arc::new(ProcProbe { timeouts: Mutex::new(Vec::new()) });
        (Arc::new(Self { behavior, probe: probe.clone() }), probe)
    }
}

impl CommandRunner for FakeSpawner {
    fn run(&self, args: &[String], timeout: Option<Duration>) -> Result<CommandOutput, CommandError> {
        self.probe.timeouts.lock().unwrap().push(timeout);
        let returncode = match &self.behavior {
            Behavior::WritesJson { content, returncode } => {
                fs::write(&args[3], content).map_err(|e| CommandError::Failed(e.to_string()))?;
                *returncode
            }
            Behavior::Noop { returncode } => *returncode,
            Behavior::NotInstalled => return Err(CommandError::NotFound),
            Behavior::TimesOut => return Err(CommandError::Timeout),
        };
        Ok(CommandOutput { returncode, ..CommandOutput::default() })
    }

    fn which(&self, _program: &str) -> Option<PathBuf> {
        None
    }
}

/// epubcheck --json 출력을 흉내내는 JSON 문자열. 실제 epubcheck는 checker에 nInfo를 넣지 않고, 메시지 ID 키는 대문자 "ID"다.
fn make_epubcheck_json_integration(valid: bool, messages: Option<Value>, publication: Option<Value>) -> String {
    let no_messages = messages.as_ref().is_none_or(|m| m.as_array().is_some_and(|a| a.is_empty()));
    let mut data = json!({"messages": messages.unwrap_or_else(|| json!([])), "checker": {"nFatal": 0, "nError": if valid { 0 } else { 1 }, "nWarning": 0, "nUsage": 0}});
    if let Some(publication) = publication {
        data["publication"] = publication;
    }
    if !valid && no_messages {
        data["messages"] = json!([{"severity": "ERROR", "ID": "RSC-005", "message": "Test error message", "locations": [{"path": "OEBPS/ch1.xhtml", "line": 10, "column": 5}]}]);
    }
    data.to_string()
}

/// `GET /validate/{book_id}` 엔드포인트 테스트 (epubcheck mock).
mod test_validate_epub {
    use super::*;

    /// 정상 EPUB → valid=true 응답.
    #[tokio::test]
    async fn test_validate_valid_epub() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Valid Epub Check.epub");
        create_test_epub(&epub_path, 2, false, false);
        let book = register(&setup, &epub_path).await;

        book.run(async {
            let json_bytes = make_epubcheck_json_integration(true, None, Some(json!({"title": "Test Book", "creator": "Test Author", "date": "", "publisher": ""})));

            let (spawner, _probe) = FakeSpawner::new(Behavior::WritesJson { content: json_bytes, returncode: 0 });
            let client = setup.client_with_spawner(spawner);
            let response = client.get(&format!("/validate/{}", book.book_id)).await;

            assert_eq!(response.status, 200);
            let body = response.json();
            assert_eq!(body["status"], "success");
            assert_eq!(body["result"]["valid"], json!(true));
            assert_eq!(body["result"]["summary"]["error"], json!(0));
            assert!(body["result"].get("publication").is_some());
        })
        .await;
    }

    /// 에러가 있는 EPUB → valid=false, messages 포함.
    #[tokio::test]
    async fn test_validate_epub_with_errors() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Error Epub Check.epub");
        create_test_epub(&epub_path, 2, false, false);
        let book = register(&setup, &epub_path).await;

        book.run(async {
            let json_bytes = make_epubcheck_json_integration(false, None, None);

            let (spawner, _probe) = FakeSpawner::new(Behavior::WritesJson { content: json_bytes, returncode: 1 });
            let client = setup.client_with_spawner(spawner);
            let response = client.get(&format!("/validate/{}", book.book_id)).await;

            assert_eq!(response.status, 200);
            let body = response.json();
            assert_eq!(body["status"], "success");
            assert_eq!(body["result"]["valid"], json!(false));
            assert_eq!(body["result"]["summary"]["error"], json!(1));
            assert!(!body["result"]["messages"].as_array().unwrap().is_empty());
            let msg = &body["result"]["messages"][0];
            assert_eq!(msg["severity"], "ERROR");
            assert_eq!(msg["id"], "RSC-005");
            assert_eq!(msg["location"]["path"], "OEBPS/ch1.xhtml");
            // nInfo가 없는 실제 출력에서도 summary.info가 채워져야 한다
            assert_eq!(body["result"]["summary"]["info"], json!(0));
        })
        .await;
    }

    /// 지원하지 않는 파일 타입 → failure + 에러 메시지.
    #[tokio::test]
    async fn test_validate_unsupported_type() {
        let setup = setup!();

        // TXT 타입의 book 등록
        let txt_dir = setup.bm.path_prefix().join("_txt");
        fs::create_dir_all(&txt_dir).unwrap();
        let txt_path = txt_dir.join("[Test Author] Plain Text.txt");
        fs::write(&txt_path, "plain text content").unwrap();

        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(&txt_path).unwrap();
        let rel_path = txt_path.strip_prefix(setup.bm.path_prefix()).unwrap();
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f").to_string();
        let mut data = serde_json::Map::new();
        data.insert(
            meta.ino().to_string(),
            json!({"category": "_txt", "title": "Plain Text", "author": "Test Author", "file_path": rel_path.to_string_lossy(), "file_type": "txt", "file_size": meta.len(), "line_count": 1, "page_count": 0, "isbn": "", "summary": "test txt", "updated_time": now}),
        );
        setup.es.with(|s| s.docs.extend(serde_json::from_value::<std::collections::BTreeMap<i64, Value>>(Value::Object(data.clone())).unwrap()));
        let (book_id, _error) = split(setup.bm.add_book(&serde_json::from_value(Value::Object(data)).unwrap()).await);
        let book_id = book_id.expect("book_id");

        let book = Registered { setup: &setup, book_id, path: txt_path.clone() };
        book.run(async {
            let response = setup.client.get(&format!("/validate/{book_id}")).await;
            assert_eq!(response.status, 200);
            let body = response.json();
            assert_eq!(body["status"], "failure");
            assert!(body["error"].as_str().unwrap().to_lowercase().contains("not supported"));
        })
        .await;
    }

    /// 파일이 존재하지 않는 EPUB → failure + 에러 메시지.
    #[tokio::test]
    async fn test_validate_file_not_found() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Missing File.epub");
        create_test_epub(&epub_path, 1, false, false);
        let book = register(&setup, &epub_path).await;
        // 파일 삭제
        fs::remove_file(&epub_path).unwrap();

        book.run(async {
            let response = setup.client.get(&format!("/validate/{}", book.book_id)).await;
            assert_eq!(response.status, 200);
            let body = response.json();
            assert_eq!(body["status"], "failure");
            assert!(body["error"].as_str().unwrap().contains("File not found"));
        })
        .await;
    }

    /// 존재하지 않는 book_id → failure + 에러 메시지.
    #[tokio::test]
    async fn test_validate_book_not_found() {
        let setup = setup!();

        let response = setup.client.get("/validate/999999999").await;
        assert_eq!(response.status, 200);
        let body = response.json();
        assert_eq!(body["status"], "failure");
        assert!(body["error"].as_str().unwrap().contains("Book not found"));
    }

    /// epubcheck 미설치 → failure + 에러 메시지.
    #[tokio::test]
    async fn test_validate_epubcheck_not_installed() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] No Epubcheck.epub");
        create_test_epub(&epub_path, 1, false, false);
        let book = register(&setup, &epub_path).await;

        book.run(async {
            let (spawner, _probe) = FakeSpawner::new(Behavior::NotInstalled);
            let client = setup.client_with_spawner(spawner);
            let response = client.get(&format!("/validate/{}", book.book_id)).await;

            assert_eq!(response.status, 200);
            let body = response.json();
            assert_eq!(body["status"], "failure");
            assert!(body["error"].as_str().unwrap().contains("not installed"));
        })
        .await;
    }

    /// epubcheck 타임아웃 → failure + 에러 메시지.
    #[tokio::test]
    async fn test_validate_epubcheck_timeout() {
        let setup = setup!();
        let epub_dir = setup.bm.path_prefix().join(CATEGORY);
        let epub_path = epub_dir.join("[Test Author] Timeout Epubcheck.epub");
        create_test_epub(&epub_path, 1, false, false);
        let book = register(&setup, &epub_path).await;

        book.run(async {
            let (spawner, probe) = FakeSpawner::new(Behavior::TimesOut);
            let client = setup.client_with_spawner(spawner);
            let response = client.get(&format!("/validate/{}", book.book_id)).await;

            assert_eq!(response.status, 200);
            let body = response.json();
            assert_eq!(body["status"], "failure");
            assert!(body["error"].as_str().unwrap().contains("timed out"));
            assert_eq!(*probe.timeouts.lock().unwrap(), vec![Some(Duration::from_secs(60))]);
        })
        .await;
    }
}

// ── tests: epubcheck validate_epub 단위 테스트 (Docker 불필요) ──

/// 단위 테스트용 책 조회 설정. `manager`가 Deps를 만들 때마다 같은 책 조회 대역과 `search_by_id`가 `{}`를 돌려주는 ES 대역을 꽂는다.
/// epubcheck/PDF 대역은 만들 때 꽂아야 하므로 `validate_*_with`가 `configure`로 더한다.
struct UnitSetup {
    tmp_path: PathBuf,
    book: Option<BookRef>,
    error: Option<String>,
}

impl UnitSetup {
    fn manager(&self, configure: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> BookManager {
        common::setup_env();
        let es = FakeEsManager::new();
        if let Some(book) = &self.book {
            es.set_doc(Some(json!({"category":"test", "title":"Test", "author":"", "file_path":book.file_path.strip_prefix(&self.tmp_path).unwrap(), "file_type":book.file_type, "file_size":0, "updated_time":"2024-01-01T00:00:00"})));
        }
        if let Some(error) = &self.error {
            es.fail("search_by_id", error);
        }
        let deps = BookManagerDeps::new(self.tmp_path.clone(), es);
        BookManager::with_deps(configure(deps))
    }
}

/// 가짜 Book 객체를 돌려주는 조회 대역과 그 BookManager 설정.
fn unit_manager(tmp_path: &Path, book: Option<BookRef>, error: Option<&str>) -> UnitSetup {
    UnitSetup { tmp_path: tmp_path.to_path_buf(), book, error: error.map(String::from) }
}

/// validate_epub 단위 테스트에 쓰는 가짜 Book. 내용 `b"PK fake epub"`.
fn make_mock_epub_book(tmp_path: &Path, file_type: &str) -> BookRef {
    let epub_path = tmp_path.join("test.epub");
    fs::write(&epub_path, b"PK fake epub").unwrap();
    BookRef { book_id: 1, file_type: file_type.to_string(), file_path: epub_path }
}

fn make_epubcheck_json_unit(valid: bool, messages: Option<Value>, publication: Option<Value>) -> String {
    let no_messages = messages.as_ref().is_none_or(|m| m.as_array().is_some_and(|a| a.is_empty()));
    let mut data = json!({"messages": messages.unwrap_or_else(|| json!([])), "checker": {"nFatal": 0, "nError": if valid { 0 } else { 1 }, "nWarning": 0, "nUsage": 0, "nInfo": 0}});
    if let Some(publication) = publication {
        data["publication"] = publication;
    }
    if !valid && no_messages {
        data["messages"] = json!([{"severity": "ERROR", "id": "RSC-005", "message": "Test error", "locations": [{"path": "ch1.xhtml", "line": 10, "column": 5}]}]);
    }
    data.to_string()
}

/// `validate_epub` 메서드의 에지 케이스를 Docker 없이 직접 테스트.
mod test_validate_epub_unit {
    use super::*;

    /// epubcheck 대역을 끼운 manager로 `validate_epub(1)`을 부른다.
    async fn validate_with(bm: &UnitSetup, spawner: Arc<FakeSpawner>) -> (Option<Value>, Option<String>) {
        validate_configured(bm, |d| d.with_runner(spawner)).await
    }

    /// `configure`로 Deps를 더 꽂은 manager로 `validate_epub(1)`을 부른다.
    async fn validate_configured(bm: &UnitSetup, configure: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> (Option<Value>, Option<String>) {
        split(bm.manager(configure).validate_epub(1).await)
    }

    /// epubcheck가 유효하지 않은 JSON을 출력 → parse error.
    #[tokio::test]
    async fn test_invalid_json_output() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_epub_book(tmp.path(), "epub");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let (spawner, _probe) = FakeSpawner::new(Behavior::WritesJson { content: "NOT VALID JSON {{{}".to_string(), returncode: 0 });
        let (result, error) = validate_with(&bm, spawner).await;

        assert!(result.is_none());
        assert!(err_has(&error, "Failed to parse epubveri output"));
    }

    /// epubcheck가 빈 파일을 남김 → parse error.
    #[tokio::test]
    async fn test_empty_json_file() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_epub_book(tmp.path(), "epub");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let (spawner, _probe) = FakeSpawner::new(Behavior::Noop { returncode: 0 });
        let (result, error) = validate_with(&bm, spawner).await;

        assert!(result.is_none());
        assert!(err_has(&error, "Failed to parse epubveri output"));
    }

    /// 성공 시 임시 JSON 파일이 삭제되는지 검증.
    #[tokio::test]
    async fn test_temp_file_cleanup_on_success() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_epub_book(tmp.path(), "epub");
        let bm = unit_manager(tmp.path(), Some(book), None);

        // `tempfile.mkstemp` 추적 대신 FaultyFs의 `remove_file` 호출 기록으로 임시 JSON의 정리를 본다.
        let faulty = FaultyFs::new();

        let valid_json = make_epubcheck_json_unit(true, None, None);
        let (spawner, _probe) = FakeSpawner::new(Behavior::WritesJson { content: valid_json, returncode: 0 });
        let (_result, error) = validate_configured(&bm, |d| d.with_runner(spawner).with_fs(faulty.clone())).await;

        assert!(error.is_none());
        let created = faulty.calls_of("remove_file");
        assert_eq!(created.len(), 1);
        assert!(!created[0].exists(), "Temp file should be cleaned up");
    }

    /// JSON 파싱 실패 시에도 임시 파일이 삭제되는지 검증.
    #[tokio::test]
    async fn test_temp_file_cleanup_on_parse_error() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_epub_book(tmp.path(), "epub");
        let bm = unit_manager(tmp.path(), Some(book), None);

        // `tempfile.mkstemp` 추적 대신 FaultyFs의 `remove_file` 호출 기록으로 임시 JSON의 정리를 본다.
        let faulty = FaultyFs::new();

        let (spawner, _probe) = FakeSpawner::new(Behavior::WritesJson { content: "BROKEN".to_string(), returncode: 0 });
        let (result, _error) = validate_configured(&bm, |d| d.with_runner(spawner).with_fs(faulty.clone())).await;

        assert!(result.is_none());
        let created = faulty.calls_of("remove_file");
        assert_eq!(created.len(), 1);
        assert!(!created[0].exists(), "Temp file should be cleaned up even on error");
    }

    /// publication 없는 결과에서 publication 키가 빠지는지 검증.
    #[tokio::test]
    async fn test_no_publication_in_result() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_epub_book(tmp.path(), "epub");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let valid_json = make_epubcheck_json_unit(true, None, None); // publication 없음
        let (spawner, _probe) = FakeSpawner::new(Behavior::WritesJson { content: valid_json, returncode: 0 });
        let (result, error) = validate_with(&bm, spawner).await;

        assert!(error.is_none());
        let result = result.unwrap();
        assert_eq!(result["valid"], json!(true));
        assert!(result.get("publication").is_none());
    }

    /// publication 메타데이터가 결과에 포함되는지 검증.
    #[tokio::test]
    async fn test_with_publication_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_epub_book(tmp.path(), "epub");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let publication = json!({"title": "Test", "creator": "Author", "date": "2024", "publisher": "Pub"});
        let valid_json = make_epubcheck_json_unit(true, None, Some(publication));
        let (spawner, _probe) = FakeSpawner::new(Behavior::WritesJson { content: valid_json, returncode: 0 });
        let (result, error) = validate_with(&bm, spawner).await;

        assert!(error.is_none());
        let result = result.unwrap();
        assert_eq!(result["publication"]["title"], "Test");
        assert_eq!(result["publication"]["creator"], "Author");
    }

    /// 여러 메시지가 올바르게 파싱되는지 검증.
    #[tokio::test]
    async fn test_multiple_messages_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_epub_book(tmp.path(), "epub");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let messages = json!([
            {"severity": "ERROR", "id": "RSC-005", "message": "err1", "locations": [{"path": "ch1.xhtml", "line": 1, "column": 1}]},
            {"severity": "WARNING", "id": "HTM-003", "message": "warn1", "locations": [{"path": "ch2.xhtml", "line": 5, "column": 10}]}
        ]);
        let json_str = make_epubcheck_json_unit(false, Some(messages), None);
        let (spawner, _probe) = FakeSpawner::new(Behavior::WritesJson { content: json_str, returncode: 1 });
        let (result, error) = validate_with(&bm, spawner).await;

        assert!(error.is_none());
        let result = result.unwrap();
        assert_eq!(result["valid"], json!(false));
        assert_eq!(result["messages"].as_array().unwrap().len(), 2);
        assert_eq!(result["messages"][0]["severity"], "ERROR");
        assert_eq!(result["messages"][1]["severity"], "WARNING");
        assert_eq!(result["messages"][1]["location"]["path"], "ch2.xhtml");
    }
}

// ── tests: validate_pdf 단위 테스트 (pikepdf mock) ────────

/// `MagicMock` pikepdf 문서. `close` 호출 횟수는 `close_calls`로 확인한다.
struct MockPdf {
    issues: Result<Vec<String>, String>,
    docinfo: HashMap<String, String>,
    page_count: usize,
    pdf_version: String,
    close_calls: Arc<AtomicUsize>,
}

// TODO(common): `common/fake_pdf.rs`(PdfOpener/PdfDoc 대역)가 아직 없어 로컬로 둔다. `check_pdf_syntax`/`close` 호출 기록이 필요해 `StaticPdfDoc`로는 모자란다.
impl PdfDoc for MockPdf {
    fn check_pdf_syntax(&self) -> Result<Vec<String>, PdfError> {
        self.issues.clone().map_err(|message| PdfError { kind: "Exception".to_string(), message })
    }

    fn docinfo(&self) -> HashMap<String, String> {
        self.docinfo.clone()
    }

    fn page_count(&self) -> usize {
        self.page_count
    }

    fn pdf_version(&self) -> String {
        self.pdf_version.clone()
    }

    fn close(&self) {
        self.close_calls.fetch_add(1, Ordering::SeqCst);
    }
}

/// `patch("pikepdf.open", return_value=mock_pdf)` 또는 `side_effect=Exception(...)`.
struct FakeOpener(Mutex<Option<Result<MockPdf, String>>>);

impl PdfOpener for FakeOpener {
    fn open(&self, _path: &Path) -> Result<Box<dyn PdfDoc>, PdfError> {
        self.0.lock().unwrap().take().expect("pikepdf.open이 두 번 불렸다").map(|pdf| Box::new(pdf) as Box<dyn PdfDoc>).map_err(|message| PdfError { kind: "Exception".to_string(), message })
    }
}

fn make_mock_pdf(issues: Vec<String>, docinfo: &[(&str, &str)], page_count: usize, pdf_version: &str) -> (MockPdf, Arc<AtomicUsize>) {
    let close_calls = Arc::new(AtomicUsize::new(0));
    let pdf = MockPdf {
        issues: Ok(issues),
        docinfo: docinfo.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        page_count,
        pdf_version: pdf_version.to_string(),
        close_calls: close_calls.clone(),
    };
    (pdf, close_calls)
}

fn make_mock_pdf_book(tmp_path: &Path, file_type: &str) -> BookRef {
    let pdf_path = tmp_path.join("test.pdf");
    fs::write(&pdf_path, b"%PDF-1.4 fake").unwrap();
    BookRef { book_id: 1, file_type: file_type.to_string(), file_path: pdf_path }
}

/// pikepdf 대역을 끼운 manager로 `validate_pdf(1)`을 부른다.
async fn validate_pdf_with(bm: &UnitSetup, opened: Result<MockPdf, String>) -> (Option<Value>, Option<String>) {
    split(bm.manager(|d| d.with_pdf_opener(Arc::new(FakeOpener(Mutex::new(Some(opened)))))).validate_pdf(1).await)
}

/// `validate_pdf` 메서드의 단위 테스트 (pikepdf mock).
mod test_validate_pdf_unit {
    use super::*;

    /// 정상 PDF → valid=True, messages 비어있음.
    #[tokio::test]
    async fn test_valid_pdf() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_pdf_book(tmp.path(), "pdf");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let (mock_pdf, _close) = make_mock_pdf(vec![], &[], 10, "1.7");
        let (result, error) = validate_pdf_with(&bm, Ok(mock_pdf)).await;

        assert!(error.is_none());
        let result = result.unwrap();
        assert_eq!(result["valid"], json!(true));
        assert_eq!(result["messages"], json!([]));
        assert_eq!(result["summary"]["warning"], json!(0));
    }

    /// 구문 문제 있는 PDF → valid=False, messages에 WARNING 포함.
    #[tokio::test]
    async fn test_pdf_with_syntax_issues() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_pdf_book(tmp.path(), "pdf");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let issues = vec!["cross-reference table mismatch".to_string(), "missing object stream".to_string()];
        let (mock_pdf, _close) = make_mock_pdf(issues, &[], 10, "1.7");
        let (result, error) = validate_pdf_with(&bm, Ok(mock_pdf)).await;

        assert!(error.is_none());
        let result = result.unwrap();
        assert_eq!(result["valid"], json!(false));
        assert_eq!(result["messages"].as_array().unwrap().len(), 2);
        assert_eq!(result["messages"][0]["severity"], "WARNING");
        assert_eq!(result["messages"][0]["message"], "cross-reference table mismatch");
        assert_eq!(result["summary"]["warning"], json!(2));
    }

    /// 열기 실패 (손상 파일) → error 반환.
    #[tokio::test]
    async fn test_corrupted_pdf_open_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_pdf_book(tmp.path(), "pdf");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let (result, error) = validate_pdf_with(&bm, Err("not a PDF file".to_string())).await;

        assert!(result.is_none());
        assert!(err_has(&error, "Failed to open PDF"));
    }

    /// 메타데이터 추출 검증 (title, author, page count).
    #[tokio::test]
    async fn test_metadata_extraction() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_pdf_book(tmp.path(), "pdf");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let docinfo = [("/Title", "Test PDF Title"), ("/Author", "Test Author"), ("/Producer", "Test Producer"), ("/CreationDate", "D:20240101120000")];
        let (mock_pdf, _close) = make_mock_pdf(vec![], &docinfo, 42, "1.5");
        let (result, error) = validate_pdf_with(&bm, Ok(mock_pdf)).await;

        assert!(error.is_none());
        let result = result.unwrap();
        let publication = &result["publication"];
        assert_eq!(publication["title"], "Test PDF Title");
        assert_eq!(publication["creator"], "Test Author");
        assert_eq!(publication["producer"], "Test Producer");
        assert_eq!(publication["page_count"], json!(42));
        assert_eq!(publication["pdf_version"], "1.5");
    }

    /// 비-PDF 파일 → error 반환.
    #[tokio::test]
    async fn test_non_pdf_file_type() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_pdf_book(tmp.path(), "epub");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let (result, error) = split(bm.manager(|d| d).validate_pdf(1).await);

        assert!(result.is_none());
        assert!(err_has(&error, "Not a PDF file"));
    }

    /// 파일 미존재 → error 반환.
    #[tokio::test]
    async fn test_file_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let missing_path = tmp.path().join("nonexistent.pdf");
        let book = BookRef { book_id: 1, file_type: "pdf".to_string(), file_path: missing_path };
        let bm = unit_manager(tmp.path(), Some(book), None);

        let (result, error) = split(bm.manager(|d| d).validate_pdf(1).await);

        assert!(result.is_none());
        assert!(err_has(&error, "File not found"));
    }

    /// 존재하지 않는 book_id → error 반환.
    #[tokio::test]
    async fn test_book_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let bm = unit_manager(tmp.path(), None, Some("No book found"));

        let (result, error) = split(bm.manager(|d| d).validate_pdf(1).await);

        assert!(result.is_none());
        assert!(err_has(&error, "Book not found"));
    }

    /// docinfo가 비어있는 PDF → publication에 page_count/pdf_version만 존재.
    #[tokio::test]
    async fn test_empty_docinfo() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_pdf_book(tmp.path(), "pdf");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let (mock_pdf, _close) = make_mock_pdf(vec![], &[], 5, "2.0");
        let (result, error) = validate_pdf_with(&bm, Ok(mock_pdf)).await;

        assert!(error.is_none());
        let result = result.unwrap();
        let publication = result["publication"].as_object().unwrap();
        assert!(!publication.contains_key("title"));
        assert!(!publication.contains_key("creator"));
        assert!(!publication.contains_key("producer"));
        assert_eq!(publication["page_count"], json!(5));
        assert_eq!(publication["pdf_version"], "2.0");
    }

    /// check() 예외 발생 시에도 pdf.close()가 호출되는지 검증.
    #[tokio::test]
    async fn test_close_called_on_check_exception() {
        let tmp = tempfile::tempdir().unwrap();
        let book = make_mock_pdf_book(tmp.path(), "pdf");
        let bm = unit_manager(tmp.path(), Some(book), None);

        let (mut mock_pdf, close_calls) = make_mock_pdf(vec![], &[], 10, "1.7");
        mock_pdf.issues = Err("internal check error".to_string());
        let outcome = validate_pdf_with(&bm, Ok(mock_pdf)).await;

        assert!(outcome.0.is_none());
        assert!(err_has(&outcome.1, "internal check error"));

        assert_eq!(close_calls.load(Ordering::SeqCst), 1, "close는 한 번만 불려야 한다");
    }
}

#[tokio::test]
async fn validate_pdf_reports_reconstructed_cross_reference() {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("damaged.pdf");
    common::pdf::make_blank_pdf(&path, 1);
    let mut bytes = fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 10);
    fs::write(&path, bytes).unwrap();

    let pdf = LopdfOpener.open(&path).unwrap();
    let issues = pdf.check_pdf_syntax().unwrap();

    assert!(!issues.is_empty(), "xref recovery should be reported as a syntax warning");
    let book = BookRef { book_id: 1, file_type: "pdf".to_string(), file_path: path };
    let manager = unit_manager(tmp.path(), Some(book), None).manager(|deps| deps);
    let result = manager.validate_pdf(1).await.unwrap();
    assert_eq!(result["valid"], false);
    assert!(result["summary"]["warning"].as_u64().unwrap() > 0);
    assert!(!result["messages"].as_array().unwrap().is_empty());
}

#[test]
fn lopdf_reader_decodes_pdf_metadata_strings_and_omits_unsupported_values() {
    use lopdf::{Object, StringFormat, dictionary};

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("metadata.pdf");
    common::pdf::make_blank_pdf(&path, 1);
    let mut document = lopdf::Document::load(&path).unwrap();
    let indirect_author = document.add_object(Object::string_literal("Indirect Author"));
    let info = document.add_object(lopdf::dictionary! {
        "Title" => Object::String(vec![0xFE, 0xFF, 0x00, b'K', 0x00, b'O'], StringFormat::Hexadecimal),
        "Author" => Object::Reference(indirect_author),
        "Producer" => Object::Name(b"ExampleProducer".to_vec()),
        "Legacy" => Object::String(vec![0xE9], StringFormat::Hexadecimal),
        "Ignored" => Object::Integer(7),
    });
    document.trailer.set("Info", Object::Reference(info));
    document.save(&path).unwrap();

    let pdf = LopdfOpener.open(&path).unwrap();
    let metadata = pdf.docinfo();

    assert_eq!(metadata["/Title"], "KO");
    assert_eq!(metadata["/Author"], "Indirect Author");
    assert_eq!(metadata["/Producer"], "ExampleProducer");
    assert_eq!(metadata["/Legacy"], "é");
    assert!(!metadata.contains_key("/Ignored"));
}

#[test]
fn lopdf_reader_distinguishes_io_and_invalid_pdf_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = LopdfOpener.open(&tmp.path().join("missing.pdf")).err().unwrap();
    assert_eq!(missing.kind, "OSError");

    let invalid_path = tmp.path().join("invalid.pdf");
    fs::write(&invalid_path, b"not a PDF").unwrap();
    let invalid = LopdfOpener.open(&invalid_path).err().unwrap();
    assert_eq!(invalid.kind, "OpenError");
}

#[test]
fn lopdf_reader_reports_a_repaired_startxref_offset() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("offset-corrected.pdf");
    let header = "%PDF-1.5\n";
    let first = "1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n";
    let second = "2 0 obj<</Type/Pages/Kids[]/Count 0>>endobj\n";
    let first_offset = header.len();
    let second_offset = first_offset + first.len();
    let body = format!("{header}{first}{second}");
    let xref_offset = body.len();
    let pdf = format!("{body}xref\n0 3\n0000000000 65535 f \n{first_offset:010} 00000 n \n{second_offset:010} 00000 n \ntrailer\n<</Root 1 0 R/Size 3>>\nstartxref\n{}\n%%EOF", xref_offset + 4);
    fs::write(&path, pdf).unwrap();

    let document = LopdfOpener.open(&path).unwrap();
    assert_eq!(document.check_pdf_syntax().unwrap(), ["Cross-reference section offset required repair"]);
}

/// URL 인코딩된 href 항목 및 전체보기(chapters=0) 시 EPUB 구조가 완전 보존되는지 검증.
#[tokio::test]
async fn test_epub_unquoted_href_and_full_view() {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path();

    let epub_file = tmp_path.join("encoded_book.epub");
    fs::create_dir_all(epub_file.parent().unwrap()).unwrap();
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Encoded Book</dc:title>
  </metadata>
  <manifest>
    <item id="ch1" href="ch%201.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover" href="cover%20page.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
  <guide>
    <reference type="cover" title="Cover Page" href="cover%20page.xhtml"/>
  </guide>
</package>"#;
    let ch1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Ch1</title></head>
<body><img src="img%201.jpg"/></body>
</html>"#;
    let cover = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Cover</title></head>
<body><h1>Cover</h1></body>
</html>"#;
    write_zip_stored(
        &epub_file,
        &[
            ("mimetype", b"application/epub+zip"),
            ("OEBPS/content.opf", opf.as_bytes()),
            ("OEBPS/ch 1.xhtml", ch1.as_bytes()),
            ("OEBPS/cover page.xhtml", cover.as_bytes()),
            ("OEBPS/img 1.jpg", b"fake_image_data"),
        ],
    );
    let _cleanup = TempFile(epub_file.clone());

    let es = FakeEsManager::with_doc(
        json!({"category": "test", "title": "Test Book", "author": "Author", "file_path": "encoded_book.epub", "file_type": "epub", "file_size": 1000, "updated_time": "2026-01-01T00:00:00.000000"}),
    );
    let bm = BookManager::with_deps(BookManagerDeps::new(tmp_path.to_path_buf(), es));

    let resp = bm.get_book_preview(1, None, Some(0)).await;
    assert_eq!(resp.status_code, 200);

    let cache_file = tmp_path.join(".preview_cache").join(format!("1_ch1_v{}.epub", BookManager::EPUB_IMAGE_POLICY_VERSION));
    assert!(cache_file.exists());

    let mut zout = ZipArchive::new(File::open(&cache_file).unwrap()).unwrap();
    let names = namelist(&mut zout);
    assert!(names.contains(&"OEBPS/cover page.xhtml".to_string()));
    assert!(names.contains(&"OEBPS/img 1.jpg".to_string()));
    let opf_data = zip_read_string(&mut zout, "OEBPS/content.opf");
    assert!(opf_data.contains("id=\"cover\""));
}

/// XHTML 자기닫힘 `<title/>` 등이 HTML 파서에서 본문을 삼키지 않게 펴는지 검증.
mod test_self_closing_rcdata {
    use super::*;

    #[test]
    fn test_expands_self_closing_rcdata_tags() {
        assert_eq!(BookManager::_expand_self_closing_rcdata(b"<title/>"), b"<title></title>");
        assert_eq!(BookManager::_expand_self_closing_rcdata(b"<TITLE />"), b"<TITLE ></TITLE>");
        assert_eq!(BookManager::_expand_self_closing_rcdata(b"<script src=\"a.js\"/>"), b"<script src=\"a.js\"></script>");
        assert_eq!(BookManager::_expand_self_closing_rcdata(b"<style/><textarea/>"), b"<style></style><textarea></textarea>");
    }

    #[test]
    fn test_keeps_paired_and_unrelated_tags() {
        for data in [&b"<title>keep</title>"[..], b"<br/>", b"<img src=\"a.jpg\"/>", b"<titlebar/>"] {
            assert_eq!(BookManager::_expand_self_closing_rcdata(data), data, "data={}", String::from_utf8_lossy(data));
        }
    }
}

/// `<title/>`을 쓴 EPUB의 프리뷰 산출물에서 본문이 살아남는지 검증.
#[tokio::test]
async fn test_preview_expands_self_closing_title() {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path();

    let epub_file = tmp_path.join("self_closing_title.epub");
    fs::create_dir_all(epub_file.parent().unwrap()).unwrap();
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Self Closing Title</dc:title>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.html" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml">
<head><title/></head>
<body><p>본문이 보여야 한다</p></body>
</html>"#;
    write_zip_stored(
        &epub_file,
        &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("OEBPS/content.opf", opf.as_bytes()), ("OEBPS/ch1.html", ch1.as_bytes())],
    );
    let _cleanup = TempFile(epub_file.clone());

    let es = FakeEsManager::with_doc(
        json!({"category": "test", "title": "Self Closing Title", "author": "Author", "file_path": "self_closing_title.epub", "file_type": "epub", "file_size": 1000, "updated_time": "2026-01-01T00:00:00.000000"}),
    );
    let bm = BookManager::with_deps(BookManagerDeps::new(tmp_path.to_path_buf(), es));

    let resp = bm.get_book_preview(2, None, Some(0)).await;
    assert_eq!(resp.status_code, 200);

    let cache_file = tmp_path.join(".preview_cache").join(format!("2_ch1_v{}.epub", BookManager::EPUB_IMAGE_POLICY_VERSION));
    let mut zout = ZipArchive::new(File::open(&cache_file).unwrap()).unwrap();
    let chapter = zip_read_string(&mut zout, "OEBPS/ch1.html");

    assert!(chapter.contains("<title></title>"));
    assert!(!chapter.contains("<title/>"));
    // HTML 파서로 읽어도 본문이 title 안으로 삼켜지지 않는다
    let soup = scraper::Html::parse_document(&chapter);
    let body_selector = scraper::Selector::parse("body").unwrap();
    let body = soup.select(&body_selector).next().expect("body 요소");
    assert!(body.text().collect::<String>().contains("본문이 보여야 한다"));
}
