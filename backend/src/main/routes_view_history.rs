//! 조회 이력 라우트. Python `main.py`의 `record_book_view`(책·만화 라우터)와 `list_view_history`.
//!
//! 책 라우터는 루트, 만화 라우터는 `/comics` prefix에 붙으므로 경로는 `/books/view-history/{id}`와 `/comics/view-history/{id}`다.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::deps::AppDeps;
use super::errors::{ApiError, ApiResult, RequestValidationError};
use super::extract::{ApiQuery, RequireAdmin, RequireAuth};
use super::routes::Routes;
use super::routes_books::parse_book_id;
use crate::view_history_store::MAX_RECENT_VIEWS;

pub(super) fn routes() -> Routes {
    Routes::new().post("/books/view-history/{book_id}", &["require_auth"], record_book_view).post("/comics/view-history/{book_id}", &["require_auth"], record_comic_view).get(
        "/view-history",
        &["require_admin"],
        list_view_history,
    )
}

/// FastAPI의 `book_id: int` 검증. 정수가 아니면 422다.
async fn record_book_view(State(deps): State<Arc<AppDeps>>, RequireAuth(payload): RequireAuth, Path(book_id): Path<String>) -> ApiResult<Json<Value>> {
    record_view(&deps, &payload, parse_book_id(&book_id)?, "book").await
}

async fn record_comic_view(State(deps): State<Arc<AppDeps>>, RequireAuth(payload): RequireAuth, Path(book_id): Path<String>) -> ApiResult<Json<Value>> {
    record_view(&deps, &payload, parse_book_id(&book_id)?, "comic").await
}

/// 열람 뷰어 진입을 1건 기록한다. 제목·카테고리는 클라이언트를 믿지 않고 서버 레코드에서 뜬다.
/// 접근 검사를 먼저 하므로 viewer가 볼 수 없는 책을 기록하거나 존재 확인 수단으로 쓰는 것도 막힌다.
async fn record_view(deps: &AppDeps, payload: &Value, book_id: i64, content_type: &str) -> ApiResult<Json<Value>> {
    log::debug!("# record_book_view(book_id={book_id})");
    let book = deps.book_access().get_book_and_ensure_viewer_access(book_id, payload, content_type).await?;
    let email = payload.get("email").and_then(Value::as_str).unwrap_or("");
    let recorded = deps
        .view_history()
        .map_err(|e| e.to_string())
        .and_then(|store| store.record_view(email, content_type, book_id, &book.title, book.category.as_deref().unwrap_or("")).map_err(|e| e.to_string()));
    // 이력 기록 실패가 열람을 막아서는 안 된다. 실패는 로그로 드러내고 응답에도 표시한다.
    if let Err(e) = &recorded {
        log::warn!("조회 이력 기록 실패 (content_type={content_type}, book_id={book_id}): {e}");
    }
    Ok(Json(json!({"status": "success", "result": {"recorded": recorded.is_ok()}})))
}

#[derive(Deserialize)]
struct LimitQuery {
    limit: Option<i64>,
}

/// 사용자별 최근 조회 목록(책/만화 각각)을 돌려준다.
async fn list_view_history(State(deps): State<Arc<AppDeps>>, RequireAdmin(_): RequireAdmin, ApiQuery(query): ApiQuery<LimitQuery>) -> ApiResult<Json<Value>> {
    let max = MAX_RECENT_VIEWS as i64;
    let limit = query.limit.unwrap_or(max);
    if limit < 1 {
        return Err(RequestValidationError::single(&["query", "limit"], "Input should be greater than or equal to 1", json!(limit)).into());
    }
    if limit > max {
        return Err(RequestValidationError::single(&["query", "limit"], &format!("Input should be less than or equal to {max}"), json!(limit)).into());
    }
    let result = deps.view_history().map_err(|e| e.to_string()).and_then(|store| store.list_recent_views(limit as usize).map_err(|e| e.to_string()));
    match result {
        Ok(result) => Ok(Json(json!({"status": "success", "result": result}))),
        Err(e) => {
            log::error!("view_history_store.list_recent_views() failed: {e}");
            Err(ApiError::unavailable())
        }
    }
}
