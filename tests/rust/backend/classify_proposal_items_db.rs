//! 원본: tests/test_classify_proposal_items_db.py
//!
//! classify_proposal_items 테이블을 실제 MySQL 8.0에 대해 검증한다. fake cursor로는 CREATE TABLE의 JSON 컬럼/인덱스 문법,
//! utf8mb4 한글 왕복, 100건 배치의 꼬리 유실을 증명할 수 없다.
//!
//! 가정한 API (아직 구현 전이다. docs/rust-test-seams.md 3.4, 3.13):
//! - `backend::category_mapping::CategoryMapping::new() -> Result<CategoryMapping, DbError>`  (생성자가 `_init_db`로 테이블을 만들고 마이그레이션한다)
//! - `CategoryMapping::get_connection(&self) -> Result<MappingConnection, DbError>` (`execute`/`fetch_all`/`fetch_one`/`commit`을 가진 래퍼)  (`_get_connection`. 구 `Connection`/`mysql::Conn`을 `DbConnection`으로 통일)
//! - `MappingConnection` 편의 메서드:
//!   `execute(&mut self, sql: &str, params: &[DbValue]) -> Result<u64, DbError>`, `fetch_all(.., &[DbValue]) -> Result<Vec<Row>, DbError>`,
//!   `fetch_one(.., &[DbValue]) -> Result<Option<Row>, DbError>`, `commit(&mut self) -> Result<(), DbError>`
//! - `backend::db::{DbValue, Row}`: `enum DbValue { Null, Int(i64), Text(String), DateTime(NaiveDateTime) }` (`Clone + PartialEq + Debug`, `From<&str>`, `From<i64>`, `From<Value>`),
//!   `type Row = BTreeMap<String, DbValue>`. 구 `serde_json::Value` 파라미터는 `DbValue`로 쓰고, 행 값은 `DbValue::Text`로 읽는다.
//! - `CategoryMapping::add_classify_proposal_items(&self, items: &[Value], source_category: &str, content_type: &str)`
//! - `CategoryMapping::get_classify_proposal_items(&self, content_type: &str) -> Vec<Value>`
//! - `CategoryMapping::update_classify_proposal_item_status(&self, file_path: &str, status: &str, error: Option<&str>, content_type: &str)`
//! - `CategoryMapping::update_classify_proposal_item_payloads(&self, items: &[Value], content_type: &str)`
//! - `CategoryMapping::clear_classify_proposal_items(&self, content_type: &str)`
//! - `CategoryMapping::delete_applied_classify_proposal_items(&self, content_type: &str) -> u64`
//!
//! Python의 `content_type` 기본값 "book"은 Rust에 기본 인자가 없어 매번 명시한다.
//! `cm()`이 `common/mysql_fixture.rs`로 일회용 MySQL 컨테이너를 띄우고 `TM_MYSQL_*`를 설정한다(Docker가 없으면 panic).
//! `SHOW CREATE TABLE`/`EXPLAIN`은 준비된 문장이 결과 열 정보를 주지 않아 `mysql` crate(dev-dependency)의 텍스트 프로토콜로 읽는다. 실제 MySQL이 필요하므로 모든 테스트는 `#[ignore]`이고 `--ignored`로 실행한다. 테이블을 공유하므로 직렬 실행(`--test-threads=1`)을 가정한다.

mod common;
#[path = "common/mysql_fixture.rs"]
mod mysql_fixture;

use backend::category_mapping::CategoryMapping;

use backend::db::{DbValue, Row};
use serde_json::{Value, json};

/// 행의 `Text` 컬럼 값. Python은 dict 값을 그대로 문자열로 썼다.
fn text(row: &Row, key: &str) -> String {
    match &row[key] {
        DbValue::Text(s) => s.clone(),
        other => panic!("{key}가 문자열이 아니다: {other:?}"),
    }
}

/// `cm` fixture. 대상 테이블을 비운 CategoryMapping을 돌려준다. MySQL이 없으면 panic.
fn cm() -> CategoryMapping {
    common::setup_env();
    // 일회용 MySQL 컨테이너를 세션당 한 번 띄우고 TM_MYSQL_*를 설정한다(Python `mysql_container` fixture). Docker가 없으면 panic한다.
    mysql_fixture::mysql_container();
    let mapping = CategoryMapping::new().expect("MySQL 연결");
    let mut conn = mapping.get_connection().unwrap();
    conn.execute("TRUNCATE TABLE classify_proposal_items", &[]).unwrap();
    conn.commit().unwrap();
    mapping
}

