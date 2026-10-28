//! 책·만화의 미리보기, 다운로드, 리소스 및 검증 라우트.

use std::sync::Arc;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use super::deps::AppDeps;
use super::errors::{ApiError, ApiResult, RequestValidationError};
use super::extract::{ApiQuery, RequireAuth};
use super::routes::Routes;
use super::traits::{BookContent, BookManagerApi, BookResponse, MappingError};
use crate::book::BookLookupError;
use crate::book_manager::BookManager;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

mod admin;
mod browse;
mod catalog;
mod mismatch;

type ByteRange = (u64, u64);

enum RangeError {
    Malformed(&'static str),
    Unsatisfiable,
}

fn clamp_u64(value: i128) -> u64 {
    value.clamp(0, i128::from(u64::MAX)) as u64
}

fn parse_byte_ranges(value: &str, file_size: u64) -> Result<Vec<ByteRange>, RangeError> {
    let Some((units, ranges)) = value.split_once('=') else { return Err(RangeError::Malformed("Malformed range header.")) };
    if !units.trim().eq_ignore_ascii_case("bytes") {
        return Err(RangeError::Malformed("Only support bytes range"));
    }
    let mut parsed = Vec::new();
    for part in ranges.split(',').map(str::trim) {
        if part.is_empty() || part == "-" {
            continue;
        }
        let Some((start, end)) = part.split_once('-') else { continue };
        let start = start.trim();
        let end = end.trim();
        let range = if start.is_empty() {
            let Ok(suffix) = end.parse::<i128>() else { continue };
            (clamp_u64(i128::from(file_size).saturating_sub(suffix)), file_size)
        } else {
            let Ok(start) = start.parse::<i128>() else { continue };
            let end = if end.is_empty() {
                i128::from(file_size)
            } else {
                let Ok(end) = end.parse::<i128>() else { continue };
                if end < i128::from(file_size) { end + 1 } else { i128::from(file_size) }
            };
            (clamp_u64(start), clamp_u64(end))
        };
        parsed.push(range);
    }
    if parsed.is_empty() {
        return Err(RangeError::Malformed("Range header: range must be requested"));
    }
    if parsed.iter().any(|(start, _)| *start >= file_size) {
        return Err(RangeError::Unsatisfiable);
    }
    if parsed.iter().any(|(start, end)| start > end) {
        return Err(RangeError::Malformed("Range header: start must be less than end"));
    }
    parsed.sort_unstable();
    let mut merged: Vec<ByteRange> = Vec::with_capacity(parsed.len());
    for (start, end) in parsed {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    Ok(merged)
}

fn set_header(headers: &mut Vec<(String, String)>, key: &str, value: String) {
    if let Some((_, old)) = headers.iter_mut().find(|(name, _)| name.eq_ignore_ascii_case(key)) {
        *old = value;
    } else {
        headers.push((key.to_string(), value));
    }
}

fn add_file_headers(modified: SystemTime, file_size: u64, headers: &mut Vec<(String, String)>) -> Result<(String, String), ApiError> {
    use md5::Digest;
    let modified_secs = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs_f64(),
        Err(before_epoch) => -before_epoch.duration().as_secs_f64(),
    };
    let last_modified = chrono::DateTime::<chrono::Utc>::from(modified).format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let etag_input = format!("{modified_secs}-{file_size}");
    let etag_hash = md5::Md5::digest(etag_input.as_bytes());
    let etag = format!("\"{}\"", etag_hash.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
    set_header(headers, "Accept-Ranges", "bytes".to_string());
    set_header(headers, "Content-Length", file_size.to_string());
    set_header(headers, "Last-Modified", last_modified.clone());
    set_header(headers, "ETag", etag.clone());
    Ok((last_modified, etag))
}

fn response_with_body(status: u16, media_type: Option<&str>, headers: Vec<(String, String)>, body: Body) -> ApiResult<Response> {
    let mut response = Response::builder().status(status);
    if let Some(media_type) = media_type {
        response = response.header("content-type", media_type);
    }
    for (name, value) in headers {
        response = response.header(name, value);
    }
    response.body(body).map_err(|e| ApiError::Internal(e.to_string()))
}

fn file_stream(file: impl tokio::io::AsyncRead + Send + Unpin + 'static) -> Body {
    Body::from_stream(ReaderStream::with_capacity(file, 64 * 1024))
}

fn multipart_content_length(ranges: &[ByteRange], media_type: &str, file_size: u64, boundary: &str) -> u64 {
    let mut length = (boundary.len() + 4) as u64;
    for (start, end) in ranges {
        let header = format!("--{boundary}\r\nContent-Type: {media_type}\r\nContent-Range: bytes {start}-{}/{file_size}\r\n\r\n", end - 1);
        length += header.len() as u64 + (end - start) + 2;
    }
    length
}

fn multipart_file_body(mut file: tokio::fs::File, ranges: Vec<ByteRange>, media_type: String, file_size: u64, boundary: String) -> Body {
    const CHUNK_SIZE: usize = 64 * 1024;
    let (sender, receiver) = mpsc::channel::<Result<Bytes, std::io::Error>>(2);
    tokio::spawn(async move {
        for (start, end) in ranges {
            let header = format!("--{boundary}\r\nContent-Type: {media_type}\r\nContent-Range: bytes {start}-{}/{file_size}\r\n\r\n", end - 1);
            if sender.send(Ok(Bytes::from(header))).await.is_err() {
                return;
            }
            if let Err(error) = file.seek(SeekFrom::Start(start)).await {
                let _ = sender.send(Err(error)).await;
                return;
            }
            let mut remaining = end - start;
            while remaining > 0 {
                let mut chunk = vec![0; remaining.min(CHUNK_SIZE as u64) as usize];
                let count = match file.read(&mut chunk).await {
                    Ok(count) => count,
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                };
                if count == 0 {
                    break;
                }
                chunk.truncate(count);
                remaining -= count as u64;
                if sender.send(Ok(Bytes::from(chunk))).await.is_err() {
                    return;
                }
            }
            if sender.send(Ok(Bytes::from_static(b"\r\n"))).await.is_err() {
                return;
            }
        }
        let _ = sender.send(Ok(Bytes::from(format!("--{boundary}--")))).await;
    });
    Body::from_stream(ReceiverStream::new(receiver))
}

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .get(&format!("{prefix}/preview/{{book_id}}"), &["require_auth"], preview)
            .get(&format!("{prefix}/download/{{book_id}}"), &["require_auth"], download)
            .get(&format!("{prefix}/html-resource/{{book_id}}"), &["require_auth"], html_resource)
            .get(&format!("{prefix}/cover/{{book_id}}"), &["require_auth"], cover)
            .get(&format!("{prefix}/validate/{{book_id}}"), &["require_auth"], validate);
    }
    routes.merge(browse::routes()).merge(catalog::routes()).merge(admin::routes()).merge(mismatch::routes())
}

