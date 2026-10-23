//! 만화 디렉토리 이름을 안의 파일 이름과 서점 검색 결과로 `[저자] 제목 (1-10권 완)` 꼴로 정리한다.
//! 원본: `utils/normalize_comic_directories.py`.
//!
//! 서점 조회는 `bookstore` crate(HTTPS, `rustls`)를 쓴다. `--offline`이면 캐시(JSONL)만 쓰고,
//! 캐시에 없는 질의는 `SKIP_NOT_CACHED`로 건너뛴다.

pub mod cache;
pub mod cli;
pub mod names;
pub mod plan;

pub use cli::{print_usage, run, run_with_searcher};
