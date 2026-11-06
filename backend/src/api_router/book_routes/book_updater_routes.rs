//! 책 수정·삭제·색인 라우트(`book_manager::book_updater`): 책 정보 수정·삭제, 파일 이름 일괄 변경, 불일치 파일 색인·삭제, ES 문서 삭제.

use super::manager;
use crate::api_router::ApiError;
use crate::api_router::ApiJson;
use crate::api_router::ApiQuery;
use crate::api_router::ApiResult;
use crate::api_router::AppDeps;
use crate::api_router::RequireAdmin;
use crate::api_router::Routes;
use crate::api_router::request_extractors::lenient_bool;
use axum::Json;
use axum::extract::OriginalUri;
use axum::extract::Path;
use axum::extract::State;
use book_manager::BookDeletion;
use book_manager::book_metadata::Book;
use fancy_regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashSet;
use std::path::Path as FsPath;
use std::path::PathBuf;
use std::sync::Arc;

#[allow(unused_imports)]
use super::*;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .put(&format!("{prefix}/books/{{book_id}}"), &["require_admin"], update_book)
            .delete(&format!("{prefix}/books/{{book_id}}"), &["require_admin"], delete_book)
            .post(&format!("{prefix}/categories/bulk-rename-files"), &["require_admin"], bulk_rename_files)
            .post(&format!("{prefix}/category-mismatches/index-file"), &["require_admin"], index_file)
            .post(&format!("{prefix}/category-mismatches/delete-file"), &["require_admin"], delete_file)
            .delete(&format!("{prefix}/category-mismatches/es-doc/{{book_id}}"), &["require_admin"], delete_es_doc);
    }
    routes
}

#[derive(Deserialize)]
struct BulkRenameFiles {
    category: String,
    pattern: String,
    replacement: String,
}

async fn bulk_rename_files(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<BulkRenameFiles>) -> ApiResult<Json<Value>> {
    if body.category.is_empty() || body.category == "_root" {
        return Err(ApiError::http(400, "A directory category is required"));
    }
    if body.pattern.is_empty() || body.pattern.chars().count() > 256 {
        return Ok(Json(json!({"status":"failure","error":"정규표현식은 1~256자여야 합니다.","changed_count":0})));
    }
    let regex = match Regex::new(&body.pattern) {
        Ok(regex) => regex,
        Err(error) => return Ok(Json(json!({"status":"failure","error":format!("정규표현식 오류: {error}"),"changed_count":0}))),
    };
    let manager = manager(&deps, &uri)?;
    let categories = match manager.get_categories().await {
        Ok(categories) => categories,
        Err(error) => return Ok(Json(json!({"status":"failure","error":error,"changed_count":0}))),
    };
    let mut books = Vec::<Book>::new();
    if let Some(categories) = categories.as_array() {
        for category in categories.iter().filter_map(Value::as_str).filter(|category| *category == body.category || category.starts_with(&format!("{}/", body.category))) {
            let mut cursor: Option<String> = None;
            loop {
                let page = match manager.get_books_in_category_paged(category, None, cursor.as_deref()).await {
                    Ok(page) => page,
                    Err(error) => return Ok(Json(json!({"status":"failure","error":error,"changed_count":0}))),
                };
                books.extend(page.books);
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }
    }
    books.sort_by(|a, b| a.file_path.to_string_lossy().to_lowercase().cmp(&b.file_path.to_string_lossy().to_lowercase()));
    let mut plans = Vec::new();
    for book in books {
        let name = book.file_path.file_name().unwrap_or_default().to_string_lossy();
        let changed = match regex.try_replacen(&name, 0, body.replacement.as_str()) {
            Ok(value) => value.into_owned(),
            Err(error) => return Ok(Json(json!({"status":"failure","error":format!("정규표현식 오류: {error}"),"changed_count":0}))),
        };
        if changed.is_empty() || changed == "." || changed == ".." || changed.contains('/') || changed.contains('\\') {
            return Ok(Json(json!({"status":"failure","error":format!("변경할 수 없는 파일 이름입니다: {changed}"),"changed_count":0})));
        }
        if changed != name {
            plans.push((book, changed));
        }
    }

    let fs = deps.fs();
    let mut reserved = HashSet::<PathBuf>::new();
    let mut changed_count = 0usize;
    let mut failures = Vec::new();
    for (book, desired_name) in plans {
        let source = &book.file_path;
        let mut target = source.with_file_name(&desired_name);
        let exists = fs.is_file(&target).unwrap_or(false);
        let same = exists && fs.same_file(source, &target).unwrap_or(false);
        if (exists && !same) || reserved.contains(&target) {
            let extension = FsPath::new(&desired_name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            let stem = desired_name.strip_suffix(&extension).unwrap_or(&desired_name);
            let serial_re = Regex::new(&format!(r"^{} \((\d+)\){}$", fancy_regex::escape(stem), fancy_regex::escape(&extension))).expect("escaped serial regex");
            let mut largest = 0u64;
            if exists && !same {
                match fs.read_dir(source.parent().unwrap_or_else(|| FsPath::new("."))) {
                    Ok(entries) => {
                        for entry in entries {
                            if let Some(file_name) = entry.file_name().map(|n| n.to_string_lossy().into_owned())
                                && let Ok(Some(captures)) = serial_re.captures(&file_name)
                            {
                                largest = largest.max(captures.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0));
                            }
                        }
                    }
                    Err(error) => {
                        failures.push(json!({"file":source.file_name().unwrap_or_default().to_string_lossy(),"error":error.to_string()}));
                        continue;
                    }
                }
            } else {
                for path in &reserved {
                    if let Some(file_name) = path.file_name().map(|n| n.to_string_lossy())
                        && let Ok(Some(captures)) = serial_re.captures(&file_name)
                    {
                        largest = largest.max(captures.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0));
                    }
                }
            }
            let mut serial = largest + 1;
            loop {
                let candidate = format!("{stem} ({serial}){extension}");
                target = source.with_file_name(&candidate);
                if !fs.is_file(&target).unwrap_or(false) && !reserved.contains(&target) {
                    break;
                }
                serial += 1;
            }
        }
        let extension = FsPath::new(&target).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let title = target.file_name().unwrap_or_default().to_string_lossy();
        let title = if extension.is_empty() { title.to_string() } else { title.strip_suffix(&format!(".{extension}")).unwrap_or(&title).to_string() };
        match manager.update_book(book.book_id, &book.category, &title, &book.author, &target, &book.file_type, false).await {
            Ok(()) => {
                changed_count += 1;
                reserved.insert(target);
            }
            Err(error) => failures.push(json!({"file":source.file_name().unwrap_or_default().to_string_lossy(),"error":error})),
        }
    }
    if let Err(error) = manager.es_manager().refresh() {
        log::warn!("bulk rename 후 색인 갱신 실패: {error}");
    }
    let failed_count = failures.len();
    failures.truncate(20);
    Ok(Json(json!({"status":"success","result":{"changed_count":changed_count,"failed_count":failed_count,"failures":failures}})))
}

