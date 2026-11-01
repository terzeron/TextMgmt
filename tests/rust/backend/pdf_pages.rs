//! 원본: tests/test_pdf_pages.py
//!
//! get_pdf_pages 메서드 및 /pdf-pages/ 엔드포인트 단위 테스트. Docker/ES 없이 mock 기반이다.
//!
//! 실제 PDF fixture로 페이지 범위·캐시 예산·디스크 캐시 재사용을 검증한다.
//! HTTP 테스트는 같은 BookManager를 AppDeps에 주입한다.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use backend::book_manager::{BookManager, BookManagerDeps};
use backend::main::{AppDeps, AuthMode, app_with};
use backend::pdf::{LopdfPageOps, PageCountCache, PdfCacheLimits, PdfError, PdfPageOps, PdfReaderCache, PdfReaderHandle};
use common::TestClient;
use common::es_fake::FakeEsManager;
use common::faulty_fs::FaultyFs;
use common::pdf::{make_blank_pdf, page_count_bytes};
use serde_json::{Value, json};
use tempfile::TempDir;

trait ResponseExt {
    fn header(&self, name: &str) -> Option<&str>;
}

impl ResponseExt for backend::main::BookResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

/// `create_test_pdf`
fn create_test_pdf(num_pages: usize, output_path: &Path) {
    make_blank_pdf(output_path, num_pages);
}

/// ES 문서 dict 생성 헬퍼.
fn make_doc(relative_path: &str, file_size: u64, page_count: i64, file_type: &str) -> Value {
    json!({
        "category": "test_category", "title": "Test Book", "author": "Author", "file_path": relative_path, "file_type": file_type,
        "file_size": file_size, "line_count": 0, "page_count": page_count, "isbn": "", "summary": "test", "updated_time": "2024-01-01T00:00:00.000000"
    })
}

struct Env {
    bm: Arc<BookManager>,
    es: Arc<FakeEsManager>,
    temp_dir: PathBuf,
    _tmp: TempDir,
}

/// `configure`는 임시 디렉터리와 기본 Deps를 받는다. 캐시 한도처럼 만들어 둔 파일 크기가 필요한 설정에 쓴다.
fn build(configure: impl FnOnce(&Path, BookManagerDeps) -> BookManagerDeps) -> Env {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let temp_dir = tmp.path().to_path_buf();
    let pdf = temp_dir.join("test_category").join("test_book.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();
    create_test_pdf(5, &pdf);
    let default_doc = make_doc("test_category/test_book.pdf", fs::metadata(&pdf).unwrap().len(), 5, "pdf");
    let es = FakeEsManager::with_doc(default_doc.clone());
    let deps = configure(&temp_dir, BookManagerDeps::new(temp_dir.clone(), es.clone()));
    Env { bm: Arc::new(BookManager::with_deps(deps)), es, temp_dir, _tmp: tmp }
}

fn env() -> Env {
    build(|_, d| d)
}

fn body_contains(body: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(body).contains(needle)
}

#[test]
fn pdf_caches_expose_consistent_empty_clear_and_lookup_state() {
    let reader_cache = PdfReaderCache::new(PdfCacheLimits::default());
    assert!(reader_cache.is_empty());
    assert_eq!(reader_cache.len(), 0);
    assert!(!reader_cache.contains("book.pdf"));
    reader_cache.insert_raw("book.pdf", 1, 5, 12);
    assert!(!reader_cache.is_empty());
    assert!(reader_cache.contains("book.pdf"));
    reader_cache.clear();
    assert!(reader_cache.is_empty());

    let page_cache = PageCountCache::new(2);
    let first = ("one.pdf".to_string(), 10);
    let second = ("two.pdf".to_string(), 20);
    assert!(page_cache.is_empty());
    assert!(!page_cache.contains(&first));
    page_cache.insert(first.clone(), 3);
    page_cache.insert(second.clone(), 4);
    assert_eq!(page_cache.get(&first), Some(3));
    assert!(page_cache.contains(&second));
    page_cache.evict();
    assert_eq!(page_cache.len(), 2);
    page_cache.clear();
    assert!(page_cache.is_empty());
}

#[test]
fn lopdf_page_ops_falls_back_to_path_for_a_reader_without_a_document() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("reader-fallback.pdf");
    create_test_pdf(2, &path);
    let reader = PdfReaderHandle::new(path, 2);

    let bytes = LopdfPageOps.extract_pages_from_reader(&reader, 0, 1).unwrap();

    assert_eq!(page_count_bytes(&bytes), 1);
}

