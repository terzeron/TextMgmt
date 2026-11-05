//! `doc_reader::pdf_page_reader` 단위 테스트.

use doc_reader::pdf_page_reader::{LopdfPageOps, PdfPageOps, PdfReaderHandle};
use lopdf::{Document, Object, dictionary};
use std::path::Path;

/// 빈 쪽 `pages`개짜리 PDF.
fn blank_pdf(path: &Path, pages: usize) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let kids: Vec<Object> = (0..pages).map(|_| doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()]}).into()).collect();
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => pages as i64}));
    let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}

#[test]
fn lopdf_page_ops_falls_back_to_path_for_a_reader_without_a_document() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("reader-fallback.pdf");
    blank_pdf(&path, 2);
    let reader = PdfReaderHandle::new(path, 2);

    let bytes = LopdfPageOps.extract_pages_from_reader(&reader, 0, 1).unwrap();

    assert_eq!(Document::load_mem(&bytes).unwrap().get_pages().len(), 1);
}
