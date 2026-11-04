//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]
use classifier::ClassifierOptions;
use classifier::inference_model::CategoryModel;
use classifier::tokenizer::{AnalyzerKind, AnalyzerSpec};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// 시험용 데이터
//
// 카테고리마다 어휘를 확실히 갈라 둔다. 선형 모델이 배울 신호가 있어야
// 학습 경로 전체를 실제로 통과시킬 수 있다.
// ---------------------------------------------------------------------------

pub const VOCAB: [(&str, &str); 3] =
    [("3_무협", "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객"), ("3_판타지", "드래곤 마나 던전 길드 마법사 레벨업 소환 마왕"), ("5_수학과학일반", "미분 적분 함수 벡터 행렬 확률 통계 방정식")];

pub fn approx(a: f64, b: f64, abs: f64) -> bool {
    (a - b).abs() <= abs
}

pub fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/book_classifier_parity.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

pub fn floats(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()
}

/// pytest fixture `trained`에 대응한다. Python sklearn 학습 결과와 비교하는 parity oracle을 읽는다
/// (`python tests/test_classifier_export.py trained` 로 다시 쓴다).
pub fn load_trained(name: &str) -> (CategoryModel, Value) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny").join(name);
    let model = CategoryModel::load(Some(&dir)).expect("학습 결과 픽스처를 읽어야 한다");
    let report: Value = serde_json::from_slice(&fs::read(dir.join("report.json")).unwrap()).unwrap();
    (model, report)
}

pub fn make_docs(per_category: usize) -> Vec<Value> {
    let mut docs = Vec::new();
    for (cat, words) in VOCAB {
        let tokens: Vec<&str> = words.split_whitespace().collect();
        for i in 0..per_category {
            // 문서마다 어순을 바꿔 같은 문자열이 반복되지 않게 한다.
            let k = i % tokens.len();
            let shifted: Vec<&str> = tokens[k..].iter().chain(tokens[..k].iter()).copied().collect();
            let text = shifted.repeat(3).join(" ");
            docs.push(json!({
                "id": format!("{cat}-{i}"), "cat": cat, "text": text, "title": format!("{} 이야기 {i}", tokens[k]), "author": format!("작가{}", i % 5),
                "publisher": "테스트출판사", "type": if i % 2 == 0 { "epub" } else { "txt" }, "name": format!("{}_{i}.epub", tokens[k]), "path": format!("/mnt/data/text/{cat}/{i}.epub"),
            }));
        }
    }
    docs
}

pub fn options(root: &Path) -> ClassifierOptions {
    ClassifierOptions { library_root: Some(root.to_path_buf()), ..Default::default() }
}

pub fn parity() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/parity.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// 학습 기록(판정률-정답률 곡선)에서 만드는 눈금
// ---------------------------------------------------------------------------

/// 실제 모델의 meta["holdout"] 모양. 누적 곡선이다.
pub fn real_holdout() -> Value {
    json!({
        "n": 47008,
        "top1_accuracy": 0.864788972089857,
        "curve": [
            {"coverage": 0.5, "precision": 0.974004424778761, "threshold": 0.1281779488151631},
            {"coverage": 0.7, "precision": 0.942136453426531, "threshold": 0.10072196828305498},
            {"coverage": 0.8, "precision": 0.9265010902515556, "threshold": 0.08265990361070291},
            {"coverage": 0.9, "precision": 0.9044602548041695, "threshold": 0.060971974205370344},
            {"coverage": 0.95, "precision": 0.8889759724119399, "threshold": 0.04635731124736481},
            {"coverage": 1.0, "precision": 0.864788972089857, "threshold": 0.019518310797334633},
        ],
        "at_precision_90": {"coverage": 0.917290673927842, "precision": 0.9, "threshold": 0.056374553813004144},
        "at_precision_95": {"coverage": 0.6435287610619469, "precision": 0.9500181812171499, "threshold": 0.10885944805582125},
    })
}

pub fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

pub fn spec(kind: &str, lo: usize, hi: usize) -> AnalyzerSpec {
    AnalyzerSpec { kind: AnalyzerKind::parse(kind).unwrap(), ngram_lo: lo, ngram_hi: hi, lowercase: true }
}

pub fn trained() -> (CategoryModel, Value) {
    load_trained("trained")
}

pub fn vocab(cat: &str) -> &'static str {
    VOCAB.iter().find(|(c, _)| *c == cat).unwrap().1
}
