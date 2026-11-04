//! `classifier::inference_model` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

mod common;

// `tests/rust/classifier/classifier_model.rs`에서 옮긴 테스트.
mod classifier_model {
    //! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
    //!
    //! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
    //! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

    use std::fs;
    use std::path::{Path, PathBuf};

    use classifier::export_format::{ModelError, sha256_hex};
    use classifier::inference_model::{CategoryModel, CategoryModelApi};

    use serde_json::{Value, json};

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/classifier_tiny");

    #[test]
    fn rust_trained_model_classifies_clear_examples() {
        let model = CategoryModel::load_dir(&Path::new(FIXTURE).join("trained")).expect("Rust 학습 모델 로드");
        let cases = [
            ("3_무협", "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객"),
            ("3_판타지", "드래곤 마나 던전 길드 마법사 레벨업 소환 마왕"),
            ("5_수학과학일반", "미분 적분 함수 벡터 행렬 확률 통계 방정식"),
        ];

        for (expected, words) in cases {
            let tokens: Vec<_> = words.split_whitespace().collect();
            let text = tokens.repeat(3).join(" ");
            let title = format!("{} 이야기 0", tokens[0]);
            let name = format!("{}_0.epub", tokens[0]);
            let doc = json!({"cat": expected, "text": text, "title": title, "name": name, "type": "epub", "author": "작가0", "publisher": "테스트출판사"});
            let prediction = CategoryModelApi::predict_one(&model, &doc, None);
            assert_eq!(prediction.category.as_deref(), Some(expected), "입력 문서: {words}");
            assert!(prediction.confidence >= model.min_confidence(), "확신도 {} < 임계값 {}", prediction.confidence, model.min_confidence());
        }
    }

    #[test]
    fn min_confidence_priority_config_then_meta_then_one() {
        let dir = copy_fixture();
        let model = CategoryModel::load_dir(dir.path()).unwrap();
        // fixture: config.min_confidence=null, meta.calibrated_threshold=0.4
        assert_eq!(model.min_confidence(), 0.4);
        assert_eq!(model.temperature(), 0.5);
        edit_manifest(dir.path(), |m| m["config"]["decision"]["min_confidence"] = json!(0.9));
        assert_eq!(CategoryModel::load_dir(dir.path()).unwrap().min_confidence(), 0.9);
        edit_manifest(dir.path(), |m| {
            m["config"]["decision"]["min_confidence"] = Value::Null;
            m["meta"].as_object_mut().unwrap().remove("calibrated_threshold");
        });
        let model = CategoryModel::load_dir(dir.path()).unwrap();
        assert_eq!(model.min_confidence(), 1.0);
        // 임계값 1.0 이면 아무것도 판정하지 않는다. 호출 인자는 모델 값을 이긴다.
        let doc = json!({"text": "무림맹 장문인 내공 화산파"});
        let pred = CategoryModelApi::predict_one(&model, &doc, None);
        assert!(pred.category.is_none());
        assert!(pred.reason.contains("< 임계값 1.00 이라 판정하지 않음 (1위 후보 "));
        let forced = CategoryModelApi::predict_one(&model, &doc, Some(0.0));
        assert_eq!(forced.category.as_deref(), Some(pred.ranked[0].0.as_str()));
        assert!(forced.reason.starts_with("모델 확신도 "));
    }

    #[test]
    fn saving_model_omits_private_metadata_like_python_exporter() {
        let model = CategoryModel::load_dir(Path::new(FIXTURE)).unwrap();
        let mut model = model;
        model.meta["_private_cache"] = json!({"discard": true});
        model.meta["public_note"] = json!("keep");

        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("exported");
        model.save(Some(&output)).unwrap();
        let manifest: Value = serde_json::from_slice(&fs::read(output.join("model.json")).unwrap()).unwrap();

        assert!(manifest["meta"].get("_private_cache").is_none());
        assert_eq!(manifest["meta"]["public_note"], "keep");
        let author = manifest["fields"].as_array().unwrap().iter().find(|field| field["name"] == "author").unwrap();
        assert_eq!(author["weight"].as_f64(), Some(0.3));
    }

    #[test]
    fn predict_handles_top_k_and_empty_batch() {
        let model = CategoryModel::load_dir(Path::new(FIXTURE)).unwrap();
        assert!(model.predict_top_k(&[], None, 3).is_empty());
        let doc = json!({"text": "무림맹"});
        let one = &model.predict_top_k(std::slice::from_ref(&doc), Some(0.0), 1)[0];
        assert_eq!(one.ranked.len(), 1);
        assert!(!one.reason.contains("2위"));
        let all = &model.predict_top_k(std::slice::from_ref(&doc), Some(0.0), 10)[0];
        assert_eq!(all.ranked.len(), 3);
        assert!((all.ranked.iter().map(|r| r.1).sum::<f64>() - 1.0).abs() < 1e-9);
    }

