//! 만화 디렉토리 이름을 안의 파일 이름과 서점 검색 결과로 `[저자] 제목 (1-10권 완)` 꼴로 정리한다.
//! 원본: `utils/normalize_comic_directories.py`.
//!
//! 서점 조회는 캐시 전용이다. 서점 3곳이 HTTPS이고 HTML을 파싱해야 하는데, 이 워크스페이스는
//! 내부망 http만 쓰고 HTML 파서 의존성도 없다. 캐시(JSONL)에 있는 결과로만 계획을 세우고,
//! 캐시에 없는 질의는 `SKIP_NOT_CACHED`로 건너뛴다. 새 질의는 Python 원본이 채운다.

pub mod cache;
pub mod cli;
pub mod names;
pub mod plan;

pub use cli::{print_usage, run};
