//! `cover_manager::epub_cover::epub_writer` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use crate::common::epub_book::multi;
use crate::common::epub_fixtures::{assert_refused, full};
use cover_manager::epub_cover::EditImages;
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

#[test]
fn test_splitting_two_identical_images_gives_the_second_part_a_numbered_id() {
    let tmp = tempfile::tempdir().unwrap();
    // 글이 있는 두 쪽이 바이트까지 같은 이미지를 쓴다.
    let path = crate::common::epub_book::make_book(
        &tmp.path().join("book.epub"),
        &[
            ("OPS/p1.xhtml", crate::common::epub_book::page(&["img/1.jpg"], "글 하나").into_bytes()),
            ("OPS/p2.xhtml", crate::common::epub_book::page(&["img/2.png"], "글 둘").into_bytes()),
            ("OPS/img/2.png", crate::common::epub_fixtures::jpeg("red")),
        ],
    );
    let halves = |idref: &str| json!({"idref": idref, "index": 0, "first": [0, 0, 0.5, 1, 0], "second": [0.5, 0, 1, 1, 0]});

    crate::common::epub_fixtures::edit(&path, &tmp.path().join("0_trash"), EditImages { splits: vec![halves("p1"), halves("p2")], ..EditImages::default() }).unwrap();

    let names = crate::common::zipx::member_names(&path);
    let splits: Vec<&String> = names.iter().filter(|name| name.contains("tm-split-")).collect();
    assert_eq!(splits.len(), 2, "{names:?}");
    assert!(splits.iter().any(|name| name.ends_with("-2.jpg")), "{splits:?}");
}
