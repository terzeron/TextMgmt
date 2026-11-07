//! `db` 테스트: Diesel(MySQL) 위의 드라이버 추상화(열 이름 맵 행, `?` 자리표시자).
//! 단위 테스트는 서버 없이 돈다. 통합 테스트는 일회용 MySQL 컨테이너를 fixture가 띄운다(Docker 필요).

use crate::mysql_fixture;

use std::collections::BTreeMap;

use chrono::NaiveDate;
use db::{DbError, DbValue, MysqlConfig, Row, connect_from_env, mysql_url};

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: BTreeMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |k| map.get(k).cloned()
}

mod values {
    use super::*;

    #[test]
    fn conversions_from_rust_types() {
        assert_eq!(DbValue::from("가나"), DbValue::Text("가나".into()));
        assert_eq!(DbValue::from(String::from("x")), DbValue::Text("x".into()));
        assert_eq!(DbValue::from(7i64), DbValue::Int(7));
        assert_eq!(DbValue::from(7i32), DbValue::Int(7));
        assert_eq!(DbValue::from(None::<&str>), DbValue::Null);
        assert_eq!(DbValue::from(Some("a")), DbValue::Text("a".into()));
        let dt = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap().and_hms_opt(3, 4, 5).unwrap();
        assert_eq!(DbValue::from(dt), DbValue::DateTime(dt));
    }

    #[test]
    fn json_values_convert_to_query_parameters() {
        use serde_json::json;
        assert_eq!(DbValue::from(json!(null)), DbValue::Null);
        assert_eq!(DbValue::from(json!(true)), DbValue::Int(1));
        assert_eq!(DbValue::from(json!(false)), DbValue::Int(0));
        assert_eq!(DbValue::from(json!(42)), DbValue::Int(42));
        assert_eq!(DbValue::from(json!("가")), DbValue::Text("가".into()));
        assert_eq!(DbValue::from(json!(["a", 1])), DbValue::Text("[\"a\",1]".into()));
        // i64 범위를 넘는 정수는 잘리지 않게 문자열로 둔다.
        assert_eq!(DbValue::from(json!(18446744073709551615u64)), DbValue::Text("18446744073709551615".into()));
    }

    #[test]
    fn accessors_return_the_matching_variant_only() {
        assert_eq!(DbValue::Int(3).as_i64(), Some(3));
        assert_eq!(DbValue::Text("3".into()).as_i64(), None);
        assert_eq!(DbValue::Text("a".into()).as_str(), Some("a"));
        assert_eq!(DbValue::Null.as_str(), None);
        assert!(DbValue::Null.is_null());
    }
}

mod config {
    use super::*;

    #[test]
    fn defaults_match_the_documented_values() {
        let c = MysqlConfig::from_env(&env_of(&[]), "tmuser");
        assert_eq!((c.host.as_str(), c.port, c.database.as_str(), c.user.as_str(), c.password.as_str()), ("localhost", 3306, "textmanager", "tmuser", ""));
    }

    #[test]
    fn environment_overrides_every_field() {
        let env = env_of(&[("TM_MYSQL_HOST", "db.local"), ("TM_MYSQL_PORT", "3307"), ("TM_MYSQL_DATABASE", "d"), ("TM_MYSQL_USER", "u"), ("TM_MYSQL_PASSWORD", "p")]);
        let c = MysqlConfig::from_env(&env, "ignored");
        assert_eq!((c.host.as_str(), c.port, c.database.as_str(), c.user.as_str(), c.password.as_str()), ("db.local", 3307, "d", "u", "p"));
    }

    #[test]
    fn a_non_numeric_port_falls_back_to_the_default() {
        let c = MysqlConfig::from_env(&env_of(&[("TM_MYSQL_PORT", "abc")]), "u");
        assert_eq!(c.port, 3306);
    }

    #[test]
    fn url_percent_encodes_credentials() {
        let c = MysqlConfig { host: "h".into(), port: 3306, database: "d".into(), user: "u@x".into(), password: "p:/ w?#%".into() };
        assert_eq!(mysql_url(&c), "mysql://u%40x:p%3A%2F%20w%3F%23%25@h:3306/d");
    }

