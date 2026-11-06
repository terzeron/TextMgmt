//! 문서 컨테이너의 바이트 수준 해석. 리틀엔디언 정수 읽기, OLE(CFB) 복합 문서와 그 안의 VBA 프로젝트,
//! 앞머리 매직 바이트로 형식을 가리는 일을 맡는다.
//!
//! 검사 판단(심각도, 보고서)은 하지 않는다. 그건 `doc_checker`가 맡는다.

pub mod byte_reader;
pub mod magic_sniffer;
pub mod ole_parser;
