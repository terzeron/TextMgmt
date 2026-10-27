use super::*;

#[test]
fn bounded_reader_rejects_directories_and_files_over_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();
    assert!(matches!(read_bounded(dir.path(), "subdir", None, 8), Err(ModelError::Invalid(_))));

    std::fs::write(dir.path().join("large.bin"), b"four").unwrap();
    assert!(matches!(read_bounded(dir.path(), "large.bin", None, 3), Err(ModelError::LengthMismatch { expected: 3, actual: 4, .. })));
}

#[cfg(target_os = "linux")]
#[test]
fn bounded_reader_detects_a_file_whose_reported_length_is_shorter_than_its_contents() {
    let contents = read_bounded(Path::new("/proc/self"), "cmdline", None, 1024).unwrap_err();
    assert!(matches!(contents, ModelError::LengthMismatch { expected: 0, actual, .. } if actual > 0));
}

#[test]
fn export_feature_count_and_manifest_weight_guards_accept_limits_and_reject_invalid_values() {
    assert!(check_export_feature_count(u32::MAX as usize).is_ok());
    assert!(matches!(check_export_feature_count(u32::MAX as usize + 1), Err(ModelError::Invalid(message)) if message.contains("u32")));
    assert!(check_finite_manifest_weight(0.5).is_ok());
    assert!(matches!(check_finite_manifest_weight(f64::INFINITY), Err(ModelError::NotFinite { file }) if file == "model.json"));
}

#[test]
fn trailing_data_check_distinguishes_eof_extra_bytes_and_read_errors() {
    let mut empty = std::io::Cursor::new(Vec::<u8>::new());
    assert!(verify_no_trailing_data(&mut empty, "coef.f32", 4).is_ok());

    let mut extra = std::io::Cursor::new(vec![9]);
    assert!(matches!(verify_no_trailing_data(&mut extra, "coef.f32", 4), Err(ModelError::LengthMismatch { expected: 4, actual: 5, .. })));

    struct BrokenReader;
    impl std::io::Read for BrokenReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("read failed"))
        }
    }
    assert!(matches!(verify_no_trailing_data(&mut BrokenReader, "coef.f32", 4), Err(ModelError::Io { file, .. }) if file == "coef.f32"));
}

#[test]
fn manifest_validation_rejects_non_finite_field_weights() {
    let mut manifest: Manifest = serde_json::from_value(serde_json::json!({
        "format_version": 1,
        "classes": ["A"],
        "config": {},
        "meta": {},
        "n_features": 1,
        "n_classes": 1,
        "fields": [{
            "name": "title", "source": "title", "weight": 1.0, "max_chars": 0,
            "analyzer": "word", "ngram_range": [1, 1], "lowercase": true,
            "sublinear_tf": true, "norm": "l2", "token_pattern": "(?u)\\b\\w+\\b",
            "offset": 0, "size": 1, "vocab_file": "vocab.json", "idf_file": "idf.f32",
            "idf_sha256": "0", "vocab_sha256": "0"
        }],
        "coef_file": "coef.f32", "coef_sha256": "0", "intercept": [0.0]
    }))
    .unwrap();
    manifest.fields[0].weight = f64::INFINITY;

    assert!(matches!(validate_manifest(&manifest), Err(ModelError::Invalid(message)) if message.contains("weight")));
}

#[test]
fn exporter_filters_private_metadata_from_the_written_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("model");
    let spec = super::super::tokenize::AnalyzerSpec { kind: super::super::tokenize::AnalyzerKind::Word, ngram_lo: 1, ngram_hi: 1, lowercase: true };
    let field = FieldVectorizer::new("title", "title", 1.0, 0, spec, true, 0, vec!["alpha".into()], vec![1.0]).unwrap();
    write_dir(&output, &["A".into()], &[field], &[1.0], &[0.0], &serde_json::json!({}), &serde_json::json!({"_private":true,"public":true})).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(output.join("model.json")).unwrap()).unwrap();

    assert!(manifest["meta"].get("_private").is_none());
    assert_eq!(manifest["meta"]["public"], true);
}
