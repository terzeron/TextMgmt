//! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
//!
//! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
//! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

use std::fs;
use std::path::{Path, PathBuf};

use backend::classifier::export_format::{ModelError, sha256_hex};
use backend::classifier::model::{CategoryModel, CategoryModelApi};
use backend::classifier::softmax::{fixed, rank_desc, softmax_confidence};
use backend::classifier::tokenize::{AnalyzerKind, AnalyzerSpec, analyze, collapse_whitespace, is_py_digit, is_py_space, is_py_word};
use backend::classifier::vectorize::field_text;
use serde_json::{Value, json};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/classifier_tiny");

fn spec(kind: &str, lo: usize, hi: usize) -> AnalyzerSpec {
    AnalyzerSpec { kind: AnalyzerKind::parse(kind).unwrap(), ngram_lo: lo, ngram_hi: hi, lowercase: true }
}

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
fn sklearn_analyzer_table() {
    for (kind, lo, hi, text, expected) in TOKEN_CASES {
        assert_eq!(&terms(kind, *lo, *hi, text), expected, "{kind} ({lo},{hi}) {text:?}");
    }
}

// ---------------------------------------------------------------------------
// field_text, softmax
// ---------------------------------------------------------------------------

#[test]
fn field_text_matches_python_field_text() {
    let doc = json!({"text": "abcdef", "name": "vol_12345_x9_77.epub", "author": ["가", "", null, "나"], "n": 0, "t": 12, "b": true});
    assert_eq!(field_text(&doc, "text", 3), "abc");
    assert_eq!(field_text(&doc, "text", 0), "abcdef");
    assert_eq!(field_text(&doc, "text", 100), "abcdef");
    // name: 두 자리 이상 숫자열만 공백으로, 그 뒤에 자른다.
    assert_eq!(field_text(&doc, "name", 0), "vol_ _x9_ .epub");
    assert_eq!(field_text(&doc, "name", 6), "vol_ _");
    assert_eq!(field_text(&doc, "author", 0), "가 나");
    assert_eq!(field_text(&doc, "missing", 0), "");
    assert_eq!(field_text(&doc, "n", 0), "");
    assert_eq!(field_text(&doc, "t", 0), "12");
    assert_eq!(field_text(&doc, "b", 0), "True");
    assert_eq!(field_text(&json!({"text": "가나다라"}), "text", 2), "가나");
    assert_eq!(field_text(&Value::Null, "text", 0), "");
}

