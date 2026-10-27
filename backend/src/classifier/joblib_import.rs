//! Python sklearn joblib 모델을 Rust의 중립 export 디렉토리로 가져온다.
//!
//! `pyferrite`는 pickle 명령을 실행하지 않고 구조와 NumPy 배열만 읽는다. 가져오기는
//! 기존 `backend/classifier/export.py`의 일회성 모델 변환 경로를 대신한다.

use std::path::Path;

use pyferrite::{Array, ReadOptions, Value as PyValue, read_with};
use serde_json::{Map, Number, Value, json};

use super::model::CategoryModel;
use super::tokenize::{AnalyzerKind, AnalyzerSpec};
use super::vectorize::FieldVectorizer;

const MAX_ARRAY_ALLOCATION: usize = 2 * 1024 * 1024 * 1024;
const WORD_TOKEN_PATTERN: &str = r"(?u)\b\w+\b";

/// joblib을 읽고 Rust 추론용 export 디렉토리를 만든다. pickle 안의 코드는 실행하지 않는다.
pub fn import_joblib(input: &Path, output: &Path) -> Result<Value, String> {
    let options = ReadOptions::new().max_alloc(MAX_ARRAY_ALLOCATION);
    let payload = read_with(input, &options).map_err(|error| format!("joblib 모델을 읽지 못했다: {error}"))?;
    let model = model_from_pickle(&payload)?;
    model.save(Some(output)).map_err(|error| format!("모델을 저장하지 못했다: {error}"))?;

    let manifest: Value = serde_json::from_slice(&std::fs::read(output.join("model.json")).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let mut files = Map::new();
    let mut paths: Vec<_> = std::fs::read_dir(output).map_err(|error| error.to_string())?.filter_map(Result::ok).map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if let (Some(name), Ok(metadata)) = (path.file_name().and_then(|name| name.to_str()), path.metadata()) {
            files.insert(name.to_string(), json!(metadata.len()));
        }
    }
    Ok(json!({
        "out_dir": output,
        "n_features": manifest["n_features"],
        "n_classes": manifest["n_classes"],
        "fields": manifest["fields"].as_array().into_iter().flatten().filter_map(|field| field["name"].as_str()).collect::<Vec<_>>(),
        "files": files,
    }))
}

