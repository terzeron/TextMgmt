//! EPUB 표지 설정·삭제와 삽화 삽입 라우트. Python `main.py`의 `set_epub_cover`, `insert_epub_illustration`, `remove_epub_cover`.
//!
//! 책 라우터는 루트, 만화 라우터는 `/comics` prefix에 붙으므로 경로가 둘씩이다.

use std::sync::Arc;

use axum::Json;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::handler::Handler;
use serde::Deserialize;
use serde_json::{Value, json};

use super::deps::AppDeps;
use super::errors::{ApiError, ApiResult, RequestValidationError};
use super::extract::{ApiJson, RequireAdmin};
use super::routes::Routes;
use super::traits::BookManagerApi;
use crate::epub_cover::MAX_IMAGE_BYTES;

pub(super) fn routes() -> Routes {
    // base64 본문이 이미지 상한의 4/3배라서 axum의 기본 2MiB 상한을 넓힌다.
    let limit = || DefaultBodyLimit::max(MAX_IMAGE_BYTES.div_ceil(3) * 4 + 64 * 1024);
    Routes::new()
        .post("/epub-cover/{book_id}", &["require_admin"], book_set_cover.layer(limit()))
        .post("/epub-illustration/{book_id}", &["require_admin"], book_insert_illustration.layer(limit()))
        .delete("/epub-cover/{book_id}", &["require_admin"], book_remove_cover)
        .post("/comics/epub-cover/{book_id}", &["require_admin"], comic_set_cover.layer(limit()))
        .post("/comics/epub-illustration/{book_id}", &["require_admin"], comic_insert_illustration.layer(limit()))
        .delete("/comics/epub-cover/{book_id}", &["require_admin"], comic_remove_cover)
}

/// FastAPI의 `book_id: int` 검증. 정수가 아니면 422다.
fn parse_book_id(raw: &str) -> Result<i64, ApiError> {
    raw.trim().parse::<i64>().map_err(|_| RequestValidationError::single(&["path", "book_id"], "Input should be a valid integer, unable to parse string as an integer", json!(raw)).into())
}

/// 이미지 출처: 공개 URL(`url`)이거나 올린 이미지(`image_data`, base64)다. 하나만 받는다.
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawSource")]
struct EpubCoverBody {
    url: Option<String>,
    image_data: Option<String>,
}

#[derive(Deserialize)]
struct RawSource {
    url: Option<String>,
    image_data: Option<String>,
}

fn check_source(url: &Option<String>, image_data: &Option<String>) -> Result<(), String> {
    let image_max = MAX_IMAGE_BYTES.div_ceil(3) * 4;
    if let Some(url) = url
        && (url.is_empty() || url.chars().count() > 4096)
    {
        return Err("url: String should have between 1 and 4096 characters".to_string());
    }
    if let Some(data) = image_data
        && (data.is_empty() || data.chars().count() > image_max)
    {
        return Err("image_data: String should have between 1 and the image size limit characters".to_string());
    }
    if url.is_none() == image_data.is_none() {
        return Err("url 과 image_data 중 하나만 지정해야 합니다".to_string());
    }
    Ok(())
}

impl TryFrom<RawSource> for EpubCoverBody {
    type Error = String;

    fn try_from(raw: RawSource) -> Result<Self, String> {
        check_source(&raw.url, &raw.image_data)?;
        Ok(Self { url: raw.url, image_data: raw.image_data })
    }
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "RawIllustration")]
struct EpubIllustrationBody {
    url: Option<String>,
    image_data: Option<String>,
    before_idref: String,
}

#[derive(Deserialize)]
struct RawIllustration {
    url: Option<String>,
    image_data: Option<String>,
    before_idref: String,
}

impl TryFrom<RawIllustration> for EpubIllustrationBody {
    type Error = String;

    fn try_from(raw: RawIllustration) -> Result<Self, String> {
        check_source(&raw.url, &raw.image_data)?;
        if raw.before_idref.is_empty() || raw.before_idref.chars().count() > 256 {
            return Err("before_idref: String should have between 1 and 256 characters".to_string());
        }
        Ok(Self { url: raw.url, image_data: raw.image_data, before_idref: raw.before_idref })
    }
}

fn outcome((result, error): (Option<Value>, Option<String>)) -> Json<Value> {
    match error {
        None => Json(json!({"status": "success", "result": result})),
        Some(error) => Json(json!({"status": "failure", "error": error})),
    }
}

fn manager(deps: &AppDeps, comics: bool) -> ApiResult<Arc<dyn BookManagerApi>> {
    let found = if comics { deps.comics_manager() } else { deps.book_manager() };
    found.map_err(|e| {
        log::warn!("EPUB 표지 관리자를 쓸 수 없다: {e}");
        ApiError::unavailable()
    })
}

async fn set_cover(deps: &AppDeps, comics: bool, book_id: &str, body: EpubCoverBody) -> ApiResult<Json<Value>> {
    let book_id = parse_book_id(book_id)?;
    log::debug!("# set_epub_cover(book_id={book_id}, url={})", if body.url.is_some() { "<url>" } else { "<upload>" });
    Ok(outcome(manager(deps, comics)?.set_epub_cover(book_id, body.url.as_deref(), body.image_data.as_deref()).await))
}

async fn insert_illustration(deps: &AppDeps, comics: bool, book_id: &str, body: EpubIllustrationBody) -> ApiResult<Json<Value>> {
    let book_id = parse_book_id(book_id)?;
    log::debug!("# insert_epub_illustration(book_id={book_id}, before={:?})", body.before_idref);
    Ok(outcome(manager(deps, comics)?.insert_epub_illustration(book_id, &body.before_idref, body.url.as_deref(), body.image_data.as_deref()).await))
}

async fn remove_cover(deps: &AppDeps, comics: bool, book_id: &str) -> ApiResult<Json<Value>> {
    let book_id = parse_book_id(book_id)?;
    log::debug!("# remove_epub_cover(book_id={book_id})");
    Ok(outcome(manager(deps, comics)?.remove_epub_cover(book_id).await))
}

async fn book_set_cover(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(book_id): Path<String>, ApiJson(body): ApiJson<EpubCoverBody>) -> ApiResult<Json<Value>> {
    set_cover(&deps, false, &book_id, body).await
}

async fn comic_set_cover(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(book_id): Path<String>, ApiJson(body): ApiJson<EpubCoverBody>) -> ApiResult<Json<Value>> {
    set_cover(&deps, true, &book_id, body).await
}

async fn book_insert_illustration(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(book_id): Path<String>, ApiJson(body): ApiJson<EpubIllustrationBody>) -> ApiResult<Json<Value>> {
    insert_illustration(&deps, false, &book_id, body).await
}

async fn comic_insert_illustration(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(book_id): Path<String>, ApiJson(body): ApiJson<EpubIllustrationBody>) -> ApiResult<Json<Value>> {
    insert_illustration(&deps, true, &book_id, body).await
}

async fn book_remove_cover(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(book_id): Path<String>) -> ApiResult<Json<Value>> {
    remove_cover(&deps, false, &book_id).await
}

async fn comic_remove_cover(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(book_id): Path<String>) -> ApiResult<Json<Value>> {
    remove_cover(&deps, true, &book_id).await
}
