//! `backend::epub_cover::epub_writer` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use crate::common::epub_book::multi;
use crate::common::shared_fixtures::{assert_refused, full};
use backend::epub_cover::EditImages;
use serde_json::json;
use std::fs;

#[test]
fn test_index_out_of_range_is_refused() {
    let b = multi();
    let original = fs::read(&b.path).unwrap();
    for kind in ["crops", "splits", "deletes"] {
        let mut entry = json!({"idref": "multi", "index": 2});
        match kind {
            "crops" => entry["rect"] = full(),
            "splits" => {
                entry["first"] = full();
                entry["second"] = full();
            }
            _ => {}
        }
        let mut args = EditImages::default();
        match kind {
            "crops" => args.crops = vec![entry],
            "splits" => args.splits = vec![entry],
            _ => args.deletes = vec![entry],
        }
        assert_refused(b.edit(args), "이미지를 찾을 수 없음", kind);
    }
    assert_eq!(fs::read(&b.path).unwrap(), original);
}
