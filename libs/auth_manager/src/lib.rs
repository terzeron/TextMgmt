//! 인증 도메인. backend의 `api_router`가 쓴다.
//!
//! - `auth_service`: JWT 발급·검증, 역할 판정, 요청에서 토큰 읽기(`RequestParts`).
//! - `refresh_token_store`: refresh token 저장, 회전·재사용 탐지, 세션 목록·폐기(MySQL).

pub mod auth_service;
pub mod google_token_verifier;
pub mod refresh_token_store;