    // ---------------------------------------------------------------------------
    // 로더 오류
    // ---------------------------------------------------------------------------

    fn copy_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for entry in fs::read_dir(FIXTURE).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
            }
        }
        dir
    }

    fn edit_manifest(dir: &Path, edit: impl FnOnce(&mut Value)) {
        let path: PathBuf = dir.join("model.json");
        let mut m: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        edit(&mut m);
        fs::write(path, serde_json::to_vec(&m).unwrap()).unwrap();
    }

    fn load_err(dir: &Path) -> ModelError {
        CategoryModel::load_dir(dir).expect_err("로드가 실패해야 한다")
    }

    #[test]
    fn loader_rejects_sha256_mismatch_in_every_binary_and_vocab_file() {
        for file in ["coef.f32", "idf.title.f32", "vocab.title.json"] {
            let dir = copy_fixture();
            let path = dir.path().join(file);
            let mut bytes = fs::read(&path).unwrap();
            // 길이는 그대로 두고 한 바이트만 바꾼다(어휘는 JSON 이 유지되도록 공백 뒤 글자).
            let i = if file.ends_with("json") { bytes.iter().position(|b| *b == b'"').unwrap() + 1 } else { 5 };
            bytes[i] = if bytes[i] == b'x' { b'y' } else { b'x' };
            fs::write(&path, bytes).unwrap();
            assert!(matches!(load_err(dir.path()), ModelError::Sha256Mismatch { file: ref f } if f == file), "{file}");
        }
    }

    #[test]
    fn loader_rejects_length_mismatch() {
        let dir = copy_fixture();
        let path = dir.path().join("coef.f32");
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() - 4]).unwrap();
        assert!(matches!(load_err(dir.path()), ModelError::LengthMismatch { .. }));
        let dir = copy_fixture();
        let path = dir.path().join("idf.author.f32");
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        fs::write(&path, bytes).unwrap();
        assert!(matches!(load_err(dir.path()), ModelError::LengthMismatch { .. }));
    }

    #[test]
    fn loader_rejects_column_total_mismatch() {
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["n_features"] = json!(m["n_features"].as_u64().unwrap() + 1));
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(_)));
        // 필드 size 를 바꾸면 offset 연쇄가 깨진다.
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["fields"][0]["size"] = json!(m["fields"][0]["size"].as_u64().unwrap() + 1));
        assert!(load_err(dir.path()).to_string().contains("offset") || matches!(load_err(dir.path()), ModelError::LengthMismatch { .. }));
    }

    #[test]
    fn loader_rejects_duplicate_vocab_terms() {
        let dir = copy_fixture();
        let path = dir.path().join("vocab.publisher.json");
        let mut vocab: Vec<String> = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        vocab[1] = vocab[0].clone();
        let bytes = serde_json::to_vec(&vocab).unwrap();
        fs::write(&path, &bytes).unwrap();
        edit_manifest(dir.path(), |m| {
            let f = m["fields"].as_array_mut().unwrap().iter_mut().find(|f| f["name"] == "publisher").unwrap();
            f["vocab_sha256"] = json!(sha256_hex(&bytes));
        });
        assert!(matches!(load_err(dir.path()), ModelError::DuplicateTerm { ref field } if field == "publisher"));
    }

    #[test]
    fn loader_rejects_vocab_count_not_matching_size() {
        let dir = copy_fixture();
        let path = dir.path().join("vocab.publisher.json");
        let mut vocab: Vec<String> = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        vocab.pop();
        let bytes = serde_json::to_vec(&vocab).unwrap();
        fs::write(&path, &bytes).unwrap();
        edit_manifest(dir.path(), |m| {
            let f = m["fields"].as_array_mut().unwrap().iter_mut().find(|f| f["name"] == "publisher").unwrap();
            f["vocab_sha256"] = json!(sha256_hex(&bytes));
        });
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(_)));
    }

    #[test]
    fn loader_rejects_unknown_format_version() {
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["format_version"] = json!(2));
        assert!(matches!(load_err(dir.path()), ModelError::UnsupportedVersion(2)));
    }

    #[test]
    fn loader_rejects_missing_files() {
        for file in ["model.json", "coef.f32", "idf.body_word.f32", "vocab.body_word.json"] {
            let dir = copy_fixture();
            fs::remove_file(dir.path().join(file)).unwrap();
            assert!(matches!(load_err(dir.path()), ModelError::Io { .. }), "{file}");
        }
        assert!(matches!(CategoryModel::load_dir(Path::new("/nonexistent/classifier-dir")), Err(ModelError::Io { .. })));
    }

    #[test]
    fn loader_rejects_malformed_manifest_shapes_and_non_regular_model_files() {
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["classes"] = json!([]));
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(message) if message.contains("classes")));

        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["intercept"] = json!([]));
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(message) if message.contains("intercept")));

        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["fields"] = json!([]));
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(message) if message.contains("fields")));

        let dir = copy_fixture();
        let too_many = u64::from(u32::MAX) + 1;
        edit_manifest(dir.path(), |m| {
            m["fields"].as_array_mut().unwrap().truncate(1);
            m["n_features"] = json!(too_many);
            m["fields"][0]["size"] = json!(too_many);
        });
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(message) if message.contains("u32")));

        let dir = copy_fixture();
        fs::remove_file(dir.path().join("coef.f32")).unwrap();
        fs::create_dir(dir.path().join("coef.f32")).unwrap();
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(message) if message.contains("일반 파일")));
    }

    #[test]
    fn loader_rejects_unsafe_file_names() {
        for bad in ["../coef.f32", "/etc/passwd", "sub/coef.f32", "..", ".hidden", "", "a\\b", "x\0y"] {
            let dir = copy_fixture();
            edit_manifest(dir.path(), |m| m["coef_file"] = json!(bad));
            assert!(matches!(load_err(dir.path()), ModelError::UnsafePath(_)), "{bad:?}");
            let dir = copy_fixture();
            edit_manifest(dir.path(), |m| m["fields"][0]["vocab_file"] = json!(bad));
            assert!(matches!(load_err(dir.path()), ModelError::UnsafePath(_)), "{bad:?}");
        }
    }

    #[test]
    fn loader_rejects_nonfinite_coefficients_and_idf() {
        for (file, offset) in [("coef.f32", 8usize), ("idf.body_word.f32", 0)] {
            let dir = copy_fixture();
            let path = dir.path().join(file);
            let mut bytes = fs::read(&path).unwrap();
            bytes[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
            fs::write(&path, &bytes).unwrap();
            // sha256 도 새 내용에 맞춰 두어야 NaN 검사에 닿는다.
            edit_manifest(dir.path(), |m| {
                if file == "coef.f32" {
                    m["coef_sha256"] = json!(sha256_hex(&bytes));
                } else {
                    m["fields"][0]["idf_sha256"] = json!(sha256_hex(&bytes));
                }
            });
            assert!(matches!(load_err(dir.path()), ModelError::NotFinite { .. }), "{file}");
        }
    }

    #[test]
    fn loader_rejects_unsupported_vectorizer_options() {
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["fields"][0]["analyzer"] = json!("regex"));
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(_)));
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["fields"][0]["token_pattern"] = json!(r"(?u)\b\w\w+\b"));
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(_)));
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["fields"][0]["norm"] = json!("l1"));
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(_)));
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| m["fields"][0]["ngram_range"] = json!([3, 2]));
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(_)));
    }

    #[test]
    fn loader_rejects_oversized_declared_sizes_without_allocating() {
        let dir = copy_fixture();
        edit_manifest(dir.path(), |m| {
            m["n_features"] = json!(u64::MAX);
            m["fields"][0]["size"] = json!(u64::MAX);
        });
        assert!(matches!(load_err(dir.path()), ModelError::Invalid(_)));
    }

    // ---------------------------------------------------------------------------
    // 모델 대역의 기본 구현, 학습 계수 검증, 접근자
    // ---------------------------------------------------------------------------

    struct Undecided;

    impl CategoryModelApi for Undecided {}

    #[test]
    fn the_default_model_api_never_decides() {
        let m = Undecided;
        assert_eq!(m.min_confidence(), 1.0);
        let p = m.predict(&[json!({}), json!({})], None);
        assert_eq!(p.len(), 2);
        assert!(p.iter().all(|p| p.category.is_none() && p.reason == "판정하지 않는 모델"));
        assert_eq!(m.expected_accuracy(0.9), None);
    }

    #[test]
    fn training_output_is_validated_and_exposed_through_accessors() {
        let classes = || vec!["소설".to_string()];
        assert!(CategoryModel::from_training(Vec::new(), json!({}), json!({}), Vec::new(), Vec::new(), Vec::new()).is_err());
        let nan = CategoryModel::from_training(classes(), json!({}), json!({}), Vec::new(), Vec::new(), vec![f64::NAN]).unwrap_err();
        assert!(nan.to_string().contains("NaN"), "{nan}");
        let model = CategoryModel::from_training(classes(), json!({"decision": {}}), json!({"v": 1}), Vec::new(), Vec::new(), vec![0.0]).unwrap();
        assert_eq!(model.classes(), ["소설"]);
        assert_eq!(model.n_features(), 0);
        assert_eq!(model.config()["decision"], json!({}));
        assert_eq!(model.meta()["v"], 1);
        let api: &dyn CategoryModelApi = &model;
        let p = api.predict(&[json!({"title": "제목"})], Some(0.0));
        assert_eq!(p[0].ranked[0].0, "소설");
    }
}

