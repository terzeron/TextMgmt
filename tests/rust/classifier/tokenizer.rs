//! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
//!
//! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
//! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

mod common;

use std::fs;
use std::path::PathBuf;

use classifier::export_format::{ModelError, write_dir};
use classifier::tokenizer::{AnalyzerSpec, analyze, collapse_whitespace, is_py_digit, is_py_space, is_py_word};
use classifier::vectorizer::FieldVectorizer;
use serde_json::{Value, json};
use crate::common::shared_fixtures::{spec};

fn terms(kind: &str, lo: usize, hi: usize, text: &str) -> Vec<String> {
    analyze(text, &spec(kind, lo, hi))
}

// ---------------------------------------------------------------------------
// 분석기 단위 (손으로 쓴 사례)
// ---------------------------------------------------------------------------

#[test]
fn char_wb_pads_each_word_and_keeps_boundaries() {
    // sklearn: char_wb (2,4) 에서 "a" → [' a', 'a ', ' a ']
    assert_eq!(terms("char_wb", 2, 4, "a"), [" a", "a ", " a "]);
    // 단어가 n 보다 짧으면 한 번만 센다.
    assert_eq!(terms("char_wb", 2, 4, "ab"), [" a", "ab", "b ", " ab", "ab ", " ab "]);
    // 단어 경계를 넘는 n-gram 은 없다.
    assert_eq!(terms("char_wb", 2, 2, "ab cd"), [" a", "ab", "b ", " c", "cd", "d "]);
    assert!(terms("char_wb", 2, 4, "").is_empty());
    assert!(terms("char_wb", 2, 4, " \t\n ").is_empty());
}

#[test]
fn char_collapses_whitespace_runs_but_keeps_single_whitespace() {
    assert_eq!(terms("char", 2, 2, "a  b"), ["a ", " b"]);
    assert_eq!(terms("char", 1, 1, "a\nb"), ["a", "\n", "b"]);
    assert_eq!(terms("char", 1, 2, "a\n\tb"), ["a", " ", "b", "a ", " b"]);
    assert!(terms("char", 2, 4, "x").is_empty());
}

#[test]
fn word_allows_single_char_tokens_and_unicode_boundaries() {
    assert_eq!(terms("word", 1, 1, "a b  c"), ["a", "b", "c"]);
    assert_eq!(terms("word", 1, 1, "Hello, World! it's"), ["hello", "world", "it", "s"]);
    // 밑줄과 숫자는 \w 다.
    assert_eq!(terms("word", 1, 1, "snake_case x_1 3.14"), ["snake_case", "x_1", "3", "14"]);
    // 한글·한자·일본어는 이어진 덩어리 하나다.
    assert_eq!(terms("word", 1, 1, "漢字 한글 日本語テスト"), ["漢字", "한글", "日本語テスト"]);
    assert!(terms("word", 1, 1, "").is_empty());
    assert_eq!(terms("word", 1, 2, "a b c"), ["a", "b", "c", "a b", "b c"]);
}

#[test]
fn word_nfd_jamo_stay_one_token_but_combining_marks_split() {
    // 한글 자모(L 범주)는 \w 이므로 NFD 도 한 토큰이다.
    let nfd = "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}";
    assert_eq!(terms("word", 1, 1, nfd), [nfd]);
    // 결합 악센트(Mn)는 Python \w 가 아니라서 토큰을 가른다(Rust is_alphanumeric 과 다른 지점).
    assert_eq!(terms("word", 1, 1, "e\u{301}x"), ["e", "x"]);
    assert!(!is_py_word('\u{301}'));
    assert!(is_py_word('\u{b2}')); // ² 는 isdigit
    assert!(is_py_word('_') && !is_py_word('-'));
}

#[test]
fn python_whitespace_and_digit_sets() {
    for c in ['\t', '\n', '\x0b', '\x0c', '\r', '\x1c', '\x1f', ' ', '\u{85}', '\u{a0}', '\u{3000}', '\u{2003}'] {
        assert!(is_py_space(c), "{c:?}");
    }
    assert!(!is_py_space('\u{200b}') && !is_py_space('a') && !is_py_space('\u{180e}'));
    assert!(is_py_digit('7') && is_py_digit('\u{663}') && !is_py_digit('\u{b2}') && !is_py_digit('a'));
    assert_eq!(collapse_whitespace("a  b\tc \n d"), "a b\tc d");
}

