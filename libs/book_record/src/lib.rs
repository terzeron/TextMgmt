//! 책 한 권의 기록. 메타데이터 모델(`Book`, `BookView`, `BookPage`)과 ES 문서에서 `Book`을 만드는 변환,
//! 파일시스템 상태와 무관한 경로·파일 종류 규칙을 둔다.
//!
//! 책 루트에 맞춘 검증, 파일 이동, 검색 호출은 하지 않는다. 그건 `book_manager`가 맡는다.

pub mod book_metadata;
pub mod es_doc_converter;
pub mod path_validator;