fn model_from_pickle(payload: &PyValue) -> Result<CategoryModel, String> {
    let root = dict(payload, "model payload")?;
    let feature_space = object(root.get("feature_space").ok_or("feature_space가 없다")?, "FeatureSpace")?;
    let feature_state = object_state(feature_space)?;
    let specs = dict(field(feature_state, "fields")?, "feature_space.fields")?;
    let vectorizers = dict(field(feature_state, "vectorizers")?, "feature_space.vectorizers")?;
    if vectorizers.is_empty() {
        return Err("벡터라이저가 하나도 없다".into());
    }

    let mut fields = Vec::with_capacity(vectorizers.len());
    let mut offset = 0usize;
    let mut per_class = Vec::new();
    let mut intercept = Vec::new();
    let classes = strings(root.get("classes").ok_or("classes가 없다")?, "classes")?;

    for (name_value, vectorizer_value) in vectorizers.iter() {
        let name = name_value.as_str().ok_or("벡터라이저 이름이 문자열이 아니다")?;
        let spec = dict(specs.get(name).ok_or_else(|| format!("필드 설정이 없다: {name}"))?, "field spec")?;
        let vectorizer = object(vectorizer_value, "TfidfVectorizer")?;
        let state = object_state(vectorizer)?;
        check_vectorizer(name, state)?;

        let source = string_field(spec, "source")?;
        let weight = number_field(spec, "weight").unwrap_or(1.0);
        let max_chars = number_field(spec, "max_chars").unwrap_or(0.0) as usize;
        let analyzer = string_field(state, "analyzer")?;
        let kind = AnalyzerKind::parse(analyzer).ok_or_else(|| format!("필드 '{name}': 지원하지 않는 analyzer {analyzer:?}"))?;
        let (ngram_lo, ngram_hi) = usize_pair(field(state, "ngram_range")?, "ngram_range")?;
        let lowercase = bool_field(state, "lowercase")?;
        let sublinear_tf = bool_field(state, "sublinear_tf")?;
        if string_field(state, "norm")? != "l2" {
            return Err(format!("필드 '{name}': norm은 l2만 지원한다"));
        }
        let token_pattern = string_field(state, "token_pattern")?;
        if analyzer == "word" && token_pattern != WORD_TOKEN_PATTERN {
            return Err(format!("필드 '{name}': 지원하지 않는 token_pattern {token_pattern:?}"));
        }
        let vocabulary = vocabulary(field(state, "vocabulary_")?, name)?;
        let idf_object = object(field(state, "_tfidf")?, "TfidfTransformer")?;
        let idf_state = object_state(idf_object)?;
        let idf = float_array(field(idf_state, "idf_")?, "idf_")?;
        if idf.len() != vocabulary.len() {
            return Err(format!("필드 '{name}': idf 길이와 vocabulary 크기가 다르다"));
        }
        offset = offset.checked_add(vocabulary.len()).ok_or("feature 수가 넘쳤다")?;
        let field = FieldVectorizer::new(
            name,
            source,
            weight as f32,
            max_chars,
            AnalyzerSpec { kind, ngram_lo, ngram_hi, lowercase },
            sublinear_tf,
            (offset - vocabulary.len()).try_into().map_err(|_| "feature offset이 u32 범위를 넘었다")?,
            vocabulary,
            idf,
        )
        .map_err(|error| error.to_string())?;
        fields.push(field);
    }

    let classifier = object(root.get("classifier").ok_or("classifier가 없다")?, "OneVsRestClassifier")?;
    let classifier_state = object_state(classifier)?;
    if let Some(classifier_classes) = classifier_state.get("classes_")
        && strings(classifier_classes, "classifier.classes_")? != classes
    {
        return Err("classifier.classes_ 와 model.classes 의 순서가 다르다".into());
    }
    let estimators = list(field(classifier_state, "estimators_")?, "estimators_")?;
    if estimators.len() != classes.len() {
        return Err("classifier가 클래스마다 추정기를 하나씩 가진 OneVsRest가 아니다".into());
    }
    let n_features = offset;
    for estimator in estimators {
        let estimator = object(estimator, "LinearSVC")?;
        let state = object_state(estimator)?;
        let coef = float_array(field(state, "coef_")?, "coef_")?;
        if array_shape(field(state, "coef_")?)? != [1, n_features] {
            return Err(format!("추정기 coef_ shape이 (1, {n_features})가 아니다"));
        }
        per_class.push(coef);
        let bias = float_array(field(state, "intercept_")?, "intercept_")?;
        if bias.len() != 1 {
            return Err("추정기 intercept_ 크기가 1이 아니다".into());
        }
        intercept.push(f64::from(bias[0]));
    }
    let mut coef = Vec::with_capacity(n_features.saturating_mul(classes.len()));
    for feature in 0..n_features {
        for class_coef in &per_class {
            coef.push(class_coef[feature]);
        }
    }

    let mut config = json_value(root.get("config").ok_or("config가 없다")?)?;
    // `feature_space.fields` is the exact configuration used to fit these
    // vectorizers. Some older joblib payloads do not preserve nested `config`
    // dictionaries, so restore this authoritative section from the feature space.
    let mut config_fields = Map::new();
    for (name, spec) in specs.iter() {
        let name = name.as_str().ok_or("필드 이름이 문자열이 아니다")?;
        config_fields.insert(name.to_owned(), json_value(spec)?);
    }
    if let Some(config) = config.as_object_mut() {
        config.insert("fields".into(), Value::Object(config_fields));
    }
    let mut meta = json_value(root.get("meta").ok_or("meta가 없다")?)?;
    if let Some(meta) = meta.as_object_mut() {
        meta.retain(|key, _| !key.starts_with('_'));
    }
    CategoryModel::from_training(classes, config, meta, fields, coef, intercept).map_err(|error| error.to_string())
}