#[test]
fn lowercase_follows_python_lower() {
    // İ 는 "i" + U+0307(결합 점)으로 소문자가 되고, 결합 점은 \w 가 아니라 토큰을 가른다. ΣΑΣ 는 끝 시그마(ς)가 된다.
    assert_eq!(terms("word", 1, 1, "ÀB İx ΣΑΣ"), ["àb", "i", "x", "σας"]);
}

#[test]
fn analyzer_keeps_case_when_lowercase_is_disabled() {
    let case_sensitive = AnalyzerSpec { lowercase: false, ..spec("word", 1, 1) };
    assert_eq!(analyze("ABC", &case_sensitive), ["ABC"]);
}

#[test]
fn sklearn_analyzer_table() {
    for (kind, lo, hi, text, expected) in TOKEN_CASES {
        assert_eq!(&terms(kind, *lo, *hi, text), expected, "{kind} ({lo},{hi}) {text:?}");
    }
}

#[test]
fn exporter_rejects_noncontiguous_field_offsets_instead_of_writing_unloadable_model() {
    let classes = vec!["A".to_string()];
    let fields = [
        FieldVectorizer::new("title", "title", 1.0, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap(),
        FieldVectorizer::new("body", "text", 1.0, 0, spec("word", 1, 1), true, 2, vec!["b".into()], vec![1.0]).unwrap(),
    ];
    let dir = tempfile::tempdir().unwrap();

    let result = write_dir(&dir.path().join("export"), &classes, &fields, &[1.0, 1.0], &[0.0], &json!({}), &json!({}));

    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("offset")));
}

#[test]
fn exporter_rejects_nonfinite_weights_idf_coefficients_and_intercepts_like_python() {
    let classes = vec!["A".to_string()];
    for (file, weight, idf, coef, intercept) in
        [("model.json", f32::NAN, 1.0, 1.0, 0.0), ("idf.title.f32", 1.0, f32::NAN, 1.0, 0.0), ("coef.f32", 1.0, 1.0, f32::INFINITY, 0.0), ("model.json", 1.0, 1.0, 1.0, f64::INFINITY)]
    {
        let field = FieldVectorizer::new("title", "title", weight, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![idf]).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = write_dir(&dir.path().join("export"), &classes, &[field], &[coef], &[intercept], &json!({}), &json!({}));
        assert!(matches!(result, Err(ModelError::NotFinite { file: ref found }) if found == file), "{file}");
    }
}

#[test]
fn analyzers_match_sklearn_on_random_strings() {
    let path = std::env::var_os("TM_CLASSIFIER_FUZZ_FILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/classifier_analyzer_fuzz.json")));
    let cases: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let mut bad = 0;
    let mut total = 0;
    for case in cases.as_array().unwrap() {
        let (kind, lo, hi) = (case["analyzer"].as_str().unwrap(), case["lo"].as_u64().unwrap() as usize, case["hi"].as_u64().unwrap() as usize);
        let text = case["text"].as_str().unwrap();
        let expected: Vec<String> = case["terms"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect();
        total += 1;
        let got = terms(kind, lo, hi, text);
        if got != expected {
            bad += 1;
            if bad <= 5 {
                let at = got.iter().zip(&expected).position(|(a, b)| a != b);
                eprintln!(
                    "분석기 불일치 {kind} ({lo},{hi}) 코드포인트 {:?}: rust {} 항, python {} 항, 첫 차이 {at:?}",
                    text.chars().map(|c| c as u32).take(24).collect::<Vec<_>>(),
                    got.len(),
                    expected.len()
                );
            }
        }
    }
    assert_eq!(bad, 0, "{bad}/{total} 불일치");
}

