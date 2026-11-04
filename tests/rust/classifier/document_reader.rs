mod common;
use std::fs;
use std::io::Write;
use std::path::Path;
use classifier::document_reader::read_epub_publisher;
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;
use common::epub::{EpubSpec, make_epub};
use common::zipx::write_zip;

/// Python `_make_epub(path, docs, opf)` 대응. `opf`가 있으면 `content.opf`로 넣는다.
fn make_opf_epub(path: &Path, docs: &[(&str, &str)], opf: Option<&str>) {
    let mut z = ZipWriter::new(fs::File::create(path).unwrap());
    let opts = SimpleFileOptions::default();
    for (name, body) in docs {
        z.start_file(*name, opts).unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    if let Some(opf) = opf {
        z.start_file("content.opf", opts).unwrap();
        z.write_all(opf.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

#[test]
fn test_read_epub_publisher() {
    let tmp = TempDir::new().unwrap();
    let opf = "<package><metadata><dc:publisher>민음사</dc:publisher></metadata></package>";
    let epub = tmp.path().join("p.epub");
    make_opf_epub(&epub, &[("body.xhtml", "<p>x</p>")], Some(opf));
    assert_eq!(read_epub_publisher(&epub), "민음사");
}

#[test]
fn test_read_epub_publisher_joins_multiple_and_strips_tags() {
    let tmp = TempDir::new().unwrap();
    let opf = "<package><dc:publisher><b>민음사</b></dc:publisher><dc:publisher>창비</dc:publisher><dc:publisher>  </dc:publisher></package>";
    let epub = tmp.path().join("q.epub");
    make_opf_epub(&epub, &[("body.xhtml", "<p>x</p>")], Some(opf));
    assert_eq!(read_epub_publisher(&epub), "민음사 창비");
}

#[test]
fn test_read_epub_publisher_without_opf() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("r.epub");
    make_opf_epub(&epub, &[("body.xhtml", "<p>x</p>")], None);
    assert_eq!(read_epub_publisher(&epub), "");
}

#[test]
fn test_read_epub_publisher_on_broken_zip() {
    let tmp = TempDir::new().unwrap();
    let broken = tmp.path().join("broken.epub");
    fs::write(&broken, b"not a zip").unwrap();
    assert_eq!(read_epub_publisher(&broken), "");
}

#[test]
fn test_read_epub_publisher_reads_opf_and_tolerates_broken_files() {
    let tmp = TempDir::new().unwrap();
    let good = make_epub(&tmp.path().join("good.epub"), &EpubSpec::publisher("을유문화사"));
    assert_eq!(read_epub_publisher(&good), "을유문화사");

    let no_opf = tmp.path().join("no_opf.epub");
    write_zip(&no_opf, &[("a.txt", "내용".as_bytes())]);
    assert_eq!(read_epub_publisher(&no_opf), "");

    let broken = tmp.path().join("broken.epub");
    fs::write(&broken, b"not a zip").unwrap();
    assert_eq!(read_epub_publisher(&broken), "");
}
