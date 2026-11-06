//! `book_manager::preview_builder::epub_package` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use common::faulty_fs::FaultyFs;
use common::fsx::set_mtime_ago;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod spine_and_chapters {
    use super::common;
    use crate::common::shared_fixtures::epub_with_missing_page;
    use crate::common::shared_fixtures::{tmp_dir, verdict};

    use std::fs;

    use common::zipx::{write_zip, zip_header_offset};

    // ── 경계 사례 ──

    #[test]
    fn test_empty_epub_spine_href_is_rejected() {
        let tmp = tmp_dir();
        let epub_path = tmp.path().join("empty_path.epub");
        write_zip(
            &epub_path,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#),
                ("content.opf", br#"<?xml version="1.0"?><package version="2.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="BookId"><manifest><item id="item1" href="" media-type="text/html"/></manifest><spine toc="ncx"><itemref idref="item1"/></spine></package>"#),
            ],
        );
        let (valid, _msg) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub_path));
        assert!(!valid);
    }

    #[test]
    fn test_total_chapters_counts_only_spine_items_that_exist_in_the_zip() {
        // manifest 에 없는 idref 와 ZIP 에 없는 문서는 뷰어가 보여 줄 수 없으므로 세지 않는다.
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        assert_eq!(book_manager::preview_builder::epub_package::get_epub_total_chapters(&epub_with_missing_page(&tmp_path.join("missing.epub"), false)), 1);
        assert_eq!(book_manager::preview_builder::epub_package::get_epub_total_chapters(&epub_with_missing_page(&tmp_path.join("full.epub"), true)), 2);
    }

    #[test]
    fn test_total_chapters_counts_percent_encoded_zip_entry_names() {
        // ZIP 항목 이름이 퍼센트 인코딩된 채 저장돼 있어도(href 는 풀어서 비교) 세어야 한다.
        let tmp = tmp_dir();
        let epub = tmp.path().join("pct.epub");
        write_zip(
            &epub,
            &[
                ("META-INF/container.xml", br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#),
                ("content.opf", br#"<package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="a" href="a%20b.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/></spine></package>"#),
                ("a%20b.xhtml", b"<html/>"),
            ],
        );
        assert_eq!(book_manager::preview_builder::epub_package::get_epub_total_chapters(&epub), 1);
    }

    #[test]
    fn test_total_chapters_does_not_count_chapter_with_broken_zip_entry() {
        let tmp = tmp_dir();
        let epub = epub_with_missing_page(&tmp.path().join("broken.epub"), true);
        let offset = zip_header_offset(&epub, "OEBPS/Text/colophon page.xhtml") as usize;
        let mut data = fs::read(&epub).unwrap();
        data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]);
        fs::write(&epub, &data).unwrap();
        assert_eq!(book_manager::preview_builder::epub_package::get_epub_total_chapters(&epub), 1);
    }
}

mod image_downscaling {
    use crate::common::shared_fixtures::image_bytes;

    // ---- 이미지/EPUB 미리보기 ----

    #[test]
    fn test_downscale_epub_image_keeps_non_jpeg_png_format() {
        // GIF 같은 형식은 확장자와 실제 형식이 어긋나지 않게 그대로 돌려준다.
        let data = image_bytes(image::ImageFormat::Gif, (3000, 3000));
        assert_eq!(book_manager::preview_builder::epub_package::downscale_epub_image(&data), data);
    }
}

mod find_opf_path {
    use crate::common::shared_fixtures::CONTAINER_XML;
    use crate::common::shared_fixtures::write_zip_stored;

    use std::fs::File;

    use std::path::Path;

    use zip::ZipArchive;

