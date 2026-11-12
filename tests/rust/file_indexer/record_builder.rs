//! `file_indexer::record_builder` 단위 테스트.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use doc_reader::pdf::InProcess;
use file_indexer::child_process_runner::read_file_with_child_stages;
use file_indexer::env_config::Config;
use file_indexer::record_builder::{ReadHooks, ReadOutcome, read_file, read_file_with_hooks, split_author_title};
use parse_stat_counter::Stat;

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
    let without_read_time = |outcome: file_indexer::record_builder::ReadOutcome| {
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

fn library() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("books/1_소설")).unwrap();
    fs::create_dir_all(dir.path().join("comics")).unwrap();
    let cfg = Config::new(dir.path().join("books"), dir.path().join("comics"));
    (dir, cfg)
}

fn read(cfg: &Config, path: &Path) -> ReadOutcome {
    read_file(cfg, path, None, false, None, &InProcess, &Stat::new())
}

#[test]
fn a_page_count_from_the_fallback_counter_is_used_when_the_fast_counter_finds_none() {
    let (dir, cfg) = library();
    let pdf = dir.path().join("comics/book.pdf");
    fs::write(&pdf, b"%PDF-1.4\n").unwrap();
    let fast = |_: &Path| -> Option<usize> { None };
    let fallback = |_: &Path| -> Result<usize, sysdeps::timeout_runner::ParserTimeout> { Ok(7) };
    let hooks = ReadHooks { fast_page_count: Some(&fast), fallback_page_count: Some(&fallback), ..Default::default() };

    let outcome = read_file_with_hooks(&cfg, &pdf, None, false, None, &InProcess, &Stat::new(), &hooks);

    let (_, doc) = outcome.item.expect("만화 PDF는 문서가 된다");
    assert_eq!(doc["page_count"], 7);
}

#[test]
fn an_unreadable_text_file_reports_the_read_error() {
    let (dir, cfg) = library();
    let path = dir.path().join("books/1_소설/a.txt");
    fs::write(&path, "본문\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    let outcome = read(&cfg, &path);

    assert!(!outcome.messages.is_empty(), "{outcome:?}");
}

#[test]
fn a_file_that_is_not_a_docx_reports_the_read_error() {
    let (dir, cfg) = library();
    let path = dir.path().join("books/1_소설/a.docx");
    fs::write(&path, b"PK\x03\x04 not really a docx").unwrap();

    let outcome = read(&cfg, &path);

    assert!(outcome.messages.iter().any(|m| m.starts_with("can't read docx file")), "{outcome:?}");
}

#[test]
fn an_rtf_file_that_is_not_utf8_reports_its_path_and_the_error() {
    let (dir, cfg) = library();
    let path = dir.path().join("books/1_소설/a.rtf");
    fs::write(&path, b"{\\rtf1 \xff\xfe}").unwrap();

    let outcome = read(&cfg, &path);

    assert_eq!(outcome.messages.first().map(String::as_str), Some(path.to_str().unwrap()), "{outcome:?}");
    assert!(outcome.messages.len() >= 2);
}

#[test]
fn an_unreadable_html_file_reports_the_read_error() {
    let (dir, cfg) = library();
    let path = dir.path().join("books/1_소설/a.html");
    fs::write(&path, "<p>본문</p>").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    let outcome = read(&cfg, &path);

    assert!(outcome.messages.iter().any(|m| m.starts_with("can't read html file")), "{outcome:?}");
}

/// `updated_time`은 읽은 시각이 아니라 파일 mtime이다. 최신 목록이 적재 순서가 아닌 파일 변경 순서를 따르게 한다.
#[test]
fn updated_time_is_file_mtime_with_and_without_text() {
    use file_indexer::file_time::{datetime_from_parts, iso_short};
    use std::time::{Duration, UNIX_EPOCH};
    let (dir, cfg) = library();
    let path = dir.path().join("books/1_소설/작가 - 제목.txt");
    fs::write(&path, "본문\n").unwrap();
    let mtime = UNIX_EPOCH + Duration::new(1_600_000_000, 123_456_000);
    fs::File::options().write(true).open(&path).unwrap().set_modified(mtime).unwrap();
    let expected = iso_short(datetime_from_parts(1_600_000_000, 123_456_000));

    for skip_text in [false, true] {
        let outcome = read_file(&cfg, &path, None, skip_text, None, &InProcess, &Stat::new());
        let (_, doc) = outcome.item.unwrap_or_else(|| panic!("문서가 된다(skip_text={skip_text}): {:?}", outcome.messages));
        assert_eq!(doc["updated_time"], expected, "skip_text={skip_text}");
    }
}
