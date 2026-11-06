//! `book_manager` 단위 테스트.

mod common;
#[path = "../shared/book_fixtures.rs"]
mod fx;
use common::faulty_fs::FaultyFs;
use fx::{Opts, with_deps};
use std::{fs, io};

/// `fx::env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
// TODO(common): `book_fixtures::Opts`가 Deps 설정 훅(`configure: Option<Box<dyn FnOnce(BookManagerDeps) -> BookManagerDeps>>`)을 받으면 이 헬퍼가 필요 없다.
#[tokio::test]
async fn test_os_error_while_saving_reports_the_system_reason() {
    let faulty = FaultyFs::new();
    // OSError(28): 편집 파일을 백업하기 전에 동기화 단계에서 디스크 부족이 난다.
    faulty.set_sync_file(|_| Some(io::Error::from_raw_os_error(28)));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("파일 처리 실패: No space left on device"), "{error:?}");
    e.assert_nothing_changed(&original);
}

#[tokio::test]
async fn test_unexpected_error_while_saving_does_not_leak_panic_details() {
    let faulty = FaultyFs::new();
    faulty.set_sync_file(|_| panic!("internal path /x/y"));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("PDF 페이지 삭제 실패 (InternalError)"), "{error:?}");
    e.assert_nothing_changed(&original);
}

mod category_cursor {
    use base64::Engine;
    use book_manager::{decode_category_cursor, encode_category_cursor};
    use serde_json::json;

    #[test]
    fn test_encode_decode_category_cursor() {
        let cur = encode_category_cursor(&json!([1, "val"]));
        assert_eq!(decode_category_cursor(&cur), Some(json!([1, "val"])));
        assert_eq!(decode_category_cursor("invalid_base64!!!"), None);
        let not_list = base64::engine::general_purpose::URL_SAFE.encode(br#"{"a": 1}"#);
        assert_eq!(decode_category_cursor(&not_list), None);
    }
}

#[test]
fn test_from_env_with_es_accepts_a_loader_program_override() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_string_lossy().into_owned();
    let env = move |key: &str| match key {
        "TM_BOOK_DIR" => Some(root.clone()),
        "TM_LOADER_BIN" => Some("/opt/tm/loader".to_string()),
        _ => None,
    };
    assert!(book_manager::BookManager::from_env_with_es(&env, common::es_fake::FakeEsManager::new(), |deps| deps).is_ok());
}