/// 비내장 UHC 한글 글꼴과 쪽별 content/주석으로 원본 객체 보존을 확인한다.
fn make_korean_annotation_pdf(path: &Path, encrypted: bool) {
    use common::pdf::PdfBuilder;
    use lopdf::{EncryptionState, EncryptionVersion, Object, Permissions, dictionary};

    let mut builder = PdfBuilder::new();
    let descriptor = builder.doc.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => "Batang", "Flags" => 6,
        "FontBBox" => vec![Object::Integer(-100), Object::Integer(-300), Object::Integer(1200), Object::Integer(1000)],
        "ItalicAngle" => 0, "Ascent" => 880, "Descent" => -120, "CapHeight" => 700, "StemV" => 80,
    });
    let descendant = builder.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "Batang",
        "CIDSystemInfo" => dictionary! {"Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Korea1"), "Supplement" => 1},
        "FontDescriptor" => descriptor, "DW" => 1000,
    });
    let font = builder.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Batang",
        "Encoding" => "KSCms-UHC-H", "DescendantFonts" => vec![Object::Reference(descendant)],
    });
    let latin = builder.doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"});
    for number in 1..=3 {
        let page = builder.add_page(200, 200);
        builder.page_mut(page).set("Resources", dictionary! {"Font" => dictionary! {"F1" => font, "F2" => latin}});
        builder.content(page, format!("BT /F1 28 Tf 50 100 Td <C7D1B1DB> Tj ET BT /F2 12 Tf 50 50 Td (PAGE-{number}) Tj ET").as_bytes());
        let annotation = builder.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![10.into(), 10.into(), 20.into(), 20.into()],
            "Contents" => Object::string_literal(format!("annotation-{number}")), "P" => page,
        });
        builder.page_mut(page).set("Annots", vec![Object::Reference(annotation)]);
    }
    builder.save(path);
    if encrypted {
        let mut doc = lopdf::Document::load(path).unwrap();
        doc.trailer.set("ID", vec![Object::string_literal("korean-fixture"), Object::string_literal("korean-fixture")]);
        let encryption = EncryptionState::try_from(EncryptionVersion::V2 { document: &doc, owner_password: "owner", user_password: "", key_length: 128, permissions: Permissions::PRINTABLE }).unwrap();
        doc.encrypt(&encryption).unwrap();
        doc.save(path).unwrap();
    }
}

#[test]
fn preview_preserves_korean_font_objects_annotations_and_page_order() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("korean-annotations.pdf");
    make_korean_annotation_pdf(&path, false);
    let original = fs::read(&path).unwrap();

    let bytes = LopdfPageOps.extract_pages(&path, 1, 3).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 2);
    for (output_number, original_number) in [(1, 2), (2, 3)] {
        let id = pages[&output_number];
        let page = doc.get_dictionary(id).unwrap();
        let resources = doc.dereference(page.get(b"Resources").unwrap()).unwrap().1.as_dict().unwrap();
        let fonts = doc.dereference(resources.get(b"Font").unwrap()).unwrap().1.as_dict().unwrap();
        let font = doc.dereference(fonts.get(b"F1").expect("원본 F1 글꼴을 보존한다")).unwrap().1.as_dict().unwrap();
        assert_eq!(font.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
        assert_eq!(font.get(b"BaseFont").unwrap().as_name().unwrap(), b"Batang");
        assert_eq!(font.get(b"Encoding").unwrap().as_name().unwrap(), b"KSCms-UHC-H");
        let descendant = doc.dereference(&font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]).unwrap().1.as_dict().unwrap();
        let descriptor = doc.dereference(descendant.get(b"FontDescriptor").unwrap()).unwrap().1.as_dict().unwrap();
        assert!(![b"FontFile".as_slice(), b"FontFile2", b"FontFile3"].iter().any(|key| descriptor.has(key)));
        let content = lopdf::content::Content::decode(&doc.get_page_content(id)).unwrap();
        let marker = format!("PAGE-{original_number}");
        assert!(
            content.operations.iter().any(|op| op.operator == "Tj" && op.operands.iter().any(|value| value.as_str().ok() == Some(marker.as_bytes()))),
            "page marker {marker} must retain its order"
        );
        let annotations = page.get(b"Annots").unwrap().as_array().unwrap();
        assert_eq!(annotations.len(), 1);
        let annotation = doc.dereference(&annotations[0]).unwrap().1.as_dict().unwrap();
        assert_eq!(annotation.get(b"Contents").unwrap().as_str().unwrap(), format!("annotation-{original_number}").as_bytes());
        assert_eq!(annotation.get(b"P").unwrap().as_reference().unwrap(), id);
    }
    assert_eq!(fs::read(&path).unwrap(), original, "미리보기 생성은 원본을 수정하지 않는다");
}

#[test]
fn preview_rejects_invalid_page_ranges_without_rewriting_the_source() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("ranges.pdf");
    create_test_pdf(3, &path);
    let original = fs::read(&path).unwrap();
    for (start, end) in [(0, 0), (2, 1), (3, 4), (0, 4)] {
        assert!(LopdfPageOps.extract_pages(&path, start, end).is_err(), "범위 [{start}, {end})는 오류여야 한다");
    }
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn preview_reports_missing_input_as_a_storage_error() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing.pdf");
    assert_eq!(LopdfPageOps.page_count(&missing).unwrap_err().kind, "OSError");
    assert_eq!(LopdfPageOps.extract_pages(&missing, 0, 1).unwrap_err().kind, "OSError");
}

#[test]
fn encrypted_preview_removes_encryption_but_preserves_source_bytes_and_annotations() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("encrypted-annotations.pdf");
    make_korean_annotation_pdf(&path, true);
    let original = fs::read(&path).unwrap();
    assert!(lopdf::Document::load_metadata_mem(&original).unwrap().encrypted);
    let bytes = LopdfPageOps.extract_pages(&path, 1, 3).unwrap();
    assert!(!lopdf::Document::load_metadata_mem(&bytes).unwrap().encrypted);
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.get_pages().len(), 2);
    for (output_number, original_number) in [(1, 2), (2, 3)] {
        let page = common::pdf::page_dict(&doc, output_number);
        let annotations = page.get(b"Annots").expect("복호화 뒤에도 주석을 보존한다").as_array().unwrap();
        assert_eq!(annotations.len(), 1);
        let annotation = doc.dereference(&annotations[0]).unwrap().1.as_dict().unwrap();
        assert_eq!(annotation.get(b"Contents").unwrap().as_str().unwrap(), format!("annotation-{original_number}").as_bytes());
    }
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn extracting_encrypted_korean_pages_keeps_readable_korean_text() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("korean.pdf");
    make_korean_annotation_pdf(&path, true);

    let bytes = LopdfPageOps.extract_pages(&path, 1, 3).unwrap();
    let extracted = lopdf::Document::load_mem(&bytes).unwrap();
    assert!(!extracted.is_encrypted());
    assert_eq!(extracted.get_pages().len(), 2);
    let output = tmp.path().join("preview.pdf");
    fs::write(&output, bytes).unwrap();
    let text = sysdeps::pdf::extract_text(&sysdeps::SystemRunner, &output, 1, 2).unwrap();
    assert_eq!(text.matches("한글").count(), 2);
}

