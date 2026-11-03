//! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
//!
//! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
//! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

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

// ---------------------------------------------------------------------------
// 설정 파일 읽기
// ---------------------------------------------------------------------------

mod config_file {
    use classifier::config::{CONFIG_ENV, MAX_CONFIG_BYTES, default_config, enabled_fields, load_config, load_config_from_env, merge_config};
    use serde_json::json;

    #[test]
    fn unusable_config_files_fall_back_to_the_defaults() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(load_config(d.path()), default_config(), "디렉터리");
        let big = d.path().join("big.json");
        // 상한을 넘는 올바른 JSON 객체. 크기 때문에만 거부된다.
        let mut body = br#"{"pad": ""#.to_vec();
        body.extend(vec![b'x'; MAX_CONFIG_BYTES as usize]);
        body.extend(br#""}"#);
        std::fs::write(&big, body).unwrap();
        assert_eq!(load_config(&big), default_config(), "너무 큼");
        let array = d.path().join("array.json");
        std::fs::write(&array, "[1, 2]").unwrap();
        assert_eq!(load_config(&array), default_config(), "객체 아님");
    }

    #[test]
    fn the_environment_variable_points_at_the_config_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.json");
        std::fs::write(&p, r#"{"decision": {"temperature": 3}}"#).unwrap();
        // SAFETY: 이 테스트 바이너리에서 CONFIG_ENV를 쓰는 테스트는 이것뿐이다.
        unsafe { std::env::set_var(CONFIG_ENV, &p) };
        let cfg = load_config_from_env();
        unsafe { std::env::remove_var(CONFIG_ENV) };
        assert_eq!(cfg["decision"]["temperature"], 3);
    }

    #[test]
    fn merging_into_a_non_object_keeps_the_base_and_truthiness_follows_python() {
        assert_eq!(merge_config(&json!(1), &json!({"a": 1})), json!(1));
        let cfg = json!({"fields": {
            "null": {"enabled": null}, "zero": {"enabled": 0}, "one": {"enabled": 1},
            "empty": {"enabled": ""}, "text": {"enabled": "y"}, "list": {"enabled": [1]}, "nolist": {"enabled": []},
            "obj": {"enabled": {"a": 1}}, "noobj": {"enabled": {}},
        }});
        let on: Vec<String> = enabled_fields(&cfg).as_object().unwrap().keys().cloned().collect();
        assert_eq!(on, ["one", "text", "list", "obj"]);
    }
}
