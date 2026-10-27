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

pub(crate) fn indexed_corpus(path: &Path) -> Result<corpus::IndexedJsonl, String> {
    if !path.exists() {
        return Err(format!("학습 데이터가 없다: {}\n먼저 'collect' 를 돌릴 것.", path.display()));
    }
    let docs = corpus::IndexedJsonl::open(path).map_err(|error| format!("학습 데이터를 읽지 못했다: {error}\n먼저 'collect'를 돌릴 것."))?;
    if docs.is_empty() {
        return Err(format!("학습 데이터가 비었다: {}", path.display()));
    }
    Ok(docs)
}

/// JSONL 본문을 한꺼번에 올리지 않고 manifest 또는 Python 호환 분할로 holdout 위치를 찾는다.
pub(crate) fn indexed_holdout_indices(docs: &corpus::IndexedJsonl, manifest_path: &Path, min_per_category: usize, ratio: f64, seed: u64, err: &mut dyn Write) -> Result<Vec<usize>, String> {
    if manifest_path.exists() {
        let value = json_file(manifest_path)?;
        let keys = value["document_keys"].as_array().ok_or("잘못된 holdout manifest")?;
        let keys: HashSet<&str> = keys.iter().map(|key| key.as_str().ok_or("잘못된 holdout manifest")).collect::<Result<_, _>>()?;
        let mut found = HashSet::new();
        let indices: Vec<usize> = (0..docs.len())
            .filter(|&index| {
                let Some(key) = docs.document_key(index) else { return false };
                if keys.contains(key) {
                    found.insert(key);
                    true
                } else {
                    false
                }
            })
            .collect();
        if !keys.is_empty() && indices.is_empty() {
            return Err("holdout manifest 문서를 현재 코퍼스에서 찾지 못했다".into());
        }
        if found.len() < keys.len() {
            writeln!(err, "holdout manifest 문서 {}건을 현재 코퍼스에서 찾지 못했다", keys.len() - found.len()).map_err(|error| error.to_string())?;
        }
        return Ok(indices);
    }

    writeln!(err, "holdout manifest가 없어 해시 분할을 재구성한다. 코퍼스가 바뀌었다면 결과를 신뢰할 수 없다.").map_err(|error| error.to_string())?;
    Ok(training::holdout_indices_indexed(docs, min_per_category, ratio, seed))
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

#[cfg(test)]
mod indexed_holdout_tests {
    use super::*;
    use serde_json::json;

    fn corpus(dir: &Path, docs: &[Value]) -> PathBuf {
        let path = dir.join("corpus.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        for doc in docs {
            serde_json::to_writer(&mut file, doc).unwrap();
            writeln!(file).unwrap();
        }
        path
    }

    #[test]
    fn indexed_manifest_selects_only_matching_documents_and_reports_missing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let docs = vec![json!({"id":"a","cat":"A"}), json!({"id":"b","cat":"B"}), json!({"id":"c","cat":"A"})];
        let corpus_path = corpus(dir.path(), &docs);
        let manifest = dir.path().join("holdout.json");
        fs::write(&manifest, json!({"document_keys":["b","missing"]}).to_string()).unwrap();
        let mut indexed = crate::data::indexed_corpus(&corpus_path).unwrap();
        let mut warning = Vec::new();

        let indices = indexed_holdout_indices(&indexed, &manifest, 1, 0.2, 42, &mut warning).unwrap();
        let selected: Vec<Value> = indices.iter().map(|&i| indexed.read_document(i).unwrap()).collect();

        assert_eq!(selected, vec![docs[1].clone()]);
        assert!(String::from_utf8(warning).unwrap().contains("1건을 현재 코퍼스에서 찾지 못했다"));
    }

    #[test]
    fn indexed_manifest_without_matching_keys_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let corpus_path = corpus(dir.path(), &[json!({"id":"a","cat":"A"})]);
        let manifest = dir.path().join("holdout.json");
        fs::write(&manifest, json!({"document_keys":["missing"]}).to_string()).unwrap();
        let indexed = crate::data::indexed_corpus(&corpus_path).unwrap();

        assert!(indexed_holdout_indices(&indexed, &manifest, 1, 0.2, 42, &mut Vec::new()).unwrap_err().contains("찾지 못했다"));
    }

    #[test]
    fn indexed_fallback_matches_materialized_holdout_selection() {
        let dir = tempfile::tempdir().unwrap();
        let docs: Vec<Value> = (0..18).map(|i| json!({"id":format!("{i}"),"cat":if i % 2 == 0 {"A"} else {"B"},"name":format!("책 {i}")})).collect();
        let corpus_path = corpus(dir.path(), &docs);
        let indexed = crate::data::indexed_corpus(&corpus_path).unwrap();
        let materialized = corpus::read_jsonl(&corpus_path).unwrap();
        let (filtered, _) = training::filter_by_size(&materialized, 2);
        let expected: HashSet<String> = training::split_holdout(&filtered, 0.25, 91).1.iter().map(training::document_key).collect();
        let mut warning = Vec::new();

        let actual = indexed_holdout_indices(&indexed, &dir.path().join("absent.json"), 2, 0.25, 91, &mut warning).unwrap();
        let actual: HashSet<String> = actual.iter().map(|&i| indexed.document_key(i).unwrap().to_owned()).collect();

        assert_eq!(actual, expected);
        assert!(String::from_utf8(warning).unwrap().contains("manifest가 없어 해시 분할을 재구성한다"));
    }
}

#[cfg(test)]
mod json_io_tests {
    use super::{json_file, save_json};
    use serde_json::json;

    #[test]
    fn json_file_reports_missing_non_file_and_invalid_json_inputs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(json_file(&dir.path().join("missing.json")).unwrap_err().contains("missing.json"));
        assert_eq!(json_file(dir.path()).unwrap_err(), "일반 파일이 아니다");

        let malformed = dir.path().join("malformed.json");
        std::fs::write(&malformed, b"{").unwrap();
        assert!(json_file(&malformed).unwrap_err().contains("EOF"));
    }

    #[test]
    fn save_json_creates_parent_directories_and_writes_parseable_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/output.json");

        save_json(&path, &json!({"answer": 42})).unwrap();

        assert_eq!(json_file(&path).unwrap(), json!({"answer": 42}));
        assert!(std::fs::read_to_string(path).unwrap().ends_with('\n'));
    }
}
