//! 책 조회 라우트(`book_manager::catalog_query`): 책 한 권, 카테고리 목록과 안의 책, 최신·검색·유사 책, 내려받기, 비슷한 카테고리 이름, 카테고리별 PDF 통계.

use super::super::app_deps::AppDeps;
use super::super::error_responses::ApiError;
use super::super::error_responses::ApiResult;
use super::super::request_extractors::ApiQuery;
use super::super::request_extractors::RequireAuth;
use super::super::route_registry::Routes;
use super::ensure_allowed;
use super::hidden_categories;
use super::kind;
use super::manager;
use crate::api_router::CategoryMappingApi;
use crate::api_router::RequireAdmin;
use auth_manager::auth_service::Claims;
use axum::Json;
use axum::body::Body;
use axum::body::Bytes;
use axum::extract::OriginalUri;
use axum::extract::Path;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::response::Response;
use book_manager::BookContent;
use book_manager::BookManagerApi;
use book_manager::BookResponse;
use book_manager::MAX_LATEST_BOOK_COUNT;
use book_manager::book_metadata::Book;
use book_manager::book_metadata::BookView;
use db::category_mapping::is_hidden_category;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::io::SeekFrom;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

#[allow(unused_imports)]
use super::*;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .get(&format!("{prefix}/books/{{book_id}}"), &["require_auth"], metadata)
            .get(&format!("{prefix}/categories"), &["require_auth"], categories)
            .get(&format!("{prefix}/categories/{{*category}}"), &["require_auth"], in_category)
            .get(&format!("{prefix}/categories/rename"), &["require_auth"], reserved_category)
            .get(&format!("{prefix}/categories/delete"), &["require_auth"], reserved_category)
            .get(&format!("{prefix}/latest"), &["require_auth"], latest)
            .get(&format!("{prefix}/search/{{keyword}}"), &["require_auth"], search)
            .get(&format!("{prefix}/similar/{{book_id}}"), &["require_auth"], similar)
            .get(&format!("{prefix}/download/{{book_id}}"), &["require_auth"], download)
            .get(&format!("{prefix}/similar-names"), &["require_admin"], similar_names)
            .get(&format!("{prefix}/category-pdf-stats"), &["require_admin"], pdf_stats);
    }
    routes
}

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

async fn download(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, Path(raw_id): Path<String>, OriginalUri(uri): OriginalUri, request_headers: HeaderMap) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    match manager.get_book_content(book_id).await {
        BookContent::Text(text) => Ok(Json(text).into_response()),
        BookContent::Error(message) => Err(ApiError::Internal(message)),
        BookContent::File(path) => {
            let (media_type, headers) = book_manager::preview_builder::html_previewer::file_response_meta(&path);
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
struct SimilarNamesQuery {
    name: String,
    #[serde(default, rename = "author")]
    _author: String,
    #[serde(default)]
    exclude_category: String,
}

fn normalized_category_name(value: &str) -> Vec<char> {
    value.to_lowercase().replace(['_', '-'], " ").split_whitespace().collect::<Vec<_>>().join(" ").chars().collect()
}

fn sequence_ratio(a: &[char], b: &[char]) -> f64 {
    let (mut stack, mut matches) = (vec![(0, a.len(), 0, b.len())], 0usize);
    while let Some((alo, ahi, blo, bhi)) = stack.pop() {
        let (mut best_i, mut best_j, mut best_size) = (alo, blo, 0usize);
        let mut previous = vec![0usize; bhi.saturating_sub(blo) + 1];
        for (i, a_char) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut current = vec![0usize; previous.len()];
            for j in blo..bhi {
                if *a_char == b[j] {
                    current[j - blo + 1] = previous[j - blo] + 1;
                    if current[j - blo + 1] > best_size {
                        (best_i, best_j, best_size) = (i + 1 - current[j - blo + 1], j + 1 - current[j - blo + 1], current[j - blo + 1]);
                    }
                }
            }
            previous = current;
        }
        if best_size > 0 {
            matches += best_size;
            if alo < best_i && blo < best_j {
                stack.push((alo, best_i, blo, best_j));
            }
            if best_i + best_size < ahi && best_j + best_size < bhi {
                stack.push((best_i + best_size, ahi, best_j + best_size, bhi));
            }
        }
    }
    if a.is_empty() && b.is_empty() { 1.0 } else { 2.0 * matches as f64 / (a.len() + b.len()) as f64 }
}

async fn similar_names(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<SimilarNamesQuery>) -> ApiResult<Json<Value>> {
    let failure = |error: &str| Json(json!({"status":"failure","error":error}));
    let name = query.name.trim();
    if name.is_empty() {
        return Ok(failure("name is required"));
    }
    let manager = manager(&deps, &uri)?;
    let categories = match manager.get_categories().await {
        Ok(categories) => categories,
        Err(error) => return Ok(failure(&error)),
    };
    let categories: Vec<String> = match categories {
        Value::Array(values) => values.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Value::Object(values) => values.into_iter().map(|(key, _)| key).collect(),
        _ => Vec::new(),
    };
    let normalized = normalized_category_name(name);
    let excluded_prefix = (!query.exclude_category.is_empty()).then(|| format!("{}/", query.exclude_category));
    let mut matches: Vec<(String, f64)> = categories
        .into_iter()
        .filter(|category| *category != "_root" && *category != query.exclude_category && excluded_prefix.as_ref().is_none_or(|prefix| !category.starts_with(prefix)))
        .filter_map(|category| {
            let leaf = category.rsplit('/').next().unwrap_or(&category);
            let score = sequence_ratio(&normalized, &normalized_category_name(leaf));
            (score >= 0.35).then_some((category, score))
        })
        .collect();
    let mut stats_categories: Vec<String> = matches.iter().map(|(category, _)| category.clone()).collect();
    if !query.exclude_category.is_empty() && query.exclude_category != "_root" {
        stats_categories.push(query.exclude_category.clone());
    }
    let es = manager.es_manager();
    let stats = tokio::task::spawn_blocking(move || es.search_categories_pdf_stats(&stats_categories)).await;
    let stats = match stats {
        Ok(Ok(stats)) => stats,
        Err(error) => {
            log::error!("similar names join failed: {error}");
            return Ok(failure("유사 이름 검색에 실패했습니다."));
        }
        Ok(Err(error)) => {
            log::error!("similar names ES lookup failed: {error}");
            return Ok(failure("유사 이름 검색에 실패했습니다."));
        }
    };
    // 비교 기준 카테고리와 PDF 수·크기·쪽수가 가까울수록 점수를 더한다.
    let reference = stats.get(&query.exclude_category).copied();
    let similarity = |v: i64, r: i64| {
        if v == r {
            10.0
        } else if v.max(r) > 0 {
            10.0 * v.min(r) as f64 / v.max(r) as f64
        } else {
            0.0
        }
    };
    let mut result: Vec<Value> = matches.drain(..).map(|(category, name_score)| {
        let value = stats.get(&category).copied().unwrap_or_default();
        let mut score = name_score * 70.0;
        if let Some(r) = reference {
            score += similarity(value.file_count, r.file_count) + similarity(value.total_file_size, r.total_file_size) + similarity(value.page_count, r.page_count);
        }
        json!({"kind":"directory","id":category,"category":category,"label":category,"file_count":value.file_count,"page_count":value.page_count,"total_file_size":value.total_file_size,"score":score})
    }).collect();
    result.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
            .then_with(|| a["label"].as_str().unwrap_or("").to_lowercase().cmp(&b["label"].as_str().unwrap_or("").to_lowercase()))
    });
    result.truncate(10);
    Ok(Json(json!({"status":"success","result":result})))
}

