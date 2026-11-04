use doc_reader::py_str_compat::{collapse_spaces, head_chars, is_space, keep_ascii_hangul, keep_word_chars, universal_newlines};

#[test]
fn python_string_helpers() {
    assert!(is_space('\u{1c}') && is_space('\u{a0}') && is_space('\u{3000}') && !is_space('a'));
    assert_eq!(head_chars("가나다라", 2), "가나");
    assert_eq!(head_chars("ab", 5), "ab");
    assert_eq!(collapse_spaces("  a \t\n b\u{a0}c  "), "a b c");
    assert_eq!(keep_ascii_hangul("a,b가_나\u{3131}é"), "a b가 나\u{3131} ");
    assert_eq!(keep_word_chars("a,b가_나é!"), "a b가_나é ");
    assert_eq!(universal_newlines("a\r\nb\rc\nd"), "a\nb\nc\nd");
}