#[test]
fn large_pdf_preview_preserves_korean_fonts_annotations_and_order() {
    use std::io::{Read, Seek, SeekFrom, Write};

    let tmp = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        let path = tmp.path().join(format!("large-korean-{encrypted}.pdf"));
        make_korean_annotation_pdf(&path, encrypted);
        let original = fs::read(&path).unwrap();
        assert_eq!(lopdf::Document::load_metadata_mem(&original).unwrap().encrypted, encrypted);
        let source_text = String::from_utf8_lossy(&original);
        let xref = source_text.rsplit_once("startxref").unwrap().1.split_whitespace().next().unwrap();
        let tail = format!("\nstartxref\n{xref}\n%%EOF\n").into_bytes();
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        // 64MiB 초과 분기를 강제하되 실제 디스크 공간은 작은 원본과 마지막 trailer만 쓴다.
        file.set_len(65 * 1024 * 1024).unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(&tail).unwrap();
        drop(file);
        let source_len = fs::metadata(&path).unwrap().len();
        assert!(source_len > 64 * 1024 * 1024);

        let bytes = LopdfPageOps.extract_pages(&path, 1, 3).unwrap();
        assert!(!lopdf::Document::load_metadata_mem(&bytes).unwrap().encrypted);
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert!(!doc.trailer.has(b"Encrypt"));
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 2);
        for (output_number, original_number) in [(1, 2), (2, 3)] {
            let id = pages[&output_number];
            let page = doc.get_dictionary(id).unwrap();
            let resources = doc.dereference(page.get(b"Resources").unwrap()).unwrap().1.as_dict().unwrap();
            let fonts = doc.dereference(resources.get(b"Font").unwrap()).unwrap().1.as_dict().unwrap();
            let font = doc.dereference(fonts.get(b"F1").expect("큰 PDF의 원본 F1 글꼴을 보존한다")).unwrap().1.as_dict().unwrap();
            assert_eq!(font.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
            assert_eq!(font.get(b"BaseFont").unwrap().as_name().unwrap(), b"Batang");
            assert_eq!(font.get(b"Encoding").unwrap().as_name().unwrap(), b"KSCms-UHC-H");
            let descendant = doc.dereference(&font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0]).unwrap().1.as_dict().unwrap();
            let descriptor = doc.dereference(descendant.get(b"FontDescriptor").unwrap()).unwrap().1.as_dict().unwrap();
            assert!(![b"FontFile".as_slice(), b"FontFile2", b"FontFile3"].iter().any(|key| descriptor.has(key)));
            let content = lopdf::content::Content::decode(&doc.get_page_content(id)).unwrap();
            let marker = format!("PAGE-{original_number}");
            assert!(
                content.operations.iter().any(|op| op.operator == "Tj" && op.operands.iter().any(|value| value.as_str().ok() == Some(marker.as_bytes()))),
                "large PDF page marker {marker} must retain its order (encrypted={encrypted})"
            );
            let annotations = page.get(b"Annots").expect("큰 PDF의 주석을 보존한다").as_array().unwrap();
            assert_eq!(annotations.len(), 1);
            let annotation = doc.dereference(&annotations[0]).unwrap().1.as_dict().unwrap();
            assert_eq!(annotation.get(b"Contents").unwrap().as_str().unwrap(), format!("annotation-{original_number}").as_bytes());
            assert_eq!(annotation.get(b"P").unwrap().as_reference().unwrap(), id);
        }

        // sparse 원본 전체를 읽지 않고도 앞부분·trailer·파일 크기가 바뀌지 않았는지 확인한다.
        assert_eq!(fs::metadata(&path).unwrap().len(), source_len);
        let mut source = fs::File::open(&path).unwrap();
        let mut prefix = vec![0; original.len()];
        source.read_exact(&mut prefix).unwrap();
        assert_eq!(prefix, original);
        source.seek(SeekFrom::End(-(tail.len() as i64))).unwrap();
        let mut actual_tail = vec![0; tail.len()];
        source.read_exact(&mut actual_tail).unwrap();
        assert_eq!(actual_tail, tail);
    }
}

