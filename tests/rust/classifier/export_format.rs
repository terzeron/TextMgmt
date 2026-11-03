//! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
//!
//! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
//! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

use std::fs;

use classifier::export_format::{ModelError, write_dir};
use classifier::tokenize::{AnalyzerKind, AnalyzerSpec};
use classifier::vectorize::FieldVectorizer;
use serde_json::{Value, json};

fn spec(kind: &str, lo: usize, hi: usize) -> AnalyzerSpec {
    AnalyzerSpec { kind: AnalyzerKind::parse(kind).unwrap(), ngram_lo: lo, ngram_hi: hi, lowercase: true }
}

#[test]
fn exporter_rejects_models_without_vectorizers_like_python() {
    let dir = tempfile::tempdir().unwrap();
    let result = write_dir(&dir.path().join("export"), &[], &[], &[], &[], &json!({}), &json!({}));
    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("벡터라이저")));
}

#[test]
fn exporter_rejects_coefficient_shape_and_unsafe_field_names() {
    let classes = vec!["A".to_string()];
    let dir = tempfile::tempdir().unwrap();
    let field = FieldVectorizer::new("title", "title", 1.0, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap();
    let result = write_dir(&dir.path().join("bad-shape"), &classes, &[field], &[], &[0.0], &json!({}), &json!({}));
    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("계수 개수")));

    let field = FieldVectorizer::new("title-name", "title", 1.0, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap();
    let result = write_dir(&dir.path().join("bad-name"), &classes, &[field], &[1.0], &[0.0], &json!({}), &json!({}));
    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("파일 이름")));
}

#[test]
fn model_errors_render_all_variants_and_expose_only_io_sources() {
    let errors = [
        ModelError::Json { file: "model.json".into(), message: "bad json".into() },
        ModelError::UnsupportedVersion(2),
        ModelError::UnsafePath("../secret".into()),
        ModelError::LengthMismatch { file: "coef.f32".into(), expected: 8, actual: 4 },
        ModelError::Sha256Mismatch { file: "coef.f32".into() },
        ModelError::NotFinite { file: "coef.f32".into() },
        ModelError::DuplicateTerm { field: "title".into() },
    ];
    for error in errors {
        assert!(!error.to_string().is_empty());
        assert!(std::error::Error::source(&error).is_none());
    }

    let io_error = ModelError::Io { file: "model.json".into(), source: std::io::Error::other("disk error") };
    assert!(io_error.to_string().contains("disk error"));
    assert!(std::error::Error::source(&io_error).is_some());
    assert!(ModelError::Invalid("bad shape".into()).to_string().contains("bad shape"));
}

#[test]
fn exporter_defaults_a_missing_field_weight_and_rejects_non_numeric_weight() {
    let classes = vec!["A".to_string()];
    let field = FieldVectorizer::new("title", "title", 0.25, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("export-default-weight");
    write_dir(&output, &classes, &[field], &[1.0], &[0.0], &json!({"fields":{"title":{}}}), &json!({})).unwrap();
    let manifest: Value = serde_json::from_slice(&fs::read(output.join("model.json")).unwrap()).unwrap();
    assert_eq!(manifest["fields"][0]["weight"], json!(1.0));

    let output = dir.path().join("export-invalid-weight");
    let field = FieldVectorizer::new("title", "title", 0.25, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap();
    let result = write_dir(&output, &classes, &[field], &[1.0], &[0.0], &json!({"fields":{"title":{"weight":"heavy"}}}), &json!({}));
    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("weight")));
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
