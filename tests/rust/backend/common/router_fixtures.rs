//! HTTP 라우터 테스트 준비물: 앱 조립(`Fx`), 요청 헤더·쿠키 helper, `CategoryMappingApi` 대역 구현.
//! backend(`api_router`) 테스트만 쓴다. 도메인 대역(`Mock`, `MockCat`)은 `shared_fixtures`에 있다.

#![allow(dead_code)]

use super::env::EnvGuard;
use super::shared_fixtures::*;
use super::{Response, TestClient};
use auth_manager::auth_service::Claims;
use auth_manager::refresh_token_store::{RefreshTokenStoreApi, Rotation, SessionPage, SessionQuery, SessionRevocation, StoreError};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use backend::api_router::{AppDeps, AuthMode, AuthSlots, CategoryMappingApi, MappingError};
use db::{DbValue, Row};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, atomic::Ordering};

pub fn ip_headers(headers: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in headers {
        map.insert(HeaderName::from_bytes(k.to_lowercase().as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
    }
    map
}

pub fn set_cookies(r: &Response) -> Vec<String> {
    r.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.clone()).collect()
}

/// 테스트 client와 manager 대역, 인증 우회를 한데 묶는다.
/// 인증 우회는 기본이 관리자이고, 테스트가 `set_auth`로 viewer로 바꾼다.
/// 원본에서 이 fixture들을 요청하지 않은 테스트도 같은 가짜 매니저를 쓴다. 그 테스트들은 매니저를 호출하지 않는다.
pub struct Fx {
    pub app: axum::Router,
    pub slots: Arc<AuthSlots>,
    pub bm: Mock,
    pub cm: Mock,
    pub cat: Mock,
    pub store: Store,
    pub _guard: Guard,
}

impl Fx {
    pub fn new() -> Self {
        super::setup_env();
        Self::build(Guard::Read(EnvGuard::read()), |deps| deps)
    }

    /// 환경 변수를 바꾸는 테스트용.
    pub fn with_env(vars: &[(&str, Option<&str>)]) -> Self {
        Self::build(Guard::Env(EnvGuard::apply(vars)), |deps| deps)
    }

    /// 환경 변수와 대역을 함께 바꾸는 테스트용.
    pub fn with_env_tweaked(vars: &[(&str, Option<&str>)], tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        Self::build(Guard::Env(EnvGuard::apply(vars)), tweak)
    }

    /// `mock.patch`로 내부 함수를 바꾸던 테스트용. `AppDeps`의 `with_*` 빌더를 `tweak`에서 이어 붙인다.
    pub fn tweaked(tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        super::setup_env();
        Self::build(Guard::Read(EnvGuard::read()), tweak)
    }

    /// `AppDeps`를 만드는 유일한 곳이다. 인증은 `AuthMode::Shared`라 테스트가 중간에 바꿀 수 있다.
    pub fn build(guard: Guard, tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        let (bm, cm, cat) = (Mock::new(), Mock::new(), Mock::new());
        let store = install_classify_status_store(&cat, None);
        let slots = Arc::new(AuthSlots::new(admin_payload(), admin_payload()));
        let deps = AppDeps::new(AuthMode::Shared(slots.clone()))
            .with_book_manager(Arc::new(bm.clone()))
            .with_comics_manager(Arc::new(cm.clone()))
            .with_category_mapping(Arc::new(cat.clone()))
            .with_refresh_token_store(Arc::new(Mock::new()))
            .with_task_runner(Arc::new(JoinedRunner));
        Self { app: backend::api_router::app_with(tweak(deps)), slots, bm, cm, cat, store, _guard: guard }
    }

    /// 인증 결과를 `payload`로 고정한다.
    pub fn set_auth(&self, payload: Claims) {
        *self.slots.auth.lock().unwrap() = payload;
    }

    pub fn client(&self) -> TestClient {
        TestClient::with_app(self.app.clone()).header("host", "testserver")
    }

    pub async fn get(&self, path: &str) -> Response {
        self.client().get(path).await
    }

    pub async fn delete(&self, path: &str) -> Response {
        self.client().delete(path).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Response {
        self.client().post_json(path, body).await
    }

    pub async fn put_json(&self, path: &str, body: &Value) -> Response {
        self.client().put_json(path, body).await
    }

    /// 본문 없는 POST.
    pub async fn post_empty(&self, path: &str) -> Response {
        self.client().request(Method::POST, path, None, Vec::new()).await
    }

    pub async fn post_with_cookie(&self, path: &str, name: &str, value: &str) -> Response {
        self.client().cookie(name, value).request(Method::POST, path, None, Vec::new()).await
    }
}

/// category_mapping 대역. 이름 붙은 인자(`content_type=...`)로 호출을 기록한다.
impl CategoryMappingApi for Mock {
    fn get_all_mappings(&self, ct: &str) -> Result<BTreeMap<String, Vec<String>>, MappingError> {
        self.invoke_sync_try("get_all_mappings", call_args!([], {"content_type": ct})).and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())).map_err(MappingError::Other)
    }

    fn get_keywords(&self, category: &str, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_keywords", call_args!([category], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_keywords(&self, category: &str, keywords: &[String], ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_keywords", call_args!([category, keywords], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn add_keyword(&self, category: &str, keyword: &str, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("add_keyword", call_args!([category, keyword], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn remove_keyword(&self, category: &str, keyword: &str, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("remove_keyword", call_args!([category, keyword], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn delete_category(&self, category: &str, ct: &str, prefix: bool) -> Result<bool, MappingError> {
        self.invoke_sync_try("delete_category", call_args!([category], {"content_type": ct, "prefix": prefix})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn update_all_mappings(&self, mappings: &Value, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("update_all_mappings", call_args!([mappings], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn get_hidden_categories(&self, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_hidden_categories", call_args!([], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_hidden(&self, category: &str, hidden: bool, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_hidden", call_args!([category, hidden], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn get_latest_excluded_categories(&self, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_latest_excluded_categories", call_args!([], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_latest_excluded(&self, category: &str, excluded: bool, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_latest_excluded", call_args!([category, excluded], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn acquire_reload_lock(&self, ct: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        let v = self.invoke_sync("acquire_reload_lock", call_args!([ct, category, reload_source], {}));
        Ok((flag(&arr(&v, 0)), opt_string(arr(&v, 1)), opt_json(arr(&v, 2))))
    }

    fn heartbeat_reload_lock(&self, ct: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        self.invoke_sync_try("heartbeat_reload_lock", CallArgs::new(vec![json!(ct), json!(category)], counts.clone())).map(|_| ())
    }

    fn complete_reload_lock(&self, ct: &str, status: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let mut kwargs = counts.clone();
        if let Some(category) = category {
            kwargs["category"] = json!(category);
        }
        self.invoke_sync_try("complete_reload_lock", CallArgs::new(vec![json!(ct), json!(status), json!(error)], kwargs)).map(|_| ())
    }

    fn get_reload_status(&self, ct: &str, category: Option<&str>) -> Result<Option<Value>, MappingError> {
        Ok(opt_json(self.invoke_sync("get_reload_status", call_args!([ct, category], {}))))
    }

    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, MappingError> {
        self.invoke_sync_try("get_classify_proposal_status", call_args!([content_type, stale_seconds], {})).map_err(MappingError::Other)
    }

    fn set_classify_proposal_status(&self, status: Value, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("set_classify_proposal_status", call_args!([status, content_type], {})).map(|_| ()).map_err(MappingError::Other)
    }

    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("merge_classify_proposal_status", call_args!([fields, content_type], {})).map(|_| ()).map_err(MappingError::Other)
    }

    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let v = self.invoke_sync("try_start_classify_proposal", call_args!([status, content_type, stale_seconds], {}));
        Ok((flag(&arr(&v, 0)), arr(&v, 1)))
    }

    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let v = self.invoke_sync("try_begin_classify_apply", call_args!([apply_token, content_type, stale_seconds], {}));
        Ok((flag(&arr(&v, 0)), arr(&v, 1)))
    }

    fn clear_classify_proposal_items(&self, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync("clear_classify_proposal_items", call_args!([], {"content_type": content_type}));
        Ok(())
    }

    fn get_classify_proposal_items(&self, content_type: &str) -> Result<Vec<Value>, MappingError> {
        Ok(self.invoke_sync("get_classify_proposal_items", call_args!([], {"content_type": content_type})).as_array().cloned().unwrap_or_default())
    }

    fn add_classify_proposal_items(&self, items: &[Value], source_category: &str, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("add_classify_proposal_items", call_args!([items, source_category], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn update_classify_proposal_item_payloads(&self, items: &[Value], content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("update_classify_proposal_item_payloads", call_args!([items], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("update_classify_proposal_item_status", call_args!([file_path, apply_status, apply_error], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn delete_applied_classify_proposal_items(&self, content_type: &str) -> Result<u64, MappingError> {
        Ok(count(&self.invoke_sync("delete_applied_classify_proposal_items", call_args!([], {"content_type": content_type}))).max(0) as u64)
    }

    fn delete_classify_proposal_item(&self, file_path: &str, content_type: &str) -> Result<u64, MappingError> {
        Ok(count(&self.invoke_sync("delete_classify_proposal_item", call_args!([file_path], {"content_type": content_type}))).max(0) as u64)
    }
}

impl CategoryMappingApi for MockCat {
    fn acquire_reload_lock(&self, content_type: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        let lock_key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let blocker = if category.is_some() {
            locks.get("_bulk").filter(|s| s["status"] == "running").or_else(|| locks.get(lock_key).filter(|s| s["status"] == "running"))
        } else {
            locks.values().find(|s| s["status"] == "running")
        };
        if let Some(status) = blocker {
            return Ok((false, Some("already running".into()), Some(status.clone())));
        }
        let status = json!({"status":"running","category":category,"reload_source":reload_source.unwrap_or("bulk"),"content_type":content_type,"indexed_count":0,"deleted_count":0,"failed_count":0,"before_count":0,"after_count":0,"error":null});
        locks.insert(lock_key.to_string(), status);
        Ok((true, None, None))
    }

    fn heartbeat_reload_lock(&self, _content_type: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let status = locks.get_mut(key).ok_or_else(|| "missing reload lock".to_string())?;
        for (field, value) in counts.as_object().ok_or_else(|| "counts must be an object".to_string())? {
            status[field] = value.clone();
        }
        Ok(())
    }

    fn complete_reload_lock(&self, _content_type: &str, state: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let status = locks.get_mut(key).ok_or_else(|| "missing reload lock".to_string())?;
        status["status"] = json!(state);
        status["error"] = json!(error);
        for (field, value) in counts.as_object().ok_or_else(|| "counts must be an object".to_string())? {
            status[field] = value.clone();
        }
        Ok(())
    }

    fn get_all_mappings(&self, _content_type: &str) -> Result<BTreeMap<String, Vec<String>>, backend::api_router::MappingError> {
        Ok(BTreeMap::new())
    }

    fn get_reload_status(&self, _content_type: &str, category: Option<&str>) -> Result<Option<Value>, MappingError> {
        Ok(self.reload_locks.lock().unwrap().get(category.unwrap_or("_bulk")).cloned())
    }

    fn add_classify_proposal_items(&self, items: &[Value], _source_category: &str, _content_type: &str) -> Result<(), MappingError> {
        *self.proposal_write_thread.lock().unwrap() = Some(std::thread::current().id());
        self.proposal_add_batch_sizes.lock().unwrap().push(items.len());
        self.proposal_items.lock().unwrap().extend_from_slice(items);
        Ok(())
    }

    fn update_classify_proposal_item_payloads(&self, items: &[Value], _content_type: &str) -> Result<(), MappingError> {
        let mut current = self.proposal_items.lock().unwrap();
        for item in items {
            let path = item["file_path"].as_str();
            if let Some(existing) = current.iter_mut().find(|old| old["file_path"].as_str() == path) {
                *existing = item.clone();
            }
        }
        Ok(())
    }

    fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, _content_type: &str) -> Result<(), MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let item = items.iter_mut().find(|item| item["file_path"] == file_path).ok_or_else(|| MappingError::Other("proposal item not found".into()))?;
        item["apply_status"] = json!(apply_status);
        item["apply_error"] = apply_error.map_or(Value::Null, |error| json!(error));
        Ok(())
    }

    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, MappingError> {
        Ok(self.effective(content_type, stale_seconds))
    }

    fn set_classify_proposal_status(&self, mut status: Value, content_type: &str) -> Result<(), MappingError> {
        status["content_type"] = json!(content_type);
        self.store.lock().unwrap().insert(content_type.to_string(), status);
        Ok(())
    }

    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) -> Result<(), MappingError> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(content_type.to_string()).or_insert(json!({"status": "idle", "content_type": content_type}));
        for (key, value) in fields.as_object().unwrap() {
            if key != "status" {
                entry[key] = value.clone();
            }
        }
        Ok(())
    }

    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let current = self.effective(content_type, stale_seconds);
        if matches!(current["status"].as_str(), Some("running" | "applying")) {
            return Ok((false, current));
        }
        self.set_classify_proposal_status(status, content_type).unwrap();
        Ok((true, self.effective(content_type, stale_seconds)))
    }

    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let current = self.effective(content_type, stale_seconds);
        if !matches!(current["status"].as_str(), Some("ready" | "done" | "failed")) {
            return Ok((false, current));
        }
        let mut next = current;
        next["status"] = json!("applying");
        next["apply_token"] = json!(apply_token);
        self.set_classify_proposal_status(next, content_type).unwrap();
        Ok((true, self.effective(content_type, stale_seconds)))
    }

    fn clear_classify_proposal_items(&self, _content_type: &str) -> Result<(), MappingError> {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        self.proposal_items.lock().unwrap().clear();
        Ok(())
    }

    fn get_classify_proposal_items(&self, _content_type: &str) -> Result<Vec<Value>, MappingError> {
        Ok(self.proposal_items.lock().unwrap().clone())
    }

    fn delete_classify_proposal_item(&self, file_path: &str, _content_type: &str) -> Result<u64, MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let before = items.len();
        items.retain(|item| item["file_path"] != file_path);
        Ok((before - items.len()) as u64)
    }

    fn delete_applied_classify_proposal_items(&self, _content_type: &str) -> Result<u64, MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let before = items.len();
        items.retain(|item| item["apply_status"] != "moved");
        Ok((before - items.len()) as u64)
    }
}

pub fn admin_payload() -> Claims {
    Claims { email: "admin@test.com".into(), role: "admin".into(), name: "Admin".into(), exp: now_plus_3600() as i64, ..Claims::default() }
}

#[allow(clippy::too_many_arguments)]
impl RefreshTokenStoreApi for FakeStore {
    fn store_issued(&self, token_id: &str, family_id: &str, email: &str, _issued_at: i64, _expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError> {
        self.issued.lock().unwrap().push(json!({"token_id": token_id, "family_id": family_id, "email": email, "client_ip": client_ip, "user_agent": user_agent}));
        Ok(())
    }

    fn rotate(
        &self,
        current_token_id: &str,
        new_token_id: &str,
        family_id: &str,
        email: &str,
        _issued_at: i64,
        _expires_at: i64,
        client_ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<Rotation, StoreError> {
        self.rotated
            .lock()
            .unwrap()
            .push(json!({"current_token_id": current_token_id, "new_token_id": new_token_id, "family_id": family_id, "email": email, "client_ip": client_ip, "user_agent": user_agent}));
        Ok(self.rotate_status.lock().unwrap().as_deref().map_or(Rotation::Ok, |status| status.parse().unwrap()))
    }

    fn revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError> {
        self.revoked_families.lock().unwrap().push((family_id.to_string(), reason.to_string()));
        Ok(())
    }

    fn get_token_observation(&self, _token_id: &str) -> Result<Option<Value>, StoreError> {
        match self.observation.lock().unwrap().clone() {
            Some(result) => result.map_err(StoreError::Unsupported),
            None => Ok(None),
        }
    }

    fn session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
        if let Some(Err(error)) = self.sessions.lock().unwrap().as_ref() {
            return Err(StoreError::Unsupported(error.clone()));
        }
        let revoked = self.revoked_families.lock().unwrap();
        let revoked_sessions = self.revoke_session_calls.lock().unwrap();
        Ok(self
            .issued
            .lock()
            .unwrap()
            .iter()
            .filter(|row| email.is_none_or(|e| row["email"] == e) && family_id.is_none_or(|fid| row["family_id"] == fid))
            .map(|row| {
                let fid = row["family_id"].as_str().unwrap();
                let active = !revoked.iter().any(|(family, _)| family == fid) && !revoked_sessions.iter().any(|session| session["family_id"] == fid);
                Row::from([("family_id".into(), fid.into()), ("email".into(), row["email"].as_str().unwrap().into()), ("valid_token_count".into(), i64::from(active).into())])
            })
            .collect())
    }

    fn list_sessions(&self, q: &SessionQuery) -> Result<SessionPage, StoreError> {
        // 이름 붙은 인자를 객체로 담아 둔다.
        self.list_calls.lock().unwrap().push(json!({"status": q.status, "current_family_id": q.current_family_id, "page": q.page, "page_size": q.page_size, "email": q.email}));
        let page = self.sessions.lock().unwrap().clone().unwrap_or_else(|| Err("unset".to_string())).map_err(StoreError::Unsupported)?;
        Ok(serde_json::from_value(page).expect("세션 목록 모양"))
    }

    fn revoke_session(&self, family_id: &str) -> Result<SessionRevocation, StoreError> {
        self.revoke_session_calls.lock().unwrap().push(json!({"family_id": family_id}));
        let outcome = self.revoke_result.lock().unwrap().clone().unwrap_or_else(|| Err("unset".to_string())).map_err(StoreError::Unsupported)?;
        Ok(serde_json::from_value(outcome).expect("세션 폐기 결과 모양"))
    }
}

/// refresh token 저장소 대역. `rotate`/`revoke_family`만 쓴다.
/// TODO(common): `RefreshTokenStoreApi`의 시그니처는 설계 문서에 없다.
impl RefreshTokenStoreApi for Mock {
    fn store_issued(&self, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError> {
        let kwargs = json!({"token_id":token_id,"family_id":family_id,"email":email,"issued_at":issued_at,"expires_at":expires_at,"client_ip":client_ip,"user_agent":user_agent});
        self.invoke_sync_try("store_issued", CallArgs::new(vec![], kwargs)).map(|_| ()).map_err(StoreError::Unsupported)
    }
    #[allow(clippy::too_many_arguments)]
    fn rotate(
        &self,
        current_token_id: &str,
        new_token_id: &str,
        family_id: &str,
        email: &str,
        issued_at: i64,
        expires_at: i64,
        client_ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<Rotation, StoreError> {
        let kwargs = json!({"current_token_id": current_token_id, "new_token_id": new_token_id, "family_id": family_id, "email": email, "issued_at": issued_at, "expires_at": expires_at, "client_ip": client_ip, "user_agent": user_agent});
        self.invoke_sync_try("rotate", CallArgs::new(vec![], kwargs)).and_then(|v| v.as_str().unwrap_or("").parse()).map_err(StoreError::Unsupported)
    }

    fn revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError> {
        self.invoke_sync_try("revoke_family", call_args!([family_id, reason], {})).map(|_| ()).map_err(StoreError::Unsupported)
    }

    fn get_token_observation(&self, token_id: &str) -> Result<Option<Value>, StoreError> {
        self.invoke_sync_try("get_token_observation", call_args!([token_id], {})).map(opt_json).map_err(StoreError::Unsupported)
    }

    fn session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
        let value = self.invoke_sync_try("session_rows", call_args!([email, family_id], {})).map_err(StoreError::Unsupported)?;
        let rows = value
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                row.as_object()
                    .into_iter()
                    .flat_map(|obj| obj.iter())
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            match value {
                                Value::Null => DbValue::Null,
                                Value::Number(n) => DbValue::Int(n.as_i64().unwrap_or_default()),
                                Value::String(s) => DbValue::Text(s.clone()),
                                _ => DbValue::Text(value.to_string()),
                            },
                        )
                    })
                    .collect()
            })
            .collect();
        Ok(rows)
    }
}
