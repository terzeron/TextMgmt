//! PDF 쪽 삭제·이미지 삽입 미리보기 라우트(`book_manager::pdf_edit`).

use super::super::app_deps::AppDeps;
use super::super::error_responses::ApiError;
use super::super::error_responses::ApiResult;
use super::super::error_responses::RequestValidationError;
use super::super::request_extractors::ApiJson;
use super::super::request_extractors::RequireAdmin;
use super::super::route_registry::Routes;
use super::BookResponse;
use super::book_response;
use super::checked_manager;
use super::parse_book_id;
use axum::Json;
use axum::extract::DefaultBodyLimit;
use axum::extract::Multipart;
use axum::extract::OriginalUri;
use axum::extract::Path;
use axum::extract::State;
use axum::handler::Handler;
use axum::response::Response;
use book_manager::MAX_IMAGE_BYTES;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;

#[allow(unused_imports)]
use super::*;

pub(super) fn routes() -> Routes {
    let limit = || DefaultBodyLimit::max(MAX_IMAGE_BYTES.div_ceil(3) * 4 * 5 + 64 * 1024);
    Routes::new()
        .post("/pdf-pages/{book_id}/delete", &["require_admin"], delete_pages.layer(limit()))
        .post("/comics/pdf-pages/{book_id}/delete", &["require_admin"], delete_pages.layer(limit()))
        .post("/pdf-pages/{book_id}/insert-preview", &["require_admin"], preview_image)
        .post("/comics/pdf-pages/{book_id}/insert-preview", &["require_admin"], preview_image)
        .post("/pdf-pages/{book_id}/insert-preview-upload", &["require_admin"], preview_upload.layer(limit()))
        .post("/comics/pdf-pages/{book_id}/insert-preview-upload", &["require_admin"], preview_upload.layer(limit()))
}

#[derive(Deserialize, Serialize)]
struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    #[serde(default)]
    angle: f64,
}

#[derive(Deserialize, Serialize)]
struct Crop {
    page: i64,
    #[serde(flatten)]
    rect: Rect,
}

#[derive(Deserialize, Serialize)]
struct Split {
    page: i64,
    first: Rect,
    second: Rect,
}

#[derive(Deserialize, Serialize)]
#[serde(try_from = "RawInsert")]
struct Insert {
    page: i64,
    url: Option<String>,
    image_data: Option<String>,
}

#[derive(Deserialize)]
struct RawInsert {
    page: i64,
    url: Option<String>,
    image_data: Option<String>,
}

impl TryFrom<RawInsert> for Insert {
    type Error = String;
    fn try_from(raw: RawInsert) -> Result<Self, String> {
        if raw.url.is_none() == raw.image_data.is_none() {
            return Err("url 과 image_data 중 하나만 지정해야 합니다".into());
        }
        if raw.url.as_ref().is_some_and(|s| s.is_empty() || s.chars().count() > 4096) {
            return Err("url 길이가 올바르지 않습니다".into());
        }
        if raw.image_data.as_ref().is_some_and(|s| s.is_empty() || s.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4) {
            return Err("image_data 길이가 올바르지 않습니다".into());
        }
        Ok(Self { page: raw.page, url: raw.url, image_data: raw.image_data })
    }
}

#[derive(Deserialize)]
struct EditBody {
    #[serde(default)]
    pages: Vec<i64>,
    #[serde(default)]
    crops: Vec<Crop>,
    #[serde(default)]
    splits: Vec<Split>,
    #[serde(default)]
    inserts: Vec<Insert>,
    #[serde(default)]
    order: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(try_from = "RawPreview")]
struct PreviewBody {
    url: String,
}

#[derive(Deserialize)]
struct RawPreview {
    url: String,
}

impl TryFrom<RawPreview> for PreviewBody {
    type Error = String;
    fn try_from(raw: RawPreview) -> Result<Self, String> {
        if raw.url.is_empty() || raw.url.chars().count() > 4096 {
            return Err("url 길이가 올바르지 않습니다".into());
        }
        Ok(Self { url: raw.url })
    }
}

async fn preview_image(
    RequireAdmin(payload): RequireAdmin,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiJson(body): ApiJson<PreviewBody>,
) -> ApiResult<Response> {
    let id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, id, uri.path().starts_with("/comics/")).await?;
    book_response(match manager.preview_insert_image(&body.url).await {
        Ok(body) => BookResponse { status_code: 200, media_type: Some("image/jpeg".into()), headers: vec![("Cache-Control".into(), "private, no-store".into())], body },
        Err(error) => BookResponse { status_code: 400, body: error.into_bytes(), ..BookResponse::default() },
    })
}

async fn preview_upload(
    RequireAdmin(payload): RequireAdmin,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    multipart: Option<Multipart>,
) -> ApiResult<Response> {
    let id = parse_book_id(&raw_id)?;
    let Some(mut multipart) = multipart else {
        return Err(ApiError::Validation(RequestValidationError::single(&["body", "file"], "Field required", Value::Null)));
    };
    let mut data = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| ApiError::Validation(RequestValidationError::single(&["body", "file"], "Field required", Value::Null)))? {
        if field.name() == Some("file") {
            data = Some(field.bytes().await.map_err(|_| ApiError::Validation(RequestValidationError::single(&["body", "file"], "Field required", Value::Null)))?);
            break;
        }
    }
    let Some(data) = data else {
        return Err(ApiError::Validation(RequestValidationError::single(&["body", "file"], "Field required", Value::Null)));
    };
    let manager = checked_manager(&deps, &payload, id, uri.path().starts_with("/comics/")).await?;
    book_response(match manager.preview_insert_upload(&data).await {
        Ok(body) => BookResponse { status_code: 200, media_type: Some("image/jpeg".into()), headers: vec![("Cache-Control".into(), "private, no-store".into())], body },
        Err(error) => BookResponse { status_code: 400, body: error.into_bytes(), ..BookResponse::default() },
    })
}

async fn delete_pages(
    RequireAdmin(payload): RequireAdmin,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiJson(body): ApiJson<EditBody>,
) -> ApiResult<Json<Value>> {
    let id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, id, uri.path().starts_with("/comics/")).await?;
    let crops: Vec<Value> = body.crops.iter().map(|v| json!(v)).collect();
    let splits: Vec<Value> = body.splits.iter().map(|v| json!(v)).collect();
    let inserts: Vec<Value> = body.inserts.iter().map(|v| json!(v)).collect();
    Ok(Json(match manager.delete_pdf_pages(id, &body.pages, Some(&crops), Some(&splits), Some(&inserts), Some(&body.order)).await {
        Ok(result) => json!({"status":"success","result":result}),
        Err(error) => json!({"status":"failure","error":error}),
    }))
}
