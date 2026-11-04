mod common;

use std::fs;
use std::io::Write;
use std::path::Path;

use classifier::document_reader::read_epub_publisher;
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// Python `_make_epub(path, docs, opf)` 대응. `opf`가 있으면 `content.opf`로 넣는다.
fn make_epub(path: &Path, docs: &[(&str, &str)], opf: Option<&str>) {
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
    make_epub(&epub, &[("body.xhtml", "<p>x</p>")], Some(opf));
    assert_eq!(read_epub_publisher(&epub), "민음사");
}

#[test]
fn test_read_epub_publisher_joins_multiple_and_strips_tags() {
    let tmp = TempDir::new().unwrap();
    let opf = "<package><dc:publisher><b>민음사</b></dc:publisher><dc:publisher>창비</dc:publisher><dc:publisher>  </dc:publisher></package>";
    let epub = tmp.path().join("q.epub");
    make_epub(&epub, &[("body.xhtml", "<p>x</p>")], Some(opf));
    assert_eq!(read_epub_publisher(&epub), "민음사 창비");
}

#[test]
fn test_read_epub_publisher_without_opf() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("r.epub");
    make_epub(&epub, &[("body.xhtml", "<p>x</p>")], None);
    assert_eq!(read_epub_publisher(&epub), "");
}

#[test]
fn test_read_epub_publisher_on_broken_zip() {
    let tmp = TempDir::new().unwrap();
    let broken = tmp.path().join("broken.epub");
    fs::write(&broken, b"not a zip").unwrap();
    assert_eq!(read_epub_publisher(&broken), "");
}