async fn pdf_stats(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<CategoryNameQuery>) -> ApiResult<Json<Value>> {
    if query.category.is_empty() || query.category == "_root" {
        return Err(ApiError::http(400, "A directory category is required"));
    }
    let es = manager(&deps, &uri)?.es_manager();
    match tokio::task::spawn_blocking(move || es.search_category_pdf_stats(&query.category)).await {
        Ok(Ok(value)) => Ok(result(Ok(value))),
        error => {
            log::error!("get_category_pdf_stats failed: {error:?}");
            Ok(result(Err::<Value, _>("PDF 통계를 불러오지 못했습니다.".into())))
        }
    }
}

fn comics(uri: &Uri) -> bool {
    kind(uri) == "comic"
}

fn mapping(deps: &AppDeps) -> ApiResult<Arc<dyn CategoryMappingApi>> {
    deps.category_mapping().map_err(|e| {
        log::warn!("카테고리 접근 정책을 읽을 수 없다: {e}");
        ApiError::unavailable()
    })
}

fn hidden(deps: &AppDeps, payload: &Claims, uri: &Uri) -> ApiResult<Vec<String>> {
    hidden_categories(deps, payload, comics(uri))
}

fn book_views(books: &[Book], manager: &dyn BookManagerApi) -> Vec<BookView> {
    books.iter().map(|b| book_view(b, manager)).collect()
}

fn failure(error: Option<String>) -> Json<Value> {
    Json(json!({"status":"failure","error":error}))
}

async fn metadata(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    let manager = manager(&deps, &uri)?;
    match manager.get_book(id).await {
        Ok(book) => {
            ensure_allowed(&book.category, &hidden(&deps, &payload, &uri)?)?;
            Ok(Json(json!({"status":"success","result":book_view(&book,manager.as_ref())})))
        }
        Err(error) => Ok(failure(Some(error.to_string()))),
    }
}