fn check_vectorizer(name: &str, state: &pyferrite::Dict) -> Result<(), String> {
    let analyzer = string_field(state, "analyzer")?;
    if !matches!(analyzer, "word" | "char" | "char_wb") {
        return Err(format!("필드 '{name}': 지원하지 않는 analyzer {analyzer:?}"));
    }
    for key in ["strip_accents", "stop_words", "tokenizer", "preprocessor"] {
        if state.get(key).is_some_and(truthy) {
            return Err(format!("필드 '{name}': {key}는 지원하지 않는다"));
        }
    }
    if state.get("binary").is_some_and(truthy) {
        return Err(format!("필드 '{name}': binary=True는 지원하지 않는다"));
    }
    if state.get("use_idf").is_some_and(|value| !truthy(value)) {
        return Err(format!("필드 '{name}': use_idf=False는 지원하지 않는다"));
    }
    Ok(())
}

fn truthy(value: &PyValue) -> bool {
    match value {
        PyValue::None => false,
        PyValue::Bool(value) => *value,
        PyValue::Int(value) => *value != 0,
        PyValue::Float(value) => *value != 0.0,
        PyValue::Str(value) => !value.is_empty(),
        PyValue::Bytes(value) => !value.is_empty(),
        PyValue::List(values) | PyValue::Tuple(values) | PyValue::Set(values) => !values.is_empty(),
        PyValue::Dict(values) => !values.is_empty(),
        PyValue::Array(values) => !values.is_empty(),
        PyValue::Object(_) | PyValue::Frame(_) | PyValue::BigInt(_) | PyValue::Complex(_) => true,
    }
}

fn vocabulary(value: &PyValue, name: &str) -> Result<Vec<String>, String> {
    let dict = dict(value, "vocabulary_")?;
    let mut terms: Vec<Option<String>> = vec![None; dict.len()];
    for (term, index) in dict.iter() {
        let term = term.as_str().ok_or_else(|| format!("필드 '{name}': vocabulary term이 문자열이 아니다"))?;
        let index = integer(index).ok_or_else(|| format!("필드 '{name}': vocabulary index가 정수가 아니다"))?;
        let slot = terms.get_mut(index).ok_or_else(|| format!("필드 '{name}': vocabulary index가 연속 범위를 벗어났다"))?;
        if slot.replace(term.to_string()).is_some() {
            return Err(format!("필드 '{name}': vocabulary index가 중복됐다"));
        }
    }
    // There are `len` distinct indices, each below `len`; all slots must be filled.
    Ok(terms.into_iter().flatten().collect())
}

fn float_array(value: &PyValue, name: &str) -> Result<Vec<f32>, String> {
    let array = value.as_array().ok_or_else(|| format!("{name}가 NumPy 배열이 아니다"))?;
    let values = match array {
        Array::F32(values) => values.iter().copied().collect(),
        Array::F64(values) => values.iter().map(|value| *value as f32).collect(),
        _ => return Err(format!("{name}는 float32 또는 float64 배열이어야 한다")),
    };
    Ok(values)
}

fn array_shape(value: &PyValue) -> Result<&[usize], String> {
    value.as_array().map(Array::shape).ok_or_else(|| "기대값이 NumPy 배열이 아니다".into())
}

fn strings(value: &PyValue, name: &str) -> Result<Vec<String>, String> {
    let array = value.as_array().ok_or_else(|| format!("{name}가 문자열 배열이 아니다"))?;
    match array {
        Array::Str(values) => Ok(values.iter().cloned().collect()),
        _ => Err(format!("{name}가 문자열 배열이 아니다")),
    }
}

