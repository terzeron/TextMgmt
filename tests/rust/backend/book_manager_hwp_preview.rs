//! 원본: tests/test_loader.py (후반부) `TestBookManagerPreviewEmptyContent`
//!
//! book_manager HWP preview에서 빈 결과 시 안내 메시지를 돌려주는지 확인한다.
//! 원본이 loader 테스트 파일에 있지만 대상은 BookManager라 backend 테스트로 둔다.
//!
//! 설계: docs/rust-test-seams.md 3.6. 구 `set_convert_with_libreoffice_hook`(정적 훅)은 `BookManagerDeps::with_runner(FakeRunner)`로 바뀌었다.
//! "변환기가 빈 문자열을 돌려준다"는 `common::runner::FakeRunner::office_output("")`다.
//!
//! 가정한 API (실제 crate에 없는 것만):
//! - `BookManager::with_deps(BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>).with_runner(Arc<dyn CommandRunner>))` (`tests/rust/backend/book_manager_gaps.rs`와 같다)
//! - `async manager.get_book_preview(i64, chapters) -> BookResponse` (`status_code: u16`, `body: Vec<u8>`. 구 `PreviewResponse.body: String`은 UTF-8 본문이다)
//!
//! 이 테스트는 실제 HWP 샘플(`tests/books/_hwp/v1.20_부하의약혼녀.hwp`)이 필요해서 `#[ignore]`다. `--ignored`로 돌렸는데 샘플이 없으면 실패한다.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use backend::book_manager::{BookManager, BookManagerDeps};
use backend::main::BookManagerApi;
use common::es_fake::FakeEsManager;
use common::runner::FakeRunner;
use serde_json::{Value, json};

fn hwp_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|a| a.join("tests/books/_hwp")).find(|p| p.exists()).unwrap_or_else(|| PathBuf::from("tests/books/_hwp"))
}

/// Python `make_doc(path, file_type)`.
fn make_doc(rel_path: &str, file_type: &str) -> Value {
    json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
}

mod test_book_manager_preview_empty_content {
    use super::*;

    // book_manager HWP preview에서 빈 결과 시 안내 메시지 반환

    #[tokio::test]
    async fn test_hwp_preview_empty_returns_message() {
        // LO+hwp3 모두 빈 결과 -> '미리보기를 생성할 수 없습니다' 메시지
        let hwp_src = hwp_dir().join("v1.20_부하의약혼녀.hwp");
        assert!(hwp_src.exists(), "HWP 샘플 파일이 필요하다: {}", hwp_src.display());
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("A")).unwrap();
        fs::copy(&hwp_src, dir.path().join("A").join("old.hwp")).unwrap();

        let doc = make_doc("A/old.hwp", "hwp");
        // libreoffice 변환기가 빈 문자열을 돌려준다
        let deps = BookManagerDeps::new(dir.path().to_path_buf(), FakeEsManager::with_doc(doc)).with_runner(FakeRunner::office_output(""));
        let manager = BookManager::with_deps(deps);
        let resp = BookManagerApi::get_book_preview(&manager, 1, Some(1), None).await;
        assert_eq!(resp.status_code, 200);
        assert!(String::from_utf8_lossy(&resp.body).contains("미리보기를 생성할 수 없습니다"));
    }
}
