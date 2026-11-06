//! 카테고리 이름 변경·삭제 라우트(`book_manager::category_editor`).

use super::kind;
use super::manager;
use crate::api_router::ApiJson;
use crate::api_router::ApiQuery;
use crate::api_router::ApiResult;
use crate::api_router::AppDeps;
use crate::api_router::RequireAdmin;
use crate::api_router::Routes;
use crate::api_router::request_extractors::lenient_bool;
use axum::Json;
use axum::extract::OriginalUri;
use axum::extract::State;
use book_manager::CategoryDeletion;
use book_manager::CategoryRename;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

#[allow(unused_imports)]
use super::*;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes.put(&format!("{prefix}/categories/rename"), &["require_admin"], rename_category).post(&format!("{prefix}/categories/delete"), &["require_admin"], delete_category).get(
            &format!("{prefix}/category-delete-preview"),
            &["require_admin"],
            delete_preview,
        );
    }
    routes
}

#[derive(Deserialize)]
struct Rename {
    old_category: String,
    new_category: String,
}

async fn rename_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Rename>) -> ApiResult<Json<Value>> {
    /// 이름 변경 결과에 MySQL 매핑 갱신 여부를 더한 응답.
    #[derive(Serialize)]
    struct Renamed {
        #[serde(flatten)]
        renamed: CategoryRename,
        mapping_updated: bool,
    }
    let renamed = match manager(&deps, &uri)?.rename_category(&body.old_category, &body.new_category).await {
        Ok(renamed) => renamed,
        Err(error) => return Ok(result(Err::<Renamed, _>(error))),
    };
    let ct = kind(&uri);
    let mapping_updated = with_mapping(deps, move |m| m.rename_category(&body.old_category, &body.new_category, ct)).await?;
    if !mapping_updated {
        log::warn!("rename_category: MySQL 매핑 갱신 실패");
    }
    Ok(result(Ok(Renamed { renamed, mapping_updated })))
}

#[derive(Deserialize)]
struct Delete {
    category: String,
    #[serde(default, deserialize_with = "lenient_bool")]
    delete_files: bool,
}

async fn delete_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Delete>) -> ApiResult<Json<Value>> {
    /// 삭제 결과에 MySQL 매핑 삭제 여부를 더한 응답.
    #[derive(Serialize)]
    struct Deleted {
        #[serde(flatten)]
        deleted: CategoryDeletion,
        mapping_deleted: bool,
    }
    let deleted_category = match manager(&deps, &uri)?.delete_category(&body.category, body.delete_files).await {
        Ok(deleted) => deleted,
        Err(error) => return Ok(result(Err::<Deleted, _>(error))),
    };
    {
        let ct = kind(&uri);
        let deleted = with_mapping(deps, move |m| {
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
        Ok(result(Ok(Deleted { deleted: deleted_category, mapping_deleted: deleted })))
    }
}

async fn delete_preview(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<CategoryNameQuery>) -> ApiResult<Json<Value>> {
    Ok(result(manager(&deps, &uri)?.preview_delete_category(&query.category).await))
}
