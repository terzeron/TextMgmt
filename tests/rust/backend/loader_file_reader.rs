//! Backend FileReader는 실제 loader/doc_reader 결과를 Elasticsearch 문서로 전달한다.
mod common;

use backend::book_manager::ReadFileError;
use backend::book_manager::{FileReader, LoaderFileReader};
use common::epub::{EpubSpec, make_epub};
use common::pdf::write_blank_pdf;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn loader_reads_book_text_and_epub_content_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    let comics = tmp.path().join("comics");
    std::fs::create_dir_all(books.join("fiction")).unwrap();
    std::fs::create_dir_all(&comics).unwrap();
    let text = books.join("fiction/[Author] Story.txt");
    std::fs::write(&text, "one line\nsecond line\n").unwrap();
    let epub = make_epub(&books.join("fiction/[Writer] Novel.epub"), &EpubSpec::chapters(&["<p>첫 문단</p><p>둘째 문단</p>"]));
    let reader = LoaderFileReader::new(books.clone(), comics);
    let text_docs = reader.read_file(&text, Some(&books));
    let text_doc = &text_docs[&(std::fs::metadata(&text).unwrap().ino() as i64)];
    assert_eq!(text_doc["category"], "fiction");
    assert_eq!(text_doc["title"], "Story");
    assert_eq!(text_doc["author"], "Author");
    assert_eq!(text_doc["line_count"], 3);
    assert!(text_doc["summary"].as_str().unwrap().contains("one line"));
    let epub_docs = reader.read_file(&epub, Some(&books));
    let epub_doc = &epub_docs[&(std::fs::metadata(&epub).unwrap().ino() as i64)];
    assert_eq!(epub_doc["title"], "Novel");
    assert_eq!(epub_doc["author"], "Writer");
    assert_eq!(epub_doc["line_count"], 2);
}

#[test]
fn loader_honors_comics_skip_text_and_requested_path_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    let comics = tmp.path().join("comics");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::create_dir_all(comics.join("series")).unwrap();
    let pdf = comics.join("series/page.pdf");
    write_blank_pdf(&pdf, 480, 623);
    let reader = LoaderFileReader::new(books, comics.clone());
    let docs = reader.read_file_with_options(&pdf, Some(&comics), true);
    let item = &docs[&(std::fs::metadata(&pdf).unwrap().ino() as i64)];
    assert_eq!(item["category"], "series");
    assert_eq!(item["file_path"], "series/page.pdf");
    assert_eq!(item["file_type"], "pdf");
    assert_eq!(item["page_count"], 1);
    assert_eq!(item["line_count"], 0);
    assert_eq!(item["summary"], "");
    let wrong_prefix = reader.read_file(&pdf, Some(Path::new("/missing/library")));
    assert!(wrong_prefix.is_empty());
}

#[test]
fn timed_loader_read_uses_the_backend_worker_process() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    let comics = tmp.path().join("comics");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::create_dir_all(&comics).unwrap();
    let path = books.join("sample.txt");
    std::fs::write(&path, "one line\n").unwrap();
    let reader = Arc::new(LoaderFileReader::new(books.clone(), comics).with_worker_executable(PathBuf::from(env!("CARGO_BIN_EXE_backend"))));

    let docs = reader.try_read_file_with_timeout(path.clone(), Some(books), false, Duration::from_secs(5)).unwrap();

    assert_eq!(docs[&(std::fs::metadata(&path).unwrap().ino() as i64)]["line_count"], 2);
}

#[test]
fn timed_loader_read_kills_a_worker_that_exceeds_its_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    let comics = tmp.path().join("comics");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::create_dir_all(&comics).unwrap();
    let path = books.join("sample.txt");
    std::fs::write(&path, "one line\n").unwrap();
    let worker = tmp.path().join("slow-worker");
    std::fs::write(&worker, "#!/bin/sh\nexec sleep 5\n").unwrap();
    let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&worker, permissions).unwrap();
    let reader = Arc::new(LoaderFileReader::new(books, comics).with_worker_executable(worker));
    let started = Instant::now();

    let result = reader.try_read_file_with_timeout(path, None, false, Duration::from_millis(50));

    assert!(matches!(result, Err(ReadFileError::Timeout(_))));
    assert!(started.elapsed() < Duration::from_secs(1));
}
