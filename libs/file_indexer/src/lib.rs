//! 파일 하나를 색인 레코드로 읽는다: 종류 판별, 본문·쪽수·ISBN·생성 시각 추출, 자식 프로세스 격리.
//!
//! loader(일괄 적재)와 backend(파일 하나 다시 읽기)가 같은 규칙으로 읽도록 함께 쓴다. 색인 쓰기와 명령행은 loader에 둔다.

pub mod child_process_runner;
pub mod env_config;
pub mod file_time;
pub mod ignore_rules;
pub mod record_builder;
