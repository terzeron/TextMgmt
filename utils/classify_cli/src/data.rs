use backend::classifier::{corpus, model::CategoryModel, training};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub(crate) fn json_file(path: &Path) -> Result<Value, String> {
    let mut bytes = Vec::new();
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("일반 파일이 아니다".into());
    }
    file.take((128 << 20) + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 128 << 20 {
        return Err("JSON 파일이 128MiB를 넘는다".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub(crate) fn save_json(path: &Path, value: &Value) -> Result<(), String> {
    // 기존 파일을 열어 truncate하지 않는다. 쓰기 실패 시 원본을 보존한다.
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(".{}.{}.tmp", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(|e| e.to_string())?;
    let result = (|| {
        serde_json::to_writer_pretty(&mut file, value).map_err(std::io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

pub(crate) fn model(path: &Path) -> Result<CategoryModel, String> {
    CategoryModel::load_dir(path).map_err(|e| format!("모델을 읽지 못했다: {e}\n먼저 'collect'와 'train'을 돌릴 것. --model은 Rust export 디렉토리다."))
}

pub(crate) fn corpus(path: &Path) -> Result<Vec<Value>, String> {
    let docs = corpus::read_jsonl(path).map_err(|e| format!("학습 데이터를 읽지 못했다: {e}\n먼저 'collect'를 돌릴 것."))?;
    if docs.is_empty() {
        return Err(format!("학습 데이터가 비었다: {}", path.display()));
    }
    Ok(docs)
}

pub fn holdout_manifest_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".holdout.json");
    PathBuf::from(name)
}

pub fn load_manifest_holdout(docs: &[Value], path: &Path) -> Result<Vec<Value>, String> {
    let value = json_file(path)?;
    let keys = value["document_keys"].as_array().ok_or("잘못된 holdout manifest")?;
    let keys: HashSet<&str> = keys.iter().map(|k| k.as_str().ok_or("잘못된 holdout manifest")).collect::<Result<_, _>>()?;
    let hold: Vec<Value> = docs.iter().filter(|d| keys.contains(training::document_key(d).as_str())).cloned().collect();
    if !keys.is_empty() && hold.is_empty() {
        return Err("holdout manifest 문서를 현재 코퍼스에서 찾지 못했다".into());
    }
    Ok(hold)
}

pub(crate) fn holdout(docs: &[Value], model: &CategoryModel, path: &Path, err: &mut dyn Write) -> Result<Vec<Value>, String> {
    let manifest = holdout_manifest_path(path);
    if manifest.exists() {
        let hold = load_manifest_holdout(docs, &manifest)?;
        let found: HashSet<String> = hold.iter().map(training::document_key).collect();
        let payload = json_file(&manifest)?;
        let keys: HashSet<&str> = payload["document_keys"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        if found.len() < keys.len() {
            writeln!(err, "holdout manifest 문서 {}건을 현재 코퍼스에서 찾지 못했다", keys.len() - found.len()).map_err(|e| e.to_string())?;
        }
        return Ok(hold);
    }
    writeln!(err, "holdout manifest가 없어 해시 분할을 재구성한다. 코퍼스가 바뀌었다면 결과를 신뢰할 수 없다.").map_err(|e| e.to_string())?;
    let cfg = &model.config;
    let (kept, _) = training::filter_by_size(docs, cfg["corpus"]["min_per_category"].as_u64().unwrap_or(30) as usize);
    Ok(training::split_holdout(&kept, cfg["holdout"].as_f64().unwrap_or(0.2), cfg["seed"].as_u64().unwrap_or(42)).1)
}

pub(crate) fn count(n: usize) -> String {
    let s = n.to_string();
    s.chars().enumerate().fold(String::new(), |mut out, (i, c)| {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
        out
    })
}
