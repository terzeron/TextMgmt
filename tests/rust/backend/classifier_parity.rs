//! 2단계 Python 대조: 교정(isotonic·곡선), BLAKE2b·홀드아웃 분할, 설정 해시, JSON 직렬화, 리포트 출력.
//!
//! 기대값은 `tests/fixtures/classifier_tiny/parity.json` 이다. Python 구현이 만들고
//! (`python tests/test_classifier_export.py parity`), `tests/test_classifier_export.py::test_parity_fixture_is_current` 가
//! 최신인지 확인한다. 어떤 수치도 허용 오차를 키워 맞추지 않는다(부동소수점 연산 순서 차이 1e-12 만 허용).
//!
//! 이 파일은 또 JSONL·publisher 캐시가 Python 과 바이트 호환인지, 입력 상한이 지켜지는지 확인한다.

use std::fs;
use std::path::Path;

use backend::classifier::calibration::{expected_accuracy, fit_expected_accuracy, from_coverage_report};
use backend::classifier::config::{config_hash, default_config};
use backend::classifier::corpus::{AttachOptions, MAX_JSONL_LINE_BYTES, absolute_path, attach_publishers_with, read_jsonl, write_jsonl};
use backend::classifier::model::CategoryModel;
use backend::classifier::pyjson::{dumps, float_repr};
use backend::classifier::training::{blake2b_keyed, format_report, split_holdout};
use serde_json::{Value, json};
use tempfile::TempDir;

fn parity() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/parity.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()
}

/// Python 테스트 생성기와 같은 LCG.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn assert_close(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: 길이");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= 1e-12, "{what}[{i}]: rust {g} python {w}");
    }
}

#[test]
fn isotonic_fit_matches_sklearn() {
    let p = parity();
    let mut rng = Lcg(1);
    let (mut conf, mut correct) = (Vec::new(), Vec::new());
    for _ in 0..p["isotonic"]["n"].as_u64().unwrap() {
        let c = 0.013 + 0.137 * rng.next();
        conf.push(c);
        correct.push(rng.next() < (0.4 + (c - 0.013) * 4.3).clamp(0.0, 0.99));
    }
    let cal = fit_expected_accuracy(&conf, &correct).unwrap().unwrap();
    let want = &p["isotonic"]["expected"];
    assert_eq!(cal["n"], want["n"]);
    assert_close(&floats(&cal["x"]), &floats(&want["x"]), "x");
    assert_close(&floats(&cal["y"]), &floats(&want["y"]), "y");
    let probes = floats(&p["isotonic"]["probes"]);
    let got: Vec<f64> = probes.iter().map(|x| expected_accuracy(Some(&cal), Some(*x)).unwrap()).collect();
    assert_close(&got, &floats(&p["isotonic"]["probe_values"]), "probe");
}

#[test]
fn coverage_curve_scale_matches_python() {
    let p = parity();
    let cal = from_coverage_report(Some(&p["coverage"]["holdout"])).unwrap();
    let want = &p["coverage"]["expected"];
    assert_eq!(cal["source"], want["source"]);
    assert_eq!(cal["n"], want["n"]);
    assert_close(&floats(&cal["x"]), &floats(&want["x"]), "x");
    assert_close(&floats(&cal["y"]), &floats(&want["y"]), "y");
    let probes = floats(&p["coverage"]["probes"]);
    let got: Vec<f64> = probes.iter().map(|x| expected_accuracy(Some(&cal), Some(*x)).unwrap()).collect();
    assert_close(&got, &floats(&p["coverage"]["probe_values"]), "probe");
}

#[test]
fn blake2b_matches_hashlib() {
    for case in parity()["blake2b"].as_array().unwrap() {
        let digest = blake2b_keyed(case["data"].as_str().unwrap().as_bytes(), case["key"].as_str().unwrap().as_bytes(), 8);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, case["hex"].as_str().unwrap(), "data={:?}", case["data"].as_str().unwrap().chars().take(8).collect::<String>());
    }
}

