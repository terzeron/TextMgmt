//! 문서 파일 교정 기법. EPUB·PDF의 보안 정리(`epub_sanitizer`, `pdf_sanitizer`), EPUB 결함 수리(`defect_repairer`),
//! TXT의 UTF-8 재인코딩(`encoding_repairer`)을 둔다.
//!
//! 파일 하나의 바이트를 받아 고친 바이트와 그 검증 결과를 돌려준다. 무엇이 위험하거나 깨졌는지의 판정은 `doc_checker`를 쓴다.
//! 파일 순회, 적용(휴지통 보관·덮어쓰기), 출력은 하지 않는다. 그런 흐름은 `bookdoctor`가 맡는다.

pub mod defect_repairer;
pub mod encoding_repairer;
pub mod epub_sanitizer;
pub mod pdf_sanitizer;