    fn find_opf_path(epub_path: &Path) -> String {
        let mut zin = ZipArchive::new(File::open(epub_path).unwrap()).unwrap();
        book_manager::preview_builder::epub_package::find_opf_path(&mut zin).unwrap_or_default()
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

    book_manager::preview_builder::epub_package::evict_old_cache(tmp_path);

    assert!(!old_file.exists());
    assert!(new_file.exists());
}

/// 1일 미만 파일은 보존된다.
#[test]
fn test_recent_files_are_kept() {
    let tmp = tempfile::tempdir().unwrap();

    let recent = tmp.path().join("recent.pdf");
    fs::write(&recent, "recent").unwrap();

    book_manager::preview_builder::epub_package::evict_old_cache(tmp.path());

    assert!(recent.exists());
}

/// 정확히 1일 된 파일도 삭제된다.
#[test]
fn test_exactly_one_day_old_is_deleted() {
    let tmp = tempfile::tempdir().unwrap();

    let edge_file = tmp.path().join("edge.html");
    fs::write(&edge_file, "edge").unwrap();
    set_mtime_ago(&edge_file, DAY + 1); // 1일 + 1초

    book_manager::preview_builder::epub_package::evict_old_cache(tmp.path());

    assert!(!edge_file.exists());
}

/// 하위 디렉토리는 삭제하지 않는다.
#[test]
fn test_subdirectories_are_not_deleted() {
    let tmp = tempfile::tempdir().unwrap();

    let sub_dir = tmp.path().join("subdir");
    fs::create_dir(&sub_dir).unwrap();
    File::open(&sub_dir).unwrap().set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * DAY))).unwrap();

    book_manager::preview_builder::epub_package::evict_old_cache(tmp.path());

    assert!(sub_dir.exists());
}

/// 빈 디렉토리에서 에러 없이 동작한다.
#[test]
fn test_empty_directory_no_error() {
    let tmp = tempfile::tempdir().unwrap();

    book_manager::preview_builder::epub_package::evict_old_cache(tmp.path()); // 에러 없이 완료
}

/// 존재하지 않는 디렉토리에서 에러 없이 동작한다.
#[test]
fn test_nonexistent_directory_no_error() {
    let tmp = tempfile::tempdir().unwrap();

    let fake_dir = tmp.path().join("nonexistent");
    book_manager::preview_builder::epub_package::evict_old_cache(&fake_dir); // 에러 없이 완료
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

    book_manager::preview_builder::epub_package::evict_old_cache(tmp.path());

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
    book_manager::preview_builder::epub_package::evict_old_cache_with(faulty.as_ref(), tmp.path());

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

    book_manager::preview_builder::epub_package::evict_old_cache(tmp.path());

    for f in &old_files {
        assert!(!f.exists(), "{} should have been evicted", f.file_name().unwrap().to_string_lossy());
    }
    assert!(just_written.exists());
}

mod validate_preview_epub {
    use crate::common::preview_epubs::{OPF_NS, create_corrupted_epub, create_epub_with_invalid_spine, create_epub_without_spine, create_test_epub};
    use crate::common::shared_fixtures::CONTAINER_XML;
    use crate::common::shared_fixtures::{chapter, err_has, make_opf, ns_children, ns_nodes, verdict, write_zip_stored, zip_read_string};
    use std::fs;
    use std::fs::File;
    use std::path::Path;

    use zip::ZipArchive;

    /// 파일 안 OPF의 `(spine toc 속성, spine itemref idref 목록)`.
    fn spine_of(path: &Path, opf_member: &str) -> (Option<String>, Vec<String>) {
        let mut zf = ZipArchive::new(File::open(path).unwrap()).unwrap();
        let text = zip_read_string(&mut zf, opf_member);
        let opf = roxmltree::Document::parse(&text).unwrap();
        let spine_el = ns_nodes(&opf, OPF_NS, "spine")[0];
        let idrefs = ns_children(spine_el, OPF_NS, "itemref").iter().map(|r| r.attribute("idref").unwrap().to_string()).collect();
        (spine_el.attribute("toc").map(String::from), idrefs)
    }

