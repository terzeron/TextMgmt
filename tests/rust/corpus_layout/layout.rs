use corpus_layout::{IGNORED_DIR_NAMES, is_ignored_segment};

#[test]
fn ignored_dir_names_match_python_original() {
    let mut got: Vec<&str> = IGNORED_DIR_NAMES.to_vec();
    got.sort_unstable();
    assert_eq!(got, ["META-INF", "OEBPS", "page_images", "source_chapters", "tesseract_text"]);
}

#[test]
fn pipeline_dirs_are_ignored() {
    for name in IGNORED_DIR_NAMES {
        assert!(is_ignored_segment(name), "{name}");
    }
}

#[test]
fn dot_prefixed_segments_are_ignored() {
    assert!(is_ignored_segment(".git"));
    assert!(is_ignored_segment("."));
    assert!(is_ignored_segment(".hidden"));
}

#[test]
fn ordinary_category_names_are_kept() {
    assert!(!is_ignored_segment("4_인문일반논픽션"));
    assert!(!is_ignored_segment("동양고전"));
    assert!(!is_ignored_segment(""));
}

#[test]
fn match_is_exact_and_case_sensitive() {
    assert!(!is_ignored_segment("oebps"));
    assert!(!is_ignored_segment("OEBPS2"));
    assert!(!is_ignored_segment("page_images_old"));
}