#[derive(Deserialize)]
struct ForceQuery {
    #[serde(default, deserialize_with = "lenient_bool")]
    force: bool,
}

// 사용하지 않는 BookModel 필드도 HTTP 입력 계약에 따라 검증한다.
#[derive(Deserialize)]
struct BookModel {
    #[serde(rename = "book_id")]
    _book_id: i64,
    category: String,
    title: String,
    author: String,
    file_path: String,
    file_type: String,
    #[serde(rename = "file_size")]
    _file_size: i64,
    #[serde(rename = "updated_time")]
    _updated_time: String,
    #[serde(default, rename = "line_count")]
    _line_count: i64,
    #[serde(default, rename = "page_count")]
    _page_count: i64,
    #[serde(default, rename = "isbn")]
    _isbn: String,
    #[serde(default, rename = "created_time")]
    _created_time: String,
    #[serde(default, rename = "score")]
    _score: f64,
}

async fn update_book(
    _: RequireAdmin,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<ForceQuery>,
    ApiJson(body): ApiJson<BookModel>,
) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    let manager = manager(&deps, &uri)?;
    let path = manager.path_prefix().join(&body.file_path);
    Ok(Json(match manager.update_book(id, &body.category, &body.title, &body.author, &path, &body.file_type, query.force).await {
        Ok(()) => json!({"status":"success","result":"Ok"}),
        Err(error) => json!({"status":"failure","error":error}),
    }))
}

async fn delete_book(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    Ok(Json(match manager(&deps, &uri)?.delete_book(id).await {
        Ok(BookDeletion::Done) => json!({"status":"success","result":"Ok"}),
        Ok(BookDeletion::DoneWithWarning(warning)) => json!({"status":"success","result":"Warning","warning":warning}),
        Err(error) => json!({"status":"failure","error":error}),
    }))
}

#[derive(Deserialize)]
struct FilePath {
    #[serde(default)]
    file_path: String,
}

async fn index_file(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<FilePath>) -> ApiResult<Json<Value>> {
    if body.file_path.is_empty() {
        return Err(ApiError::http(400, "file_path is required"));
    }
    Ok(Json(match manager(&deps, &uri)?.index_single_file(&body.file_path).await {
        Ok(book_id) => json!({"status":"success","result":{"book_id":book_id}}),
        Err(error) => json!({"status":"failure","error":error}),
    }))
}

async fn delete_file(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<FilePath>) -> ApiResult<Json<Value>> {
    if body.file_path.is_empty() {
        return Err(ApiError::http(400, "file_path is required"));
    }
    Ok(Json(match manager(&deps, &uri)?.delete_file(&body.file_path).await {
        Ok(()) => json!({"status":"success","result":"Ok"}),
        Err(error) => json!({"status":"failure","error":error}),
    }))
}

async fn delete_es_doc(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(raw_id): Path<String>) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&raw_id)?;
    let success = manager(&deps, &uri)?.es_manager().delete(id).unwrap_or(false);
    Ok(Json(if success { json!({"status":"success"}) } else { json!({"status":"failure","error":format!("ES 문서 삭제 실패: {id}")}) }))
}
