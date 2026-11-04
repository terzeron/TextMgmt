//! EPUB·PDF·TXT 통합 진단. 보안(security), 결함(defect), 렌더링 차단(render) 지표를 한 도구에서 낸다.

pub mod cli_parser;
pub mod generic_checker;
pub mod output_formatter;
pub mod doc;
pub mod hwp;
pub mod epub;
pub mod finding_model;
pub mod mojibake_detector;
pub mod ole_parser;
pub mod pdf;
pub mod file_prefetcher;
pub mod file_quarantiner;
pub mod nested_scanner;
pub mod defect_repairer;
pub mod report;
pub mod rule_engine;
pub mod scan_runner;
pub mod epub_sanitizer;
pub mod format_dispatcher;
pub mod magic_sniffer;
pub mod txt_checker;
pub mod viewer_block_table;
pub mod zipcheck;
pub mod zip_name_rules;

pub use finding_model::{Category, FileReport, Finding, Format, Severity, ViewerBlocked};
pub use scan_runner::{Report, Summary, scan_streaming};

/// 진단 대상의 기본 라이브러리 루트.
pub const DEFAULT_LIBRARY_ROOT: &str = "/mnt/data/text";
