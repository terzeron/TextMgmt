//! 도서·만화 수정과 카테고리 관리. 실제 파일 작업은 BookManager에 위임한다.
use super::{kind, manager};
use crate::book_manager::BookDeletion;
use crate::book_metadata::Book;
use crate::main::request_extractors::lenient_bool;
use crate::main::{ApiError, ApiJson, ApiQuery, ApiResult, AppDeps, ApplyCategoryChanges, BookManagerApi, CategoryMappingApi, MappingError, ProposeCategoryChanges, RequireAdmin, Routes};
use axum::{
    Json,
    extract::{OriginalUri, Path, State},
};
use fancy_regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex},
};

mod classify_proposal_routes;

use classify_proposal_routes::{
    apply_classify_proposal, delete_applied_classify_proposal_items, delete_classify_proposal, delete_classify_proposal_item, get_classify_proposal, start_classify_proposal,
};

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .put(&format!("{prefix}/books/{{book_id}}"), &["require_admin"], update_book)
            .delete(&format!("{prefix}/books/{{book_id}}"), &["require_admin"], delete_book)
            .put(&format!("{prefix}/categories/rename"), &["require_admin"], rename_category)
            .post(&format!("{prefix}/categories/bulk-rename-files"), &["require_admin"], bulk_rename_files)
            .post(&format!("{prefix}/categories/delete"), &["require_admin"], delete_category)
            .get(&format!("{prefix}/category-delete-preview"), &["require_admin"], delete_preview)
            .delete(&format!("{prefix}/categories/classify-proposal"), &["require_admin"], delete_classify_proposal)
            .get(&format!("{prefix}/categories/classify-proposal"), &["require_admin"], get_classify_proposal)
            .post(&format!("{prefix}/categories/classify-proposal"), &["require_admin"], start_classify_proposal)
            .post(&format!("{prefix}/categories/classify-proposal/apply"), &["require_admin"], apply_classify_proposal)
            .delete(&format!("{prefix}/categories/classify-proposal/item"), &["require_admin"], delete_classify_proposal_item)
            .delete(&format!("{prefix}/categories/classify-proposal/applied"), &["require_admin"], delete_applied_classify_proposal_items)
            .get(&format!("{prefix}/similar-names"), &["require_admin"], similar_names)
            .get(&format!("{prefix}/category-pdf-stats"), &["require_admin"], pdf_stats);
    }
    routes
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
    let reference = stats.get(&query.exclude_category).cloned().unwrap_or(Value::Null);
    let mut result: Vec<Value> = matches.drain(..).map(|(category, name_score)| {
        let value = stats.get(&category).cloned().unwrap_or(Value::Null);
        let mut score = name_score * 70.0;
        for key in ["file_count", "total_file_size", "page_count"] {
            let v = value[key].as_i64().unwrap_or(0);
            let r = reference[key].as_i64().unwrap_or(0);
            if !reference.is_null() { score += if v == r { 10.0 } else if v.max(r) > 0 { 10.0 * v.min(r) as f64 / v.max(r) as f64 } else { 0.0 }; }
        }
        json!({"kind":"directory","id":category,"category":category,"label":category,"file_count":value["file_count"].as_i64().unwrap_or(0),"page_count":value["page_count"].as_i64().unwrap_or(0),"total_file_size":value["total_file_size"].as_i64().unwrap_or(0),"score":score})
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
fn result(outcome: Result<Value, String>) -> Json<Value> {
    Json(match outcome {
        Ok(value) => json!({"status":"success","result":value}),
        Err(error) => json!({"status":"failure","error":error}),
    })
}
async fn mapping<T: Send + 'static>(deps: Arc<AppDeps>, action: impl FnOnce(Arc<dyn CategoryMappingApi>) -> Result<T, MappingError> + Send + 'static) -> ApiResult<T> {
    let result = tokio::task::spawn_blocking(move || deps.category_mapping().map_err(MappingError::Other).and_then(action)).await.map_err(|e| ApiError::Internal(e.to_string()))?;
    result.map_err(|e| match e {
        MappingError::Http(code, message) => ApiError::http(code, &message),
        MappingError::Other(message) => ApiError::Internal(message),
    })
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
struct Rename {
    old_category: String,
    new_category: String,
}
async fn rename_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Rename>) -> ApiResult<Json<Value>> {
    let mut renamed = manager(&deps, &uri)?.rename_category(&body.old_category, &body.new_category).await;
    if let Ok(value) = &mut renamed {
        let ct = kind(&uri);
        let updated = mapping(deps, move |m| m.rename_category(&body.old_category, &body.new_category, ct)).await?;
        if !updated {
            log::warn!("rename_category: MySQL 매핑 갱신 실패");
        }
        value["mapping_updated"] = json!(updated);
    }
    Ok(result(renamed))
}
#[derive(Deserialize)]
struct Delete {
    category: String,
    #[serde(default, deserialize_with = "lenient_bool")]
    delete_files: bool,
}
async fn delete_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Delete>) -> ApiResult<Json<Value>> {
    let mut deleted_category = manager(&deps, &uri)?.delete_category(&body.category, body.delete_files).await;
    if let Ok(value) = &mut deleted_category {
        let ct = kind(&uri);
        let deleted = mapping(deps, move |m| {
            let deleted = m.delete_category(&body.category, ct, true)?;
            let prefix = format!("{}/", body.category);
            m.set_hidden(&body.category, false, ct)?;
            for hidden in m.get_hidden_categories(ct)? {
                if hidden.starts_with(&prefix) {
                    m.set_hidden(&hidden, false, ct)?;
                }
            }
            m.set_latest_excluded(&body.category, false, ct)?;
            for excluded in m.get_latest_excluded_categories(ct)? {
                if excluded.starts_with(&prefix) {
                    m.set_latest_excluded(&excluded, false, ct)?;
                }
            }
            Ok(deleted)
        })
        .await?;
        if !deleted {
            log::warn!("delete_category: MySQL 키워드 매핑 삭제 대상 없음");
        }
        value["mapping_deleted"] = json!(deleted);
    }
    Ok(result(deleted_category))
}
#[derive(Deserialize)]
struct CategoryQuery {
    category: String,
}
async fn delete_preview(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<CategoryQuery>) -> ApiResult<Json<Value>> {
    Ok(result(manager(&deps, &uri)?.preview_delete_category(&query.category).await))
}
async fn pdf_stats(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<CategoryQuery>) -> ApiResult<Json<Value>> {
    if query.category.is_empty() || query.category == "_root" {
        return Err(ApiError::http(400, "A directory category is required"));
    }
    let es = manager(&deps, &uri)?.es_manager();
    match tokio::task::spawn_blocking(move || es.search_category_pdf_stats(&query.category)).await {
        Ok(Ok(value)) => Ok(result(Ok(value))),
        error => {
            log::error!("get_category_pdf_stats failed: {error:?}");
            Ok(result(Err("PDF 통계를 불러오지 못했습니다.".into())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{kind, manager, mapping, normalized_category_name, result, sequence_ratio};
    use crate::main::{ApiError, AppDeps, AuthMode, CategoryMappingApi, MappingError};
    use axum::http::Uri;
    use serde_json::json;

    /// `mapping`은 매핑 저장소를 action에 넘기기만 한다. 이 테스트의 action은 저장소를 쓰지 않는다.
    struct NoMapping;

    impl CategoryMappingApi for NoMapping {}

    #[tokio::test]
    async fn mapping_runs_action_and_converts_mapping_errors() {
        let deps = std::sync::Arc::new(AppDeps::new(AuthMode::Cookies).with_category_mapping(std::sync::Arc::new(NoMapping)));

        let value = mapping(deps.clone(), |_| Ok::<_, MappingError>(42)).await.unwrap();
        assert_eq!(value, 42);

        let http_error = mapping(deps.clone(), |_| Err::<(), _>(MappingError::Http(409, "conflict".into()))).await.unwrap_err();
        assert!(matches!(http_error, ApiError::Http(error) if error.status_code == 409 && error.detail == "conflict"));

        let other_error = mapping(deps, |_| Err::<(), _>(MappingError::Other("storage failed".into()))).await.unwrap_err();
        assert!(matches!(other_error, ApiError::Internal(message) if message == "storage failed"));
    }

    #[test]
    fn category_similarity_handles_empty_equal_and_disjoint_names() {
        assert_eq!(normalized_category_name(" A__B--C "), "a b c".chars().collect::<Vec<_>>());
        assert_eq!(sequence_ratio(&[], &[]), 1.0);
        assert_eq!(sequence_ratio(&"same".chars().collect::<Vec<_>>(), &"same".chars().collect::<Vec<_>>()), 1.0);
        assert_eq!(sequence_ratio(&"abc".chars().collect::<Vec<_>>(), &"xyz".chars().collect::<Vec<_>>()), 0.0);
    }

    #[test]
    fn content_type_and_manager_follow_the_original_uri() {
        let deps = AppDeps::new(AuthMode::Cookies);
        let book_uri = Uri::from_static("/books/1");
        let comic_uri = Uri::from_static("/comics/books/1");

        assert_eq!(kind(&book_uri), "book");
        assert_eq!(kind(&comic_uri), "comic");
        let book_manager = manager(&deps, &book_uri).unwrap();
        let comic_manager = manager(&deps, &comic_uri).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&book_manager, &comic_manager));
    }

    #[test]
    fn result_serializes_success_and_failure_shapes() {
        let success = result(Ok(json!({"count":2})));
        assert_eq!(success.0, json!({"status":"success","result":{"count":2}}));

        let failure = result(Err("failed".into()));
        assert_eq!(failure.0, json!({"status":"failure","error":"failed"}));
    }
}