#[test]
fn sparse_pdf_preview_reads_only_needed_ranges() {
    use std::io::{Seek, SeekFrom, Write};

    const PROBE_PATH: &str = "TM_PDF_SPARSE_PROBE";
    if let Some(path) = std::env::var_os(PROBE_PATH) {
        let path = Path::new(&path);
        let reader = LopdfPageOps.open_reader(path).unwrap();
        assert_eq!(reader.page_count, 2);
        assert_eq!(page_count_bytes(&LopdfPageOps.extract_pages_from_reader(&reader, 0, 1).unwrap()), 1);
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sparse.pdf");
    create_test_pdf(2, &path);
    let original = fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&original);
    let xref = text.rsplit_once("startxref").unwrap().1.split_whitespace().next().unwrap();
    // EOF 뒤의 빈 구간은 읽을 필요가 없다. 마지막 startxref도 같은 객체를 가리킨다.
    let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(1024 * 1024 * 1024).unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    write!(file, "\nstartxref\n{xref}\n%%EOF\n").unwrap();
    drop(file);

    // 1GiB 파일을 256MiB 제한에서 처리해 전체 read_to_end 경로의 재발을 잡는다.
    let output = std::process::Command::new("prlimit")
        .args(["--as=268435456:536870912", "--"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "sparse_pdf_preview_reads_only_needed_ranges", "--nocapture"])
        .env(PROBE_PATH, &path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn extracting_a_page_from_a_large_tagged_pdf_does_not_rescan_each_deleted_page() {
    use common::pdf::PdfBuilder;
    use lopdf::{Object, dictionary};

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("tagged.pdf");
    let mut builder = PdfBuilder::new();
    let pages: Vec<_> = (0..100).map(|_| builder.add_page(200, 300)).collect();
    // Tagged PDF와 같이 각 쪽을 참조하는 객체가 많으면 반복 삭제가 급격히 느려진다.
    let tags: Vec<_> = (0..4000).map(|i| Object::Reference(builder.doc.add_object(dictionary! {"Pg" => pages[i % pages.len()]}))).collect();
    builder.content(pages[50], b"q 0 0 20 20 re f Q");
    builder.save(&path);
    let mut source = lopdf::Document::load(&path).unwrap();
    let structure = source.add_object(dictionary! {"Type" => "StructTreeRoot", "K" => tags});
    let catalog = source.catalog_mut().unwrap();
    catalog.set("StructTreeRoot", structure);
    catalog.set("MarkInfo", dictionary! {"Marked" => true});
    source.save(&path).unwrap();
    let reader = LopdfPageOps.open_reader(&path).unwrap();
    let started = std::time::Instant::now();
    let bytes = LopdfPageOps.extract_pages_from_reader(&reader, 50, 51).unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(3), "single-page extraction took {elapsed:?}");
    let extracted = lopdf::Document::load_mem(&bytes).unwrap();
    assert!(!extracted.catalog().unwrap().has(b"StructTreeRoot"));
    assert!(!extracted.catalog().unwrap().has(b"MarkInfo"));
    assert!(bytes.len() < 10_000, "preview must not retain the original structure tree");
    assert_eq!(extracted.get_pages().len(), 1);
    let preview_path = tmp.path().join("preview.pdf");
    fs::write(&preview_path, &bytes).unwrap();
    let render = |path: &Path, page: usize| {
        let pixels = sysdeps::pdf::render_page(&sysdeps::SystemRunner, path, page, 150).unwrap();
        assert!(!pixels.is_empty());
        pixels
    };
    assert_eq!(render(&path, 51), render(&preview_path, 1));
    // 경로와 쪽수만 보관하는 reader로 다른 범위도 추출한다.
    assert_eq!(page_count_bytes(&LopdfPageOps.extract_pages_from_reader(&reader, 0, 2).unwrap()), 2);
}

#[test]
fn extracting_pages_updates_nested_counts_and_preserves_inherited_resources() {
    use common::pdf::PdfBuilder;
    use lopdf::{Object, dictionary};

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested.pdf");
    let mut builder = PdfBuilder::new();
    let pages: Vec<_> = (0..4).map(|_| builder.add_page(200, 300)).collect();
    builder.save(&path);
    let mut doc = lopdf::Document::load(&path).unwrap();
    let root = doc.catalog().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
    let mut branches = Vec::new();
    for pair in pages.chunks(2) {
        let kids: Vec<_> = pair.iter().copied().map(Object::Reference).collect();
        let branch = doc.add_object(dictionary! {
            "Type" => "Pages", "Parent" => root, "Count" => 2, "Kids" => kids,
            "MediaBox" => vec![0.into(), 0.into(), 400.into(), 500.into()],
            "Resources" => dictionary! {"ProcSet" => vec![Object::Name(b"PDF".to_vec())]},
        });
        branches.push(Object::Reference(branch));
        for id in pair {
            let page = doc.get_dictionary_mut(*id).unwrap();
            page.set("Parent", branch);
            page.remove(b"MediaBox");
            page.remove(b"Resources");
        }
    }
    doc.get_dictionary_mut(root).unwrap().set("Kids", branches.clone());
    doc.save(&path).unwrap();

    // 두 Pages 노드에 걸친 가운데 두 쪽을 추출한다.
    let bytes = LopdfPageOps.extract_pages(&path, 1, 3).unwrap();
    let extracted = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(extracted.get_pages().len(), 2);
    for number in 1..=2 {
        assert_eq!(common::pdf::mediabox_of(&extracted, number), [0.0, 0.0, 400.0, 500.0]);
        assert!(common::pdf::page_dict(&extracted, number).has(b"Resources"));
    }
}

#[test]
fn encrypted_pdf_preview_only_contains_the_requested_pages_without_encryption() {
    use common::pdf::PdfBuilder;
    use lopdf::{EncryptionState, EncryptionVersion, Permissions};

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("encrypted book.pdf");
    let mut builder = PdfBuilder::new();
    for width in [200, 300, 400, 500] {
        let id = builder.add_page(width, 600);
        builder.content(id, b"q 0 0 20 20 re f Q");
    }
    builder.save(&path);
    let mut source = lopdf::Document::load(&path).unwrap();
    source.trailer.set("ID", vec![lopdf::Object::string_literal("preview-fixture"), lopdf::Object::string_literal("preview-fixture")]);
    let state = EncryptionState::try_from(EncryptionVersion::V2 { document: &source, owner_password: "owner", user_password: "", key_length: 128, permissions: Permissions::PRINTABLE }).unwrap();
    source.encrypt(&state).unwrap();
    source.save(&path).unwrap();

    let reader = LopdfPageOps.open_reader(&path).unwrap();
    assert_eq!(reader.page_count, 4);
    // 암호화 문서는 전체 객체를 reader에 보관하지 않고 필요할 때 파일에서 읽는다.
    let moved = tmp.path().join("moved.pdf");
    fs::rename(&path, &moved).unwrap();
    assert_eq!(LopdfPageOps.extract_pages_from_reader(&reader, 0, 1).unwrap_err().kind, "OSError");
    fs::rename(&moved, &path).unwrap();
    assert_eq!(LopdfPageOps.page_count(&path).unwrap(), 4);
    let bytes = LopdfPageOps.extract_pages_from_reader(&reader, 1, 3).unwrap();
    let metadata = lopdf::Document::load_metadata_mem(&bytes).unwrap();
    assert!(!metadata.encrypted);
    assert_eq!(metadata.page_count, 2);
    let output = tmp.path().join("preview.pdf");
    fs::write(&output, bytes).unwrap();
    assert_eq!(common::pdf::widths(&output), vec![300, 400]);
    let first = LopdfPageOps.extract_pages(&path, 0, 1).unwrap();
    assert_eq!(page_count_bytes(&first), 1);
}

struct CountingPdfPages {
    opens: Arc<AtomicUsize>,
    cached_extracts: Arc<AtomicUsize>,
}

impl PdfPageOps for CountingPdfPages {
    fn page_count(&self, _path: &Path) -> Result<usize, PdfError> {
        Ok(5)
    }

    fn extract_pages(&self, _path: &Path, _start: usize, _end: usize) -> Result<Vec<u8>, PdfError> {
        panic!("cached reader path should be used")
    }

    fn open_reader(&self, path: &Path) -> Result<PdfReaderHandle, PdfError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(PdfReaderHandle::new(path.to_path_buf(), 5))
    }

    fn extract_pages_from_reader(&self, _reader: &PdfReaderHandle, _start: usize, _end: usize) -> Result<Vec<u8>, PdfError> {
        self.cached_extracts.fetch_add(1, Ordering::SeqCst);
        Ok(b"cached PDF pages".to_vec())
    }
}