pub(super) fn parse_book_id(raw: &str) -> ApiResult<i64> {
    raw.trim().parse().map_err(|_| RequestValidationError::single(&["path", "book_id"], "Input should be a valid integer", json!(raw)).into())
}

/// 미리보기·쪽 추출은 LibreOffice 변환, EPUB 재압축, PDF 로드처럼 오래 막는 일을 한다.
/// tokio worker를 묶으면 다른 API가 멈추므로 blocking 스레드에서 돌린다.
pub(super) async fn run_blocking<T: Send + 'static>(work: impl Future<Output = T> + Send + 'static) -> ApiResult<T> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(work)).await.map_err(|e| ApiError::Internal(format!("blocking task failed: {e}")))
}

/// 경로로 도서와 만화를 가른다. 카테고리 정책과 ES가 쓰는 종류 이름이다.
fn kind(uri: &Uri) -> &'static str {
    if uri.path().starts_with("/comics/") { "comic" } else { "book" }
}

fn manager(deps: &AppDeps, uri: &Uri) -> ApiResult<Arc<dyn BookManagerApi>> {
    manager_for(deps, kind(uri) == "comic")
}

fn manager_for(deps: &AppDeps, comics: bool) -> ApiResult<Arc<dyn BookManagerApi>> {
    if comics { deps.comics_manager() } else { deps.book_manager() }.map_err(|e| {
        log::warn!("도서 관리자를 쓸 수 없다: {e}");
        ApiError::unavailable()
    })
}

impl From<MappingError> for ApiError {
    fn from(error: MappingError) -> Self {
        match error {
            MappingError::Http(status, detail) => ApiError::http(status, &detail),
            MappingError::Other(message) => ApiError::Internal(message),
        }
    }
}

fn is_viewer(payload: &Value) -> bool {
    payload.get("role").and_then(Value::as_str) == Some("viewer")
}