fn json_value(value: &PyValue) -> Result<Value, String> {
    match value {
        PyValue::None => Ok(Value::Null),
        PyValue::Bool(value) => Ok(Value::Bool(*value)),
        PyValue::Int(value) => Ok(json!(value)),
        PyValue::BigInt(_) => Err("JSON 정수 범위를 넘는 metadata 값은 지원하지 않는다".into()),
        PyValue::Float(value) => Ok(if value.is_finite() { Number::from_f64(*value).map(Value::Number).unwrap_or(Value::Null) } else { Value::Null }),
        PyValue::Complex(value) => Ok(Value::String(format!("{}+{}j", value.re, value.im))),
        PyValue::Str(value) => Ok(Value::String(value.clone())),
        PyValue::Bytes(value) => Ok(Value::String(format!("{value:?}"))),
        PyValue::List(values) | PyValue::Tuple(values) | PyValue::Set(values) => values.iter().map(json_value).collect::<Result<Vec<_>, _>>().map(Value::Array),
        PyValue::Dict(values) => {
            let mut result = Map::new();
            for (key, value) in values.iter() {
                result.insert(py_string(key)?, json_value(value)?);
            }
            Ok(Value::Object(result))
        }
        PyValue::Array(array) => array_json(array),
        PyValue::Frame(_) => Err("metadata의 DataFrame 값은 JSON으로 바꿀 수 없다".into()),
        PyValue::Object(object) => {
            if object.module == "pathlib"
                && let Some(PyValue::Str(path)) = object.args.first()
            {
                return Ok(Value::String(path.clone()));
            }
            Ok(Value::String(object.qualname()))
        }
    }
}

fn array_json(array: &Array) -> Result<Value, String> {
    let shape = array.shape();
    let flat: Vec<Value> = match array {
        Array::Bool(values) => values.iter().map(|value| Value::Bool(*value)).collect(),
        Array::I8(values) => values.iter().map(|value| json!(value)).collect(),
        Array::I16(values) => values.iter().map(|value| json!(value)).collect(),
        Array::I32(values) => values.iter().map(|value| json!(value)).collect(),
        Array::I64(values) => values.iter().map(|value| json!(value)).collect(),
        Array::U8(values) => values.iter().map(|value| json!(value)).collect(),
        Array::U16(values) => values.iter().map(|value| json!(value)).collect(),
        Array::U32(values) => values.iter().map(|value| json!(value)).collect(),
        Array::U64(values) => values.iter().map(|value| json!(value)).collect(),
        Array::F32(values) => values.iter().map(|value| finite_float(f64::from(*value))).collect(),
        Array::F64(values) => values.iter().map(|value| finite_float(*value)).collect(),
        Array::Str(values) => values.iter().cloned().map(Value::String).collect(),
        _ => return Err(format!("metadata 배열 형식 {:?}을 JSON으로 바꿀 수 없다", array.dtype())),
    };
    nest_array(shape, &flat)
}

fn finite_float(value: f64) -> Value {
    if value.is_finite() { Number::from_f64(value).map(Value::Number).unwrap_or(Value::Null) } else { Value::Null }
}

fn nest_array(shape: &[usize], values: &[Value]) -> Result<Value, String> {
    if shape.is_empty() {
        return values.first().cloned().ok_or_else(|| "scalar NumPy 배열이 비었다".into());
    }
    if shape.len() == 1 {
        if shape[0] != values.len() {
            return Err("NumPy 배열 shape와 값 개수가 다르다".into());
        }
        return Ok(Value::Array(values.to_vec()));
    }
    let child_size = shape[1..].iter().product::<usize>();
    if shape[0].saturating_mul(child_size) != values.len() {
        return Err("NumPy 배열 shape와 값 개수가 다르다".into());
    }
    if child_size == 0 {
        return (0..shape[0]).map(|_| nest_array(&shape[1..], &[])).collect::<Result<Vec<_>, _>>().map(Value::Array);
    }
    values.chunks(child_size.max(1)).take(shape[0]).map(|chunk| nest_array(&shape[1..], chunk)).collect::<Result<Vec<_>, _>>().map(Value::Array)
}

fn py_string(value: &PyValue) -> Result<String, String> {
    match value {
        PyValue::Str(value) => Ok(value.clone()),
        PyValue::Bool(value) => Ok(if *value { "True" } else { "False" }.into()),
        PyValue::Int(value) => Ok(value.to_string()),
        PyValue::Float(value) => Ok(value.to_string()),
        PyValue::None => Ok("None".into()),
        _ => Err("config/meta dict key가 JSON 문자열로 바뀌지 않는다".into()),
    }
}