fn files(paths: impl IntoIterator<Item = String>) -> Vec<Value> {
    paths.into_iter().map(|p| json!({"file_path": p})).collect()
}

fn index_names(cm: &CategoryMapping) -> Vec<String> {
    let mut conn = cm.get_connection().unwrap();
    let rows = conn.fetch_all("SELECT index_name AS idx FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'classify_proposal_items'", &[]).unwrap();
    rows.iter().map(|r| text(r, "idx")).collect()
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_create_table_statement_runs_on_real_mysql() {
    // CREATE TABLE IF NOT EXISTS classify_proposal_items가 MySQL 8.0에서 실제로 성공했는지 확인한다.
    // JSON 컬럼, 두 개의 복합/단일 인덱스, VARCHAR(1024)는 fake cursor로는 문법 오류를 잡을 수 없다.
    // 원본은 `SHOW CREATE TABLE` 문자열을 읽는다. 준비된 문장으로는 읽을 수 없어서 같은 사실을 information_schema에서 읽는다.
    let cm = cm();
    let mut conn = cm.get_connection().unwrap();
    let rows = conn.fetch_all("SELECT column_name AS name, column_type AS ty FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'classify_proposal_items'", &[]).unwrap();
    let types: Vec<String> = rows.iter().map(|r| text(r, "ty").to_lowercase()).collect();

    assert!(types.iter().any(|t| t == "json"), "JSON 컬럼이 없다: {types:?}");
    assert!(types.iter().any(|t| t == "varchar(1024)"), "VARCHAR(1024) 컬럼이 없다: {types:?}");
    let names = index_names(&cm);
    assert!(names.contains(&"idx_content_source".to_string()));
    assert!(names.contains(&"idx_content_file".to_string()));
}

/// 옵티마이저 trace에서 고른 범위 접근 계획(`chosen_range_access_summary.range_access_plan`)을 모두 찾는다.
fn chosen_range_plans<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
    match node {
        Value::Object(map) => {
            if let Some(plan) = map.get("chosen_range_access_summary").and_then(|v| v.get("range_access_plan")) {
                out.push(plan);
            }
            map.values().for_each(|v| chosen_range_plans(v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| chosen_range_plans(v, out)),
        _ => {}
    }
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_status_update_uses_an_index_instead_of_scanning_the_table() {
    // 승인이 책 한 권마다 부르는 UPDATE가 인덱스를 타는지 확인한다.
    // 실행 시간은 장비마다 달라 고정할 수 없으므로 "전수 훑기가 아니라 인덱스 조회를 고른다"는 실행 계획 자체를 잠근다.
    // 원본은 `EXPLAIN UPDATE`의 key/type을 읽는다. 준비된 문장으로는 EXPLAIN을 읽을 수 없어서, 같은 세션에서
    // 옵티마이저 trace(information_schema.OPTIMIZER_TRACE)로 실제 실행 계획이 고른 접근 경로를 읽는다.
    let cm = cm();
    cm.add_classify_proposal_items(&files((0..50).map(|i| format!("3_판타지/책 {i}.epub"))), "3_판타지", "book");

    let mut conn = cm.get_connection().unwrap();
    conn.execute("SET optimizer_trace = 'enabled=on'", &[]).unwrap();
    conn.execute("UPDATE classify_proposal_items SET apply_status = 'moved' WHERE content_type = 'book' AND file_path = '3_판타지/책 7.epub'", &[]).unwrap();
    let row = conn.fetch_one("SELECT trace AS trace FROM information_schema.OPTIMIZER_TRACE", &[]).unwrap().expect("옵티마이저 trace가 없다");
    conn.execute("SET optimizer_trace = 'enabled=off'", &[]).unwrap();
    conn.commit().unwrap();

    let trace: Value = serde_json::from_str(&text(&row, "trace")).expect("trace JSON");
    let mut plans = Vec::new();
    chosen_range_plans(&trace, &mut plans);
    assert!(!plans.is_empty(), "고른 접근 계획을 trace에서 찾지 못했다: {trace}");
    for plan in plans {
        assert_eq!(plan["index"].as_str(), Some("idx_content_file"), "file_path 인덱스를 타지 않는다: {plan}");
        assert_ne!(plan["type"].as_str(), Some("index_scan"), "인덱스 전수 훑기를 고르고 있다: {plan}");
        assert!(plan["rows"].as_i64().is_some_and(|n| n <= 1), "한 행이 아니라 여러 행을 훑는 계획이다: {plan}");
    }
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_redundant_content_type_index_is_gone() {
    // idx_content_type (content_type)은 idx_content_source (content_type, source_category)의 왼쪽 접두사와 같아 조회를 하나도 더 처리하지 못한다.
    // INSERT마다 유지 비용만 드는 중복 인덱스라 지웠다. 다시 들어오면 실패한다.
    let cm = cm();
    let names = index_names(&cm);

    assert!(names.contains(&"idx_content_source".to_string()));
    assert!(names.contains(&"idx_content_file".to_string()));
    assert!(!names.contains(&"idx_content_type".to_string()));
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_migration_adds_file_path_index_and_drops_the_redundant_one() {
    // 이미 테이블이 만들어진 환경을 재현한다. 옛 인덱스 구성으로 되돌린 뒤 _init_db를 다시 부르면,
    // 마이그레이션이 file_path 인덱스를 붙이고 중복 인덱스를 지워야 한다.
    let cm = cm();
    let mut conn = cm.get_connection().unwrap();
    conn.execute("ALTER TABLE classify_proposal_items DROP INDEX idx_content_file", &[]).unwrap();
    conn.execute("ALTER TABLE classify_proposal_items ADD INDEX idx_content_type (content_type)", &[]).unwrap();
    conn.commit().unwrap();

    let _ = CategoryMapping::new().expect("MySQL 연결");

    let names = index_names(&cm);
    assert!(names.contains(&"idx_content_file".to_string()));
    assert!(!names.contains(&"idx_content_type".to_string()));
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_payload_round_trips_korean_nested_null_and_long_path() {
    // 한글, candidates 중첩 배열(최대 2개), null 값, 매우 긴 file_path가 add -> get을 거쳐도 그대로 돌아오는지 확인한다.
    let cm = cm();
    let long_path = format!("/mnt/data/도서/{}.epub", "가".repeat(500));
    let item = json!({
        "file_path": long_path, "title": "한국어 제목 테스트: 봄날의 기억", "target_category": null,
        "candidates": [{"category": "3_SF", "source": "bookstore", "detail": "서점 2곳 일치"}, {"category": "5_에세이", "source": "model", "detail": "모델 판정"}],
    });

    cm.add_classify_proposal_items(std::slice::from_ref(&item), "0_inbox", "book");
    let fetched = cm.get_classify_proposal_items("book");

    assert_eq!(fetched.len(), 1);
    // apply_status/apply_error는 payload가 아니라 컬럼에서 실려온다. 새로 넣은 행은 아직 아무것도 이동하지 않았으므로 기본값 pending/None이어야 한다.
    let mut expected = item;
    expected["apply_status"] = json!("pending");
    expected["apply_error"] = Value::Null;
    assert_eq!(fetched[0], expected);
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_get_returns_apply_status_and_apply_error_from_columns() {
    // get이 payload뿐 아니라 apply_status/apply_error 컬럼 값도 항목에 실어 돌려준다.
    let cm = cm();
    cm.add_classify_proposal_items(&files(["a.epub".to_string()]), "0_inbox", "book");

    cm.update_classify_proposal_item_status("a.epub", "failed", Some("파일을 찾을 수 없습니다"), "book");
    let item = cm.get_classify_proposal_items("book").remove(0);

    assert_eq!(item["apply_status"], "failed");
    assert_eq!(item["apply_error"], "파일을 찾을 수 없습니다");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_update_payload_fills_the_row_without_touching_apply_status() {
    // 분류가 끝난 항목이 기존 행의 payload만 채우고 apply_status는 건드리지 않는다.
    // 제안 시작 시 이름만 담은 행을 먼저 넣고 나중에 채우는 구조라, 채우는 쪽이 INSERT면 같은 책이 두 줄로 보인다.
    // 그리고 apply_status까지 덮으면 이미 옮긴 책의 이동 기록이 사라진다.
    let cm = cm();
    cm.add_classify_proposal_items(&[json!({"file_path": "A/a.epub", "title": "가", "grade": null}), json!({"file_path": "A/b.epub", "title": "나", "grade": null})], "A", "book");
    cm.update_classify_proposal_item_status("A/a.epub", "moved", None, "book");

    cm.update_classify_proposal_item_payloads(&[json!({"file_path": "A/a.epub", "title": "가", "grade": "certain", "target_category": "3_SF"})], "book");

    let items = cm.get_classify_proposal_items("book");
    let by_path = |p: &str| items.iter().find(|i| i["file_path"] == p).unwrap().clone();
    assert_eq!(items.len(), 2, "행이 늘어났다 — 채우기가 INSERT로 동작한다");
    assert_eq!(by_path("A/a.epub")["grade"], "certain");
    assert_eq!(by_path("A/a.epub")["target_category"], "3_SF");
    assert_eq!(by_path("A/a.epub")["apply_status"], "moved", "이동 기록을 덮어썼다");
    assert!(by_path("A/b.epub")["grade"].is_null());
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_update_payload_ignores_an_empty_batch() {
    // 비어 있는 갱신 묶음은 DB에 가지 않는다(꼬리가 없을 때 매번 왕복하지 않는다).
    let cm = cm();
    cm.add_classify_proposal_items(&[json!({"file_path": "A/a.epub", "grade": null})], "A", "book");
    cm.update_classify_proposal_item_payloads(&[], "book");
    assert!(cm.get_classify_proposal_items("book")[0]["grade"].is_null());
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_update_status_changes_only_target_row() {
    // update가 file_path + content_type으로 지정한 행만 바꾸고 다른 행은 그대로 둔다.
    let cm = cm();
    cm.add_classify_proposal_items(&files(["a.epub".to_string(), "b.epub".to_string()]), "0_inbox", "book");

    cm.update_classify_proposal_item_status("a.epub", "moved", None, "book");

    let items = cm.get_classify_proposal_items("book");
    let by_path = |p: &str| items.iter().find(|i| i["file_path"] == p).unwrap().clone();
    assert_eq!(by_path("a.epub")["apply_status"], "moved");
    assert!(by_path("a.epub")["apply_error"].is_null());
    assert_eq!(by_path("b.epub")["apply_status"], "pending");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_update_status_is_scoped_by_content_type() {
    // 같은 file_path라도 content_type이 다르면 건드리지 않는다.
    let cm = cm();
    cm.add_classify_proposal_items(&files(["same.epub".to_string()]), "0_inbox", "book");
    cm.add_classify_proposal_items(&files(["same.epub".to_string()]), "0_inbox", "comic");

    cm.update_classify_proposal_item_status("same.epub", "moved", None, "book");

    assert_eq!(cm.get_classify_proposal_items("book")[0]["apply_status"], "moved");
    assert_eq!(cm.get_classify_proposal_items("comic")[0]["apply_status"], "pending");

    // 다음 테스트를 위해 comic도 정리한다 (컨테이너는 세션 스코프로 공유된다)
    cm.clear_classify_proposal_items("comic");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_delete_applied_removes_only_moved_rows_and_returns_count() {
    // delete_applied는 apply_status='moved'인 행만 지우고, 지운 개수를 돌려준다.
    let cm = cm();
    cm.add_classify_proposal_items(&files(["a.epub".to_string(), "b.epub".to_string(), "c.epub".to_string()]), "0_inbox", "book");
    cm.update_classify_proposal_item_status("a.epub", "moved", None, "book");
    cm.update_classify_proposal_item_status("b.epub", "failed", Some("실패"), "book");
    // c.epub은 pending으로 남겨둔다

    let deleted_count = cm.delete_applied_classify_proposal_items("book");

    assert_eq!(deleted_count, 1);
    let mut remaining: Vec<String> = cm.get_classify_proposal_items("book").iter().map(|i| i["file_path"].as_str().unwrap().to_string()).collect();
    remaining.sort();
    assert_eq!(remaining, vec!["b.epub".to_string(), "c.epub".to_string()]);
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_migration_adds_apply_status_columns_to_legacy_table() {
    // apply_status/apply_error 컬럼 없는 기존 테이블을 열어도 마이그레이션이 컬럼을 추가한다.
    // 컬럼 없는 옛 스키마로 테이블을 다시 만들고, 새 인스턴스를 생성해 _init_db가 마이그레이션을 수행하게 한 뒤 컬럼이 실제로 동작하는지 확인한다.
    let cm = cm();
    let mut conn = cm.get_connection().unwrap();
    conn.execute("DROP TABLE IF EXISTS classify_proposal_items", &[]).unwrap();
    conn.execute(
        "CREATE TABLE classify_proposal_items (id INT AUTO_INCREMENT PRIMARY KEY, content_type VARCHAR(10) NOT NULL DEFAULT 'book', source_category VARCHAR(255) NOT NULL, file_path VARCHAR(1024) NOT NULL, payload JSON NOT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, INDEX idx_content_source (content_type, source_category), INDEX idx_content_type (content_type)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
        &[],
    )
    .unwrap();
    conn.execute("INSERT INTO classify_proposal_items (content_type, source_category, file_path, payload) VALUES ('book', '0_inbox', 'a.epub', '{}')", &[]).unwrap();
    conn.commit().unwrap();

    let migrated = CategoryMapping::new().expect("MySQL 연결"); // _init_db가 마이그레이션을 수행한다

    let items = migrated.get_classify_proposal_items("book");
    assert_eq!(items[0]["apply_status"], "pending");
    assert!(items[0]["apply_error"].is_null());
    migrated.update_classify_proposal_item_status("a.epub", "moved", None, "book");
    assert_eq!(migrated.get_classify_proposal_items("book")[0]["apply_status"], "moved");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_batch_of_250_across_three_adds_keeps_full_tail_in_order() {
    // 설계상 배치 크기인 100건씩(100+100+50) 나눠 넣어도 get이 250건 전부를 삽입 순서대로 돌려주는지 확인한다.
    // 마지막 잔여분(꼬리)이 누락되면 관리자가 목록 끝의 책들을 못 보고 승인하게 된다.
    let cm = cm();
    cm.add_classify_proposal_items(&files((0..100).map(|i| format!("{i}.epub"))), "0_inbox", "book");
    cm.add_classify_proposal_items(&files((100..200).map(|i| format!("{i}.epub"))), "0_inbox", "book");
    cm.add_classify_proposal_items(&files((200..250).map(|i| format!("{i}.epub"))), "0_inbox", "book");

    let items = cm.get_classify_proposal_items("book");

    assert_eq!(items.len(), 250);
    let paths: Vec<String> = items.iter().map(|i| i["file_path"].as_str().unwrap().to_string()).collect();
    let expected: Vec<String> = (0..250).map(|i| format!("{i}.epub")).collect();
    assert_eq!(paths, expected);
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_get_preserves_insertion_order_across_several_add_calls() {
    // 화면은 분류가 끝난 순서 그대로 책을 보여줘야 하므로, 여러 번의 add 호출에 걸쳐 들어간 항목도 삽입 순서를 유지한 채 조회되는지 확인한다.
    let cm = cm();
    cm.add_classify_proposal_items(&files(["z.epub".to_string()]), "0_inbox", "book");
    cm.add_classify_proposal_items(&files(["a.epub".to_string()]), "0_inbox", "book");
    cm.add_classify_proposal_items(&files(["m.epub".to_string()]), "0_inbox", "book");

    let items = cm.get_classify_proposal_items("book");

    let paths: Vec<&str> = items.iter().map(|i| i["file_path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec!["z.epub", "a.epub", "m.epub"]);
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_clear_only_removes_matching_content_type() {
    // clear는 지정한 content_type 행만 지우고 다른 content_type은 그대로 남겨야 한다.
    let cm = cm();
    cm.add_classify_proposal_items(&files(["book1.epub".to_string()]), "0_inbox", "book");
    cm.add_classify_proposal_items(&files(["comic1.epub".to_string()]), "0_inbox", "comic");

    cm.clear_classify_proposal_items("book");

    assert!(cm.get_classify_proposal_items("book").is_empty());
    let remaining = cm.get_classify_proposal_items("comic");
    let paths: Vec<&str> = remaining.iter().map(|i| i["file_path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec!["comic1.epub"]);

    // 다음 테스트를 위해 comic도 정리한다 (컨테이너는 세션 스코프로 공유된다)
    cm.clear_classify_proposal_items("comic");
}

#[test]
#[ignore = "실제 MySQL 필요: TM_MYSQL_HOST"]
fn test_get_on_empty_table_returns_empty_list_not_error() {
    // 아직 아무 제안도 만들지 않은 content_type을 조회하면 빈 리스트여야지, 에러가 나거나 None이 나오면 안 된다.
    let cm = cm();
    let items = cm.get_classify_proposal_items("book");

    assert!(items.is_empty());
}