#[test]
fn softmax_and_ranking_follow_python() {
    let p = softmax_confidence(&[1.0, 2.0, 3.0], 1.0);
    assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    assert!((p[2] - 0.665_240_955_774_642).abs() < 1e-12);
    // 온도 하한 1e-6, 오버플로 없이 계산한다.
    let sharp = softmax_confidence(&[0.0, 1.0], 0.0);
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

// ---------------------------------------------------------------------------
// parity
// ---------------------------------------------------------------------------

struct ParityStats {
    docs: usize,
    max_score_err: f64,
    max_conf_err: f64,
    rank_mismatch: usize,
    tie_swaps: usize,
    reason_mismatch: usize,
    category_mismatch: usize,
}

fn check_parity(model: &CategoryModel, golden_path: &Path) -> ParityStats {
    let golden: Value = serde_json::from_slice(&fs::read(golden_path).expect("golden.json 을 읽지 못했다")).unwrap();
    let mut stats = ParityStats { docs: 0, max_score_err: 0.0, max_conf_err: 0.0, rank_mismatch: 0, tie_swaps: 0, reason_mismatch: 0, category_mismatch: 0 };
    for case in golden["cases"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let expected: Vec<f64> = case["scores"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let got = model.scores(&case["doc"]);
        assert_eq!(got.len(), expected.len(), "{label}");
        for (g, e) in got.iter().zip(&expected) {
            stats.max_score_err = stats.max_score_err.max((g - e).abs());
        }
        let pred = CategoryModelApi::predict_one(model, &case["doc"], None);
        let top = case["top"].as_array().unwrap();
        for (i, t) in top.iter().enumerate() {
            stats.max_conf_err = stats.max_conf_err.max((pred.ranked[i].1 - t["confidence"].as_f64().unwrap()).abs());
        }
        let expected_order: Vec<&str> = top.iter().map(|t| t["category"].as_str().unwrap()).collect();
        let got_order: Vec<&str> = pred.ranked.iter().map(|(c, _)| c.as_str()).collect();
        if expected_order != got_order {
            // 두 점수 차가 float32 오차 안이면 순서를 가를 수 없는 동점이다.
            let rank = rank_desc(&expected);
            let near_tie = rank.windows(2).take(3).any(|w| (expected[w[0]] - expected[w[1]]).abs() < 2e-4);
            if near_tie {
                stats.tie_swaps += 1;
            } else {
                stats.rank_mismatch += 1;
                eprintln!("순위 불일치 [{label}]: python {expected_order:?} rust {got_order:?}");
            }
        } else {
            if pred.category.as_deref() != case["category"].as_str() {
                stats.category_mismatch += 1;
                eprintln!("판정 불일치 [{label}]: python {:?} rust {:?} (확신도 {} vs {})", case["category"], pred.category, case["confidence"], pred.confidence);
            }
            if pred.reason != case["reason"].as_str().unwrap() {
                stats.reason_mismatch += 1;
                eprintln!("reason 불일치 [{label}]: python {:?} rust {:?}", case["reason"], pred.reason);
            }
        }
        stats.docs += 1;
    }
    stats
}

#[test]
fn tiny_fixture_matches_sklearn_golden() {
    let model = CategoryModel::load_dir(Path::new(FIXTURE)).expect("fixture 로드");
    assert_eq!(model.classes().len(), 3);
    let stats = check_parity(&model, &Path::new(FIXTURE).join("golden.json"));
    assert!(stats.docs >= 40, "{}", stats.docs);
    assert!(stats.max_score_err <= 1e-4, "결정값 최대 오차 {}", stats.max_score_err);
    assert!(stats.max_conf_err <= 1e-4, "확신도 최대 오차 {}", stats.max_conf_err);
    assert_eq!((stats.rank_mismatch, stats.tie_swaps, stats.reason_mismatch, stats.category_mismatch), (0, 0, 0, 0));
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
// 실제 모델 / 무작위 문자열 parity (ignore)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "실제 모델 내보내기 필요: TM_CLASSIFIER_EXPORT_DIR"]
fn real_model_matches_sklearn_golden() {
    let dir = std::env::var("TM_CLASSIFIER_EXPORT_DIR").expect("TM_CLASSIFIER_EXPORT_DIR 이 필요하다");
    let dir = Path::new(&dir);
    let started = std::time::Instant::now();
    let model = CategoryModel::load_dir(dir).expect("실제 모델 로드");
    let load_secs = started.elapsed().as_secs_f64();
    assert_eq!(model.classes().len(), 79);
    let started = std::time::Instant::now();
    let stats = check_parity(&model, &dir.join("golden.json"));
    let per_doc_ms = started.elapsed().as_secs_f64() * 1000.0 / stats.docs as f64;
    eprintln!(
        "PARITY docs={} load={load_secs:.2}s predict={per_doc_ms:.2}ms/doc max_score_err={:.3e} max_conf_err={:.3e} rank_mismatch={} tie_swaps={} reason_mismatch={} category_mismatch={}",
        stats.docs, stats.max_score_err, stats.max_conf_err, stats.rank_mismatch, stats.tie_swaps, stats.reason_mismatch, stats.category_mismatch
    );
    assert!(stats.max_score_err <= 1e-4, "결정값 최대 오차 {}", stats.max_score_err);
    assert_eq!(stats.rank_mismatch, 0);
    assert_eq!(stats.category_mismatch, 0);
}

#[test]
#[ignore = "sklearn 분석기 대조 파일 필요: TM_CLASSIFIER_FUZZ_FILE"]
fn analyzers_match_sklearn_on_random_strings() {
    let path = std::env::var("TM_CLASSIFIER_FUZZ_FILE").expect("TM_CLASSIFIER_FUZZ_FILE 이 필요하다");
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
                eprintln!("분석기 불일치 {kind} ({lo},{hi}) 코드포인트 {:?}: rust {} 항, python {} 항, 첫 차이 {at:?}", text.chars().map(|c| c as u32).take(24).collect::<Vec<_>>(), got.len(), expected.len());
            }
        }
    }
    assert_eq!(bad, 0, "{bad}/{total} 불일치");
}

