//! 코퍼스를 다루는 도구들이 함께 쓰는 공용 코드.
//!
//! - [`reader`]: 텍스트·EPUB 본문 표본 읽기, 인코딩 판별, ZIP 열기(`open_zip`)
//! - [`util`]: Python `posixpath`/`urllib` 호환 경로·URL 도구
//!
//! 원래 인코딩 손상 탐지 도구에 있던 것을 라이브러리 계층으로 옮겼다. 도구 crate끼리 서로 의존하지 않게 하려는 것이다.

pub mod reader;
pub mod util;