// `tests/rust/classifier/classifier_service.rs`에서 옮긴 테스트.
mod classifier_service {
    use std::path::Path;

    use classifier::{BookCategoryClassifier, ClassifierOptions};

    use tempfile::TempDir;

    fn options(root: &Path) -> ClassifierOptions {
        ClassifierOptions { library_root: Some(root.to_path_buf()), ..Default::default() }
    }

    #[test]
    fn test_min_confidence_falls_back_to_config_without_model() {
        let tmp = TempDir::new().unwrap();
        let clf = BookCategoryClassifier::new(None, options(tmp.path()));
        // 모델이 없으면 판정 자체를 안 하므로 어떤 값이든 실제로는 안 쓰인다.
        let default_floor: f64 = clf.min_confidence();
        assert!(default_floor.is_finite());

        let explicit = BookCategoryClassifier::new(None, ClassifierOptions { min_confidence: Some(0.42), ..options(tmp.path()) });
        assert!((explicit.min_confidence() - 0.42).abs() < 1e-9);
    }
}

// `tests/rust/classifier/classifier_training.rs`에서 옮긴 테스트.
mod classifier_training {
    use classifier::{
        classifier_config,
        inference_model::CategoryModel,
        model_trainer::{self},
    };
    use serde_json::{Value, json};

