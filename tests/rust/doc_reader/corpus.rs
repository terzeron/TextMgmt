use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn books_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/books")
}

pub fn records(jsonl: &str) -> impl Iterator<Item = Value> + '_ {
    jsonl.lines().map(|line| serde_json::from_str(line).expect("valid checked-in corpus record"))
}

pub fn fnv1a64(text: &str) -> String {
    let hash = text.as_bytes().iter().fold(0xcbf29ce484222325_u64, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3));
    format!("{hash:016x}")
}
