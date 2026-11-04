//! `backend::book_manager::preview_builder` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

#[path = "../common/mod.rs"]
mod common;

// `tests/rust/backend/book_preview.rs`에서 옮긴 테스트.
mod book_preview {
    use super::{common};

    use std::fs::{self, File};

    use std::io::Read;
    use std::path::{Path, PathBuf};

    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use backend::book_manager::{BookManager, BookManagerDeps};
    use backend::main::{AppDeps, AuthMode, app_with};
    use common::TestClient;
    use common::es_fake::FakeEsManager;
    use common::faulty_fs::FaultyFs;
    use common::fsx::set_mtime_ago;
    use common::zipx::{ZipMode, write_zip_with};
    use serde_json::json;

    use zip::ZipArchive;

    /// 검증 결과를 Python 원본의 `(유효 여부, 사유)`로 펼친다.
    fn verdict(result: Result<(), String>) -> (bool, Option<String>) {
        match result {
            Ok(()) => (true, None),
            Err(reason) => (false, Some(reason)),
        }
    }
    const OPF_NS: &str = "http://www.idpf.org/2007/opf";

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

            let (valid, error) = verdict(BookManager::_validate_preview_epub(&epub));

            assert!(!valid);
            assert!(err_has(&error, "no valid spine chapters"));
        }

        /// 정상 EPUB은 (True, None)을 반환한다.
        #[test]
        fn test_valid_epub_passes() {
            let tmp = tempfile::tempdir().unwrap();
            let epub = tmp.path().join("valid.epub");
            create_test_epub(&epub, 3, false, false);

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
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

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
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

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
            assert!(!valid);
            assert!(err_has(&err, "invalid mimetype"));
        }

        /// OPF 파일이 없으면 거부된다.
        #[test]
        fn test_missing_opf_rejected() {
            let tmp = tempfile::tempdir().unwrap();
            let epub = tmp.path().join("no_opf.epub");
            write_zip_stored(&epub, &[("mimetype", b"application/epub+zip"), ("OEBPS/ch1.xhtml", chapter(1).as_bytes())]);

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
            assert!(!valid);
            assert!(err_has(&err, "OPF"));
        }

        /// 유효한 spine 챕터가 0개이면 거부된다.
        #[test]
        fn test_no_valid_spine_chapters_rejected() {
            let tmp = tempfile::tempdir().unwrap();
            let epub = tmp.path().join("empty_spine.epub");
            create_corrupted_epub(&epub, 2, true);

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
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

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
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

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
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

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
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

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
            assert!(!valid);
            assert!(err_has(&err, "corrupted ZIP"));
        }

        /// spine 요소가 없는 OPF는 거부된다.
        #[test]
        fn test_missing_spine_element_rejected() {
            let tmp = tempfile::tempdir().unwrap();
            let epub = tmp.path().join("no_spine.epub");
            create_epub_without_spine(&epub);

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
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

            let (valid, err) = verdict(BookManager::_validate_preview_epub(&epub));
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

            let (valid, _err) = verdict(BookManager::_validate_preview_epub(&epub));
            assert!(valid);
            let mtime_after = fs::metadata(&epub).unwrap().modified().unwrap();
            assert_eq!(mtime_before, mtime_after, "clean EPUB should not be rewritten");
        }
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
}

// `tests/rust/backend/book_manager_hwp_preview.rs`에서 옮긴 테스트.
mod book_manager_hwp_preview {
    use super::{common};

    use std::fs;
    use std::path::{Path, PathBuf};

    use backend::book_manager::{BookManager, BookManagerDeps};
    use backend::main::BookManagerApi;
    use common::es_fake::FakeEsManager;
    use common::runner::FakeRunner;
    use serde_json::{Value, json};

    fn hwp_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|a| a.join("tests/books/_hwp")).find(|p| p.exists()).unwrap_or_else(|| PathBuf::from("tests/books/_hwp"))
    }

    /// Python `make_doc(path, file_type)`.
    fn make_doc(rel_path: &str, file_type: &str) -> Value {
        json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
    }

    mod test_book_manager_preview_empty_content {
        use super::*;

        // book_manager HWP preview에서 빈 결과 시 안내 메시지 반환

        #[tokio::test]
        async fn test_hwp_preview_empty_returns_message() {
            // LO+hwp3 모두 빈 결과 -> '미리보기를 생성할 수 없습니다' 메시지
            let hwp_src = hwp_dir().join("v1.20_부하의약혼녀.hwp");
            assert!(hwp_src.exists(), "HWP 샘플 파일이 필요하다: {}", hwp_src.display());
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("A")).unwrap();
            fs::copy(&hwp_src, dir.path().join("A").join("old.hwp")).unwrap();

            let doc = make_doc("A/old.hwp", "hwp");
            // libreoffice 변환기가 빈 문자열을 돌려준다
            let deps = BookManagerDeps::new(dir.path().to_path_buf(), FakeEsManager::with_doc(doc)).with_runner(FakeRunner::office_output(""));
            let manager = BookManager::with_deps(deps);
            let resp = BookManagerApi::get_book_preview(&manager, 1, Some(1), None).await;
            assert_eq!(resp.status_code, 200);
            assert!(String::from_utf8_lossy(&resp.body).contains("미리보기를 생성할 수 없습니다"));
        }
    }
}
