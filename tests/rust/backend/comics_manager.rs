mod common;
use backend::comics_manager::ComicsManager;
use backend::es_manager::EsManagerApi;
use common::es_fake::FakeEsManager;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |key: &str| map.get(key).cloned()
}

fn build(pairs: &[(&str, &str)], es: &Arc<FakeEsManager>) -> Result<ComicsManager, backend::comics_manager::BookManagerError> {
    let env = env_of(pairs);
    let es = es.clone();
    ComicsManager::from_env(&env, &move |_| es.clone() as Arc<dyn EsManagerApi>)
}

const BASE: [(&str, &str); 2] = [("TM_COMICS_DIR", "/data/comics"), ("TM_ES_COMICS_INDEX", "idx")];

fn with_flag(flag: &str) -> Vec<(&str, &str)> {
    let mut v = BASE.to_vec();
    v.push(("TM_BACKFILL_CREATED_TIME_ON_STARTUP", flag));
    v
}

#[test]
fn test_missing_comics_dir_does_not_touch_es() {
    let es = FakeEsManager::new();
    let err = build(&[("TM_ES_COMICS_INDEX", "idx")], &es).err().expect("실패해야 한다");
    assert!(err.to_string().contains("TM_COMICS_DIR"), "{err}");
    assert_eq!(es.create_index_count(), 0);
}

#[test]
fn test_missing_index_env_is_error_before_es_calls() {
    let es = FakeEsManager::new();
    let err = build(&[("TM_COMICS_DIR", "/data/comics")], &es).err().expect("실패해야 한다");
    assert!(err.to_string().contains("TM_ES_COMICS_INDEX"), "{err}");
    assert_eq!(es.create_index_count(), 0);
}

#[test]
fn test_create_index_failure_is_error() {
    let es = FakeEsManager::new();
    es.fail("create_index", "boom");
    let err = build(&BASE, &es).err().expect("실패해야 한다");
    assert!(err.to_string().contains("boom"), "{err}");
}

#[test]
fn test_backfill_is_off_by_default() {
    let es = FakeEsManager::new();
    let m = build(&BASE, &es).unwrap();
    m.join_backfill();
    assert_eq!(es.call_count("backfill_created_time"), 0);
}

#[test]
fn test_backfill_runs_once_with_path_prefix_when_enabled() {
    for flag in ["1", "true", "YES", " On "] {
        let es = FakeEsManager::new();
        let m = build(&with_flag(flag), &es).unwrap();
        m.join_backfill();
        assert_eq!(es.calls_of("backfill_created_time"), [json!(["/data/comics"])], "flag={flag:?}");
    }
}

#[test]
fn test_backfill_ignores_falsy_flag_values() {
    for flag in ["0", "", "no", "off", "2"] {
        let es = FakeEsManager::new();
        let m = build(&with_flag(flag), &es).unwrap();
        m.join_backfill();
        assert_eq!(es.call_count("backfill_created_time"), 0, "flag={flag:?}");
    }
}

#[test]
fn test_backfill_failure_is_swallowed() {
    let es = FakeEsManager::new();
    es.fail("backfill_created_time", "backfill boom");
    let m = build(&with_flag("1"), &es).unwrap();
    m.join_backfill();
    assert_eq!(es.call_count("backfill_created_time"), 1);
}

#[test]
fn test_backfill_runs_after_create_index() {
    let es = FakeEsManager::new();
    let m = build(&with_flag("1"), &es).unwrap();
    m.join_backfill();
    let names: Vec<String> = es.calls().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["create_index", "backfill_created_time"]);
}

#[test]
fn test_mismatch_cache_set_stamps_time_and_clear_resets() {
    let es = FakeEsManager::new();
    let mut m = build(&BASE, &es).unwrap();
    m.set_mismatch_cache(json!({"n": 1}));
    assert_eq!(m.mismatch_cache(), Some(&json!({"n": 1})));
    assert!(m.mismatch_cache_time() > 0.0);
    m.clear_mismatch_cache();
    assert!(m.mismatch_cache().is_none());
    assert_eq!(m.mismatch_cache_time(), 0.0);
}

#[test]
fn test_mismatch_locks_are_independent() {
    let es = FakeEsManager::new();
    let m = build(&BASE, &es).unwrap();
    let _scan = m.mismatch_scan_lock().lock().unwrap();
    // scan lock을 잡은 채로 state lock을 잡을 수 있어야 한다.
    assert!(m.mismatch_state_lock().try_lock().is_ok());
}

#[test]
fn test_es_factory_receives_index_name_once() {
    let es = FakeEsManager::new();
    let names = Arc::new(Mutex::new(Vec::<String>::new()));
    let (es2, names2) = (es.clone(), names.clone());
    let env = env_of(&BASE);
    ComicsManager::from_env(&env, &move |n: &str| {
        names2.lock().unwrap().push(n.to_string());
        es2.clone() as Arc<dyn EsManagerApi>
    })
    .unwrap();
    assert_eq!(*names.lock().unwrap(), ["idx"]);
}

#[test]
fn test_from_env_with_applies_manager_customization() {
    let es = FakeEsManager::new();
    let env = env_of(&BASE);
    let mut customized = false;
    let manager = ComicsManager::from_env_with(&env, &|_| es.clone() as Arc<dyn EsManagerApi>, |deps| {
        customized = true;
        deps.with_bulk_reindex_batch_size(17)
    })
    .unwrap();
    assert_eq!(manager.path_prefix(), std::path::Path::new("/data/comics"));
    assert!(customized);
}

#[test]
fn test_from_env_with_es_applies_manager_customization() {
    let es = FakeEsManager::new();
    let env = env_of(&BASE);
    let mut customized = false;
    let manager = ComicsManager::from_env_with_es(&env, es.clone(), |deps| {
        customized = true;
        deps.with_bulk_reindex_batch_size(17)
    })
    .unwrap();
    assert_eq!(manager.path_prefix(), std::path::Path::new("/data/comics"));
    assert!(customized);
    assert_eq!(es.create_index_count(), 1);
}
