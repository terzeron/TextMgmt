//! `backend::book_manager::epub` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

#[path = "../common/mod.rs"]
mod common;

// `tests/rust/backend/epub_image_edit.rs`에서 옮긴 테스트.
mod epub_image_edit {
    use super::{common};

    use std::collections::BTreeMap;
    use std::fs::{self};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use backend::epub_cover::{CoverError, EditImages, EpubCoverHooks, Limits, OPF_NS, edit_images, edit_images_with, edit_page_images, revision};

    use common::image::{color, encode, image_size, jpeg_bytes};
    use common::zipx::{Members as ZipMembers, ZipMode, write_zip_with};
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage, RgbaImage};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    fn full() -> Value {
        json!([0, 0, 1, 1, 0])
    }

    fn jpeg_sized(c: &str, size: (u32, u32)) -> Vec<u8> {
        jpeg_bytes(&RgbImage::from_pixel(size.0, size.1, Rgb(color(c))), 75)
    }

    fn jpeg(c: &str) -> Vec<u8> {
        jpeg_sized(c, (200, 100))
    }

    fn png(c: &str) -> Vec<u8> {
        encode(DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 100, Rgb(color(c)))), ImageFormat::Png)
    }

    fn gif() -> Vec<u8> {
        encode(DynamicImage::ImageRgba8(RgbaImage::new(20, 20)), ImageFormat::Gif)
    }

    fn page(images: &[&str], text: &str) -> String {
        let body: String = images.iter().map(|src| format!(r#"<div><img src="{src}"/></div>"#)).collect();
        format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body>{text}{body}</body></html>"#)
    }

    type Members = Vec<(String, Vec<u8>)>;

    /// mimetype을 맨 앞에 두고 나머지 순서는 그대로 둔다. 멤버는 모두 무압축으로 쓴다.
    fn write_zip(path: &Path, mut members: Members) {
        let pos = members.iter().position(|(n, _)| n == "mimetype").expect("mimetype");
        let mimetype = members.remove(pos);
        members.insert(0, mimetype);
        let entries: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        write_zip_with(path, &entries, ZipMode::Stored);
    }

    /// p1·p2·p3 는 이미지만 있는 쪽, text 는 글만 있는 쪽, two 는 이미지 둘, nav 는 p3 를 가리키는 목차.
    fn make_book(path: &Path, extra: &[(&str, Vec<u8>)]) -> PathBuf {
        let opf = format!(
            r##"<package xmlns="{OPF_NS}" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>책</dc:title></metadata><manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="p1" href="p1.xhtml" media-type="application/xhtml+xml"/><item id="p2" href="p2.xhtml" media-type="application/xhtml+xml"/>
<item id="p3" href="p3.xhtml" media-type="application/xhtml+xml"/><item id="text" href="text.xhtml" media-type="application/xhtml+xml"/>
<item id="two" href="two.xhtml" media-type="application/xhtml+xml"/><item id="shared" href="shared.xhtml" media-type="application/xhtml+xml"/>
<item id="gif" href="gif.xhtml" media-type="application/xhtml+xml"/><item id="mixed" href="mixed.xhtml" media-type="application/xhtml+xml"/>
<item id="i1" href="img/1.jpg" media-type="image/jpeg"/><item id="i2" href="img/2.png" media-type="image/png"/>
<item id="i3" href="img/3.jpg" media-type="image/jpeg"/><item id="i4" href="img/4.jpg" media-type="image/jpeg"/>
<item id="ig" href="img/g.gif" media-type="image/gif"/></manifest>
<spine><itemref idref="p1"/><itemref idref="p2"/><itemref idref="p3"/><itemref idref="text"/><itemref idref="two"/><itemref idref="shared"/><itemref idref="gif"/><itemref idref="mixed"/></spine></package>"##
        );
        let mut members: Members = vec![
            ("mimetype".into(), b"application/epub+zip".to_vec()),
            ("META-INF/container.xml".into(), br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#.to_vec()),
            ("OPS/book.opf".into(), opf.into_bytes()),
            (
                "OPS/nav.xhtml".into(),
                br#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="p3.xhtml">3</a></li></ol></nav></body></html>"#
                    .to_vec(),
            ),
            ("OPS/p1.xhtml".into(), page(&["img/1.jpg"], "").into_bytes()),
            ("OPS/p2.xhtml".into(), page(&["img/2.png"], "").into_bytes()),
            ("OPS/p3.xhtml".into(), page(&["img/3.jpg"], "").into_bytes()),
            ("OPS/text.xhtml".into(), page(&[], "본문 글").into_bytes()),
            ("OPS/two.xhtml".into(), page(&["img/4.jpg", "img/5.jpg"], "").into_bytes()),
            ("OPS/shared.xhtml".into(), page(&["img/4.jpg"], "").into_bytes()),
            ("OPS/gif.xhtml".into(), page(&["img/g.gif"], "").into_bytes()),
            ("OPS/mixed.xhtml".into(), page(&["img/5.jpg"], "설명 글").into_bytes()),
            ("OPS/img/1.jpg".into(), jpeg("red")),
            ("OPS/img/2.png".into(), png("green")),
            ("OPS/img/3.jpg".into(), jpeg("blue")),
            ("OPS/img/4.jpg".into(), jpeg("yellow")),
            ("OPS/img/5.jpg".into(), jpeg("pink")),
            ("OPS/img/g.gif".into(), gif()),
        ];
        for (name, data) in extra {
            match members.iter_mut().find(|(n, _)| n == name) {
                Some(slot) => slot.1 = data.clone(),
                None => members.push((name.to_string(), data.clone())),
            }
        }
        write_zip(path, members);
        path.to_path_buf()
    }

    fn read(path: &Path) -> BTreeMap<String, Vec<u8>> {
        let members = ZipMembers::read(path);
        members.names().into_iter().map(|name| (name.clone(), members.get(&name).to_vec())).collect()
    }

    fn size(data: &[u8]) -> (u32, u32) {
        image_size(data)
    }

    fn text_of(members: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
        String::from_utf8(members[name].clone()).unwrap()
    }

    /// `book` fixture: (책 경로, 휴지통 경로)
    struct Book {
        path: PathBuf,
        trash: PathBuf,
        _tmp: TempDir,
    }

    fn book() -> Book {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_book(&tmp.path().join("책").join("book.epub"), &[]);
        let trash = tmp.path().join("0_trash");
        Book { path, trash, _tmp: tmp }
    }

    fn edit(path: &Path, trash: &Path, args: EditImages) -> Result<Value, CoverError> {
        edit_images(path, &revision(path), trash, EditImages { backup_name: Some("book.epub".to_string()), ..args })
    }

    impl Book {
        fn edit(&self, args: EditImages) -> Result<Value, CoverError> {
            edit(&self.path, &self.trash, args)
        }
    }

    fn err_of(result: Result<Value, CoverError>) -> String {
        result.expect_err("CoverError가 나야 한다").to_string()
    }

    fn assert_refused(result: Result<Value, CoverError>, reason: &str, label: &str) {
        let text = err_of(result);
        assert!(text.contains(reason), "{label}: {text:?} 에 {reason:?}가 없다");
    }

    #[test]
    fn test_empty_and_duplicate_requests_are_rejected() {
        let b = book();
        assert_refused(b.edit(EditImages::default()), "편집이 없습니다", "빈 요청");
        assert_refused(b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], deletes: vec![json!("p1")], ..EditImages::default() }), "두 번", "중복 요청");
    }

    #[test]
    fn test_stale_revision_is_rejected_and_backup_is_kept_once() {
        let b = book();
        let stale = edit_images(&b.path, "stale", &b.trash, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() });
        assert_refused(stale, "변경", "stale");
        let original = fs::read(&b.path).unwrap();
        b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
        b.edit(EditImages { crops: vec![json!({"idref": "p1", "rect": [0, 0, 0.5, 1, 0]})], ..EditImages::default() }).unwrap();
        assert_eq!(fs::read(b.trash.join("book.epub")).unwrap(), original);
    }

    // ── <svg><image> 로 이미지를 넣은 쪽 ──────────────────────────────────────────

    fn svg_epub(path: &Path, size: (u32, u32), title_text: &str, cover: bool) {
        let jpeg_data = jpeg_bytes(&RgbImage::from_pixel(size.0, size.1, Rgb([200, 30, 30])), 75);
        let (w, h) = size;
        let page = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:xlink="http://www.w3.org/1999/xlink"><head><title>c</title></head><body><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="100%" height="100%"><title>{title_text}</title><image width="{w}" height="{h}" xlink:href="c.jpg"/></svg></body></html>"#
        );
        let opf = concat!(
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">"#,
            r#"<dc:identifier id="id">x</dc:identifier><dc:title>t</dc:title><dc:language>ko</dc:language></metadata><manifest>"#,
            r#"<item id="c" href="c.xhtml" media-type="application/xhtml+xml"/><item id="ci" href="c.jpg" media-type="image/jpeg" __COVER__/>"#,
            r#"<item id="d" href="d.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c"/><itemref idref="d"/></spine></package>"#,
        )
        .replace("__COVER__", if cover { r#"properties="cover-image""# } else { "" });
        let members: Members = vec![
            ("mimetype".into(), b"application/epub+zip".to_vec()),
            ("META-INF/container.xml".into(), br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.to_vec()),
            ("OEBPS/content.opf".into(), opf.into_bytes()),
            ("OEBPS/c.xhtml".into(), page.into_bytes()),
            ("OEBPS/c.jpg".into(), jpeg_data),
            ("OEBPS/d.xhtml".into(), r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>d</title></head><body><p>본문</p></body></html>"#.as_bytes().to_vec()),
        ];
        write_zip(path, members);
    }

    #[test]
    fn test_svg_image_page_can_be_cropped_and_viewbox_follows() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("svg.epub");
        svg_epub(&path, (300, 400), "표지", false);
        edit_images(&path, &revision(&path), &tmp.path().join("trash"), EditImages { crops: vec![json!({"idref": "c", "rect": [0.0, 0.0, 0.5, 0.5, 0.0]})], ..EditImages::default() }).unwrap();
        let members = read(&path);
        assert_eq!(size(&members["OEBPS/c.jpg"]), (150, 200));
        let page = text_of(&members, "OEBPS/c.xhtml");
        assert!(page.contains(r#"viewBox="0 0 150 200""#), "{page}");
        assert!(page.contains(r#"width="150""#) && page.contains(r#"height="200""#), "{page}");
    }

    #[test]
    fn test_svg_image_page_counts_as_image_only_despite_svg_title() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("svg.epub");
        svg_epub(&path, (300, 400), "표지", false);
        // Python은 `deletes=[{"idref": "c"}] and ["c"]`를 넘긴다. 이 식은 ["c"]다.
        let result = edit_images(&path, &revision(&path), &tmp.path().join("trash"), EditImages { deletes: vec![json!("c")], ..EditImages::default() }).unwrap();
        assert!(result["revision"].as_str().is_some_and(|r| !r.is_empty()), "{result}");
    }

    #[test]
    fn test_cover_target_crops_the_cover_image_and_fits_its_svg_page() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cover.epub");
        svg_epub(&path, (300, 400), "표지", true);
        edit_images(&path, &revision(&path), &tmp.path().join("trash"), EditImages { crops: vec![json!({"idref": "@cover", "rect": [0.0, 0.0, 0.5, 1.0, 0.0]})], ..EditImages::default() }).unwrap();
        let members = read(&path);
        assert_eq!(size(&members["OEBPS/c.jpg"]), (150, 400));
        assert!(text_of(&members, "OEBPS/c.xhtml").contains(r#"viewBox="0 0 150 400""#));
    }

    #[test]
    fn test_cover_target_without_a_cover_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nocover.epub");
        svg_epub(&path, (300, 400), "표지", false);
        let result = edit_images(&path, &revision(&path), &tmp.path().join("trash"), EditImages { crops: vec![json!({"idref": "@cover", "rect": [0.0, 0.0, 0.5, 1.0, 0.0]})], ..EditImages::default() });
        assert_refused(result, "표지 이미지를 찾을 수 없음", "@cover");
    }

    // ── 한 쪽에 이미지가 여러 장인 책(실제 책처럼 xmlns 없는 비정형 HTML) ─────────────────────

    const MULTI_PAGE: &str = concat!(
        "<html>\n  <head>\n    <title>엄마의 말뚝</title>\n  </head>\n    <body><li class=\"chapter\">\n",
        "<p><img alt=\"image\" height=\"597\" src=\"../Images/m1.jpg\" width=\"412\"/></p>\n",
        "<p><img alt=\"image\" height=\"597\" src=\"../Images/m2.jpg\" width=\"412\"/></p>\n",
        "</li></body></html>"
    );

    fn make_multi(path: &Path, page: &[u8]) -> PathBuf {
        let opf = format!(
            concat!(
                r#"<package xmlns="{ns}" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">x</dc:identifier><dc:title>t</dc:title></metadata>"#,
                r#"<manifest><item id="multi" href="Text/multi.xhtml" media-type="application/xhtml+xml"/><item id="t" href="Text/t.xhtml" media-type="application/xhtml+xml"/>"#,
                r#"<item id="m1" href="Images/m1.jpg" media-type="image/jpeg"/><item id="m2" href="Images/m2.jpg" media-type="image/jpeg"/></manifest>"#,
                r#"<spine><itemref idref="multi"/><itemref idref="t"/></spine></package>"#,
            ),
            ns = OPF_NS
        );
        let members: Members = vec![
            ("mimetype".into(), b"application/epub+zip".to_vec()),
            (
                "META-INF/container.xml".into(),
                br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#.to_vec(),
            ),
            ("OEBPS/content.opf".into(), opf.into_bytes()),
            ("OEBPS/Text/multi.xhtml".into(), page.to_vec()),
            ("OEBPS/Text/t.xhtml".into(), "<html><body><p>다른 쪽</p></body></html>".as_bytes().to_vec()),
            ("OEBPS/Images/m1.jpg".into(), jpeg("red")),
            ("OEBPS/Images/m2.jpg".into(), jpeg("green")),
        ];
        write_zip(path, members);
        path.to_path_buf()
    }

    /// `multi` fixture
    fn multi() -> Book {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_multi(&tmp.path().join("multi.epub"), MULTI_PAGE.as_bytes());
        let trash = tmp.path().join("0_trash");
        Book { path, trash, _tmp: tmp }
    }

    #[test]
    fn test_index_out_of_range_is_refused() {
        let b = multi();
        let original = fs::read(&b.path).unwrap();
        for kind in ["crops", "splits", "deletes"] {
            let mut entry = json!({"idref": "multi", "index": 2});
            match kind {
                "crops" => entry["rect"] = full(),
                "splits" => {
                    entry["first"] = full();
                    entry["second"] = full();
                }
                _ => {}
            }
            let mut args = EditImages::default();
            match kind {
                "crops" => args.crops = vec![entry],
                "splits" => args.splits = vec![entry],
                _ => args.deletes = vec![entry],
            }
            assert_refused(b.edit(args), "이미지를 찾을 수 없음", kind);
        }
        assert_eq!(fs::read(&b.path).unwrap(), original);
    }

    #[test]
    fn test_mismatched_tag_count_is_refused() {
        let err = edit_page_images(b"<html><body><img src='a.jpg'/></body></html>", 2, &[(0, "delete", None)]).expect_err("CoverError가 나야 한다");
        assert!(err.to_string().contains("쪽 구조를 해석할 수 없습니다"), "{err}");
    }

    #[test]
    fn test_page_image_edit_preserves_utf8_signature() {
        let raw = b"\xEF\xBB\xBF<html><body><img src='old.jpg'/></body></html>";

        let edited = edit_page_images(raw, 1, &[(0, "split", Some(json!("new image.jpg")))]).unwrap();

        assert!(edited.starts_with(b"\xEF\xBB\xBF"));
        assert!(String::from_utf8_lossy(&edited).contains("src='new%20image.jpg'"));
    }

    #[test]
    fn test_page_image_edit_reads_and_writes_utf16_bom() {
        let text = "<html><body><img src='old.jpg'/></body></html>";
        let raw: Vec<u8> = [vec![0xFF, 0xFE], text.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat();

        let edited = edit_page_images(&raw, 1, &[(0, "delete", None)]).unwrap();

        assert!(edited.starts_with(&[0xFF, 0xFE]));
        let words: Vec<u16> = edited[2..].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        assert!(!String::from_utf16(&words).unwrap().contains("<img"));
    }

    #[test]
    fn test_page_image_edit_round_trips_latin1_meta_charset() {
        let raw = [b"<html><head><meta charset='ISO-8859-1'></head><body>caf".as_slice(), &[0xE9], b"<img src='old.jpg'/></body></html>"].concat();

        let edited = edit_page_images(&raw, 1, &[(0, "delete", None)]).unwrap();

        assert!(edited.windows(4).any(|window| window == b"caf\xE9"));
        assert!(!edited.windows(4).any(|window| window == b"<img"));
    }

    #[test]
    fn test_page_image_edit_rejects_invalid_encoding_declarations() {
        let raw = b"<?xml version='1.0' encoding='windows-1252'?><html><body><img src='a'/></body></html>";

        let error = edit_page_images(raw, 1, &[(0, "delete", None)]).unwrap_err();

        assert!(error.to_string().contains("문자 인코딩을 읽을 수 없음"));
    }

    #[test]
    fn test_page_image_split_does_not_rewrite_data_src_attribute() {
        let raw = b"<html><body><img data-src='keep.jpg' src='old.jpg'/></body></html>";

        let edited = edit_page_images(raw, 1, &[(0, "split", Some(json!("new.jpg")))]).unwrap();
        let edited = String::from_utf8(edited).unwrap();

        assert!(edited.contains("data-src='keep.jpg'"));
        assert!(edited.contains("src='new.jpg'"));
    }

    // ── 입력 검증과 상한(Python보다 엄격한 부분) ───────────────────────────────────

    fn edit_with(b: &Book, hooks: &EpubCoverHooks, args: EditImages) -> Result<Value, CoverError> {
        edit_images_with(&b.path, &revision(&b.path), &b.trash, EditImages { backup_name: Some("book.epub".to_string()), ..args }, hooks)
    }

    #[test]
    fn test_oversized_image_is_refused_before_decoding() {
        let b = book();
        let original = fs::read(&b.path).unwrap();
        let observer = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = observer.clone();
        let observe = move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        };
        // 바이트 크기: 읽기 전에 거절한다.
        let hooks = EpubCoverHooks::default().with_limits(Limits { max_image_bytes: 10, ..Limits::default() }).with_pixel_decode_observer(Arc::new(observe.clone()));
        assert_refused(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }), "너무 큼", "bytes");
        // 화소 수: 디코딩 전에 거절한다.
        let hooks = EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() }).with_pixel_decode_observer(Arc::new(observe));
        assert_refused(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }), "해상도가 너무 큼", "pixels");
        assert_eq!(observer.load(std::sync::atomic::Ordering::SeqCst), 0, "상한을 넘는 이미지를 디코딩했다");
        assert_eq!(fs::read(&b.path).unwrap(), original);
    }

    #[test]
    fn test_oversized_page_document_is_refused() {
        let b = book();
        let hooks = EpubCoverHooks::default().with_limits(Limits { max_xml_bytes: 50, ..Limits::default() });
        assert!(edit_with(&b, &hooks, EditImages { crops: vec![json!({"idref": "p1", "rect": full()})], ..EditImages::default() }).is_err());
    }
}