mod test_get_pdf_pages {
    use super::*;

    #[tokio::test]
    async fn test_returns_single_page() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 1, 1).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(response.header("X-Total-Pages"), Some("5"));
        assert_eq!(response.media_type.as_deref(), Some("application/pdf"));
        assert_eq!(response.header("Content-Encoding"), Some("identity"));

        assert_eq!(page_count_bytes(&response.body), 1);
    }

    #[tokio::test]
    async fn legacy_pdf_caches_are_regenerated_after_korean_font_support_changes() {
        let e = env();
        let cache_dir = e.temp_dir.join(".preview_cache");
        fs::create_dir_all(&cache_dir).unwrap();
        // 이전 서버가 만든 캐시는 최신 mtime이어도 한글이 이미 빠져 있을 수 있다.
        for name in ["1_p1-1.pdf", "1_n5.pdf"] {
            fs::write(cache_dir.join(name), b"LEGACY PDF WITHOUT KOREAN").unwrap();
        }
        let pages = e.bm.get_pdf_pages(1, 1, 1).await;
        assert_eq!(pages.status_code, 200);
        assert_eq!(page_count_bytes(&pages.body), 1);
        let preview = e.bm.get_book_preview(1, None, None).await;
        assert_eq!(preview.status_code, 200);
        assert_eq!(page_count_bytes(&preview.body), 5);
    }

    #[tokio::test]
    async fn get_pdf_pages_reuses_the_cached_reader_for_different_ranges() {
        let opens = Arc::new(AtomicUsize::new(0));
        let cached_extracts = Arc::new(AtomicUsize::new(0));
        let pages = Arc::new(CountingPdfPages { opens: opens.clone(), cached_extracts: cached_extracts.clone() });
        let e = build(|_, deps| deps.with_pdf_pages(pages));

        assert_eq!(e.bm.get_pdf_pages(1, 1, 1).await.status_code, 200);
        assert_eq!(e.bm.get_pdf_pages(1, 2, 2).await.status_code, 200);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
        assert_eq!(cached_extracts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_returns_page_range() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 2, 4).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(response.header("X-Total-Pages"), Some("5"));

        assert_eq!(page_count_bytes(&response.body), 3); // 2, 3, 4
    }

    #[tokio::test]
    async fn test_returns_all_pages() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 1, 5).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(page_count_bytes(&response.body), 5);
    }

    #[tokio::test]
    async fn test_end_clamped_to_total_pages() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 3, 100).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(page_count_bytes(&response.body), 3); // 3, 4, 5
    }

    #[tokio::test]
    async fn test_start_exceeds_total_pages() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 100, 200).await;

        assert_eq!(response.status_code, 400);
    }

    #[tokio::test]
    async fn test_end_before_start_is_rejected() {
        let e = env();
        assert_eq!(e.bm.get_pdf_pages(1, 3, 2).await.status_code, 400);
        assert!(!e.temp_dir.join(".preview_cache").join(format!("1_p3-2_v{}.pdf", BookManager::PDF_PREVIEW_CACHE_VERSION)).exists());
    }

    #[tokio::test]
    async fn test_range_over_the_page_limit_is_rejected() {
        // 범위마다 캐시 파일이 생기므로 한 요청의 쪽 수를 제한한다. 뷰어는 11쪽씩 요청한다.
        let e = env();
        create_test_pdf(60, &e.temp_dir.join("test_category").join("test_book.pdf"));
        assert_eq!(e.bm.get_pdf_pages(1, 1, 50).await.status_code, 200);
        assert_eq!(e.bm.get_pdf_pages(1, 1, 51).await.status_code, 400);
    }

    #[tokio::test]
    async fn test_preview_cache_is_keyed_by_page_count() {
        // 큰 pages 요청이 캐시를 덮어 다른 사용자의 기본 미리보기가 전체 책이 되면 안 된다.
        let e = env();
        let cache_dir = e.temp_dir.join(".preview_cache");

        let two = e.bm.get_book_preview(1, Some(2), None).await;
        assert_eq!(page_count_bytes(&two.body), 2);
        let four = e.bm.get_book_preview(1, Some(4), None).await;
        assert_eq!(page_count_bytes(&four.body), 4);
        let two_again = e.bm.get_book_preview(1, Some(2), None).await;
        assert_eq!(page_count_bytes(&two_again.body), 2);

        assert!(cache_dir.join(format!("1_n2_v{}.pdf", BookManager::PDF_PREVIEW_CACHE_VERSION)).exists());
        assert!(!cache_dir.join(format!("1_n4_v{}.pdf", BookManager::PDF_PREVIEW_CACHE_VERSION)).exists(), "같은 책의 다른 쪽 수 캐시는 하나만 남긴다");
        assert!(!cache_dir.join("1.pdf").exists());
    }

    #[tokio::test]
    async fn test_cache_writes_leave_no_temp_files() {
        let e = env();
        assert_eq!(e.bm.get_book_preview(1, Some(2), None).await.status_code, 200);
        assert_eq!(e.bm.get_pdf_pages(1, 1, 2).await.status_code, 200);
        let names: Vec<String> = fs::read_dir(e.temp_dir.join(".preview_cache")).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }

    #[tokio::test]
    async fn test_book_not_found() {
        let e = env();
        e.es.set_doc(None);

        let response = e.bm.get_pdf_pages(999, 1, 1).await;
        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_cache_hit() {
        let e = env();

        // 첫 번째 호출: 캐시 생성
        let response1 = e.bm.get_pdf_pages(1, 1, 2).await;
        assert_eq!(response1.status_code, 200);

        // 캐시 파일 존재 확인
        let cache_file = e.temp_dir.join(".preview_cache").join(format!("1_p1-2_v{}.pdf", BookManager::PDF_PREVIEW_CACHE_VERSION));
        assert!(cache_file.exists());

        // 두 번째 호출: 캐시 히트 (같은 결과)
        let response2 = e.bm.get_pdf_pages(1, 1, 2).await;
        assert_eq!(response2.status_code, 200);
        assert_eq!(response2.header("X-Total-Pages"), Some("5"));
    }

    #[tokio::test]
    async fn test_not_a_pdf_file() {
        let e = env();

        let txt_path = e.temp_dir.join("test_category").join("test.txt");
        fs::write(&txt_path, "hello").unwrap();

        e.es.set_doc(Some(make_doc("test_category/test.txt", fs::metadata(&txt_path).unwrap().len(), 5, "txt")));

        let response = e.bm.get_pdf_pages(1, 1, 1).await;
        assert_eq!(response.status_code, 400);
    }

    #[tokio::test]
    async fn test_missing_file_returns_404() {
        let e = env();
        e.es.set_doc(Some(make_doc("test_category/gone.pdf", 1234, 5, "pdf")));

        let response = e.bm.get_pdf_pages(1, 1, 1).await;
        assert_eq!(response.status_code, 404);
        assert!(body_contains(&response.body, "File not found"));
    }

    /// 스토리지 마운트가 끊겨 stat()이 EIO를 내면 사유를 담은 503을 반환해야 한다.
    ///
    /// 예전에는 이 OSError가 그대로 escape해 CORSMiddleware를 우회한 500이 나갔고,
    /// 브라우저에는 "Failed to fetch"로만 보여 원인을 알 수 없었다.
    #[tokio::test]
    async fn test_storage_io_error_returns_503_with_reason() {
        // errno.EIO(5)와 os.strerror(EIO)
        let faulty = FaultyFs::new();
        faulty.set_is_file(|_| Some(std::io::Error::from_raw_os_error(5)));
        let e = build(|_, d| d.with_fs(faulty));

        let response = e.bm.get_pdf_pages(1, 1, 1).await;

        assert_eq!(response.status_code, 503);
        assert!(body_contains(&response.body, "Storage access error"));
        assert!(body_contains(&response.body, "Input/output error"));
    }
}

