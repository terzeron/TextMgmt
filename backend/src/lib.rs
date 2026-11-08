//! TextMgmt 백엔드.

// 테스트가 `backend::api_router::app()`을 쓰도록 설계했다. 바이너리 진입점은 `src/bin/backend.rs`다.
#![allow(special_module_name)]

pub mod api_router;
pub mod bookstore_result_fetcher;
pub mod stderr_logger;
