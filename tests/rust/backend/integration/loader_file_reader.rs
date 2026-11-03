#[path = "../common/mod.rs"]
mod common;

use backend::book_manager::{FileReader, LoaderFileReader};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