    #[test]
    fn localhost_uses_tcp() {
        // libmysqlclient는 호스트가 "localhost"이면 유닉스 소켓을 쓴다. 항상 TCP로 붙게 맞춘다.
        let c = MysqlConfig { host: "localhost".into(), port: 3306, database: "d".into(), user: "u".into(), password: "p".into() };
        assert_eq!(mysql_url(&c), "mysql://u:p@127.0.0.1:3306/d");
        let other = MysqlConfig { host: "db.internal".into(), ..c };
        assert_eq!(mysql_url(&other), "mysql://u:p@db.internal:3306/d");
    }

    #[test]
    fn debug_output_hides_the_password() {
        let c = MysqlConfig { host: "h".into(), port: 1, database: "d".into(), user: "u".into(), password: "secret-pw".into() };
        assert!(!format!("{c:?}").contains("secret-pw"));
    }
}

mod errors {
    use super::*;

    #[test]
    fn an_unreachable_server_is_a_db_error_not_a_panic() {
        let env = env_of(&[("TM_MYSQL_HOST", "127.0.0.1"), ("TM_MYSQL_PORT", "1"), ("TM_MYSQL_DATABASE", "d"), ("TM_MYSQL_USER", "u"), ("TM_MYSQL_PASSWORD", "p")]);
        let err = connect_from_env(&env, "u").map(|_| ()).unwrap_err();
        assert!(matches!(err, DbError::Other(_)), "{err:?}");
    }

    #[test]
    fn display_distinguishes_integrity_and_other_errors() {
        assert_eq!(DbError::Constraint("duplicate".into()).to_string(), "무결성 오류: duplicate");
        assert_eq!(DbError::Other("offline".into()).to_string(), "DB 오류: offline");
    }
}

/// 컨테이너에 접속하는 환경을 만든다.
fn container_conn() -> Box<dyn db::DbConnection> {
    mysql_fixture::mysql_container();
    connect_from_env(&|k| std::env::var(k).ok(), "testuser").expect("컨테이너 접속")
}