// 생성: sklearn TfidfVectorizer(analyzer=..., ngram_range=..., lowercase=True, token_pattern=r"(?u)\b\w+\b").build_analyzer()(text) 결과
pub const TOKEN_CASES: &[(&str, usize, usize, &str, &[&str])] = &[
    ("word", 1, 1, "a b  c", &["a", "b", "c"]),
    ("word", 1, 2, "a b  c", &["a", "b", "c", "a b", "b c"]),
    ("word", 2, 3, "a b  c", &["a b", "b c", "a b c"]),
    ("word", 1, 1, "Hello, World! it's 3.14", &["hello", "world", "it", "s", "3", "14"]),
    ("word", 1, 2, "Hello, World! it's 3.14", &["hello", "world", "it", "s", "3", "14", "hello world", "world it", "it s", "s 3", "3 14"]),
    ("word", 2, 3, "Hello, World! it's 3.14", &["hello world", "world it", "it s", "s 3", "3 14", "hello world it", "world it s", "it s 3", "s 3 14"]),
    ("word", 1, 1, "snake_case x_1 \u{663}\u{664}", &["snake_case", "x_1", "\u{663}\u{664}"]),
    ("word", 1, 2, "snake_case x_1 \u{663}\u{664}", &["snake_case", "x_1", "\u{663}\u{664}", "snake_case x_1", "x_1 \u{663}\u{664}"]),
    ("word", 2, 3, "snake_case x_1 \u{663}\u{664}", &["snake_case x_1", "x_1 \u{663}\u{664}", "snake_case x_1 \u{663}\u{664}"]),
    ("word", 1, 1, "漢字 한글 日本語テスト", &["漢字", "한글", "日本語テスト"]),
    ("word", 1, 2, "漢字 한글 日本語テスト", &["漢字", "한글", "日本語テスト", "漢字 한글", "한글 日本語テスト"]),
    ("word", 2, 3, "漢字 한글 日本語テスト", &["漢字 한글", "한글 日本語テスト", "漢字 한글 日本語テスト"]),
    ("word", 1, 1, "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc", &["한글", "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", "abc"]),
    (
        "word",
        1,
        2,
        "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc",
        &["한글", "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", "abc", "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc"],
    ),
    (
        "word",
        2,
        3,
        "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc",
        &["한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc", "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc"],
    ),
    ("word", 1, 1, "", &[]),
    ("word", 1, 2, "", &[]),
    ("word", 2, 3, "", &[]),
    ("word", 1, 1, "   ", &[]),
    ("word", 1, 2, "   ", &[]),
    ("word", 2, 3, "   ", &[]),
    ("word", 1, 1, "\u{c0}B \u{130}x \u{3a3}\u{391}\u{3a3}", &["\u{e0}b", "i", "x", "\u{3c3}\u{3b1}\u{3c2}"]),
    ("word", 1, 2, "\u{c0}B \u{130}x \u{3a3}\u{391}\u{3a3}", &["\u{e0}b", "i", "x", "\u{3c3}\u{3b1}\u{3c2}", "\u{e0}b i", "i x", "x \u{3c3}\u{3b1}\u{3c2}"]),
    ("word", 2, 3, "\u{c0}B \u{130}x \u{3a3}\u{391}\u{3a3}", &["\u{e0}b i", "i x", "x \u{3c3}\u{3b1}\u{3c2}", "\u{e0}b i x", "i x \u{3c3}\u{3b1}\u{3c2}"]),
    ("word", 1, 1, "e\u{301} caf\u{e9} x\u{b2}y \u{2167} \u{663}", &["e", "caf\u{e9}", "x\u{b2}y", "\u{2177}", "\u{663}"]),
    (
        "word",
        1,
        2,
        "e\u{301} caf\u{e9} x\u{b2}y \u{2167} \u{663}",
        &["e", "caf\u{e9}", "x\u{b2}y", "\u{2177}", "\u{663}", "e caf\u{e9}", "caf\u{e9} x\u{b2}y", "x\u{b2}y \u{2177}", "\u{2177} \u{663}"],
    ),
    (
        "word",
        2,
        3,
        "e\u{301} caf\u{e9} x\u{b2}y \u{2167} \u{663}",
        &["e caf\u{e9}", "caf\u{e9} x\u{b2}y", "x\u{b2}y \u{2177}", "\u{2177} \u{663}", "e caf\u{e9} x\u{b2}y", "caf\u{e9} x\u{b2}y \u{2177}", "x\u{b2}y \u{2177} \u{663}"],
    ),
    ("word", 1, 1, "\u{1f600}a\u{1f600} b", &["a", "b"]),
    ("word", 1, 2, "\u{1f600}a\u{1f600} b", &["a", "b", "a b"]),
    ("word", 2, 3, "\u{1f600}a\u{1f600} b", &["a b"]),
    ("word", 1, 1, "a\u{a0}b\u{3000}c\u{1f}d", &["a", "b", "c", "d"]),
    ("word", 1, 2, "a\u{a0}b\u{3000}c\u{1f}d", &["a", "b", "c", "d", "a b", "b c", "c d"]),
    ("word", 2, 3, "a\u{a0}b\u{3000}c\u{1f}d", &["a b", "b c", "c d", "a b c", "b c d"]),
    ("word", 1, 1, "한 두 세 네 다 여", &["한", "두", "세", "네", "다", "여"]),
    ("word", 1, 2, "한 두 세 네 다 여", &["한", "두", "세", "네", "다", "여", "한 두", "두 세", "세 네", "네 다", "다 여"]),
    ("word", 2, 3, "한 두 세 네 다 여", &["한 두", "두 세", "세 네", "네 다", "다 여", "한 두 세", "두 세 네", "세 네 다", "네 다 여"]),
    ("char_wb", 2, 4, "ab", &[" a", "ab", "b ", " ab", "ab ", " ab "]),
    ("char_wb", 1, 3, "ab", &[" ", "a", "b", " ", " a", "ab", "b ", " ab", "ab "]),
    ("char", 1, 3, "ab", &["a", "b", "ab"]),
    ("char", 2, 4, "ab", &["ab"]),
    ("char_wb", 2, 4, "a b", &[" a", "a ", " a ", " b", "b ", " b "]),
    ("char_wb", 1, 3, "a b", &[" ", "a", " ", " a", "a ", " a ", " ", "b", " ", " b", "b ", " b "]),
    ("char", 1, 3, "a b", &["a", " ", "b", "a ", " b", "a b"]),
    ("char", 2, 4, "a b", &["a ", " b", "a b"]),
    ("char_wb", 2, 4, "가나다 라", &[" 가", "가나", "나다", "다 ", " 가나", "가나다", "나다 ", " 가나다", "가나다 ", " 라", "라 ", " 라 "]),
    ("char_wb", 1, 3, "가나다 라", &[" ", "가", "나", "다", " ", " 가", "가나", "나다", "다 ", " 가나", "가나다", "나다 ", " ", "라", " ", " 라", "라 ", " 라 "]),
    ("char", 1, 3, "가나다 라", &["가", "나", "다", " ", "라", "가나", "나다", "다 ", " 라", "가나다", "나다 ", "다 라"]),
    ("char", 2, 4, "가나다 라", &["가나", "나다", "다 ", " 라", "가나다", "나다 ", "다 라", "가나다 ", "나다 라"]),
    ("char_wb", 2, 4, "x", &[" x", "x ", " x "]),
    ("char_wb", 1, 3, "x", &[" ", "x", " ", " x", "x ", " x "]),
    ("char", 1, 3, "x", &["x"]),
    ("char", 2, 4, "x", &[]),
    ("char_wb", 2, 4, "", &[]),
    ("char_wb", 1, 3, "", &[]),
    ("char", 1, 3, "", &[]),
    ("char", 2, 4, "", &[]),
    ("char_wb", 2, 4, "  \u{9} ", &[]),
    ("char_wb", 1, 3, "  \u{9} ", &[]),
    ("char", 1, 3, "  \u{9} ", &[" "]),
    ("char", 2, 4, "  \u{9} ", &[]),
    ("char_wb", 2, 4, "a\u{a} b\u{a}\u{a}c", &[" a", "a ", " a ", " b", "b ", " b ", " c", "c ", " c "]),
    ("char_wb", 1, 3, "a\u{a} b\u{a}\u{a}c", &[" ", "a", " ", " a", "a ", " a ", " ", "b", " ", " b", "b ", " b ", " ", "c", " ", " c", "c ", " c "]),
    ("char", 1, 3, "a\u{a} b\u{a}\u{a}c", &["a", " ", "b", " ", "c", "a ", " b", "b ", " c", "a b", " b ", "b c"]),
    ("char", 2, 4, "a\u{a} b\u{a}\u{a}c", &["a ", " b", "b ", " c", "a b", " b ", "b c", "a b ", " b c"]),
    ("char_wb", 2, 4, "\u{1f600} 한", &[" \u{1f600}", "\u{1f600} ", " \u{1f600} ", " 한", "한 ", " 한 "]),
    ("char_wb", 1, 3, "\u{1f600} 한", &[" ", "\u{1f600}", " ", " \u{1f600}", "\u{1f600} ", " \u{1f600} ", " ", "한", " ", " 한", "한 ", " 한 "]),
    ("char", 1, 3, "\u{1f600} 한", &["\u{1f600}", " ", "한", "\u{1f600} ", " 한", "\u{1f600} 한"]),
    ("char", 2, 4, "\u{1f600} 한", &["\u{1f600} ", " 한", "\u{1f600} 한"]),
    (
        "char_wb",
        2,
        4,
        "한 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}",
        &[
            " 한",
            "한 ",
            " 한 ",
            " \u{1112}",
            "\u{1112}\u{1161}",
            "\u{1161}\u{11ab}",
            "\u{11ab}\u{1100}",
            "\u{1100}\u{1173}",
            "\u{1173}\u{11af}",
            "\u{11af} ",
            " \u{1112}\u{1161}",
            "\u{1112}\u{1161}\u{11ab}",
            "\u{1161}\u{11ab}\u{1100}",
            "\u{11ab}\u{1100}\u{1173}",
            "\u{1100}\u{1173}\u{11af}",
            "\u{1173}\u{11af} ",
            " \u{1112}\u{1161}\u{11ab}",
            "\u{1112}\u{1161}\u{11ab}\u{1100}",
            "\u{1161}\u{11ab}\u{1100}\u{1173}",
            "\u{11ab}\u{1100}\u{1173}\u{11af}",
            "\u{1100}\u{1173}\u{11af} ",
        ],
    ),
    (
        "char_wb",
        1,
        3,
        "한 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}",
        &[
            " ",
            "한",
            " ",
            " 한",
            "한 ",
            " 한 ",
            " ",
            "\u{1112}",
            "\u{1161}",
            "\u{11ab}",
            "\u{1100}",
            "\u{1173}",
            "\u{11af}",
            " ",
            " \u{1112}",
            "\u{1112}\u{1161}",
            "\u{1161}\u{11ab}",
            "\u{11ab}\u{1100}",
            "\u{1100}\u{1173}",
            "\u{1173}\u{11af}",
            "\u{11af} ",
            " \u{1112}\u{1161}",
            "\u{1112}\u{1161}\u{11ab}",
            "\u{1161}\u{11ab}\u{1100}",
            "\u{11ab}\u{1100}\u{1173}",
            "\u{1100}\u{1173}\u{11af}",
            "\u{1173}\u{11af} ",
        ],
    ),
    (
        "char",
        1,
        3,
        "한 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}",
        &[
            "한",
            " ",
            "\u{1112}",
            "\u{1161}",
            "\u{11ab}",
            "\u{1100}",
            "\u{1173}",
            "\u{11af}",
            "한 ",
            " \u{1112}",
            "\u{1112}\u{1161}",
            "\u{1161}\u{11ab}",
            "\u{11ab}\u{1100}",
            "\u{1100}\u{1173}",
            "\u{1173}\u{11af}",
            "한 \u{1112}",
            " \u{1112}\u{1161}",
            "\u{1112}\u{1161}\u{11ab}",
            "\u{1161}\u{11ab}\u{1100}",
            "\u{11ab}\u{1100}\u{1173}",
            "\u{1100}\u{1173}\u{11af}",
        ],
    ),
    (
        "char",
        2,
        4,
        "한 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}",
        &[
            "한 ",
            " \u{1112}",
            "\u{1112}\u{1161}",
            "\u{1161}\u{11ab}",
            "\u{11ab}\u{1100}",
            "\u{1100}\u{1173}",
            "\u{1173}\u{11af}",
            "한 \u{1112}",
            " \u{1112}\u{1161}",
            "\u{1112}\u{1161}\u{11ab}",
            "\u{1161}\u{11ab}\u{1100}",
            "\u{11ab}\u{1100}\u{1173}",
            "\u{1100}\u{1173}\u{11af}",
            "한 \u{1112}\u{1161}",
            " \u{1112}\u{1161}\u{11ab}",
            "\u{1112}\u{1161}\u{11ab}\u{1100}",
            "\u{1161}\u{11ab}\u{1100}\u{1173}",
            "\u{11ab}\u{1100}\u{1173}\u{11af}",
        ],
    ),
    (
        "char_wb",
        2,
        4,
        " lead trail ",
        &[" l", "le", "ea", "ad", "d ", " le", "lea", "ead", "ad ", " lea", "lead", "ead ", " t", "tr", "ra", "ai", "il", "l ", " tr", "tra", "rai", "ail", "il ", " tra", "trai", "rail", "ail "],
    ),
    (
        "char_wb",
        1,
        3,
        " lead trail ",
        &[
            " ", "l", "e", "a", "d", " ", " l", "le", "ea", "ad", "d ", " le", "lea", "ead", "ad ", " ", "t", "r", "a", "i", "l", " ", " t", "tr", "ra", "ai", "il", "l ", " tr", "tra", "rai", "ail",
            "il ",
        ],
    ),
    (
        "char",
        1,
        3,
        " lead trail ",
        &[
            " ", "l", "e", "a", "d", " ", "t", "r", "a", "i", "l", " ", " l", "le", "ea", "ad", "d ", " t", "tr", "ra", "ai", "il", "l ", " le", "lea", "ead", "ad ", "d t", " tr", "tra", "rai",
            "ail", "il ",
        ],
    ),
    (
        "char",
        2,
        4,
        " lead trail ",
        &[
            " l", "le", "ea", "ad", "d ", " t", "tr", "ra", "ai", "il", "l ", " le", "lea", "ead", "ad ", "d t", " tr", "tra", "rai", "ail", "il ", " lea", "lead", "ead ", "ad t", "d tr", " tra",
            "trai", "rail", "ail ",
        ],
    ),
    ("char_wb", 2, 4, "A\u{9}B", &[" a", "a ", " a ", " b", "b ", " b "]),
    ("char_wb", 1, 3, "A\u{9}B", &[" ", "a", " ", " a", "a ", " a ", " ", "b", " ", " b", "b ", " b "]),
    ("char", 1, 3, "A\u{9}B", &["a", "\u{9}", "b", "a\u{9}", "\u{9}b", "a\u{9}b"]),
    ("char", 2, 4, "A\u{9}B", &["a\u{9}", "\u{9}b", "a\u{9}b"]),
    ("char_wb", 2, 4, "ab  cd e", &[" a", "ab", "b ", " ab", "ab ", " ab ", " c", "cd", "d ", " cd", "cd ", " cd ", " e", "e ", " e "]),
    ("char_wb", 1, 3, "ab  cd e", &[" ", "a", "b", " ", " a", "ab", "b ", " ab", "ab ", " ", "c", "d", " ", " c", "cd", "d ", " cd", "cd ", " ", "e", " ", " e", "e ", " e "]),
    ("char", 1, 3, "ab  cd e", &["a", "b", " ", "c", "d", " ", "e", "ab", "b ", " c", "cd", "d ", " e", "ab ", "b c", " cd", "cd ", "d e"]),
    ("char", 2, 4, "ab  cd e", &["ab", "b ", " c", "cd", "d ", " e", "ab ", "b c", " cd", "cd ", "d e", "ab c", "b cd", " cd ", "cd e"]),
    ("char_wb", 2, 4, "日本 語", &[" 日", "日本", "本 ", " 日本", "日本 ", " 日本 ", " 語", "語 ", " 語 "]),
    ("char_wb", 1, 3, "日本 語", &[" ", "日", "本", " ", " 日", "日本", "本 ", " 日本", "日本 ", " ", "語", " ", " 語", "語 ", " 語 "]),
    ("char", 1, 3, "日本 語", &["日", "本", " ", "語", "日本", "本 ", " 語", "日本 ", "本 語"]),
    ("char", 2, 4, "日本 語", &["日本", "本 ", " 語", "日本 ", "本 語", "日本 語"]),
];