mod test_pdf_pages_endpoint {
    use super::*;
    use backend::main::{CategoryMappingApi, MappingError};

    struct HiddenCategories;

    impl CategoryMappingApi for HiddenCategories {
        fn get_hidden_categories(&self, content_type: &str) -> Result<Vec<String>, MappingError> {
            Ok(if content_type == "book" { vec!["test_category".into()] } else { vec![] })
        }
    }

    #[tokio::test]
    async fn test_authentication_precedes_input_validation() {
        let e = env();
        let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_refresh_token_store(crate::common::session::store()).with_book_manager(e.bm.clone())));
        for path in ["/pdf-pages/1", "/pdf-pages/invalid?start=bad", "/comics/pdf-pages/1", "/preview/1", "/download/1", "/validate/1", "/html-resource/1?path=x", "/comics/preview/1"] {
            assert_eq!(client.get(path).await.status, 401);
        }
    }

    #[tokio::test]
    async fn test_invalid_integer_returns_422() {
        let (client, _e) = setup_client();
        for path in ["/pdf-pages/invalid", "/pdf-pages/1?start=no", "/pdf-pages/1?end=no"] {
            assert_eq!(client.get(path).await.status, 422);
        }
    }

    #[tokio::test]
    async fn test_viewer_hidden_category_and_descendants_are_forbidden() {
        let e = env();
        let deps = AppDeps::new(AuthMode::AuthOnly(json!({"role":"viewer"}))).with_book_manager(e.bm.clone()).with_category_mapping(Arc::new(HiddenCategories));
        let client = TestClient::with_app(app_with(deps));
        for category in ["test_category", "test_category/child"] {
            e.es.with(|state| state.default_doc.as_mut().unwrap()["category"] = json!(category));
            for path in ["/pdf-pages/1", "/preview/1", "/download/1", "/validate/1", "/cover/1", "/html-resource/1?path=x"] {
                let response = client.get(path).await;
                assert_eq!(response.status, 403, "{path}");
                assert_eq!(response.json()["detail"], "접근 권한이 없는 카테고리입니다.");
            }
        }
        e.es.with(|state| state.default_doc.as_mut().unwrap()["category"] = json!("test_category_other"));
        assert_eq!(client.get("/pdf-pages/1").await.status, 200);
    }

    #[tokio::test]
    async fn test_comics_uses_comic_manager_and_category_policy() {
        let e = env();
        let deps = AppDeps::new(AuthMode::AuthOnly(json!({"role":"viewer"}))).with_comics_manager(e.bm.clone()).with_category_mapping(Arc::new(HiddenCategories));
        let client = TestClient::with_app(app_with(deps));
        let response = client.get("/comics/pdf-pages/1?start=2&end=3").await;
        assert_eq!(response.status, 200);
        assert_eq!(page_count_bytes(&response.body), 2);
    }

    #[tokio::test]
    async fn test_preview_page_limit_and_comics_route() {
        let e = env();
        let comics = backend::comics_manager::ComicsManager::with_deps(BookManagerDeps::new(e.temp_dir.clone(), e.es.clone()));
        let deps = AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager(e.bm.clone()).with_comics_manager(Arc::new(comics));
        let client = TestClient::with_app(app_with(deps));
        for prefix in ["", "/comics"] {
            let response = client.get(&format!("{prefix}/preview/1?pages=2")).await;
            assert_eq!(response.status, 200);
            assert_eq!(page_count_bytes(&response.body), 2);
            assert_eq!(client.get(&format!("{prefix}/preview/1?pages=bad")).await.status, 422);
            let validation = client.get(&format!("{prefix}/validate/1")).await;
            assert_eq!(validation.json()["status"], "success");
            assert_eq!(validation.json()["result"]["publication"]["page_count"], 5);
        }
    }

    /// 라우터 클로저가 캡처한 manager의 내부 속성을 패치하는 fixture를 `AppDeps::with_book_manager`로 대신한다.
    fn setup_client() -> (TestClient, Env) {
        let e = env();
        let token = crate::common::session::token("admin@test.com", "admin", Some("Test Admin"), None);
        let client =
            TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_refresh_token_store(crate::common::session::store()).with_book_manager(e.bm.clone()))).cookie("tm_access_token", &token);
        (client, e)
    }

    #[tokio::test]
    async fn test_get_pdf_pages_default_params() {
        let (client, _e) = setup_client();
        let response = client.get("/pdf-pages/1").await;
        assert_eq!(response.status, 200);
        assert!(response.header("X-Total-Pages").is_some());
    }

    #[tokio::test]
    async fn test_get_pdf_pages_with_range() {
        let (client, _e) = setup_client();
        let response = client.get("/pdf-pages/1?start=2&end=3").await;
        assert_eq!(response.status, 200);
        assert_eq!(response.header("X-Total-Pages"), Some("5"));
        assert_eq!(response.header("content-type"), Some("application/pdf"));
    }

    #[tokio::test]
    async fn test_get_pdf_pages_x_total_pages_exposed() {
        let (client, _e) = setup_client();
        let response = client.get("/pdf-pages/1?start=1&end=1").await;
        assert_eq!(response.status, 200);
        assert_eq!(response.header("X-Total-Pages"), Some("5"));
    }

    #[tokio::test]
    async fn test_get_pdf_pages_start_exceeds_total() {
        let (client, _e) = setup_client();
        let response = client.get("/pdf-pages/1?start=100&end=200").await;
        assert_eq!(response.status, 400);
    }
}