async fn categories(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let manager = manager(&deps, &uri)?;
    let mut result = match manager.get_categories().await {
        Ok(result) => result,
        Err(error) => return Ok(failure(Some(error))),
    };
    let hidden = hidden(&deps, &payload, &uri)?;
    if let Some(categories) = result.as_object_mut() {
        categories.retain(|category, _| !is_hidden_category(category, &hidden));
    }
    Ok(Json(json!({"status":"success","result":result})))
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CategoryQuery {
    limit: i64,
    cursor: String,
}

// axum은 static 경로를 wildcard보다 먼저 선택한다. 같은 이름의 카테고리 GET도 유지한다.
async fn reserved_category(auth: RequireAuth, deps: State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, query: ApiQuery<CategoryQuery>) -> ApiResult<Json<Value>> {
    let category = uri.path().rsplit('/').next().unwrap_or_default().to_owned();
    in_category(auth, deps, OriginalUri(uri), Path(category), query).await
}

async fn in_category(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(category): Path<String>,
    ApiQuery(query): ApiQuery<CategoryQuery>,
) -> ApiResult<Json<Value>> {
    ensure_allowed(&category, &hidden(&deps, &payload, &uri)?)?;
    let manager = manager(&deps, &uri)?;
    if query.limit > 0 {
        return Ok(match manager.get_books_in_category_paged(&category, Some(query.limit), Some(&query.cursor)).await {
            Ok(page) => Json(json!({"status":"success","result":book_views(&page.books,manager.as_ref()),"total":page.total,"next_cursor":page.next_cursor})),
            Err(error) => failure(Some(error)),
        });
    }
    Ok(match manager.get_books_in_category(&category).await {
        Ok(books) => Json(json!({"status":"success","result":book_views(&books,manager.as_ref())})),
        Err(error) => failure(Some(error)),
    })
}

#[derive(Deserialize)]
struct LatestQuery {
    limit: Option<i64>,
}

async fn latest(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<LatestQuery>) -> ApiResult<Json<Value>> {
    let limit = query.limit.unwrap_or(MAX_LATEST_BOOK_COUNT);
    if !(1..=MAX_LATEST_BOOK_COUNT).contains(&limit) {
        return Err(crate::api_router::RequestValidationError::single(&["query", "limit"], "Invalid latest page size", json!(limit)).into());
    }
    let mut excluded = hidden(&deps, &payload, &uri)?;
    for category in mapping(&deps)?.get_latest_excluded_categories(kind(&uri))? {
        if !excluded.contains(&category) {
            excluded.push(category);
        }
    }
    let manager = manager(&deps, &uri)?;
    Ok(match manager.get_latest_books(limit, Some(&excluded)).await {
        Ok(page) => Json(json!({"status":"success","result":book_views(&page.books,manager.as_ref()),"total":page.total})),
        Err(error) => failure(Some(error)),
    })
}

#[derive(Deserialize)]
#[serde(default)]
struct SearchQuery {
    limit: i64,
    offset: i64,
    category: String,
    exclude_categories: String,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self { limit: 10, offset: 0, category: String::new(), exclude_categories: String::new() }
    }
}

async fn search(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(keyword): Path<String>,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let mut excluded: Vec<String> = query.exclude_categories.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
    let hidden = hidden(&deps, &payload, &uri)?;
    if !hidden.is_empty() {
        // 숨긴 카테고리를 합칠 때만 처음 순서를 지키며 중복을 제거한다.
        let mut merged = Vec::new();
        for category in excluded.into_iter().chain(hidden) {
            if !merged.contains(&category) {
                merged.push(category);
            }
        }
        excluded = merged;
    }
    let manager = manager(&deps, &uri)?;
    let excluded = (!excluded.is_empty()).then_some(excluded.as_slice());
    let category = (!query.category.is_empty()).then_some(query.category.as_str());
    Ok(match manager.search_by_keyword_paged(&keyword, query.limit, query.offset, excluded, category).await {
        Ok(page) => Json(json!({"status":"success","result":book_views(&page.books,manager.as_ref()),"total":page.total})),
        Err(error) => failure(Some(error)),
    })
}

async fn similar(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    let manager = super::checked_manager(&deps, &payload, id, comics(&uri)).await?;
    let similar = manager.search_similar_books_paged(id, query.limit, query.offset).await;
    let hidden = hidden(&deps, &payload, &uri)?;
    let error = match similar {
        Ok(mut page) => {
            if !hidden.is_empty() {
                page.books.retain(|book| !is_hidden_category(&book.category, &hidden));
                page.total = page.books.len() as i64;
            }
            if !page.books.is_empty() {
                return Ok(Json(json!({"status":"success","result":book_views(&page.books,manager.as_ref()),"total":page.total})));
            }
            None
        }
        Err(error) => Some(error),
    };
    match manager.get_book(id).await {
        Ok(book) => Ok(Json(json!({"status":"success","result":[book_view(&book,manager.as_ref())],"total":1}))),
        Err(second_error) => Ok(failure(error.or(Some(second_error.to_string())))),
    }
}

#[cfg(test)]
mod tests {
    use super::{normalized_category_name, sequence_ratio};

    #[test]
    fn category_similarity_handles_empty_equal_and_disjoint_names() {
        assert_eq!(normalized_category_name(" A__B--C "), "a b c".chars().collect::<Vec<_>>());
        assert_eq!(sequence_ratio(&[], &[]), 1.0);
        assert_eq!(sequence_ratio(&"same".chars().collect::<Vec<_>>(), &"same".chars().collect::<Vec<_>>()), 1.0);
        assert_eq!(sequence_ratio(&"abc".chars().collect::<Vec<_>>(), &"xyz".chars().collect::<Vec<_>>()), 0.0);
    }
}
