//! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
//!
//! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
//! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

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
