//! 원본: tests/test_classify_proposal_status_db.py
//!
//! classify_proposal_status를 실제 MySQL 8.0에 대해 검증한다. SQL 문자열이 아니라 서버가 보장하는 성질을 본다:
//! 동시에 들어온 두 시작 요청 중 하나만 성공하는가, JSON_MERGE_PATCH가 status를 건드리지 않고 필드만 합치는가,
//! 하트비트가 끊긴 작업이 조회에서만 failed로 보이고 행은 그대로인가.
//!
//! 가정한 API (아직 구현 전이다. docs/rust-test-seams.md 3.4, 3.13):
//! - `backend::category_mapping::CategoryMapping::new() -> Result<CategoryMapping, MappingError>`
//! - `CategoryMapping::get_connection(&self) -> Result<Box<dyn DbConnection>, DbError>` (구 `Connection`/`mysql::Conn`을 `DbConnection`으로 통일)
//! - `backend::db::DbConnection::{execute(&mut self, sql, &[DbValue]) -> Result<u64, DbError>, fetch_one(&mut self, sql, &[DbValue]) -> Result<Option<Row>, DbError>, commit(&mut self) -> Result<(), DbError>}`
//! - `backend::db::{DbValue, Row}`: `enum DbValue { Null, Int(i64), Text(String), DateTime(NaiveDateTime) }` (`Clone + PartialEq + Debug`, `From<&str>`, `From<i64>`, `From<Value>`),
//!   `type Row = BTreeMap<String, DbValue>`. 구 `serde_json::Value` 파라미터는 `DbValue`로 쓴다.
//! - `CategoryMapping::get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Value`
//! - `CategoryMapping::set_classify_proposal_status(&self, status: &Value, content_type: &str)`
//! - `CategoryMapping::merge_classify_proposal_status(&self, patch: &Value, content_type: &str)`
//! - `CategoryMapping::try_start_classify_proposal(&self, status: &Value, content_type: &str, stale_seconds: i64) -> (bool, Value)`
//! - `CategoryMapping::try_begin_classify_apply(&self, token: &str, content_type: &str, stale_seconds: i64) -> (bool, Value)`
//!
//! Python의 `content_type` 기본값 "book"은 매번 명시한다. `CategoryMapping`은 스레드마다 새로 만들어 각자 커넥션을 쓰게 한다(`Send` 가정).
//! `mysql_container` fixture는 대응하지 않는다. `TM_MYSQL_*`가 가리키는 MySQL 8.0이 떠 있다고 가정하고, `TM_MYSQL_HOST`가 없으면 panic한다. 모든 테스트는 `#[ignore]`이고 `--ignored`로 실행한다.
//! 테이블을 공유하므로 직렬 실행(`--test-threads=1`)을 가정한다.

mod common;

use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use backend::category_mapping::CategoryMapping;
use backend::db::DbValue;
use serde_json::{Value, json};

const STALE: i64 = 300;

/// `cm` fixture. 상태 행을 비운 CategoryMapping을 돌려준다. MySQL이 없으면 panic.
fn cm() -> CategoryMapping {
    common::setup_env();
    assert!(std::env::var_os("TM_MYSQL_HOST").is_some(), "실제 MySQL 필요: TM_MYSQL_HOST가 없다. --ignored 실행 전에 MySQL 8.0을 띄우고 TM_MYSQL_*를 설정할 것");
    let mapping = CategoryMapping::new().expect("MySQL 연결");
    let mut conn = mapping.get_connection().unwrap();
    conn.execute("TRUNCATE TABLE classify_proposal_status", &[]).unwrap();
    conn.commit().unwrap();
    mapping
}

