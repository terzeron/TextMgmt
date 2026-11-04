//! `file_record::record_builder` 단위 테스트.

use std::fs;

use doc_reader::pdf::InProcess;
use file_record::env_config::Config;
use file_record::record_builder::{read_file, read_file_in_process};
use stat::Stat;

#[test]
fn reading_in_process_matches_read_file_with_the_in_process_runner_and_global_stat() {
    let dir = tempfile::tempdir().unwrap();
    let book = dir.path().join("books");
    let category = book.join("1_소설");
    fs::create_dir_all(&category).unwrap();
    let path = category.join("작가 - 제목.txt");
    fs::write(&path, "본문 첫 줄\n둘째 줄\n").unwrap();
    let cfg = Config::new(&book, dir.path().join("comics"));

    let direct = read_file(&cfg, &path, None, false, None, &InProcess, Stat::global());
    let in_process = read_file_in_process(&cfg, &path, false, None);

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
