//! 문서 본문 요약 추출(`read_from_*`).
//!
//! 각 읽기 함수는 (요약, 줄 수, 쪽 수)를 만든다. 요약은 앞 [`TEXT_SIZE`]자만 남긴다.

mod body_decoder;
pub mod epub;
pub mod head_tail_reader;
pub mod markup;
pub mod office;
pub mod pdf;
pub mod rtf;
pub mod text;
pub mod text_normalizer;

/// 요약 길이(문자 수).
pub const TEXT_SIZE: usize = 4096;