/// viewer에게 숨긴 카테고리. viewer가 아니면 비어 있다.
fn hidden_categories(deps: &AppDeps, payload: &Value, comics: bool) -> ApiResult<Vec<String>> {
    if !is_viewer(payload) {
        return Ok(Vec::new());
    }
    let mapping = deps.category_mapping().map_err(|e| {
        log::warn!("카테고리 접근 정책을 읽을 수 없다: {e}");
        ApiError::unavailable()
    })?;
    Ok(mapping.get_hidden_categories(if comics { "comic" } else { "book" })?)
}

/// 카테고리가 숨긴 카테고리이거나 그 하위면 참이다. 모든 viewer 접근 검사가 이 판정 하나를 쓴다.
pub(super) fn is_hidden(category: &str, hidden: &[String]) -> bool {
    !category.is_empty() && hidden.iter().any(|h| category == h || category.strip_prefix(h.as_str()).is_some_and(|rest| rest.starts_with('/')))
}

fn ensure_allowed(category: &str, hidden: &[String]) -> ApiResult<()> {
    if is_hidden(category, hidden) { Err(ApiError::http(403, "접근 권한이 없는 카테고리입니다.")) } else { Ok(()) }
}

/// viewer는 카테고리 정책을 확인한 뒤 파일을 읽는다. admin은 조회 제한을 받지 않는다.
pub(super) async fn checked_manager(deps: &AppDeps, payload: &Value, book_id: i64, comics: bool) -> ApiResult<Arc<dyn BookManagerApi>> {
    let manager = manager_for(deps, comics)?;
    if is_viewer(payload) {
        // 카테고리를 모르면 숨김 여부를 판정할 수 없다. 통과시키지 않고 거절한다(fail-closed).
        let book = match manager.get_book(book_id).await {
            Ok(book) => book,
            Err(BookLookupError::NotFound(_)) => return Err(ApiError::http(404, &format!("Book not found: {book_id}"))),
            Err(error) => {
                log::warn!("viewer 접근 확인 중 도서를 조회하지 못했다: book_id={book_id}, {error}");
                return Err(ApiError::unavailable());
            }
        };
        ensure_allowed(&book.category, &hidden_categories(deps, payload, comics)?)?;
    }
    Ok(manager)
}

pub(super) fn book_response(result: BookResponse) -> ApiResult<Response> {
    let mut response = Response::builder().status(result.status_code);
    if let Some(media_type) = result.media_type {
        response = response.header("content-type", media_type);
    }
    for (name, value) in result.headers {
        response = response.header(name, value);
    }
    response.body(Body::from(result.body)).map_err(|e| ApiError::Internal(e.to_string()))
}

#[derive(Deserialize)]
struct PreviewQuery {
    pages: Option<i64>,
    chapters: Option<i64>,
}

async fn preview(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiQuery(query): ApiQuery<PreviewQuery>,
) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let comics = uri.path().starts_with("/comics/");
    let manager = checked_manager(&deps, &payload, book_id, comics).await?;
    let base = format!("{}/html-resource/{book_id}", if comics { "/comics" } else { "" });
    book_response(run_blocking(async move { manager.get_book_preview_with_pages(book_id, query.pages, query.chapters, Some(&base)).await }).await?)
}