fn integer(value: &PyValue) -> Option<usize> {
    match value {
        PyValue::Int(value) => (*value).try_into().ok(),
        _ => None,
    }
}

fn usize_pair(value: &PyValue, name: &str) -> Result<(usize, usize), String> {
    let values = match value {
        PyValue::Tuple(values) | PyValue::List(values) if values.len() == 2 => values,
        _ => return Err(format!("{name}가 정수 두 개의 tuple이 아니다")),
    };
    let lo = integer(&values[0]).ok_or_else(|| format!("{name} 시작이 정수가 아니다"))?;
    let hi = integer(&values[1]).ok_or_else(|| format!("{name} 끝이 정수가 아니다"))?;
    Ok((lo, hi))
}

fn string_field<'a>(dict: &'a pyferrite::Dict, key: &str) -> Result<&'a str, String> {
    dict.get(key).and_then(PyValue::as_str).ok_or_else(|| format!("{key}가 문자열이 아니다"))
}

fn number_field(dict: &pyferrite::Dict, key: &str) -> Option<f64> {
    dict.get(key).and_then(PyValue::as_f64)
}

fn bool_field(dict: &pyferrite::Dict, key: &str) -> Result<bool, String> {
    match dict.get(key) {
        Some(PyValue::Bool(value)) => Ok(*value),
        _ => Err(format!("{key}가 bool이 아니다")),
    }
}

fn field<'a>(dict: &'a pyferrite::Dict, key: &str) -> Result<&'a PyValue, String> {
    dict.get(key).ok_or_else(|| format!("필드가 없다: {key}"))
}

fn list<'a>(value: &'a PyValue, name: &str) -> Result<&'a [PyValue], String> {
    match value {
        PyValue::List(values) | PyValue::Tuple(values) => Ok(values),
        _ => Err(format!("{name}가 list가 아니다")),
    }
}

fn dict<'a>(value: &'a PyValue, name: &str) -> Result<&'a pyferrite::Dict, String> {
    value.as_dict().ok_or_else(|| format!("{name}가 dict가 아니다"))
}

fn object<'a>(value: &'a PyValue, name: &str) -> Result<&'a pyferrite::PyObject, String> {
    let object = value.as_object().ok_or_else(|| format!("{name}가 Python 객체가 아니다"))?;
    if object.name != name && !object.name.ends_with(name) {
        return Err(format!("기대 객체는 {name}, 실제는 {}", object.qualname()));
    }
    Ok(object)
}

