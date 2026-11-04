//! EPUB·PDF·TXT 통합 진단. 보안(security), 결함(defect), 렌더링 차단(render) 지표를 한 도구에서 낸다.

pub mod cli;
pub mod common;
pub mod display;
pub mod doc;
pub mod hwp;
pub mod epub;
pub mod finding;
pub mod mojibake;
pub mod ole;
pub mod pdf;
pub mod prefetch;
pub mod quarantine;
pub mod recurse;
pub mod repair;
pub mod report;
pub mod rules;
pub mod runner;
pub mod sanitize;
pub mod scan;
pub mod sniff;
pub mod txt;
pub mod viewer;
pub mod zipcheck;
pub mod zipnames;

pub use finding::{Category, FileReport, Finding, Format, Severity, ViewerBlocked};
pub use runner::{Report, Summary, scan_streaming};

/// 진단 대상의 기본 라이브러리 루트.
pub const DEFAULT_LIBRARY_ROOT: &str = "/mnt/data/text";