#[test]
fn split_holdout_matches_python_membership_and_order() {
    let p = parity();
    let docs: Vec<Value> = p["split"]["docs"].as_array().unwrap().clone();
    let (train, hold) = split_holdout(&docs, p["split"]["ratio"].as_f64().unwrap(), p["split"]["seed"].as_u64().unwrap());
    let ids = |v: &[Value]| v.iter().map(|d| d["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let want = |v: &Value| v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&hold), want(&p["split"]["hold_ids"]));
    assert_eq!(ids(&train), want(&p["split"]["train_ids"]));
}

#[test]
fn float_repr_and_json_dumps_match_python() {
    let p = parity();
    for case in p["float_repr"].as_array().unwrap() {
        assert_eq!(float_repr(case["value"].as_f64().unwrap()), case["repr"].as_str().unwrap(), "{}", case["value"]);
    }
    let dumped = &p["json_dumps"];
    assert_eq!(format!("{}\n", dumps(&dumped["value"], false)), dumped["line"].as_str().unwrap());
    assert_eq!(dumps(&json!({"b": [1, 2.5, {"é": null}], "a": true, "10": 1e-7}), true), dumped["sorted"].as_str().unwrap());
}

#[test]
fn config_hash_matches_python() {
    let p = parity();
    assert_eq!(config_hash(&default_config()), p["config_hash_default"].as_str().unwrap());
    // Python 이 학습한 모델이 meta 에 적어 둔 해시를, 내보낸 config 에서 Rust 가 다시 계산해도 같다.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/trained");
    let model = CategoryModel::load(Some(&dir)).unwrap();
    assert_eq!(config_hash(&model.config), model.meta["config_hash"].as_str().unwrap());
}

#[test]
fn format_report_matches_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/trained");
    let report: Value = serde_json::from_slice(&fs::read(dir.join("report.json")).unwrap()).unwrap();
    assert_eq!(format_report(&report), parity()["report_text"].as_str().unwrap());
}

#[test]
fn jsonl_is_byte_compatible_with_python_and_round_trips() {
    let p = parity();
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("c.jsonl");
    write_jsonl(&[p["json_dumps"]["value"].clone()], &path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), p["json_dumps"]["line"].as_str().unwrap());
    // Python 이 쓴 줄(와 빈 줄)을 Rust 가 읽는다.
    fs::write(&path, format!("{}\n\n{}", p["json_dumps"]["line"].as_str().unwrap(), p["json_dumps"]["line"].as_str().unwrap())).unwrap();
    assert_eq!(read_jsonl(&path).unwrap().len(), 2);
}

#[test]
fn jsonl_rejects_oversized_lines_and_broken_json() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("big.jsonl");
    fs::write(&path, vec![b'a'; MAX_JSONL_LINE_BYTES as usize + 10]).unwrap();
    assert!(read_jsonl(&path).is_err());
    fs::write(&path, "{\"a\": 1}\n이건 JSON 이 아니다\n").unwrap();
    assert!(read_jsonl(&path).is_err());
    // 깊이가 한도를 넘는 JSON 도 오류다(스택을 쓰지 않는다).
    fs::write(&path, format!("{}1{}\n", "[".repeat(5000), "]".repeat(5000))).unwrap();
    assert!(read_jsonl(&path).is_err());
}

#[test]
fn absolute_path_rejects_parent_dir_escapes() {
    assert_eq!(absolute_path(&json!({"path": "../etc/passwd"}), "/mnt/data/text"), None);
    assert_eq!(absolute_path(&json!({"path": "a/../../b.epub"}), "/mnt/data/text"), None);
    assert!(absolute_path(&json!({"path": "a/b..c.epub"}), "/mnt/data/text").is_some());
}

#[test]
fn attach_publishers_skips_symlinks_that_leave_the_library() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("lib");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.epub"), b"x").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.epub"), root.join("link.epub")).unwrap();
    fs::write(root.join("ok.epub"), b"x").unwrap();

    let mut docs = vec![json!({"id": "1", "type": "epub", "path": "link.epub", "publisher": ""}), json!({"id": "2", "type": "epub", "path": "ok.epub", "publisher": ""})];
    let seen = std::sync::Mutex::new(Vec::new());
    let reader = |p: &Path| {
        seen.lock().unwrap().push(p.file_name().unwrap().to_string_lossy().into_owned());
        "출판사".to_string()
    };
    let stats = attach_publishers_with(&mut docs, &tmp.path().join("pub.json"), &root, AttachOptions { workers: 2, progress_every: 0, log: None }, &reader);
    assert_eq!(*seen.lock().unwrap(), vec!["ok.epub"]);
    assert_eq!(stats.read, 1);
    assert_eq!(docs[0]["publisher"], "");
    assert_eq!(docs[1]["publisher"], "출판사");
}