    #[test]
    fn confidence_calibration_uses_accept_parent_as_correct_and_requires_minimum_samples() {
        let model = CategoryModel::from_training(vec!["3_무협".into(), "5_과학".into()], classifier_config::default_config(), json!({}), Vec::new(), Vec::new(), vec![1.0, 0.0]).unwrap();
        let docs: Vec<Value> = (0..200).map(|_| json!({"cat": "3_무협/하위", "text": ""})).collect();
        let parents = std::collections::HashMap::from([("3_무협/하위".to_string(), "3_무협".to_string())]);

        let calibration = model_trainer::fit_confidence_calibration(&model, &docs, Some(&parents)).unwrap().unwrap();

        assert_eq!(calibration["n"], 200);
        assert_eq!(calibration["y"], json!([1.0]), "accept_parent에 속한 예측은 맞은 표본으로 계산한다");
        assert!(model_trainer::fit_confidence_calibration(&model, &docs[..199], Some(&parents)).unwrap().is_none());
    }
}

mod softmax {
    use classifier::inference_model::{fixed, rank_desc, softmax_confidence_row};

    #[test]
    fn softmax_and_ranking_follow_python() {
        let p = softmax_confidence_row(&[1.0, 2.0, 3.0], 1.0);
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!((p[2] - 0.665_240_955_774_642).abs() < 1e-12);
        // 온도 하한 1e-6, 오버플로 없이 계산한다.
        let sharp = softmax_confidence_row(&[0.0, 1.0], 0.0);
        assert!((sharp[1] - 1.0).abs() < 1e-12);
        // 동점이면 클래스 번호가 작은 쪽이 먼저다.
        assert_eq!(rank_desc(&[0.5, 0.9, 0.9, 0.1]), [1, 2, 0, 3]);
    }

    #[test]
    fn fixed_rounds_like_python_format() {
        // Python: f"{0.0625:.3f}"='0.062', f"{0.375:.2f}"='0.38', f"{0.125:.2f}"='0.12', f"{0.3345:.3f}"='0.335'
        assert_eq!(fixed(0.0625, 3), "0.062");
        assert_eq!(fixed(0.375, 2), "0.38");
        assert_eq!(fixed(0.125, 2), "0.12");
        assert_eq!(fixed(0.3345, 3), "0.335");
        assert_eq!(fixed(1.0, 2), "1.00");
    }
}
