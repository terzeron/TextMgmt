//! 모델 파일 핫 리로드: 바뀌면 다시 읽고, 깨진 파일이면 이전 모델을 유지한다.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use classifier::model_reloader::ModelReloader;
use classifier::softmax_model::{ModelHeader, SoftmaxModel};

fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("model_reloader").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn model(categories: &[&str]) -> SoftmaxModel {
    let k = categories.len();
    SoftmaxModel {
        header: ModelHeader {
            version: 1,
            seed: 0,
            created_unix_secs: 0,
            hash_bits: 4,
            categories: categories.iter().map(|c| c.to_string()).collect(),
            population_counts: vec![1; k],
            sample_counts: vec![1; k],
        },
        weights: vec![0.0; k << 4],
        bias: vec![0.0; k],
    }
}

/// mtime 해상도에 기대지 않도록 교체할 때마다 수정 시각을 명시한다.
fn touch(path: &PathBuf, secs: u64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
}

#[test]
fn missing_file_gives_none() {
    let reloader = ModelReloader::new(temp_dir("missing").join("model.bin"));
    assert!(reloader.current().is_none());
}

#[test]
fn loads_once_and_reuses_while_unchanged() {
    let path = temp_dir("reuse").join("model.bin");
    model(&["a"]).save_atomic(&path).unwrap();
    let reloader = ModelReloader::new(path);
    let first = reloader.current().unwrap();
    let second = reloader.current().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn reloads_after_file_is_replaced() {
    let path = temp_dir("replace").join("model.bin");
    model(&["a"]).save_atomic(&path).unwrap();
    touch(&path, 1_000);
    let reloader = ModelReloader::new(path.clone());
    assert_eq!(reloader.current().unwrap().header.categories, vec!["a"]);

    model(&["a", "b"]).save_atomic(&path).unwrap();
    touch(&path, 2_000);
    assert_eq!(reloader.current().unwrap().header.categories, vec!["a", "b"]);
}

#[test]
fn keeps_previous_model_when_replacement_is_corrupt() {
    let path = temp_dir("corrupt").join("model.bin");
    model(&["a"]).save_atomic(&path).unwrap();
    touch(&path, 1_000);
    let reloader = ModelReloader::new(path.clone());
    let before = reloader.current().unwrap();

    std::fs::write(&path, b"broken").unwrap();
    touch(&path, 2_000);
    let after = reloader.current().unwrap();
    assert!(Arc::ptr_eq(&before, &after));
}

#[test]
fn file_deleted_after_load_gives_none() {
    let path = temp_dir("deleted").join("model.bin");
    model(&["a"]).save_atomic(&path).unwrap();
    let reloader = ModelReloader::new(path.clone());
    assert!(reloader.current().is_some());
    std::fs::remove_file(&path).unwrap();
    assert!(reloader.current().is_none());
}
