//! corpus_common의 경로·목록 도구 테스트 (detect_mojibake 테스트에서 옮겨 왔다).

use std::fs;
use std::path::Path;

use corpus_common::*;

fn write(p: &Path, text: &str) {
    fs::write(p, text).unwrap();
}

#[test]
fn list_library_files_skips_tool_caches_hidden_files_and_other_extensions() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    fs::create_dir_all(root.join("3_무협")).unwrap();
    write(&root.join("3_무협/a.txt"), "x");
    write(&root.join("3_무협/.숨김.txt"), "x");
    fs::write(root.join("3_무협/b.epub"), b"x").unwrap();
    fs::write(root.join("3_무협/c.pdf"), b"x").unwrap();
    for cache in ["__pycache__", ".ruff_cache", ".preview_cache"] {
        fs::create_dir_all(root.join(cache)).unwrap();
        write(&root.join(cache).join("d.txt"), "x");
    }
    let names: Vec<String> = list_library_files(root).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["a.txt", "b.epub"]);
}

#[test]
fn default_report_path_differs_per_directory_and_kind() {
    let repo = Path::new("/repo");
    let root = Path::new(DEFAULT_LIBRARY_ROOT);
    let novel = default_report_path("epub_structure", &root.join("2_소설외국"), repo);
    assert_eq!(novel, Path::new("/repo/tmp/epub_structure_report_2_소설외국.json"));
    assert_ne!(novel, default_report_path("epub_structure", &root.join("2_소설한국"), repo));
    assert_eq!(default_report_path("mojibake", &root.join("2_소설외국"), repo).file_name().unwrap(), "mojibake_report_2_소설외국.json");
    assert_ne!(default_report_path("mojibake", &root.join("2_소설외국"), repo), novel.with_file_name("mojibake_report_x.json"));
}

#[test]
fn default_report_path_for_library_root_nested_and_outside() {
    let repo = Path::new("/repo");
    let root = Path::new(DEFAULT_LIBRARY_ROOT);
    let name = |p: &Path| default_report_path("mojibake", p, repo).file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name(root), "mojibake_report_all.json");
    assert_eq!(name(&root.join("2_소설외국/completed")), "mojibake_report_2_소설외국__completed.json");
    assert_eq!(name(Path::new("/tmp/x/my dir")), "mojibake_report_my_dir.json");
    assert_eq!(name(Path::new("/")), "mojibake_report_all.json");
}

#[test]
fn default_report_path_normalizes_dot_segments_and_trailing_slash() {
    let repo = Path::new("/repo");
    let p = Path::new("/mnt/data/text/2_소설외국/../2_소설한국/");
    assert_eq!(default_report_path("mojibake", p, repo).file_name().unwrap(), "mojibake_report_2_소설한국.json");
}

#[test]
fn path_helpers_follow_posixpath() {
    use corpus_common::util::{category_of, posix_dirname, posix_join, posix_normpath, unquote};
    assert_eq!(posix_normpath(""), ".");
    assert_eq!(posix_normpath("a/./b//c/../d"), "a/b/d");
    assert_eq!(posix_normpath("//a"), "//a");
    assert_eq!(posix_normpath("///a"), "/a");
    assert_eq!(posix_normpath("../a/.."), "..");
    assert_eq!(posix_join("a", "b"), "a/b");
    assert_eq!(posix_join("a/", "b"), "a/b");
    assert_eq!(posix_join("", "b"), "b");
    assert_eq!(posix_join("a", "/b"), "/b");
    assert_eq!(posix_dirname("a/b/c"), "a/b");
    assert_eq!(posix_dirname("c"), "");
    assert_eq!(posix_dirname("/c"), "/");
    assert_eq!(unquote("a%20b%zz%2"), "a b%zz%2");
    assert_eq!(unquote("%ED%95%9C"), "한");
    assert_eq!(category_of(Path::new("/mnt/data/text/3_판타지/a.txt")), "3_판타지");
    assert_eq!(category_of(Path::new("/tmp/x/a.txt")), "");
    assert_eq!(category_of(Path::new("/mnt/data/text")), "");
}
