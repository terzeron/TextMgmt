//! 프런트엔드 에러 로그 수집. Python `main.py`의 `POST /logs/client-error`.
//!
//! 인증 없이 받으므로 본문 크기를 서버가 직접 묶는다. 상한은 프런트 `clientLogger.js`가 이미 자르는 값(message 2000,
//! stack·component_stack 5000)과 맞췄고, 나머지는 실측에 여유를 뒀다(관측된 url은 91자, 코퍼스에서 만들 수 있는 최장 카테고리 URL이 약 422자).

use std::collections::HashMap;
use std::sync::LazyLock;

use axum::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use super::errors::{ApiResult, RequestValidationError};
use super::extract::{ApiJson, OptionalAuth};
use super::routes::Routes;

/// 필드별 최대 글자 수.
pub static CLIENT_ERROR_FIELD_LIMITS: LazyLock<HashMap<&'static str, usize>> =
    LazyLock::new(|| HashMap::from([("message", 2000), ("stack", 5000), ("component_stack", 5000), ("url", 2048), ("user_agent", 512), ("timestamp", 64)]));

const ERROR_TYPES: [&str; 4] = ["REACT_RENDER_ERROR", "WINDOW_ERROR", "UNHANDLED_PROMISE", "CUSTOM_ERROR"];

#[derive(Debug, Clone, Deserialize)]
pub struct ClientErrorLogModel {
    pub error_type: String,
    pub message: String,
    pub url: String,
    pub stack: Option<String>,
    pub component_stack: Option<String>,
    pub user_agent: Option<String>,
    pub timestamp: Option<String>,
}

/// 한 줄로 기록하는 필드는 개행·제어 문자를 정리하고 글자 수 상한으로 자른다. stack 계열은 여러 줄이 의미를 가지므로 자르기만 한다.
/// 거부하지 않고 자르는 이유: 에러 보고는 앞부분이 쓸모 있어서 버리면 손해다.
fn bound(value: &str, field: &str, single_line: bool) -> String {
    let limit = CLIENT_ERROR_FIELD_LIMITS[field];
    if !single_line {
        return value.chars().take(limit).collect();
    }
    // 개행은 지우지 않고 공백으로 바꾼다. 지우면 앞뒤 단어가 붙어 읽기 어렵다. 가짜 로그 줄을 심는 공격(CWE-117)을 막는다.
    value.replace(['\r', '\n'], " ").chars().filter(|&c| c == '\t' || c >= ' ').take(limit).collect()
}

impl ClientErrorLogModel {
    /// `error_type`을 확인하고 모든 문자열 필드를 정리한다.
    fn validated(self) -> Result<Self, RequestValidationError> {
        if !ERROR_TYPES.contains(&self.error_type.as_str()) {
            return Err(RequestValidationError::single(
                &["body", "error_type"],
                "Input should be 'REACT_RENDER_ERROR', 'WINDOW_ERROR', 'UNHANDLED_PROMISE' or 'CUSTOM_ERROR'",
                json!({"error_type": self.error_type}),
            ));
        }
        Ok(Self {
            message: bound(&self.message, "message", true),
            url: bound(&self.url, "url", true),
            stack: self.stack.map(|v| bound(&v, "stack", false)),
            component_stack: self.component_stack.map(|v| bound(&v, "component_stack", false)),
            user_agent: self.user_agent.map(|v| bound(&v, "user_agent", true)),
            timestamp: self.timestamp.map(|v| bound(&v, "timestamp", true)),
            error_type: self.error_type,
        })
    }
}

pub(super) fn routes() -> Routes {
    Routes::new().post("/logs/client-error", &["optional_auth"], log_client_error)
}

/// 프런트엔드 런타임/렌더링 에러를 기록한다. 로그인한 사용자면 이메일과 역할을 함께 남긴다(운영 로그 추적용).
async fn log_client_error(OptionalAuth(auth_user): OptionalAuth, ApiJson(body): ApiJson<ClientErrorLogModel>) -> ApiResult<Json<Value>> {
    let body = body.validated()?;
    let text = |key: &str| auth_user.as_ref().and_then(|u| u.get(key)).and_then(Value::as_str).unwrap_or("anonymous").to_string();
    let (email, role) = (text("email"), text("role"));

    log::error!("[CLIENT_ERROR] type={}, user={email}({role}), url={}, message={}", body.error_type, body.url, body.message);
    // json.dumps(ensure_ascii=False): 개행이 `\n` 두 글자로 남아 한 줄 기록이 된다.
    if let Some(stack) = body.component_stack.as_deref().filter(|s| !s.is_empty()) {
        log::error!("[CLIENT_ERROR] Component Stack: {}", serde_json::to_string(stack).unwrap_or_default());
    }
    if let Some(stack) = body.stack.as_deref().filter(|s| !s.is_empty()) {
        log::error!("[CLIENT_ERROR] Stack Trace: {}", serde_json::to_string(stack).unwrap_or_default());
    }
    Ok(Json(json!({"status": "ok"})))
}