/// updated_at을 과거로 밀어 '갱신이 끊긴' 작업을 만든다.
/// DB 서버의 시계로 민다. 로컬 시간을 쓰면 앱(KST)과 DB(UTC)의 시간대 차이가 섞여 의도와 다른 나이가 된다.
fn age_the_heartbeat(mapping: &CategoryMapping, seconds: i64, content_type: &str) {
    let mut conn = mapping.get_connection().unwrap();
    conn.execute("UPDATE classify_proposal_status SET updated_at = DATE_SUB(NOW(3), INTERVAL %s SECOND) WHERE content_type = %s", &[DbValue::from(seconds), DbValue::from(content_type)]).unwrap();
    conn.commit().unwrap();
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_status_starts_idle_when_nothing_ran_yet() {
    // 작업 이력이 없으면 idle이다. 행이 아예 없어도 예외 없이 답해야 한다.
    let cm = cm();
    assert_eq!(cm.get_classify_proposal_status("book", STALE)["status"], "idle");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_set_and_get_round_trips_korean_and_nested_values() {
    // 카운트, 한글 카테고리명, 실패 목록이 그대로 돌아온다.
    let cm = cm();
    cm.set_classify_proposal_status(
        &json!({"status": "ready", "source_category": "3_판타지", "total_count": 240, "processed_count": 240, "failures": [{"file_path": "3_판타지/책.epub", "error": "분류 제안에 실패했습니다"}]}),
        "book",
    );

    let status = cm.get_classify_proposal_status("book", STALE);

    assert_eq!(status["status"], "ready");
    assert_eq!(status["source_category"], "3_판타지");
    assert_eq!(status["total_count"], 240);
    assert_eq!(status["failures"][0]["error"], "분류 제안에 실패했습니다");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_merge_updates_counts_without_touching_status() {
    // 진행률 갱신이 status를 건드리면 안 된다.
    // 제안(running)과 적용(applying)이 같은 행을 쓴다. 진행률이 status를 되돌리면 적용 중인 작업이 화면에 '제안 생성 중'으로 보인다.
    // 호출자가 status를 실어 보내더라도 무시해야 한다.
    let cm = cm();
    cm.set_classify_proposal_status(&json!({"status": "applying", "source_category": "3_판타지", "total_count": 240}), "book");

    cm.merge_classify_proposal_status(&json!({"processed_count": 52, "status": "running"}), "book");

    let status = cm.get_classify_proposal_status("book", STALE);
    assert_eq!(status["status"], "applying", "진행률 갱신이 status를 되돌렸다");
    assert_eq!(status["processed_count"], 52);
    assert_eq!(status["source_category"], "3_판타지", "합치는 대신 통째로 덮어썼다");
    assert_eq!(status["total_count"], 240);
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_stale_running_job_reads_as_failed_without_changing_the_row() {
    // 하트비트가 끊긴 running은 조회에서만 failed로 보이고 행은 그대로다.
    // 조회가 행을 고치면 GET 요청이 쓰기가 되고, 여러 클라이언트가 동시에 폴링할 때 서로의 쓰기와 경합한다.
    let cm = cm();
    cm.set_classify_proposal_status(&json!({"status": "running", "source_category": "3_판타지"}), "book");
    age_the_heartbeat(&cm, STALE + 60, "book");

    assert_eq!(cm.get_classify_proposal_status("book", STALE)["status"], "failed");

    let row = cm.get_connection().unwrap().fetch_one("SELECT status FROM classify_proposal_status WHERE content_type = 'book'", &[]).unwrap().unwrap();
    assert_eq!(row["status"], DbValue::from("running"), "조회가 행을 고쳤다");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_start_is_rejected_while_another_job_runs() {
    // 도는 작업이 있으면 새 제안을 시작하지 못하고, 도는 작업의 상태를 돌려준다.
    let cm = cm();
    let (started, _) = cm.try_start_classify_proposal(&json!({"status": "running", "source_category": "3_판타지"}), "book", STALE);
    assert!(started);

    let (started_again, current) = cm.try_start_classify_proposal(&json!({"status": "running", "source_category": "5_에세이"}), "book", STALE);

    assert!(!started_again);
    assert_eq!(current["source_category"], "3_판타지");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_start_takes_over_a_job_whose_heartbeat_died() {
    // pod가 죽어 하트비트가 끊긴 작업은 새 작업이 이어받을 수 있어야 한다. 이게 없으면 한 번 죽은 작업이 그 카테고리를 영영 막는다.
    let cm = cm();
    cm.try_start_classify_proposal(&json!({"status": "running", "source_category": "3_판타지"}), "book", STALE);
    age_the_heartbeat(&cm, STALE + 60, "book");

    let (started, current) = cm.try_start_classify_proposal(&json!({"status": "running", "source_category": "5_에세이"}), "book", STALE);

    assert!(started);
    assert_eq!(current["source_category"], "5_에세이");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_apply_is_accepted_only_when_no_job_is_running() {
    // 승인은 ready뿐 아니라 done, failed에서도 받는다.
    // 승인 도중 중단되면 상태가 applying으로 남았다가 하트비트 만료로 failed가 된다. 그때도 남은 pending 행을 이어서 승인할 수 있어야 한다.
    // parametrize: (current_status, expected) 여섯 케이스. 케이스마다 상태 행을 비우고 다시 시작한다.
    let cases = [("ready", true), ("done", true), ("failed", true), ("idle", false), ("running", false), ("applying", false)];
    for (current_status, expected) in cases {
        let cm = cm();
        cm.set_classify_proposal_status(&json!({"status": current_status, "source_category": "3_판타지"}), "book");

        let (begun, _) = cm.try_begin_classify_apply("token-1", "book", STALE);

        assert_eq!(begun, expected, "current_status={current_status}");
    }
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_apply_keeps_the_proposal_counts_it_found() {
    // 적용을 선점해도 제안 단계가 남긴 카운트는 지워지지 않는다. 이걸 덮어쓰면 화면 헤더가 제안 진행률 대신 승인 건수를 보여준다.
    let cm = cm();
    cm.set_classify_proposal_status(&json!({"status": "ready", "source_category": "3_판타지", "total_count": 240, "processed_count": 240}), "book");

    let (begun, status) = cm.try_begin_classify_apply("token-1", "book", STALE);

    assert!(begun);
    assert_eq!(status["status"], "applying");
    assert_eq!(status["apply_token"], "token-1");
    assert_eq!(status["total_count"], 240);
    assert_eq!(status["source_category"], "3_판타지");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_only_one_of_two_simultaneous_starts_wins() {
    // 이 변경의 핵심이다. 두 프로세스가 같은 순간에 시작해도 하나만 성공한다.
    // 각자 자기 커넥션을 쓰는 스레드 두 개로 실제 경합을 만들어 확인한다.
    let _fixture = cm();
    let barrier = Arc::new(Barrier::new(2));
    let results = Arc::new(Mutex::new(Vec::<bool>::new()));

    let threads: Vec<_> = ["3_판타지", "5_에세이"]
        .into_iter()
        .map(|category| {
            let (barrier, results) = (barrier.clone(), results.clone());
            thread::spawn(move || {
                let mapping = CategoryMapping::new().expect("MySQL 연결");
                barrier.wait();
                let (started, _) = mapping.try_start_classify_proposal(&json!({"status": "running", "source_category": category}), "book", STALE);
                results.lock().unwrap().push(started);
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }

    let mut results = results.lock().unwrap().clone();
    results.sort();
    assert_eq!(results, vec![false, true], "동시 시작이 둘 다 통과했다: {results:?}");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_only_one_of_two_simultaneous_applies_wins() {
    // 승인도 마찬가지다. 둘 다 통과하면 같은 파일을 두 작업이 옮기려 든다.
    let cm = cm();
    cm.set_classify_proposal_status(&json!({"status": "ready", "source_category": "3_판타지"}), "book");
    let barrier = Arc::new(Barrier::new(2));
    let results = Arc::new(Mutex::new(Vec::<bool>::new()));

    let threads: Vec<_> = ["token-a", "token-b"]
        .into_iter()
        .map(|token| {
            let (barrier, results) = (barrier.clone(), results.clone());
            thread::spawn(move || {
                let mapping = CategoryMapping::new().expect("MySQL 연결");
                barrier.wait();
                let (begun, _) = mapping.try_begin_classify_apply(token, "book", STALE);
                results.lock().unwrap().push(begun);
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }

    let mut results = results.lock().unwrap().clone();
    results.sort();
    assert_eq!(results, vec![false, true], "동시 승인이 둘 다 통과했다: {results:?}");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_status_is_isolated_by_content_type() {
    // book과 comic은 서로의 상태를 건드리지 않는다.
    let cm = cm();
    cm.set_classify_proposal_status(&json!({"status": "running", "source_category": "3_판타지"}), "book");
    cm.set_classify_proposal_status(&json!({"status": "ready", "source_category": "만화"}), "comic");

    assert_eq!(cm.get_classify_proposal_status("book", STALE)["status"], "running");
    assert_eq!(cm.get_classify_proposal_status("comic", STALE)["source_category"], "만화");
}