fn row(pairs: &[(&str, DbValue)]) -> Row {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

mod mysql {
    use super::*;

    #[test]
    fn round_trip_returns_json_rows_with_column_types() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        conn.execute("DROP TABLE IF EXISTS db_rt", &[]).unwrap();
        conn.execute("CREATE TABLE db_rt (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(50), note TEXT NULL, at DATETIME NULL)", &[]).unwrap();
        let at = NaiveDate::from_ymd_opt(2024, 5, 6).unwrap().and_hms_opt(7, 8, 9).unwrap();
        assert_eq!(conn.execute("INSERT INTO db_rt (name, note, at) VALUES (?, ?, ?)", &["한글 이름".into(), DbValue::Null, at.into()]).unwrap(), 1);
        let rows = conn.fetch_all("SELECT id, name, note, at FROM db_rt", &[]).unwrap();
        assert_eq!(rows, vec![row(&[("id", DbValue::Int(1)), ("name", "한글 이름".into()), ("note", DbValue::Null), ("at", DbValue::DateTime(at))])]);
        conn.commit().unwrap();
        conn.close();
    }

    #[test]
    fn execute_batch_sums_rows_and_fetch_one_takes_the_first_row() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        conn.execute("DROP TABLE IF EXISTS db_one", &[]).unwrap();
        conn.execute("CREATE TABLE db_one (n INT)", &[]).unwrap();
        assert_eq!(conn.execute_batch("INSERT INTO db_one (n) VALUES (?)", &[vec![1.into()], vec![2.into()]]).unwrap(), 2);
        assert_eq!(conn.fetch_one("SELECT n FROM db_one ORDER BY n", &[]).unwrap(), Some(row(&[("n", DbValue::Int(1))])));
        assert_eq!(conn.fetch_one("SELECT n FROM db_one WHERE n > 5", &[]).unwrap(), None);
        conn.commit().unwrap();
    }

    #[test]
    fn duplicate_key_is_an_integrity_error() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        conn.execute("DROP TABLE IF EXISTS db_dup", &[]).unwrap();
        conn.execute("CREATE TABLE db_dup (k VARCHAR(10) PRIMARY KEY)", &[]).unwrap();
        conn.execute("INSERT INTO db_dup VALUES (?)", &["a".into()]).unwrap();
        let err = conn.execute("INSERT INTO db_dup VALUES (?)", &["a".into()]).unwrap_err();
        assert!(matches!(err, DbError::Constraint(_)), "{err:?}");
        conn.rollback().unwrap();
    }

    #[test]
    fn rollback_discards_uncommitted_rows_and_commit_keeps_them() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        conn.execute("DROP TABLE IF EXISTS db_tx", &[]).unwrap();
        conn.execute("CREATE TABLE db_tx (n INT)", &[]).unwrap();
        conn.execute("INSERT INTO db_tx VALUES (1)", &[]).unwrap();
        conn.rollback().unwrap();
        assert_eq!(conn.fetch_one("SELECT COUNT(*) AS c FROM db_tx", &[]).unwrap(), Some(row(&[("c", DbValue::Int(0))])));
        conn.execute("INSERT INTO db_tx VALUES (2)", &[]).unwrap();
        conn.commit().unwrap();
        // 새 연결에서도 보여야 커밋된 것이다.
        let mut other = container_conn();
        assert_eq!(other.fetch_all("SELECT n FROM db_tx", &[]).unwrap(), vec![row(&[("n", DbValue::Int(2))])]);
    }

    #[test]
    fn a_bad_statement_is_a_db_error() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        let err = conn.fetch_all("SELECT * FROM no_such_table_xyz", &[]).unwrap_err();
        assert!(matches!(err, DbError::Other(_)), "{err:?}");
    }

    #[test]
    fn update_reports_matched_row_count() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        conn.execute("DROP TABLE IF EXISTS db_upd", &[]).unwrap();
        conn.execute("CREATE TABLE db_upd (n INT)", &[]).unwrap();
        conn.execute_batch("INSERT INTO db_upd VALUES (?)", &[vec![1.into()], vec![1.into()], vec![2.into()]]).unwrap();
        assert_eq!(conn.execute("DELETE FROM db_upd WHERE n=?", &[1.into()]).unwrap(), 2);
        conn.commit().unwrap();
    }

    #[test]
    fn session_time_zone_follows_the_server() {
        // Diesel은 연결할 때 time_zone을 +00:00으로 바꾼다. 서버 값을 따라야 TIMESTAMP를 읽는 결과가 맞다.
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        let r = conn.fetch_one("SELECT @@session.time_zone AS s, @@global.time_zone AS g", &[]).unwrap().unwrap();
        assert_eq!(r["s"], r["g"]);
    }

    #[test]
    fn remaining_mysql_column_types_convert_to_json_values() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        conn.execute("DROP TABLE IF EXISTS db_types", &[]).unwrap();
        conn.execute("CREATE TABLE db_types (real_value DOUBLE, timestamp_value TIMESTAMP, date_value DATE, time_value TIME, decimal_value DECIMAL(10,2))", &[]).unwrap();
        conn.execute("INSERT INTO db_types VALUES (1.25, '2024-05-06 07:08:09', '2024-05-06', '07:08:09', 12.34)", &[]).unwrap();
        let date_time = NaiveDate::from_ymd_opt(2024, 5, 6).unwrap().and_hms_opt(0, 0, 0).unwrap();
        let timestamp = NaiveDate::from_ymd_opt(2024, 5, 6).unwrap().and_hms_opt(7, 8, 9).unwrap();
        assert_eq!(
            conn.fetch_one("SELECT * FROM db_types", &[]).unwrap(),
            Some(row(&[
                ("real_value", DbValue::Text("1.25".into())),
                ("timestamp_value", DbValue::DateTime(timestamp)),
                ("date_value", DbValue::DateTime(date_time)),
                ("time_value", DbValue::Text("07:08:09".into())),
                ("decimal_value", DbValue::Text("12.34".into())),
            ])),
        );
        conn.rollback().unwrap();
    }

    #[test]
    fn closed_connection_rejects_transaction_commands() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        conn.close();
        assert!(matches!(conn.commit(), Err(DbError::Other(message)) if message == "닫힌 연결이다"));
    }

    #[test]
    fn connection_execute_and_fetch_one() {
        let _g = mysql_fixture::db_lock();
        let mut conn = container_conn();
        conn.execute("DROP TABLE IF EXISTS db_conn_helpers", &[]).unwrap();
        conn.execute("CREATE TABLE db_conn_helpers (n INT)", &[]).unwrap();
        assert_eq!(conn.execute("INSERT INTO db_conn_helpers VALUES (?), (?)", &[1.into(), 2.into()]).unwrap(), 2);
        assert_eq!(conn.fetch_one("SELECT n FROM db_conn_helpers ORDER BY n DESC", &[]).unwrap(), Some(row(&[("n", DbValue::Int(2))])));
        assert_eq!(conn.fetch_one("SELECT n FROM db_conn_helpers WHERE n=?", &[99.into()]).unwrap(), None);
        conn.commit().unwrap();
    }
}
