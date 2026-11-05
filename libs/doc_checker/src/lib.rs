//! 문서 파일 검사와 교정 기법. EPUB·PDF·TXT·HWP·RTF·ZIP 계열의 보안(security), 결함(defect), 렌더링 차단(render) 지표를 내고,
//! 보안 정리와 결함 수리에 쓰는 재구성·검증, TXT의 UTF-8 재인코딩을 제공한다.
//!
//! 파일 하나를 받아 검사 보고서(`FileReport`)나 고친 바이트를 돌려준다. 파일 순회, 병렬 실행, 적용(휴지통 보관·덮어쓰기),
//! 출력은 하지 않는다. 그런 흐름은 `bookdoctor`가 맡는다.

use std::sync::Arc;

pub(crate) mod byte_reader;
pub mod defect_repairer;
pub mod doc;
pub mod encoding_repairer;
pub mod epub;
pub mod epub_sanitizer;
pub mod finding_model;
pub mod format_dispatcher;
pub mod generic_checker;
pub mod hwp;
pub mod magic_sniffer;
pub mod mojibake_detector;
pub mod nested_scanner;
pub mod ole_parser;
pub mod pdf;
pub mod rule_engine;
pub mod text_sanitizer;
pub mod txt_checker;
pub mod viewer_block_table;
pub mod zip_name_rules;
pub mod zipcheck;

pub use finding_model::{Category, FileReport, Finding, Format, Severity, ViewerBlocked};

use rule_engine::Compiled;

/// 파일 하나에서 읽는 기본 최대 크기(MB).
pub const MAX_TOTAL_MB: u64 = 256;

/// 검사 설정. 파일 하나를 검사할 때 검사기가 읽는 값만 둔다.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckOptions {
    /// 파일 하나에서 읽는 최대 크기(MB).
    pub max_total_mb: u64,
    /// ZIP 계열 멤버를 끝까지 풀어 CRC까지 확인한다. 기본은 멤버마다 앞부분만 풀어 본다.
    pub zip_deep: bool,
    /// MuPDF worker를 부르지 않는다(PDF 본문 표본 추출을 건너뛴다). 테스트용이다.
    pub no_pdf_worker: bool,
    /// ZIP 계열 안의 컨테이너 멤버를 재귀 검사한다.
    pub recurse: bool,
    /// 읽어서 컴파일한 규칙. 없으면 규칙을 적용하지 않는다(비용 0).
    pub ruleset: Option<Arc<Compiled>>,
}

impl Default for CheckOptions {
    fn default() -> Self {
        CheckOptions { max_total_mb: MAX_TOTAL_MB, zip_deep: false, no_pdf_worker: false, recurse: true, ruleset: None }
    }
}
