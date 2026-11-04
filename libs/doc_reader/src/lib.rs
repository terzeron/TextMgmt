//! 문서 본문 요약 추출. 원본: `utils/loader.py`의 `Loader.read_from_*`.
//!
//! 각 읽기 함수는 (요약, 줄 수, 쪽 수)를 만든다. 요약은 앞 [`TEXT_SIZE`]자만 남긴다.

pub mod epub;
pub mod markup;
pub mod mojibake_counter;
pub mod office;
pub mod pdf;
pub mod py_str_compat;
pub mod rtf;
pub mod text;

/// 요약 길이(문자 수). 원본 `Loader.TEXT_SIZE`.
pub const TEXT_SIZE: usize = 4096;
