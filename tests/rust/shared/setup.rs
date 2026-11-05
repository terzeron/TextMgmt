//! 테스트 환경 변수 고정. 여러 crate 테스트의 공용 도구(`tests/rust/<crate>/common/mod.rs`)가 함께 쓴다.

use std::path::PathBuf;
use std::sync::Once;

static INIT: Once = Once::new();

/// 저장소 루트. `tests/rust/backend/common`에서 세 단계 위가 아니라 `CARGO_MANIFEST_DIR` 기준이다.
pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().find(|p| p.join("Cargo.toml").exists() && p.join("backend").is_dir()).expect("project root").to_path_buf()
}

/// 테스트용 환경 변수를 한 번만 설정한다.
/// `Book.path_prefix`처럼 import 시점에 읽는 값이 있어 backend를 쓰기 전에 불러야 한다.
/// 컨테이너 fixture 가 `TM_MYSQL_*` / `TM_ES_*` 를 컨테이너 값으로 설정했다는 표식. `setup_env()` 는 이 값을 지우지 않는다.
/// fixture 모듈(`mysql_fixture.rs`, `elasticsearch_fixture.rs`)도 같은 이름을 쓴다.
pub const MYSQL_FROM_FIXTURE: &str = "TM_TEST_MYSQL_FROM_FIXTURE";
pub const ES_FROM_FIXTURE: &str = "TM_TEST_ES_FROM_FIXTURE";

pub fn setup_env() {
    INIT.call_once(|| {
        let root = project_root();
        // SAFETY: Once 안에서 테스트 시작 전에 한 번만 설정한다.
        unsafe {
            std::env::set_var("TM_BOOK_DIR", root.join("tests/books"));
            std::env::set_var("TM_COMICS_DIR", root.join("tests/comics"));
            std::env::set_var("TM_FRONTEND_URL", "http://localhost:3000");
            std::env::set_var("TM_JWT_SECRET", "test_jwt_secret_for_testing_minimum_32bytes");
            std::env::set_var("TM_ADMIN_EMAIL", "admin@test.com");
            std::env::set_var("TM_ALLOWED_EMAILS", "viewer@test.com");
            if std::env::var_os("TM_ES_COMICS_INDEX").is_none() {
                std::env::set_var("TM_ES_COMICS_INDEX", "test_comics_index");
            }
            // 운영 모델 파일(165MB)을 읽지 않도록 없는 export 경로를 지정한다.
            std::env::set_var("TM_CLASSIFIER_MODEL_DIR", root.join("tests/model-dir-that-does-not-exist"));
            // 개발 셸(direnv 의 `.env`)에 든 실서비스 접속 정보가 테스트에 새지 않게 지운다. 그대로 두면 테스트가
            // 운영 MySQL·Elasticsearch 에 붙어 데이터를 바꾼다. 컨테이너 fixture 가 먼저 설정한 값(표식이 있음)은 둔다.
            if std::env::var_os(MYSQL_FROM_FIXTURE).is_none() {
                for k in ["TM_MYSQL_HOST", "TM_MYSQL_PORT", "TM_MYSQL_USER", "TM_MYSQL_PASSWORD", "TM_MYSQL_DATABASE", "TM_MYSQL_ROOT_PASSWORD"] {
                    std::env::remove_var(k);
                }
            }
            if std::env::var_os(ES_FROM_FIXTURE).is_none() {
                for k in ["TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"] {
                    std::env::remove_var(k);
                }
            }
        }
    });
}
