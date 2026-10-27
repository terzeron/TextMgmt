//! Python `tests/conftest.py`의 `mysql_container` fixture에 대응한다. 세션당 MySQL 컨테이너 하나를 띄운다.
//!
//! 가정한 API (아직 구현 전이다):
//! - `testcontainers_modules::mysql::Mysql`과 `testcontainers_modules::testcontainers::runners::SyncRunner` (blocking 기능)
//! - 컨테이너를 못 띄우면 panic한다. 이 fixture를 쓰는 테스트는 `#[ignore = "Docker 필요"]`이고 `--ignored`로 실행한다.
//!
//! 테스트 파일은 `#[path = "common/mysql_fixture.rs"] mod mysql_fixture;`로 가져온다.

#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard, OnceLock};

use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::testcontainers::runners::SyncRunner;
use testcontainers_modules::testcontainers::{Container, ImageExt};

pub const MYSQL_USER: &str = "testuser";
pub const MYSQL_PASSWORD: &str = "testpass";
pub const MYSQL_DATABASE: &str = "testdb";
pub const MYSQL_ROOT_PASSWORD: &str = "rootpass";

static CONTAINER: Mutex<Option<Container<Mysql>>> = Mutex::new(None);

unsafe extern "C" {
    fn atexit(callback: extern "C" fn()) -> i32;
}

/// 프로세스가 끝날 때 컨테이너를 지운다. `static`은 `Drop`이 돌지 않아서, 이 훅이 없으면 테스트를 돌릴 때마다 컨테이너가 하나씩 남는다.
extern "C" fn remove_container_at_exit() {
    // 정리 중 panic이 FFI 경계를 넘으면 abort하므로 여기서 막는다.
    let _ = std::panic::catch_unwind(|| {
        if let Ok(mut guard) = CONTAINER.lock() {
            guard.take();
        }
    });
}
static READY: OnceLock<bool> = OnceLock::new();
/// `TRUNCATE`로 테이블을 비우는 테스트끼리 병렬로 돌지 않게 한다. Python은 순차 실행이다.
static DB_LOCK: Mutex<()> = Mutex::new(());

/// 컨테이너를 한 번만 띄우고 `TM_MYSQL_*` 환경 변수를 설정한다. 못 띄우면 panic한다. 반환값은 항상 true다(시그니처 호환용).
pub fn mysql_container() -> bool {
    *READY.get_or_init(|| {
        let started = Mysql::default()
            .with_env_var("MYSQL_DATABASE", MYSQL_DATABASE)
            .with_env_var("MYSQL_USER", MYSQL_USER)
            .with_env_var("MYSQL_PASSWORD", MYSQL_PASSWORD)
            .with_env_var("MYSQL_ROOT_PASSWORD", MYSQL_ROOT_PASSWORD)
            .start();
        match started {
            Ok(container) => {
                let host = container.get_host().expect("컨테이너 호스트").to_string();
                let port = container.get_host_port_ipv4(3306).expect("컨테이너 포트");
                // SAFETY: OnceLock 초기화 안에서 한 번만 설정한다.
                unsafe {
                    // `common::setup_env()` 가 나중에 돌아도 컨테이너 값을 지우지 않게 표식을 남긴다(common::MYSQL_FROM_FIXTURE).
                    std::env::set_var("TM_TEST_MYSQL_FROM_FIXTURE", "1");
                    std::env::set_var("TM_MYSQL_HOST", host);
                    std::env::set_var("TM_MYSQL_PORT", port.to_string());
                    std::env::set_var("TM_MYSQL_DATABASE", MYSQL_DATABASE);
                    std::env::set_var("TM_MYSQL_USER", MYSQL_USER);
                    std::env::set_var("TM_MYSQL_PASSWORD", MYSQL_PASSWORD);
                }
                *CONTAINER.lock().unwrap() = Some(container);
                // SAFETY: `remove_container_at_exit`는 인자가 없는 `extern "C"` 함수이고 panic을 밖으로 내보내지 않는다.
                unsafe { atexit(remove_container_at_exit) };
                true
            }
            Err(e) => panic!("Docker 필요: MySQL 컨테이너를 띄울 수 없다: {e}"),
        }
    })
}

/// DB를 쓰는 테스트가 끝날 때까지 다른 DB 테스트를 막는 잠금.
pub fn db_lock() -> MutexGuard<'static, ()> {
    DB_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