fn object_state(object: &pyferrite::PyObject) -> Result<&pyferrite::Dict, String> {
    object.state.as_deref().and_then(PyValue::as_dict).ok_or_else(|| format!("{} state가 dict가 아니다", object.qualname()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyferrite::num::Complex64;
    use pyferrite::{Dict, Frame, PyObject};

    fn dict(entries: impl IntoIterator<Item = (PyValue, PyValue)>) -> PyValue {
        let mut dict = Dict::new();
        for (key, value) in entries {
            dict.insert(key, value);
        }
        PyValue::Dict(dict)
    }

    fn fixture_payload() -> PyValue {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/legacy.joblib");
        read_with(&path, &ReadOptions::new()).unwrap()
    }

    fn feature_state_mut(payload: &mut PyValue) -> &mut Dict {
        let feature_space = payload.as_dict_mut().unwrap().get_mut("feature_space").unwrap();
        let PyValue::Object(object) = feature_space else { panic!("fixture feature_space should be an object") };
        object.state.as_deref_mut().and_then(PyValue::as_dict_mut).unwrap()
    }

    fn npy_value(descr: &str, shape: &str, data: &[u8]) -> PyValue {
        let mut header = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
        let padding = (16 - (10 + header.len() + 1) % 16) % 16;
        header.push_str(&" ".repeat(padding));
        header.push('\n');
        let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
        bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(data);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("array.npy");
        std::fs::write(&path, bytes).unwrap();
        read_with(&path, &ReadOptions::new()).unwrap()
    }

    #[test]
    fn python_truthiness_matches_supported_pickle_values() {
        let cases = [
            (PyValue::None, false),
            (PyValue::Bool(false), false),
            (PyValue::Bool(true), true),
            (PyValue::Int(0), false),
            (PyValue::Int(-1), true),
            (PyValue::Float(0.0), false),
            (PyValue::Float(f64::NAN), true),
            (PyValue::Str(String::new()), false),
            (PyValue::Str("x".into()), true),
            (PyValue::Bytes(Vec::new()), false),
            (PyValue::Bytes(vec![1]), true),
            (PyValue::List(Vec::new()), false),
            (PyValue::Tuple(vec![PyValue::None]), true),
            (PyValue::Set(Vec::new()), false),
            (dict([]), false),
            (PyValue::BigInt(vec![0]), true),
            (PyValue::Complex(Complex64::default()), true),
            (PyValue::Object(PyObject::new("pkg", "Thing")), true),
            (PyValue::Frame(Frame::default()), true),
        ];

        for (value, expected) in cases {
            assert_eq!(truthy(&value), expected, "{value:?}");
        }
    }

    #[test]
    fn vocabulary_requires_string_terms_and_a_contiguous_unique_index() {
        let valid = dict([(PyValue::Str("second".into()), PyValue::Int(1)), (PyValue::Str("first".into()), PyValue::Int(0))]);
        assert_eq!(vocabulary(&valid, "title").unwrap(), ["first", "second"]);

        let duplicate = dict([(PyValue::Str("a".into()), PyValue::Int(0)), (PyValue::Str("b".into()), PyValue::Int(0))]);
        assert!(vocabulary(&duplicate, "title").unwrap_err().contains("중복"));
        let out_of_range = dict([(PyValue::Str("a".into()), PyValue::Int(1))]);
        assert!(vocabulary(&out_of_range, "title").unwrap_err().contains("연속 범위"));
        let non_string_term = dict([(PyValue::Int(1), PyValue::Int(0))]);
        assert!(vocabulary(&non_string_term, "title").unwrap_err().contains("문자열"));
        let non_integer_index = dict([(PyValue::Str("a".into()), PyValue::Str("0".into()))]);
        assert!(vocabulary(&non_integer_index, "title").unwrap_err().contains("정수"));
        assert!(vocabulary(&PyValue::None, "title").unwrap_err().contains("dict"));
    }

    #[test]
    fn model_import_rejects_missing_or_malformed_top_level_sections() {
        assert!(model_from_pickle(&PyValue::None).unwrap_err().contains("model payload"));

        let mut payload = fixture_payload();
        payload.as_dict_mut().unwrap().remove("feature_space");
        assert_eq!(model_from_pickle(&payload).unwrap_err(), "feature_space가 없다");

        let mut payload = fixture_payload();
        payload.as_dict_mut().unwrap().insert(PyValue::Str("feature_space".into()), PyValue::None);
        assert!(model_from_pickle(&payload).unwrap_err().contains("FeatureSpace"));

        let mut payload = fixture_payload();
        feature_state_mut(&mut payload).remove("vectorizers");
        assert!(model_from_pickle(&payload).unwrap_err().contains("vectorizers"));

        let mut payload = fixture_payload();
        feature_state_mut(&mut payload).insert(PyValue::Str("vectorizers".into()), dict([]));
        assert_eq!(model_from_pickle(&payload).unwrap_err(), "벡터라이저가 하나도 없다");

        let mut payload = fixture_payload();
        payload.as_dict_mut().unwrap().remove("classes");
        assert_eq!(model_from_pickle(&payload).unwrap_err(), "classes가 없다");
    }

    #[test]
    fn vectorizer_rejects_unsupported_sklearn_options() {
        for key in ["strip_accents", "stop_words", "tokenizer", "preprocessor"] {
            let mut state = Dict::new();
            state.insert(PyValue::Str("analyzer".into()), PyValue::Str("word".into()));
            state.insert(PyValue::Str(key.into()), PyValue::Bool(true));
            assert!(check_vectorizer("title", &state).unwrap_err().contains(key));
        }

        let mut state = Dict::new();
        state.insert(PyValue::Str("analyzer".into()), PyValue::Str("word".into()));
        state.insert(PyValue::Str("binary".into()), PyValue::Bool(true));
        assert!(check_vectorizer("title", &state).unwrap_err().contains("binary=True"));

        let mut state = Dict::new();
        state.insert(PyValue::Str("analyzer".into()), PyValue::Str("word".into()));
        state.insert(PyValue::Str("use_idf".into()), PyValue::Bool(false));
        assert!(check_vectorizer("title", &state).unwrap_err().contains("use_idf=False"));

        let mut state = Dict::new();
        state.insert(PyValue::Str("analyzer".into()), PyValue::Str("char".into()));
        assert!(check_vectorizer("title", &state).is_ok());
        state.insert(PyValue::Str("analyzer".into()), PyValue::Str("char_wb".into()));
        assert!(check_vectorizer("title", &state).is_ok());
        state.insert(PyValue::Str("analyzer".into()), PyValue::Str("ngram".into()));
        assert!(check_vectorizer("title", &state).unwrap_err().contains("analyzer"));
    }

    #[test]
    fn metadata_conversion_handles_nested_values_and_rejects_unsupported_values() {
        let object = PyObject { module: "pathlib".into(), name: "PosixPath".into(), args: vec![PyValue::Str("/tmp/model".into())], ..PyObject::default() };
        let values = PyValue::Tuple(vec![
            PyValue::None,
            PyValue::Bool(true),
            PyValue::Int(7),
            PyValue::Float(f64::INFINITY),
            PyValue::Complex(Complex64::default()),
            PyValue::Bytes(vec![65]),
            PyValue::Object(object),
            PyValue::Object(PyObject::new("custom.module", "Thing")),
        ]);
        let converted = json_value(&values).unwrap();
        assert_eq!(converted[0], Value::Null);
        assert_eq!(converted[1], Value::Bool(true));
        assert_eq!(converted[2], json!(7));
        assert_eq!(converted[3], Value::Null);
        assert_eq!(converted[4], Value::String("0+0j".into()));
        assert_eq!(converted[5], Value::String("[65]".into()));
        assert_eq!(converted[6], Value::String("/tmp/model".into()));
        assert_eq!(converted[7], Value::String("custom.module.Thing".into()));
        assert_eq!(json_value(&dict([(PyValue::Str("path".into()), PyValue::Int(1))])).unwrap()["path"], json!(1));
        assert_eq!(json_value(&PyValue::BigInt(vec![1])).unwrap_err(), "JSON 정수 범위를 넘는 metadata 값은 지원하지 않는다");
        assert_eq!(json_value(&PyValue::Frame(Frame::default())).unwrap_err(), "metadata의 DataFrame 값은 JSON으로 바꿀 수 없다");
        assert!(py_string(&PyValue::List(Vec::new())).is_err());
        assert_eq!(py_string(&PyValue::Bool(true)).unwrap(), "True");
        assert_eq!(py_string(&PyValue::None).unwrap(), "None");
        assert!(json_value(&dict([(PyValue::Tuple(vec![]), PyValue::None)])).is_err());
    }

    #[test]
    fn numpy_metadata_arrays_preserve_values_shapes_and_finite_numbers() {
        let cases: [(&str, &[u8], Value); 11] = [
            ("|b1", &[0, 1], json!([false, true])),
            ("<i1", &[1, 2], json!([1, 2])),
            ("<i2", &[1, 0, 2, 0], json!([1, 2])),
            ("<i4", &[1, 0, 0, 0, 2, 0, 0, 0], json!([1, 2])),
            ("<i8", &[1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0], json!([1, 2])),
            ("|u1", &[1, 2], json!([1, 2])),
            ("<u2", &[1, 0, 2, 0], json!([1, 2])),
            ("<u4", &[1, 0, 0, 0, 2, 0, 0, 0], json!([1, 2])),
            ("<u8", &[1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0], json!([1, 2])),
            ("<f4", &[0, 0, 128, 63, 0, 0, 0, 64], json!([1.0, 2.0])),
            ("<f8", &[0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 64], json!([1.0, 2.0])),
        ];
        for (descr, data, expected) in cases {
            let value = npy_value(descr, "(2,)", data);
            assert_eq!(array_json(value.as_array().unwrap()).unwrap(), expected, "{descr}");
            assert!(truthy(&value));
            assert_eq!(array_shape(&value).unwrap(), [2]);
        }

        let text = npy_value("<U1", "(2,)", &[b'a', 0, 0, 0, b'b', 0, 0, 0]);
        assert_eq!(array_json(text.as_array().unwrap()).unwrap(), json!(["a", "b"]));
        let matrix = npy_value("<i4", "(2, 2)", &[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0]);
        assert_eq!(array_json(matrix.as_array().unwrap()).unwrap(), json!([[1, 2], [3, 4]]));
        let scalar = npy_value("<i4", "()", &[9, 0, 0, 0]);
        assert_eq!(array_json(scalar.as_array().unwrap()).unwrap(), json!(9));
        let non_finite = npy_value("<f8", "(2,)", &[0, 0, 0, 0, 0, 0, 248, 127, 0, 0, 0, 0, 0, 0, 240, 127]);
        assert_eq!(array_json(non_finite.as_array().unwrap()).unwrap(), json!([null, null]));
        assert_eq!(float_array(&npy_value("<f4", "(2,)", &[0, 0, 128, 63, 0, 0, 0, 64]), "coef_").unwrap(), [1.0, 2.0]);
        assert_eq!(float_array(&npy_value("<f8", "(2,)", &[0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 64]), "coef_").unwrap(), [1.0, 2.0]);
        assert!(float_array(&npy_value("<i4", "(1,)", &[1, 0, 0, 0]), "coef_").is_err());
        assert!(array_shape(&PyValue::None).is_err());
    }

    #[test]
    fn shapes_pairs_and_structural_accessors_reject_malformed_values() {
        assert_eq!(usize_pair(&PyValue::Tuple(vec![PyValue::Int(1), PyValue::Int(3)]), "ngram_range").unwrap(), (1, 3));
        assert!(usize_pair(&PyValue::List(vec![PyValue::Int(1)]), "ngram_range").is_err());
        assert!(usize_pair(&PyValue::Tuple(vec![PyValue::Str("1".into()), PyValue::Int(3)]), "ngram_range").is_err());
        assert_eq!(integer(&PyValue::Int(4)), Some(4));
        assert_eq!(integer(&PyValue::Int(-1)), None);
        assert_eq!(integer(&PyValue::Bool(true)), None);

        assert!(nest_array(&[], &[]).is_err());
        assert_eq!(nest_array(&[2], &[json!(1), json!(2)]).unwrap(), json!([1, 2]));
        assert_eq!(nest_array(&[2, 2], &[json!(1), json!(2), json!(3), json!(4)]).unwrap(), json!([[1, 2], [3, 4]]));
        assert_eq!(nest_array(&[2, 0], &[]).unwrap(), json!([[], []]));
        assert!(nest_array(&[2], &[json!(1)]).is_err());
        assert!(nest_array(&[2, 2], &[json!(1), json!(2), json!(3)]).is_err());

        let mut state = Dict::new();
        state.insert(PyValue::Str("enabled".into()), PyValue::Bool(true));
        assert_eq!(string_field(&state, "missing").unwrap_err(), "missing가 문자열이 아니다");
        assert_eq!(number_field(&state, "enabled"), Some(1.0));
        assert_eq!(number_field(&state, "missing"), None);
        assert!(bool_field(&state, "missing").is_err());
        assert!(field(&state, "missing").is_err());
        assert!(list(&PyValue::None, "estimators_").is_err());
        assert!(super::dict(&PyValue::None, "model").is_err());
        assert!(object(&PyValue::None, "LinearSVC").is_err());
        assert!(object(&PyValue::Object(PyObject::new("sklearn.svm", "SVC")), "LinearSVC").is_err());
        let no_state = PyObject::new("sklearn.svm", "LinearSVC");
        assert!(object_state(&no_state).is_err());
        let invalid_state = PyObject { state: Some(Box::new(PyValue::List(Vec::new()))), ..no_state };
        assert!(object_state(&invalid_state).is_err());
    }
}
