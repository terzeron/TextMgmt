#[path = "common/mod.rs"]
mod common;

use book_manager::ReadFileError;
use book_manager::{FileReader, LoaderFileReader};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