/// `_pdf_reader_cache`가 개수가 아니라 바이트로 제한되는지 검증한다.
///
/// cached PdfReader 1개는 PDF 파일 크기와 거의 1:1로 RSS를 차지한다
/// (pod 실측: 134MiB 파일 -> RSS +134.2MiB). 개수만 제한하면 /comics 의 1.5GiB 급
/// PDF 8개가 한 worker 에서 12GiB 를 점유해 노드 메모리를 고갈시킨다.
mod test_pdf_reader_cache_budget {
    use super::*;

    /// 파일당 상한을 넘는 PDF 는 reader 를 캐시하지 않는다 (페이지 수만 캐시).
    #[test]
    fn test_large_pdf_reader_not_cached() {
        let e = build(|dir, d| {
            let pdf = dir.join("test_category").join("budget_large.pdf");
            create_test_pdf(3, &pdf);
            let size = fs::metadata(&pdf).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(size - 1), ..PdfCacheLimits::default() })
        });
        let pdf = e.temp_dir.join("test_category").join("budget_large.pdf");

        let (reader, total_pages) = e.bm._get_cached_pdf_reader(&pdf);

        assert_eq!(total_pages, 3);
        assert!(Arc::strong_count(&reader) >= 1); // reader 자체는 정상 반환되어야 한다
        assert!(!e.bm.pdf_cache().contains(&pdf.to_string_lossy())); // 그러나 보관하지 않는다
        // 페이지 수는 int 하나라 비용이 없으므로 항상 캐시한다
        assert!(e.bm.page_count_cache().contains(&(pdf.to_string_lossy().into_owned(), common::fsx::mtime_key(&pdf))));
    }

    #[test]
    fn test_small_pdf_reader_is_cached_and_reused() {
        let e = build(|dir, d| {
            let pdf = dir.join("test_category").join("budget_small.pdf");
            create_test_pdf(2, &pdf);
            let size = fs::metadata(&pdf).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(size + 1), ..PdfCacheLimits::default() })
        });
        let pdf = e.temp_dir.join("test_category").join("budget_small.pdf");

        let (reader1, _) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(e.bm.pdf_cache().contains(&pdf.to_string_lossy()));
        let (reader2, _) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(Arc::ptr_eq(&reader1, &reader2)); // 같은 객체를 재사용 (재파싱 없음)
    }

    /// 전체 예산을 넘으면 개수와 무관하게 LRU 로 축출한다.
    #[test]
    fn test_byte_budget_evicts_lru() {
        let e = build(|dir, d| {
            let first = dir.join("test_category").join("budget_0.pdf");
            create_test_pdf(2, &first);
            let one_size = fs::metadata(&first).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits {
                max_file_bytes: Some(one_size + 1),
                max_bytes: Some(one_size * 2 + 1),
                max_entries: Some(100), // 개수는 넉넉히
            })
        });
        let mut pdfs = Vec::new();
        for i in 0..3 {
            let p = e.temp_dir.join("test_category").join(format!("budget_{i}.pdf"));
            create_test_pdf(2, &p);
            pdfs.push(p);
        }

        for p in &pdfs {
            e.bm._get_cached_pdf_reader(p);
        }

        assert_eq!(e.bm.pdf_cache().len(), 2);
        assert!(!e.bm.pdf_cache().contains(&pdfs[0].to_string_lossy())); // 가장 오래된 것이 축출
        assert!(e.bm.pdf_cache().contains(&pdfs[2].to_string_lossy()));
        let total: u64 = pdfs.iter().filter(|p| e.bm.pdf_cache().contains(&p.to_string_lossy())).map(|p| fs::metadata(p).unwrap().len()).sum();
        assert!(total <= e.bm.pdf_cache().limits().max_bytes.unwrap());
    }

    #[test]
    fn test_mtime_change_invalidates_reader_cache() {
        let e = build(|_, d| d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(10 * 1024 * 1024), ..PdfCacheLimits::default() }));
        let pdf = e.temp_dir.join("test_category").join("budget_mtime.pdf");
        create_test_pdf(2, &pdf);

        let (reader1, pages1) = e.bm._get_cached_pdf_reader(&pdf);
        assert_eq!(pages1, 2);

        create_test_pdf(4, &pdf); // 내용 교체
        let meta = fs::metadata(&pdf).unwrap();
        let times = fs::FileTimes::new().set_accessed(meta.accessed().unwrap() + Duration::from_secs(10)).set_modified(meta.modified().unwrap() + Duration::from_secs(10));
        fs::OpenOptions::new().write(true).open(&pdf).unwrap().set_times(times).unwrap();

        let (reader2, pages2) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(!Arc::ptr_eq(&reader2, &reader1));
        assert_eq!(pages2, 4);
        assert_eq!(e.bm.pdf_cache().len(), 1); // 낡은 항목이 남지 않는다
    }

    /// 실제 페이지 수 조회와 추출에서 PDF를 읽는 횟수를 센다.
    struct CountingOpener {
        inner: backend::pdf::LopdfPageOps,
        opened: Arc<AtomicUsize>,
    }

    impl backend::pdf::PdfPageOps for CountingOpener {
        fn page_count(&self, path: &Path) -> Result<usize, backend::pdf::PdfError> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            self.inner.page_count(path)
        }

        fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, backend::pdf::PdfError> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            self.inner.extract_pages(path, start, end)
        }
    }

    /// 디스크 .preview_cache 히트 시 PdfReader 를 만들지 않아야 한다.
    ///
    /// 예전에는 reader 를 디스크 캐시 확인보다 먼저 만들었기 때문에, 캐시 히트여도
    /// GB급 PDF 를 통째로 파싱했다 (관측된 최장 응답 125초).
    #[tokio::test]
    async fn test_disk_cache_hit_does_not_open_pdf() {
        let opened = Arc::new(AtomicUsize::new(0));
        let counter = opened.clone();
        let e = build(move |_, d| d.with_pdf_pages(Arc::new(CountingOpener { inner: backend::pdf::LopdfPageOps, opened: counter })));

        let first = e.bm.get_pdf_pages(1, 1, 2).await;
        assert_eq!(first.status_code, 200);

        // reader 캐시만 비운다. 페이지 수 캐시와 디스크 캐시는 남은 상태.
        e.bm.pdf_cache().clear();
        let before = opened.load(Ordering::SeqCst);
        assert!(before > 0);

        let second = e.bm.get_pdf_pages(1, 1, 2).await;

        assert_eq!(second.status_code, 200);
        assert_eq!(second.header("X-Total-Pages"), Some("5"));
        // `assert not mock_reader.called`: 두 번째 호출은 PDF를 열지 않는다.
        assert_eq!(opened.load(Ordering::SeqCst), before);
    }
}