// 생성: sklearn 1.8.0 TfidfVectorizer(analyzer=..., ngram_range=..., lowercase=True).build_analyzer()(text) 결과
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
    ("word", 1, 2, "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc", &["한글", "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", "abc", "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc"]),
    ("word", 2, 3, "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc", &["한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc", "한글 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} abc"]),
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
    ("word", 1, 2, "e\u{301} caf\u{e9} x\u{b2}y \u{2167} \u{663}", &["e", "caf\u{e9}", "x\u{b2}y", "\u{2177}", "\u{663}", "e caf\u{e9}", "caf\u{e9} x\u{b2}y", "x\u{b2}y \u{2177}", "\u{2177} \u{663}"]),
    ("word", 2, 3, "e\u{301} caf\u{e9} x\u{b2}y \u{2167} \u{663}", &["e caf\u{e9}", "caf\u{e9} x\u{b2}y", "x\u{b2}y \u{2177}", "\u{2177} \u{663}", "e caf\u{e9} x\u{b2}y", "caf\u{e9} x\u{b2}y \u{2177}", "x\u{b2}y \u{2177} \u{663}"]),
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
    ("char_wb", 2, 4, "한 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", &[" 한", "한 ", " 한 ", " \u{1112}", "\u{1112}\u{1161}", "\u{1161}\u{11ab}", "\u{11ab}\u{1100}", "\u{1100}\u{1173}", "\u{1173}\u{11af}", "\u{11af} ", " \u{1112}\u{1161}", "\u{1112}\u{1161}\u{11ab}", "\u{1161}\u{11ab}\u{1100}", "\u{11ab}\u{1100}\u{1173}", "\u{1100}\u{1173}\u{11af}", "\u{1173}\u{11af} ", " \u{1112}\u{1161}\u{11ab}", "\u{1112}\u{1161}\u{11ab}\u{1100}", "\u{1161}\u{11ab}\u{1100}\u{1173}", "\u{11ab}\u{1100}\u{1173}\u{11af}", "\u{1100}\u{1173}\u{11af} "]),
    ("char_wb", 1, 3, "한 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", &[" ", "한", " ", " 한", "한 ", " 한 ", " ", "\u{1112}", "\u{1161}", "\u{11ab}", "\u{1100}", "\u{1173}", "\u{11af}", " ", " \u{1112}", "\u{1112}\u{1161}", "\u{1161}\u{11ab}", "\u{11ab}\u{1100}", "\u{1100}\u{1173}", "\u{1173}\u{11af}", "\u{11af} ", " \u{1112}\u{1161}", "\u{1112}\u{1161}\u{11ab}", "\u{1161}\u{11ab}\u{1100}", "\u{11ab}\u{1100}\u{1173}", "\u{1100}\u{1173}\u{11af}", "\u{1173}\u{11af} "]),
    ("char", 1, 3, "한 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", &["한", " ", "\u{1112}", "\u{1161}", "\u{11ab}", "\u{1100}", "\u{1173}", "\u{11af}", "한 ", " \u{1112}", "\u{1112}\u{1161}", "\u{1161}\u{11ab}", "\u{11ab}\u{1100}", "\u{1100}\u{1173}", "\u{1173}\u{11af}", "한 \u{1112}", " \u{1112}\u{1161}", "\u{1112}\u{1161}\u{11ab}", "\u{1161}\u{11ab}\u{1100}", "\u{11ab}\u{1100}\u{1173}", "\u{1100}\u{1173}\u{11af}"]),
    ("char", 2, 4, "한 \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}", &["한 ", " \u{1112}", "\u{1112}\u{1161}", "\u{1161}\u{11ab}", "\u{11ab}\u{1100}", "\u{1100}\u{1173}", "\u{1173}\u{11af}", "한 \u{1112}", " \u{1112}\u{1161}", "\u{1112}\u{1161}\u{11ab}", "\u{1161}\u{11ab}\u{1100}", "\u{11ab}\u{1100}\u{1173}", "\u{1100}\u{1173}\u{11af}", "한 \u{1112}\u{1161}", " \u{1112}\u{1161}\u{11ab}", "\u{1112}\u{1161}\u{11ab}\u{1100}", "\u{1161}\u{11ab}\u{1100}\u{1173}", "\u{11ab}\u{1100}\u{1173}\u{11af}"]),
    ("char_wb", 2, 4, " lead trail ", &[" l", "le", "ea", "ad", "d ", " le", "lea", "ead", "ad ", " lea", "lead", "ead ", " t", "tr", "ra", "ai", "il", "l ", " tr", "tra", "rai", "ail", "il ", " tra", "trai", "rail", "ail "]),
    ("char_wb", 1, 3, " lead trail ", &[" ", "l", "e", "a", "d", " ", " l", "le", "ea", "ad", "d ", " le", "lea", "ead", "ad ", " ", "t", "r", "a", "i", "l", " ", " t", "tr", "ra", "ai", "il", "l ", " tr", "tra", "rai", "ail", "il "]),
    ("char", 1, 3, " lead trail ", &[" ", "l", "e", "a", "d", " ", "t", "r", "a", "i", "l", " ", " l", "le", "ea", "ad", "d ", " t", "tr", "ra", "ai", "il", "l ", " le", "lea", "ead", "ad ", "d t", " tr", "tra", "rai", "ail", "il "]),
    ("char", 2, 4, " lead trail ", &[" l", "le", "ea", "ad", "d ", " t", "tr", "ra", "ai", "il", "l ", " le", "lea", "ead", "ad ", "d t", " tr", "tra", "rai", "ail", "il ", " lea", "lead", "ead ", "ad t", "d tr", " tra", "trai", "rail", "ail "]),
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
