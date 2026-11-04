#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use backend::book_manager::{BookManager, BookManagerDeps};
use backend::main::{AppDeps, AuthMode, app_with};
use backend::pdf_page_reader::{LopdfPageOps, PdfPageOps};
use common::TestClient;
use common::es_fake::FakeEsManager;
use common::pdf::{make_blank_pdf, page_count_bytes};
use serde_json::{Value, json};
use tempfile::TempDir;

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
    let text = sysdeps::pdf_worker_client::extract_text(&sysdeps::SystemRunner, &output, 1, 2).unwrap();
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
        let pixels = sysdeps::pdf_worker_client::render_page(&sysdeps::SystemRunner, path, page, 150).unwrap();
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
