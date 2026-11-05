//! 문서 본문 요약 추출(`read_from_*`).
//!
//! 각 읽기 함수는 (요약, 줄 수, 쪽 수)를 만든다. 요약은 앞 [`TEXT_SIZE`]자만 남긴다.

pub mod epub;
pub mod epub_metadata_reader;
pub mod head_tail_reader;
pub mod markup;
pub mod office;
pub mod pdf;
pub mod pdf_page_reader;
pub mod rtf;
pub mod text;
pub mod text_normalizer;
pub mod text_sampler;

/// 요약 길이(문자 수).
pub const TEXT_SIZE: usize = 4096;
