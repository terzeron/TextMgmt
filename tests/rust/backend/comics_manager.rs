//! 원본: backend/comics_manager.py (전용 Python 테스트 없음. tests/test_comics.py의 2개는 comics.rs에 있다)
//!
//! 가정한 API (comics.rs와 같다):
//! - `ComicsManager::from_env(env, es_factory) -> Result<ComicsManager, RuntimeError>`. `TM_COMICS_DIR`와 `TM_ES_COMICS_INDEX`가 없으면 `Err`,
//!   `create_index` 실패도 `Err`다(Python은 `KeyError`와 ES 예외가 그대로 올라간다).
//! - `TM_BACKFILL_CREATED_TIME_ON_STARTUP`가 1/true/yes/on(공백·대소문자 무시)이면 `ES.backfill_created_time(path_prefix)`를 데몬 스레드에서 한 번 부른다.
//!   실패는 경고만 남기고 삼킨다. 테스트는 `join_backfill()`로 스레드를 기다린다.
//! - `set_mismatch_cache(v)`는 캐시와 시각(0보다 큼)을 함께 정하고 `clear_mismatch_cache()`는 `None`과 0.0으로 되돌린다.
//! - `mismatch_state_lock()`, `mismatch_scan_lock()`은 `&Mutex<()>`다.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use backend::comics_manager::ComicsManager;
use backend::es_manager::EsManagerApi;
use common::es_fake::FakeEsManager;
use serde_json::json;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |key: &str| map.get(key).cloned()
}

fn build(pairs: &[(&str, &str)], es: &Arc<FakeEsManager>) -> Result<ComicsManager, backend::comics_manager::RuntimeError> {
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
    // scan lock을 잡은 채로 state lock을 잡을 수 있어야 한다(Python `_refresh_mismatch_cache`의 중첩 순서).
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