    /// 빈 spine 경로는 ZIP 항목 이름으로 인정하지 않는다.
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

        let (valid, error) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));

        assert!(!valid);
        assert!(err_has(&error, "no valid spine chapters"));
    }

    /// 정상 EPUB은 (True, None)을 반환한다.
    #[test]
    fn test_valid_epub_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("valid.epub");
        create_test_epub(&epub, 3, false, false);

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
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

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
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

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "invalid mimetype"));
    }

    /// OPF 파일이 없으면 거부된다.
    #[test]
    fn test_missing_opf_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("no_opf.epub");
        write_zip_stored(&epub, &[("mimetype", b"application/epub+zip"), ("OEBPS/ch1.xhtml", chapter(1).as_bytes())]);

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "OPF"));
    }

    /// 유효한 spine 챕터가 0개이면 거부된다.
    #[test]
    fn test_no_valid_spine_chapters_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("empty_spine.epub");
        create_corrupted_epub(&epub, 2, true);

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
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

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
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

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
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

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
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

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
        assert!(!valid);
        assert!(err_has(&err, "corrupted ZIP"));
    }

    /// spine 요소가 없는 OPF는 거부된다.
    #[test]
    fn test_missing_spine_element_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let epub = tmp.path().join("no_spine.epub");
        create_epub_without_spine(&epub);

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
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

        let (valid, err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
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

        let (valid, _err) = verdict(book_manager::preview_builder::epub_package::validate_preview_epub(&epub));
        assert!(valid);
        let mtime_after = fs::metadata(&epub).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "clean EPUB should not be rewritten");
    }
}

// ---- NCX에서 없는 문서 가리키는 항목 지우기 (book_manager 테스트에서 옮김) ----

use crate::common::shared_fixtures::{NCX_NS_STR, epub_with_missing_page, tmp_dir};
use book_manager::preview_builder::epub_package::prune_dead_ncx_navpoints;
use common::zipx::read_zip_entry;
use std::collections::BTreeSet;

fn ncx_srcs(data: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(data).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.descendants().filter(|n| n.has_tag_name((NCX_NS_STR, "content"))).filter_map(|c| c.attribute("src").map(str::to_string)).collect()
}

#[test]
fn test_prune_dead_ncx_navpoints_drops_missing_targets_and_keeps_children() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("m.epub"), false);
    let ncx = read_zip_entry(&epub, "OEBPS/toc.ncx");
    let names: BTreeSet<&str> = BTreeSet::from(["OEBPS/Text/a.xhtml"]);
    let pruned = prune_dead_ncx_navpoints(&ncx, "OEBPS", &|path| names.contains(path)).expect("pruned");
    // 판권 항목만 사라지고, 그 아래 자식은 부모 자리로 올라온다
    assert_eq!(ncx_srcs(&pruned), ["Text/a.xhtml#top", "Text/a.xhtml#kid"]);
}

#[test]
fn test_prune_dead_ncx_navpoints_returns_none_when_nothing_is_dead() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("f.epub"), true);
    let ncx = read_zip_entry(&epub, "OEBPS/toc.ncx");
    let names: BTreeSet<&str> = BTreeSet::from(["OEBPS/Text/a.xhtml", "OEBPS/Text/colophon page.xhtml"]);
    assert!(prune_dead_ncx_navpoints(&ncx, "OEBPS", &|path| names.contains(path)).is_none());
}

#[test]
fn test_prune_dead_ncx_navpoints_ignores_unparsable_ncx_and_empty_src() {
    assert!(prune_dead_ncx_navpoints(b"<ncx", "OEBPS", &|_path| false).is_none());
    let no_src = format!(r#"<ncx xmlns="{NCX_NS_STR}"><navMap><navPoint id="x"><content src=""/></navPoint><navPoint id="y"/></navMap></ncx>"#).into_bytes();
    assert!(prune_dead_ncx_navpoints(&no_src, "", &|_path| false).is_none());
}
