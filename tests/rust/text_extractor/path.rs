//! text_extractor의 경로 도구와 한국어 디코딩 테스트 (옛 인코딩 검사 도구 테스트에서 옮겨 왔다).

#[test]
fn path_helpers_follow_posixpath() {
    use text_extractor::path::{posix_dirname, posix_join, posix_normpath, unquote};
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
}
