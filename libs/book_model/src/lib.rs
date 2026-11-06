//! 책 도메인 모델. 메타데이터(`Book`, `BookView`, `BookPage`, 조회 오류), ES 문서에서 `Book`을 만드는 변환,
//! 색인에서 뺄 코퍼스 디렉터리 규칙(`ignore_rules`)을 둔다.
//!
//! 경로 검증, 파일 이동, 검색 호출은 하지 않는다. 그건 `book_manager`가 맡는다.
//! 상위 crate는 `book_manager::book_metadata`로 이 모델을 쓴다.

pub mod book_metadata;
pub mod es_doc_converter;
pub mod ignore_rules;
