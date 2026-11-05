//! `file_record::record_builder` 단위 테스트.

use std::fs;

use doc_reader::pdf::InProcess;
use file_record::child_process_runner::read_file_with_child_stages;
use file_record::env_config::Config;
use file_record::record_builder::{read_file, split_author_title};
use stat::Stat;

/// PDF 단계가 없는 TXT라서 worker 실행 파일은 불리지 않는다.
#[test]
fn reading_with_child_stages_matches_read_file_with_the_global_stat() {
    let dir = tempfile::tempdir().unwrap();
    let book = dir.path().join("books");
    let category = book.join("1_소설");
    fs::create_dir_all(&category).unwrap();
    let path = category.join("작가 - 제목.txt");
    fs::write(&path, "본문 첫 줄\n둘째 줄\n").unwrap();
    let cfg = Config::new(&book, dir.path().join("comics"));

    let direct = read_file(&cfg, &path, None, false, None, &InProcess, Stat::global());
    let in_process = read_file_with_child_stages(std::path::Path::new("/bin/false"), &cfg, &path, None, false);

    // `updated_time`은 읽은 시각이라 호출마다 다르다. 있는지만 보고 나머지를 비교한다.
    let without_read_time = |outcome: file_record::record_builder::ReadOutcome| {
        outcome.item.map(|(id, mut doc)| {
            assert!(doc.get("updated_time").and_then(|v| v.as_str()).is_some_and(|t| !t.is_empty()));
            doc.as_object_mut().unwrap().remove("updated_time");
            (id, doc)
        })
    };
    assert!(direct.item.is_some(), "TXT는 문서가 된다: {:?}", direct.messages);
    assert_eq!(in_process.messages, direct.messages);
    assert_eq!(without_read_time(in_process), without_read_time(direct));
}

/// `[저자] 제목` 꼴만 나눈다. 저자와 제목이 모두 있고 한 줄이어야 한다. backend 파일 읽기도 이 함수를 쓴다.
#[test]
fn author_title_split_needs_both_parts_on_one_line() {
    assert_eq!(split_author_title("[저자] 제목"), ("저자".into(), "제목".into()));
    for stem in ["[저자]", "[] 제목", "[저자] 제\n목", "[저자 제목"] {
        assert_eq!(split_author_title(stem), (String::new(), stem.to_string()), "{stem:?}");
    }
}
