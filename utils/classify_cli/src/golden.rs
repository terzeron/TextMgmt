use std::path::Path;

use backend::classifier::{model::CategoryModel, tokenize::is_py_space};
use serde_json::{Value, json};

use crate::shuffle::Random;

pub(super) fn write_golden(model: &CategoryModel, model_dir: &Path, count: usize, seed: i64) -> Result<usize, String> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(model_dir.join("model.json")).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let mut pools: Vec<(String, Vec<String>)> = Vec::new();
    for field in manifest["fields"].as_array().ok_or("model.json에 fields가 없다")? {
        let source = field["source"].as_str().ok_or("필드 source가 문자열이 아니다")?;
        let vocabulary: Vec<String> = serde_json::from_slice(&std::fs::read(model_dir.join(field["vocab_file"].as_str().ok_or("필드 vocab_file이 없다")?)).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        let pool = if let Some((_, terms)) = pools.iter_mut().find(|(name, _)| name == source) {
            terms
        } else {
            pools.push((source.to_string(), Vec::new()));
            &mut pools.last_mut().unwrap().1
        };
        pool.extend(vocabulary.into_iter().filter(|term| !term.trim().is_empty()));
    }

    let mut cases = special_docs(&pools.iter().map(|(source, _)| source.as_str()).collect::<Vec<_>>());
    let mut random = Random::new(seed.unsigned_abs());
    for index in 0..count {
        cases.push(json!({"label":format!("synthetic_{index}"), "doc":synthetic_doc(&mut random, &pools)}));
    }
    let docs: Vec<Value> = cases.iter().map(|case| case["doc"].clone()).collect();
    let scores: Vec<Vec<f64>> = docs.iter().map(|doc| model.scores(doc)).collect();
    let predictions = model.predict(&docs);
    for ((case, score), prediction) in cases.iter_mut().zip(scores).zip(predictions) {
        case["scores"] = json!(score);
        case["category"] = json!(prediction.category);
        case["confidence"] = json!(prediction.confidence);
        case["top"] = json!(prediction.ranked.into_iter().map(|(category, confidence)| json!({"category":category,"confidence":confidence})).collect::<Vec<_>>());
        case["reason"] = json!(prediction.reason);
    }
    let payload = json!({"temperature":model.temperature(),"min_confidence":model.min_confidence(),"seed":seed,"cases":cases});
    crate::data::save_json(&model_dir.join("golden.json"), &payload)?;
    Ok(cases.len())
}

fn special_docs(sources: &[&str]) -> Vec<Value> {
    fn fill(sources: &[&str], text: &str, name: &str) -> Value {
        let base = [
            ("text", Value::String(text.to_string())),
            ("title", Value::String(text.chars().take(40).collect())),
            ("author", Value::String(text.chars().take(10).collect())),
            ("publisher", Value::String(text.chars().take(12).collect())),
            ("type", Value::String("epub".into())),
            ("name", Value::String(name.into())),
        ];
        Value::Object(base.into_iter().filter(|(key, _)| sources.contains(key) || *key == "name").map(|(key, value)| (key.to_string(), value)).collect())
    }
    let hangul = "대한민국의 역사와 문화 이야기 무림 용사의 모험";
    let mut list_author = fill(sources, "목록 필드 시험", "sample.epub");
    list_author["author"] = json!(["홍길동", "", "Kim"]);
    vec![
        json!({"label":"empty","doc":{}}),
        json!({"label":"hangul","doc":fill(sources,hangul,"sample.epub")}),
        json!({"label":"english","doc":fill(sources,"The Quick Brown FOX jumps over a lazy dog, again and again.","sample.epub")}),
        json!({"label":"cjk","doc":fill(sources,"漢字と ひらがな カタカナ 東京 上海 運氣調息","sample.epub")}),
        json!({"label":"whitespace","doc":fill(sources,"a \t\n  b\x0b\x0c 　c\x1f d\x00e   ","sample.epub")}),
        json!({"label":"digits_name","doc":fill(sources,"숫자 이름","book_0012345678_vol01_7_v2.epub")}),
        json!({"label":"nfd","doc":fill(sources,"대한민국의 역사와 문화 이야기 무림 용사의 모험","sample.epub")}),
        json!({"label":"emoji","doc":fill(sources,"😀 café 🎉 naïve ÀÉÎ ΣΑΣ İstanbul ǅ 1٢３ x²","sample.epub")}),
        json!({"label":"list_author","doc":list_author}),
        json!({"label":"blank_fields","doc":{"text":"   ","title":"","author":null,"publisher":"  ","type":"","name":""}}),
    ]
}

fn synthetic_doc(random: &mut Random, pools: &[(String, Vec<String>)]) -> Value {
    let mut doc = serde_json::Map::new();
    for (source, terms) in pools {
        if terms.is_empty() {
            continue;
        }
        let max = if source == "text" { 40 } else { 4 };
        let count = random.randint(0, max);
        let pieces: Vec<&str> = (0..count).map(|_| terms[random.below(terms.len())].as_str()).collect();
        let mut text = String::new();
        for piece in pieces {
            text.push_str(piece);
            text.push_str(["", " ", " "][random.below(3)]);
        }
        let text = text.trim_matches(is_py_space);
        let value = if source == "name" { format!("{text}_{}.epub", random.randint(10, 99_999_999)) } else { text.to_string() };
        doc.insert(source.clone(), Value::String(value));
    }
    Value::Object(doc)
}