async fn download(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, Path(raw_id): Path<String>, OriginalUri(uri): OriginalUri, request_headers: HeaderMap) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    match manager.get_book_content(book_id).await {
        BookContent::Text(text) => Ok(Json(text).into_response()),
        BookContent::Error(message) => Err(ApiError::Internal(message)),
        BookContent::File(path) => {
            let (media_type, headers) = BookManager::file_response_meta(&path);
            let metadata = std::fs::metadata(&path).map_err(|e| ApiError::Internal(e.to_string()))?;
            let file_size = metadata.len();
            let mut headers = headers;
            let modified = metadata.modified().map_err(|e| ApiError::Internal(e.to_string()))?;
            let (last_modified, etag) = add_file_headers(modified, file_size, &mut headers)?;
            let range = request_headers.get("range").and_then(|value| value.to_str().ok());
            let if_range = request_headers.get("if-range").and_then(|value| value.to_str().ok());
            if let Some(range) = range
                && if_range.is_none_or(|value| value == last_modified || value == etag)
            {
                let ranges = match parse_byte_ranges(range, file_size) {
                    Ok(ranges) => ranges,
                    Err(error) => {
                        let (status, message, content_range) = match error {
                            RangeError::Malformed(message) => (400, message.to_string(), None),
                            RangeError::Unsatisfiable => (416, String::new(), Some(format!("bytes */{file_size}"))),
                        };
                        let mut error_headers = Vec::new();
                        set_header(&mut error_headers, "Content-Length", message.len().to_string());
                        if let Some(content_range) = content_range {
                            set_header(&mut error_headers, "Content-Range", content_range);
                        }
                        return book_response(BookResponse { status_code: status, media_type: Some("text/plain; charset=utf-8".to_string()), headers: error_headers, body: message.into_bytes() });
                    }
                };
                if ranges.len() == 1 {
                    let (start, end) = ranges[0];
                    set_header(&mut headers, "Content-Range", format!("bytes {start}-{}/{}", end - 1, file_size));
                    set_header(&mut headers, "Content-Length", (end - start).to_string());
                    let mut file = tokio::fs::File::open(&path).await.map_err(|e| ApiError::Internal(e.to_string()))?;
                    file.seek(SeekFrom::Start(start)).await.map_err(|e| ApiError::Internal(e.to_string()))?;
                    return response_with_body(206, Some(&media_type), headers, file_stream(file.take(end - start)));
                }
                let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
                let boundary = format!("{nonce:x}-{:x}", std::process::id());
                set_header(&mut headers, "Content-Type", format!("multipart/byteranges; boundary={boundary}"));
                set_header(&mut headers, "Content-Length", multipart_content_length(&ranges, &media_type, file_size, &boundary).to_string());
                let file = tokio::fs::File::open(&path).await.map_err(|e| ApiError::Internal(e.to_string()))?;
                let body = multipart_file_body(file, ranges, media_type, file_size, boundary);
                return response_with_body(206, None, headers, body);
            }
            let file = tokio::fs::File::open(&path).await.map_err(|e| ApiError::Internal(e.to_string()))?;
            response_with_body(200, Some(&media_type), headers, file_stream(file))
        }
    }
}

#[derive(Deserialize)]
struct ResourceQuery {
    path: String,
}

#[derive(Deserialize)]
struct CoverQuery {
    w: Option<i64>,
}

async fn cover(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiQuery(query): ApiQuery<CoverQuery>,
) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    book_response(manager.get_cover(book_id, query.w).await)
}

async fn html_resource(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiQuery(query): ApiQuery<ResourceQuery>,
) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    book_response(run_blocking(async move { manager.get_html_resource(book_id, &query.path).await }).await?)
}

async fn validate(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, Path(raw_id): Path<String>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    let Ok(book) = manager.get_book(book_id).await else { return Ok(Json(json!({"status":"failure", "error":format!("Book not found: {book_id}")}))) };
    let (result, error) = match book.file_type.as_str() {
        "epub" => manager.validate_epub(book_id).await,
        "pdf" => manager.validate_pdf(book_id).await,
        _ => return Ok(Json(json!({"status":"failure", "error":format!("Validation not supported for type: {}", book.file_type)}))),
    };
    Ok(Json(match (result, error) {
        (Some(result), None) => json!({"status":"success", "result":result}),
        (_, error) => json!({"status":"failure", "error":error}),
    }))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{is_hidden, run_blocking};

    #[test]
    fn is_hidden_matches_the_category_and_its_descendants_only() {
        let hidden = vec!["A".to_string()];
        assert!(is_hidden("A", &hidden));
        assert!(is_hidden("A/b/c", &hidden));
        assert!(!is_hidden("AB", &hidden), "이름이 같은 접두어인 형제 카테고리는 숨기지 않는다");
        assert!(!is_hidden("", &hidden));
        assert!(!is_hidden("A", &[]));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn run_blocking_keeps_the_worker_free() {
        // worker가 하나뿐이어도 느린 미리보기가 다른 요청을 막지 않아야 한다.
        let (started, wait_started) = tokio::sync::oneshot::channel();
        let slow = tokio::spawn(run_blocking(async move {
            let _ = started.send(());
            std::thread::sleep(Duration::from_millis(500));
            7
        }));
        wait_started.await.unwrap();
        let start = Instant::now();
        tokio::spawn(async { 1 }).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(200), "{:?}", start.elapsed());
        assert_eq!(slow.await.unwrap().unwrap(), 7);
    }
}
