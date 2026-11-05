//! EPUB·PDF·TXT 통합 진단. 보안(security), 결함(defect), 렌더링 차단(render) 지표를 한 도구에서 낸다.

pub(crate) mod byte_reader;
pub mod cli_parser;
pub mod defect_repairer;
pub mod doc;
pub mod epub;
pub mod epub_sanitizer;
pub mod file_prefetcher;
pub mod file_quarantiner;
pub mod finding_model;
pub mod format_dispatcher;
pub mod generic_checker;
pub mod hwp;
pub mod magic_sniffer;
pub mod mojibake_detector;
pub mod nested_scanner;
pub mod ole_parser;
pub mod output_formatter;
pub mod pdf;
pub mod report;
pub mod rule_engine;
pub mod scan_runner;
pub mod text_sampler;
pub mod txt_checker;
pub mod viewer_block_table;
pub mod zip_name_rules;
pub mod zipcheck;

pub use finding_model::{Category, FileReport, Finding, Format, Severity, ViewerBlocked};
pub use scan_runner::{Report, Summary, scan_streaming};

/// 진단 대상의 기본 라이브러리 루트.
pub const DEFAULT_LIBRARY_ROOT: &str = "/mnt/data/text";
