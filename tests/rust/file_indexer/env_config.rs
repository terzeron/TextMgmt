//! `file_indexer::env_config` 단위 테스트.

use std::path::Path;

use file_indexer::env_config::Config;

#[test]
fn from_env_names_the_first_missing_variable() {
    let none = |_: &str| None;
    assert!(Config::from_env(&none).unwrap_err().contains("TM_BOOK_DIR"));
    let book_only = |key: &str| (key == "TM_BOOK_DIR").then(|| "/books".to_string());
    assert!(Config::from_env(&book_only).unwrap_err().contains("TM_COMICS_DIR"));
    let both = |key: &str| Some(if key == "TM_BOOK_DIR" { "/books" } else { "/comics" }.to_string());
    let config = Config::from_env(&both).unwrap();
    assert_eq!(config.book_prefix, Path::new("/books"));
    assert_eq!(config.comics_prefix, Path::new("/comics"));
    assert!(!config.reencode && !config.reencode_dry_run && config.reencode_backup_dir.is_none());
}

#[test]
fn path_prefix_and_is_comics_follow_the_comics_root_by_path_components() {
    let config = Config::new("/data/text", "/data/comics");
    assert_eq!(config.path_prefix(Path::new("/data/comics/x/a.pdf")), Path::new("/data/comics"));
    assert!(config.is_comics(Path::new("/data/comics/x/a.pdf")));
    assert_eq!(config.path_prefix(Path::new("/data/text/a.txt")), Path::new("/data/text"));
    // 이름이 비슷한 형제 디렉토리는 만화가 아니다(경로 구성 요소 단위 비교).
    assert!(!config.is_comics(Path::new("/data/comics2/a.pdf")));
    assert_eq!(config.path_prefix(Path::new("/data/comics2/a.pdf")), Path::new("/data/text"));
}