// `tests/rust/backend/epub_line_count.rs`에서 옮긴 테스트.
mod epub_line_count {
    use super::{common};

    use std::path::Path;

    use common::epub::{EpubSpec, make_epub};
    use doc_reader::TEXT_SIZE;
    use doc_reader::epub::{EpubHooks, EpubPath, EpubRead, read_from_epub, read_from_epub_with, read_from_epub_with_extracting_zip};

    /// 본문 조각을 `<head>`(제목, 스타일)가 붙은 XHTML 챕터로 감싸 `c0.xhtml`, `c1.xhtml`, ...로 쓴다. 헤드는 행수에 들어가면 안 된다.
    fn write_epub(path: &Path, bodies: &[&str]) {
        let pages: Vec<String> = bodies
            .iter()
            .map(|body| format!(r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>본문 제목</title><style>p {{ color: black; }}</style></head><body>{body}</body></html>"#))
            .collect();
        let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
        make_epub(path, &EpubSpec::chapters(&refs));
    }

    /// Python `reader` fixture의 params: (이름, 읽기 함수).
    type Reader = (&'static str, fn(&Path) -> EpubRead);
    const READERS: [Reader; 2] = [("read_from_epub", read_from_epub), ("read_from_epub_with_extracting_zip", read_from_epub_with_extracting_zip)];

    #[test]
    fn test_line_count_ignores_source_formatting() {
        for (reader_name, reader) in READERS {
            let tmp = tempfile::tempdir().unwrap();
            let bodies = ["<p>첫 문단의 <span>강조</span> 내용</p><p>둘째 문단</p>", "\n  <p>\n    첫 문단의\n    <span>강조</span>\n    내용\n  </p>\n\n  <p>둘째 문단</p>\n"];
            let mut counts = Vec::new();
            for (i, body) in bodies.iter().enumerate() {
                let path = tmp.path().join(format!("format-{i}.epub"));
                write_epub(&path, &[*body]);
                // 일반 경로도 fallback 없이 읽을 수 있는 fixture다.
                assert_eq!(read_from_epub(&path).path, EpubPath::Normal, "reader={reader_name}, body={body:?}");
                counts.push(reader(&path).line_count);
            }
            assert_eq!(counts, [2, 2], "reader={reader_name}");
        }
    }

    #[test]
    fn test_logical_line_count() {
        let cases: [(&str, usize); 10] = [
            ("<p>첫 줄<br/>둘째 줄<br/>셋째 줄</p>", 3),
            ("<p>첫 줄<br/><br/>둘째 줄</p><p> </p><p>&#160;</p>", 2),
            ("<div><h1>제목</h1><p>본문 <em>강조</em></p></div><ul><li>하나</li><li>둘</li></ul>", 4),
            ("<div>앞부분<p>문단</p>뒷부분</div>", 3),
            ("<div><div><p>중첩 문단</p></div></div>", 1),
            ("<table><tr><td>칸 하나</td><td>칸 둘</td></tr></table>", 2),
            ("<pre>첫 줄\n둘째 줄\n\n셋째 줄</pre>", 3),
            ("태그 없는\n본문", 1),
            ("<p>본문</p><!-- 주석 --><script>ignored();</script><style>p {color:red}</style>", 1),
            ("\n<p> </p><p>&#160;</p><br/><img src='cover.jpg'/><!-- 주석 -->", 0),
        ];
        for (reader_name, reader) in READERS {
            for (body, expected) in cases {
                let tmp = tempfile::tempdir().unwrap();
                let path = tmp.path().join("lines.epub");
                write_epub(&path, &[body]);
                assert_eq!(reader(&path).line_count, expected, "reader={reader_name}, body={body:?}");
            }
        }
    }

    #[test]
    fn test_counts_all_chapters_beyond_summary_limit() {
        for (reader_name, reader) in READERS {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("chapters.epub");
            let long = format!("<p>{}</p>", "가".repeat(TEXT_SIZE + 1));
            write_epub(&path, &[long.as_str(), "<p>둘째 장</p>", "<p>셋째 장</p>"]);
            assert_eq!(reader(&path).line_count, 3, "reader={reader_name}");
        }
    }

    #[test]
    fn test_forced_zip_fallback_uses_same_line_count() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fallback.epub");
        write_epub(&path, &["<h1>제목</h1><p>첫 줄<br/>둘째 줄</p>"]);
        let expected = read_from_epub(&path).line_count;

        let fail_read = |_path: &Path| -> Result<(String, usize), String> { Err("force ZIP fallback".to_string()) };
        let hooks = EpubHooks { read_epub: Some(&fail_read) };
        let forced = read_from_epub_with(&path, &hooks);
        assert_eq!(forced.path, EpubPath::Zipped);
        assert_eq!(forced.line_count, 3);
        assert_eq!(expected, 3);
        assert_eq!(forced.line_count, expected);
    }
}
