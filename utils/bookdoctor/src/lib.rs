//! EPUB·PDF·TXT 통합 진단 도구. 명령행, 파일 순회와 병렬 실행, 적용(휴지통 보관·덮어쓰기), 보고 출력을 맡는다.
//! 검사와 교정 기법은 `doc_checker`에 있다.

pub mod cli_parser;
pub mod file_prefetcher;
pub mod file_quarantiner;
pub mod file_repairer;
pub(crate) mod file_rewriter;
pub mod file_sanitizer;
pub mod output_formatter;
pub mod report;
pub mod scan_runner;

pub use doc_checker::{Category, FileReport, Finding, Format, Severity, ViewerBlocked};
pub use scan_runner::{Report, Summary, scan_streaming};

/// 진단 대상의 기본 라이브러리 루트.
pub const DEFAULT_LIBRARY_ROOT: &str = "/mnt/data/text";
