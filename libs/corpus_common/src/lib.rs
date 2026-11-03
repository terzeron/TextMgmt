//! 코퍼스를 다루는 도구들이 함께 쓰는 공용 코드.
//!
//! - [`reader`]: 텍스트·EPUB 본문 표본 읽기, 인코딩 판별, ZIP 열기(`open_zip`)
//! - [`util`]: Python `posixpath`/`urllib` 호환 경로·URL 도구, 카테고리 판정
//! - 병렬 풀, 천 단위 구분 같은 작은 도구들
//!
//! 원래 인코딩 손상 탐지 도구에 있던 것을 라이브러리 계층으로 옮겼다. 도구 crate끼리 서로 의존하지 않게 하려는 것이다.

pub mod reader;
pub mod util;

pub const DEFAULT_LIBRARY_ROOT: &str = "/mnt/data/text";

mod workers;
pub use workers::{DEFAULT_WORKERS, limited, pool, workers_within_memory};

pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
