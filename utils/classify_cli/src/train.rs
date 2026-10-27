use crate::{CliIo, Deps, args::Args, data};
use backend::classifier::{config, training};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SPILL_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TemporarySpillDir {
    path: PathBuf,
}

impl TemporarySpillDir {
    fn create_in(parent: &Path) -> Result<Self, String> {
        for _ in 0..100 {
            let sequence = SPILL_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("classify-cli-spill-{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("임시 spill 디렉토리를 만들지 못했다: {error}")),
            }
        }
        Err("임시 spill 디렉토리 이름을 확보하지 못했다".into())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporarySpillDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn resolve_spill_dir(requested: Option<PathBuf>, temporary_parent: &Path) -> Result<(PathBuf, Option<TemporarySpillDir>), String> {
    if let Some(path) = requested {
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        return Ok((path, None));
    }
    let temporary = TemporarySpillDir::create_in(temporary_parent)?;
    let path = temporary.path().to_path_buf();
    Ok((path, Some(temporary)))
}

fn default_spill_parent() -> PathBuf {
    #[cfg(unix)]
    {
        let persistent_temp = PathBuf::from("/var/tmp");
        if persistent_temp.is_dir() {
            return persistent_temp;
        }
    }
    std::env::temp_dir()
}

/// Rust 분류기를 학습해 모델과 재현 가능한 holdout 명단을 저장한다.
pub(super) fn run(args: &Args, io: &mut CliIo<'_>, deps: &Deps) -> Result<i32, String> {
    let corpus_path = args.path("--corpus", "tmp/classifier/corpus.jsonl");
    log::info!("학습 corpus 적재 시작 {}", training::rss_note());
    let mut docs = data::indexed_corpus(&corpus_path)?;
    log::info!("학습 corpus 적재 완료: {}건 {}", docs.len(), training::rss_note());
    if !docs.has_publisher() {
        writeln!(io.err, "WARNING 학습 데이터에 publisher 가 하나도 없다. 'collect --with-publisher' 로 다시 모으면 전집 판정이 좋아진다.").map_err(|e| e.to_string())?;
    }
    let mut cfg = args.get("--config").map(|p| config::load_config(&PathBuf::from(p))).or_else(|| deps.config.clone()).unwrap_or_else(config::load_config_from_env);
    for (option, pointer) in [
        ("--min-per-category", "/corpus/min_per_category"),
        ("--C", "/model/C"),
        ("--class-weight", "/model/class_weight"),
        ("--holdout", "/holdout"),
        ("--max-iter", "/model/max_iter"),
        ("--n-jobs", "/model/n_jobs"),
    ] {
        if let Some(raw) = args.get(option) {
            if option == "--class-weight" && raw == "none" {
                continue;
            }
            let value = if option == "--class-weight" {
                json!(raw)
            } else if option == "--n-jobs" {
                json!(raw.parse::<i64>().map_err(|e| e.to_string())?)
            } else if matches!(option, "--min-per-category" | "--max-iter") {
                json!(raw.parse::<u64>().map_err(|e| e.to_string())?)
            } else {
                json!(raw.parse::<f64>().map_err(|e| e.to_string())?)
            };
            if let Some(slot) = cfg.pointer_mut(pointer) {
                *slot = value;
            }
        }
    }
    if let Some(option) = args.original.iter().rev().find(|option| matches!(option.as_str(), "--char-ngram" | "--no-char-ngram")) {
        cfg["fields"]["body_char"]["enabled"] = json!(option == "--char-ngram");
    }
    for (option, key) in [("--body-max-features", "max_features"), ("--body-min-df", "min_df"), ("--body-char-max-chars", "max_chars")] {
        if let Some(raw) = args.get(option) {
            let value = raw.parse::<u64>().map_err(|e| e.to_string())?;
            if option == "--body-char-max-chars" {
                cfg["fields"]["body_char"][key] = json!(value);
            } else {
                for field in ["body_word", "body_char"] {
                    cfg["fields"][field][key] = json!(value);
                }
            }
        }
    }
    let (spill, _temporary_spill) = resolve_spill_dir(args.get("--spill-dir").map(PathBuf::from), &default_spill_parent())?;
    let manifest = json!({"document_keys":training::holdout_document_keys_indexed(
        &docs,
        cfg["corpus"]["min_per_category"].as_u64().unwrap_or(30) as usize,
        cfg["holdout"].as_f64().unwrap_or(0.2),
        cfg["seed"].as_u64().unwrap_or(20260914),
    )});
    let opts = training::TrainOptions { accept_parent: None, spill_dir: Some(spill), release_holdout_text: false };
    let (model, report) = training::train_indexed(&mut docs, &cfg, opts).map_err(|e| e.to_string())?;
    let out = args.get("--out").map(PathBuf::from).unwrap_or_else(|| args.model());
    if out.is_file() {
        return Err("--out은 Rust 모델을 저장할 디렉토리여야 한다".into());
    }
    model.save(Some(&out)).map_err(|e| format!("모델 저장 실패: {e}"))?;
    data::save_json(&data::holdout_manifest_path(&out), &manifest)?;
    if args.flag("--save-config") {
        let config_path = out.join("config.json");
        config::save_config(&cfg, &config_path).map_err(|e| e.to_string())?;
        writeln!(io.out, "설정 저장: {}", config_path.display()).map_err(|e| e.to_string())?;
    }
    writeln!(io.out, "{}\nRust 모델 저장: {}\n홀드아웃 명단 저장: {}", training::format_report(&report), out.display(), data::holdout_manifest_path(&out).display()).map_err(|e| e.to_string())?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::{default_spill_parent, resolve_spill_dir};
    use std::fs;
    use std::path::{Path, PathBuf};

    #[test]
    fn default_spill_directory_is_temporary_and_removed_when_training_scope_ends() {
        let parent = tempfile::tempdir().unwrap();
        let (path, spill) = resolve_spill_dir(None, parent.path()).unwrap();
        assert!(path.is_dir());
        assert!(spill.is_some());
        fs::write(path.join("matrix"), b"temporary data").unwrap();

        drop(spill);

        assert!(!path.exists());
        assert!(parent.path().is_dir());
    }

    #[test]
    fn requested_spill_directory_is_preserved() {
        let parent = tempfile::tempdir().unwrap();
        let requested = parent.path().join("user-spill");
        let (path, spill) = resolve_spill_dir(Some(requested.clone()), parent.path()).unwrap();

        assert_eq!(path, requested);
        assert!(spill.is_none());
        assert!(path.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn default_spill_uses_var_tmp_when_available() {
        if Path::new("/var/tmp").is_dir() {
            assert_eq!(default_spill_parent(), PathBuf::from("/var/tmp"));
        }
    }
}
