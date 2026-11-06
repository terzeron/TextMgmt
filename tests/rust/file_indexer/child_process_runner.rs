//! `file_indexer::child_process_runner` 단위 테스트. 워커 실행 파일의 동작은 `integration/index_worker.rs`가 본다.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use file_indexer::child_process_runner::{ChildReadError, read_file_in_child};
use file_indexer::env_config::Config;

#[test]
fn a_worker_answer_whose_item_has_the_wrong_shape_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let worker = dir.path().join("fake-worker");
    fs::write(&worker, "#!/bin/sh\necho '{\"item\": 5, \"messages\": []}'\n").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = Config::new(dir.path().join("book"), dir.path().join("comics"));

    let result = read_file_in_child(&worker, &cfg, &dir.path().join("a.txt"), None, false, Duration::from_secs(10));

    assert!(matches!(&result, Err(ChildReadError::Other(message)) if message == "worker returned invalid document"), "{result:?}");
}
