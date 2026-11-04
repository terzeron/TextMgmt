//! `backend::auth_service` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

mod common;

// `tests/rust/backend/auth.rs`에서 옮긴 테스트.
mod auth {
    //! 원본: tests/test_auth.py
    //!
    //! JWT 발급·검증, 권한 의존성(`require_auth`/`require_admin`), 쿠키 설정, Google OAuth 로그인, refresh 회전 관측 로그,
    //! 접속 IP 추출, admin 로그인 세션 관리 API 테스트. Python의 autouse `setup_env` fixture는 `fixture()`(`common::env::EnvGuard`)로 옮겼다.
    //! Python `monkeypatch.setattr(main_mod, ...)`는 `AppDeps`의 `with_google_verifier`/`with_role_resolver`/`with_refresh_token_store`로 옮겼다.
    //! 토큰 발급 함수의 반환값을 고정해야 하는 `test_auth_refresh_and_logout`은 `AppDeps::with_token_issuer(Arc<dyn TokenIssuer>)` seam으로 풀었다.
    //!
    //! 가정한 API (아직 구현 전이다. 설계: docs/rust-test-seams.md 3.2, 3.4, 3.9, 3.10):
    //! - `backend::auth_service` (환경 변수는 호출 시점에 읽는다. Python의 `importlib.reload` 불필요)
    //!   - `determine_role(email: &str) -> Option<String>`
    //!   - `create_jwt_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String` (4인자 통일안. None은 빈 문자열)
    //!   - `create_refresh_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String`
    //!   - `decode_refresh_token(token: &str) -> Result<Value, HTTPException>`, `_extract_payload<R: RequestParts>(req: &R) -> Result<Value, HTTPException>`
    //!   - `async require_auth<R: RequestParts>(req: &R) -> Result<Value, HTTPException>`, `async require_admin<R: RequestParts>(req: &R) -> Result<Value, HTTPException>`
    //!   - `require_auth_with(req, resolver: &dyn RoleResolver)`, `require_admin_with(..)` (async, 같은 반환형. `monkeypatch.setattr(auth_mod, "determine_role", ..)` 대응)
    //!   - `trait RequestParts { fn header(&self, name: &str) -> Option<String>; fn cookie(&self, name: &str) -> Option<String>; }`
    //!   - `struct HTTPException { pub status_code: u16, pub detail: String }` + `HTTPException::new(u16, &str)`, `struct SystemExit(pub i32)`
    //!   - `load_jwt_secret(env: EnvFn) -> Result<String, SystemExit>` (`EnvFn = &dyn Fn(&str) -> Option<String>`. Python `import backend.auth` 시 TM_JWT_SECRET 없으면 SystemExit)
    //!   - `ACCESS_TOKEN_EXPIRATION_SECONDS: i64`
    //! - `backend::main`
    //!   - `app() -> axum::Router`, `app_with(deps: AppDeps) -> axum::Router`, `AppDeps::new(auth: AuthMode) -> AppDeps`
    //!   - `AuthMode::{Cookies, Fixed(Value), Split { auth: Value, admin: Value }, AuthOnly(Value), Shared(..)}`: `require_auth`/`require_admin` dependency_overrides 대응
    //!     (`AuthOnly`는 require_auth만 고정하고 require_admin은 실제 구현을 쓴다. 구 `Real`/`Overrides`는 `Cookies`/`Split`)
    //!   - `AppDeps` 소비형 빌더: `.with_google_verifier(Arc<dyn GoogleVerifier>)`, `.with_role_resolver(Arc<dyn RoleResolver>)`, `.with_refresh_token_store(Arc<dyn RefreshTokenStoreApi>)`
    //!   - `trait GoogleVerifier { fn client_id(&self) -> Option<String>; fn verify(&self, credential: &str, audience: &str) -> Result<Value, String>; }`
    //!   - `trait RoleResolver { fn determine_role(&self, email: &str) -> Option<String>; }`
    //!   - (모든 trait 메서드에 기본 구현이 있다. fake는 쓰는 메서드만 구현한다)
    //!   - `trait TokenIssuer` (`backend::auth_service`, 모든 메서드에 기본 구현): `create_access_token`, `create_refresh_token`, `decode_refresh_token`, `new_id`. `AppDeps::with_token_issuer(Arc<dyn TokenIssuer>)`
    //!   - `ACCESS_COOKIE_NAME`, `REFRESH_COOKIE_NAME: &str`, `observation_hash(&str) -> String`
    //!   - `_get_cookie_settings() -> (bool, String)`, `_set_auth_cookies(&mut CustomJSONResponse, &str, &str)`, `_clear_auth_cookies(&mut CustomJSONResponse)`
    //!   - `CustomJSONResponse::{new(Value) -> Self, headers(&self) -> &HeaderMap}`, `custom_jsonable_encoder(&Value) -> Value`
    //!   - `_client_ip(headers: &HeaderMap, client_host: Option<&str>) -> String`, `_request_ip_prefix(headers: &HeaderMap, client_host: Option<&str>) -> String`
    //!   - `async validation_exception_handler(&Parts, &RequestValidationError) -> Response`, `async http_exception_handler(&Parts, &HTTPException) -> Response`,
    //!     `async general_exception_handler(&Parts, &(dyn Error + Send + Sync)) -> Response`, `RequestValidationError::new(Vec<Value>, Value /*body*/)`
    //!   - `route_table() -> Vec<RouteInfo>`, `struct RouteInfo { path: String, methods: Vec<String>, dependencies: Vec<String> }` (FastAPI `route.dependant.dependencies` 대응)
    //! - `backend::refresh_token_store`: `trait RefreshTokenStoreApi`(store_issued, rotate, revoke_family, get_token_observation, list_sessions, revoke_session),
    //!   `SessionQuery { status, current_family_id, page, page_size, email }`, `StoreError::Unsupported(String)`
    //! - 로그 캡처는 `common::log_capture::LogCapture`(`log` crate 기반), 환경 변수는 `common::env::EnvGuard`.
    //! - dev-dependency: `jsonwebtoken`, `tokio`
    //!
    //! 토큰 관련 대역: `decode_refresh_token`/`create_*_token` 전역 덮어쓰기는 seam이 없다. refresh 쿠키는 `crafted_refresh()`가 실제 서명한 토큰으로 만든다.
    #![allow(clippy::await_holding_lock)]
    use super::{common};

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::Router;
    use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
    use backend::auth_service::{HTTPException, RequestParts, SystemExit};
    use db::Row;
    use backend::main::{ACCESS_COOKIE_NAME, AppDeps, AuthMode, CustomJSONResponse, REFRESH_COOKIE_NAME, RequestValidationError, RoleResolver, app_with, observation_hash};
    use backend::refresh_token_store::{RefreshTokenStoreApi, Rotation, SessionQuery, StoreError};
    use common::env::EnvGuard;
    use common::log_capture::LogCapture;
    use common::{Response, TestClient};
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
    use serde_json::{Value, json};

    // ---- 환경 변수 / 토큰 / 요청 헬퍼 ----

    const SECRET: &str = "testsecret123";

    /// Python autouse `setup_env` fixture: JWT 비밀키·관리자·허용 이메일을 테스트 값으로 맞춘다.
    fn fixture() -> EnvGuard {
        let mut env = EnvGuard::new();
        env.set("TM_JWT_SECRET", SECRET);
        env.set("TM_ADMIN_EMAIL", "admin@example.com");
        env.set("TM_ALLOWED_EMAILS", "viewer1@example.com,viewer2@example.com");
        env
    }

    /// `load_jwt_secret(env: EnvFn)`에 넘기는 실제 프로세스 환경.
    fn process_env(key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn now() -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
    }

    fn decode(token: &str) -> Value {
        jsonwebtoken::decode::<Value>(token, &DecodingKey::from_secret(SECRET.as_bytes()), &Validation::new(Algorithm::HS256)).unwrap().claims
    }

    fn encode_hs256(payload: &Value) -> String {
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), payload, &EncodingKey::from_secret(SECRET.as_bytes())).unwrap()
    }

    fn access_token(email: &str, role: &str) -> String {
        backend::auth_service::create_jwt_token(email, role, None, None)
    }

    fn refresh_token(email: &str, role: &str) -> String {
        backend::auth_service::create_refresh_token(email, role, None, None)
    }

    /// Python `FakeRequest`: headers/cookies 딕셔너리만 가진 요청 대역.
    struct FakeRequest {
        headers: HashMap<String, String>,
        cookies: HashMap<String, String>,
    }

    impl RequestParts for FakeRequest {
        fn header(&self, name: &str) -> Option<String> {
            self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
        }

        fn cookie(&self, name: &str) -> Option<String> {
            self.cookies.get(name).cloned()
        }
    }

    fn make_request(token: Option<&str>, cookie_token: Option<&str>) -> FakeRequest {
        let mut headers = HashMap::new();
        let mut cookies = HashMap::new();
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            headers.insert("Authorization".to_string(), format!("Bearer {t}"));
        }
        if let Some(c) = cookie_token.filter(|c| !c.is_empty()) {
            cookies.insert("tm_access_token".to_string(), c.to_string());
        }
        FakeRequest { headers, cookies }
    }

    fn detail_of(err: &HTTPException) -> String {
        err.detail.to_lowercase()
    }

    async fn post_empty(client: &TestClient, path: &str) -> Response {
        client.request(Method::POST, path, None, Vec::new()).await
    }

    /// Python `auth_client` fixture: require_auth/require_admin을 고정 payload로 바꾼 앱 (`AuthMode::Split`).
    fn auth_deps() -> AppDeps {
        AppDeps::new(AuthMode::Split { auth: json!({"email": "user@example.com", "role": "user"}), admin: json!({"email": "admin@example.com", "role": "admin"}) })
    }

    fn auth_app(deps: AppDeps) -> Router {
        app_with(deps)
    }

    fn client(app: &Router) -> TestClient {
        TestClient::with_app(app.clone())
    }

    fn refresh_client(app: &Router, token: &str) -> TestClient {
        client(app).cookie(REFRESH_COOKIE_NAME, token)
    }

    /// `decode_refresh_token`을 가로채는 seam이 없으므로 같은 비밀키로 서명한 실제 refresh 토큰을 쓴다.
    fn crafted_refresh(email: &str, fid: &str, jti: &str) -> String {
        encode_hs256(&json!({"type": "refresh", "email": email, "role": "user", "name": "n", "picture": "p", "fid": fid, "jti": jti, "iat": now(), "exp": now() + 3600}))
    }

    // TODO(common): FixedRole, FakeGoogle, FakeStore는 main_mock, view_history_api도 쓴다. common/auth_fakes.rs로 옮긴다.

    /// `RoleResolver` fake. 어떤 이메일이든 같은 역할을 돌려준다 (구 `.determine_role(|_| ..)` 클로저).
    struct FixedRole(Option<String>);

    impl RoleResolver for FixedRole {
        fn determine_role(&self, _email: &str) -> Option<String> {
            self.0.clone()
        }
    }

    fn role(r: &str) -> Arc<FixedRole> {
        Arc::new(FixedRole(Some(r.to_string())))
    }

    /// `RefreshTokenStoreApi` fake. 호출 인자를 기록하고 정해 둔 결과를 돌려준다.
    /// 안 정한 메서드는 trait의 기본 구현(Unimplemented 오류)을 쓴다.
    struct FakeStore {
        rotate_status: Mutex<Option<String>>,
        observation: Mutex<Option<Result<Option<Value>, String>>>,
        sessions: Mutex<Option<Result<Value, String>>>,
        revoke_result: Mutex<Option<Result<Value, String>>>,
        issued: Mutex<Vec<Value>>,
        rotated: Mutex<Vec<Value>>,
        revoked_families: Mutex<Vec<(String, String)>>,
        list_calls: Mutex<Vec<Value>>,
        revoke_session_calls: Mutex<Vec<Value>>,
    }

    // 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
    #[allow(dead_code)]
    impl FakeStore {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                rotate_status: Mutex::new(None),
                observation: Mutex::new(None),
                sessions: Mutex::new(None),
                revoke_result: Mutex::new(None),
                issued: Mutex::new(Vec::new()),
                rotated: Mutex::new(Vec::new()),
                revoked_families: Mutex::new(Vec::new()),
                list_calls: Mutex::new(Vec::new()),
                revoke_session_calls: Mutex::new(Vec::new()),
            })
        }

        fn rotating(status: &str) -> Arc<Self> {
            let store = Self::new();
            *store.rotate_status.lock().unwrap() = Some(status.to_string());
            store
        }

        fn with_observation(self: Arc<Self>, observation: Result<Option<Value>, String>) -> Arc<Self> {
            *self.observation.lock().unwrap() = Some(observation);
            self
        }

        fn listing(page: Result<Value, String>) -> Arc<Self> {
            let store = Self::new();
            *store.sessions.lock().unwrap() = Some(page);
            store
        }

        fn revoking(result: Result<Value, String>) -> Arc<Self> {
            let store = Self::new();
            *store.revoke_result.lock().unwrap() = Some(result);
            store
        }
    }

    // 실제 `StoreError`에는 `Other`가 없어서 임의 사유 문자열은 `Unsupported(String)`으로 옮긴다.
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

        fn _session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
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

        fn list_sessions(&self, q: &SessionQuery) -> Result<Value, StoreError> {
            // Python 키워드 인자를 객체로 담아 둔다. 키 이름은 Python과 같다.
            self.list_calls.lock().unwrap().push(json!({"status": q.status, "current_family_id": q.current_family_id, "page": q.page, "page_size": q.page_size, "email": q.email}));
            self.sessions.lock().unwrap().clone().unwrap_or_else(|| Err("unset".to_string())).map_err(StoreError::Unsupported)
        }

        fn revoke_session(&self, family_id: &str) -> Result<Value, StoreError> {
            self.revoke_session_calls.lock().unwrap().push(json!({"family_id": family_id}));
            self.revoke_result.lock().unwrap().clone().unwrap_or_else(|| Err("unset".to_string())).map_err(StoreError::Unsupported)
        }
    }

    // ========== determine_role ==========

    mod test_determine_role {
        use super::*;

        #[test]
        fn test_admin_email() {
            let _env = fixture();
            assert_eq!(backend::auth_service::determine_role("admin@example.com").as_deref(), Some("admin"));
        }

        #[test]
        fn test_viewer_email() {
            let _env = fixture();
            assert_eq!(backend::auth_service::determine_role("viewer1@example.com").as_deref(), Some("viewer"));
            assert_eq!(backend::auth_service::determine_role("viewer2@example.com").as_deref(), Some("viewer"));
        }

        #[test]
        fn test_unknown_email() {
            let _env = fixture();
            assert_eq!(backend::auth_service::determine_role("unknown@example.com"), None);
        }

        #[test]
        fn test_empty_email() {
            let _env = fixture();
            assert_eq!(backend::auth_service::determine_role(""), None);
        }

        #[test]
        fn test_reads_current_env_without_reload() {
            let mut env = fixture();
            env.set("TM_ADMIN_EMAIL", "new-admin@example.com");
            env.set("TM_ALLOWED_EMAILS", "new-viewer@example.com");

            assert_eq!(backend::auth_service::determine_role("new-admin@example.com").as_deref(), Some("admin"));
            assert_eq!(backend::auth_service::determine_role("new-viewer@example.com").as_deref(), Some("viewer"));
            assert_eq!(backend::auth_service::determine_role("admin@example.com"), None);
        }
    }

    #[test]
    fn test_auth_requires_jwt_secret() {
        let mut env = fixture();
        env.remove("TM_JWT_SECRET");
        let result = backend::auth_service::load_jwt_secret(&process_env);
        assert!(matches!(result, Err(SystemExit(_))), "{result:?}");
    }

    // ========== 토큰 발급 ==========

    mod test_create_jwt_token {
        use super::*;

        #[test]
        fn test_token_contains_claims() {
            let _env = fixture();
            let token = backend::auth_service::create_jwt_token("admin@example.com", "admin", Some("Admin"), Some("pic.jpg"));
            let payload = decode(&token);
            assert_eq!(payload["email"], "admin@example.com");
            assert_eq!(payload["role"], "admin");
            assert_eq!(payload["name"], "Admin");
            assert_eq!(payload["picture"], "pic.jpg");
            assert_eq!(payload["type"], "access");
            assert!(payload.get("exp").is_some());
            assert!(payload.get("iat").is_some());
        }

        #[test]
        fn test_token_expiration() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let payload = decode(&token);
            let exp = payload["exp"].as_i64().unwrap();
            assert!(exp > now());
            assert!(exp <= now() + 2 * 3600 + 5);
        }
    }

    mod test_create_refresh_token {
        use super::*;

        #[test]
        fn test_refresh_token_contains_claims() {
            let _env = fixture();
            let token = backend::auth_service::create_refresh_token("admin@example.com", "admin", Some("Admin"), Some("pic.jpg"));
            let payload = decode(&token);
            assert_eq!(payload["email"], "admin@example.com");
            assert_eq!(payload["role"], "admin");
            assert_eq!(payload["type"], "refresh");
            assert_eq!(payload["name"], "Admin");
            assert_eq!(payload["picture"], "pic.jpg");
            assert!(payload["fid"].as_str().is_some_and(|s| !s.is_empty()));
            assert!(payload["jti"].as_str().is_some_and(|s| !s.is_empty()));
        }

        #[test]
        fn test_refresh_token_expiration() {
            let _env = fixture();
            let token = refresh_token("admin@example.com", "admin");
            let payload = decode(&token);
            let exp = payload["exp"].as_i64().unwrap();
            assert!(exp > now());
            assert!(exp <= now() + 30 * 24 * 3600 + 5);
        }
    }

    mod test_decode_refresh_token {
        use super::*;

        #[test]
        fn test_valid_refresh_token() {
            let _env = fixture();
            let token = refresh_token("admin@example.com", "admin");
            let payload = backend::auth_service::decode_refresh_token(&token).unwrap();
            assert_eq!(payload["email"], "admin@example.com");
            assert_eq!(payload["type"], "refresh");
        }

        #[test]
        fn test_expired_refresh_token() {
            let _env = fixture();
            let expired_payload = json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "exp": now() - 100, "iat": now() - 200});
            let token = encode_hs256(&expired_payload);
            let err = backend::auth_service::decode_refresh_token(&token).unwrap_err();
            assert_eq!(err.status_code, 401);
            assert!(detail_of(&err).contains("expired"), "{}", err.detail);
        }

        #[test]
        fn test_access_token_rejected() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let err = backend::auth_service::decode_refresh_token(&token).unwrap_err();
            assert_eq!(err.status_code, 401);
            assert!(detail_of(&err).contains("type"), "{}", err.detail);
        }

        #[test]
        fn test_invalid_token() {
            let _env = fixture();
            let err = backend::auth_service::decode_refresh_token("invalid.token.value").unwrap_err();
            assert_eq!(err.status_code, 401);
        }

        #[test]
        fn test_refresh_token_without_rotation_claims_is_rejected() {
            let _env = fixture();
            let payload = json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "exp": now() + 100, "iat": now()});
            let token = encode_hs256(&payload);
            let err = backend::auth_service::decode_refresh_token(&token).unwrap_err();
            assert_eq!(err.status_code, 401);
            assert!(detail_of(&err).contains("claims"), "{}", err.detail);
        }

        #[test]
        fn test_refresh_token_missing_fid_only_is_rejected() {
            // fid만 없는 경우도 claims 에러로 거부된다.
            let _env = fixture();
            let payload = json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "jti": "some-jti", "exp": now() + 100, "iat": now()});
            let token = encode_hs256(&payload);
            let err = backend::auth_service::decode_refresh_token(&token).unwrap_err();
            assert_eq!(err.status_code, 401);
            assert!(detail_of(&err).contains("claims"), "{}", err.detail);
            assert!(detail_of(&err).contains("log in again"), "{}", err.detail);
        }

        #[test]
        fn test_refresh_token_missing_jti_only_is_rejected() {
            // jti만 없는 경우도 claims 에러로 거부된다.
            let _env = fixture();
            let payload = json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "fid": "some-fid", "exp": now() + 100, "iat": now()});
            let token = encode_hs256(&payload);
            let err = backend::auth_service::decode_refresh_token(&token).unwrap_err();
            assert_eq!(err.status_code, 401);
            assert!(detail_of(&err).contains("claims"), "{}", err.detail);
        }
    }

    // ========== 요청에서 payload 추출 ==========

    mod test_extract_payload {
        use super::*;

        #[test]
        fn test_missing_header() {
            let _env = fixture();
            let req = make_request(None, None);
            let err = backend::auth_service::_extract_payload(&req).unwrap_err();
            assert_eq!(err.status_code, 401);
        }

        #[test]
        fn test_valid_token() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let req = make_request(Some(&token), None);
            let payload = backend::auth_service::_extract_payload(&req).unwrap();
            assert_eq!(payload["email"], "admin@example.com");
            assert_eq!(payload["role"], "admin");
        }

        #[test]
        fn test_valid_cookie_token() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let req = make_request(None, Some(&token));
            let payload = backend::auth_service::_extract_payload(&req).unwrap();
            assert_eq!(payload["email"], "admin@example.com");
            assert_eq!(payload["role"], "admin");
        }

        #[test]
        fn test_expired_token() {
            let _env = fixture();
            let expired_payload = json!({"type": "access", "email": "admin@example.com", "role": "admin", "exp": now() - 100, "iat": now() - 200});
            let token = encode_hs256(&expired_payload);
            let req = make_request(Some(&token), None);
            let err = backend::auth_service::_extract_payload(&req).unwrap_err();
            assert_eq!(err.status_code, 401);
            assert!(detail_of(&err).contains("expired"), "{}", err.detail);
        }

        #[test]
        fn test_invalid_token() {
            let _env = fixture();
            let req = make_request(Some("invalid.token.value"), None);
            let err = backend::auth_service::_extract_payload(&req).unwrap_err();
            assert_eq!(err.status_code, 401);
        }

        #[test]
        fn test_refresh_token_rejected() {
            let _env = fixture();
            let token = refresh_token("admin@example.com", "admin");
            let req = make_request(Some(&token), None);
            let err = backend::auth_service::_extract_payload(&req).unwrap_err();
            assert_eq!(err.status_code, 401);
            assert!(detail_of(&err).contains("type"), "{}", err.detail);
        }

        #[test]
        fn test_bearer_header_takes_priority_over_cookie() {
            let _env = fixture();
            let header_token = access_token("admin@example.com", "admin");
            let cookie_token = access_token("viewer1@example.com", "viewer");
            let req = make_request(Some(&header_token), Some(&cookie_token));
            let payload = backend::auth_service::_extract_payload(&req).unwrap();
            assert_eq!(payload["email"], "admin@example.com");
        }

        #[test]
        fn test_empty_bearer_falls_back_to_cookie() {
            let _env = fixture();
            let cookie_token = access_token("viewer1@example.com", "viewer");
            let req = FakeRequest { headers: HashMap::from([("Authorization".to_string(), "Bearer ".to_string())]), cookies: HashMap::from([("tm_access_token".to_string(), cookie_token)]) };

            let payload = backend::auth_service::_extract_payload(&req).unwrap();
            assert_eq!(payload["email"], "viewer1@example.com");
        }
    }

    mod test_require_auth {
        use super::*;

        #[tokio::test]
        async fn test_admin_passes() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let req = make_request(Some(&token), None);
            let payload = backend::auth_service::require_auth(&req).await.unwrap();
            assert_eq!(payload["role"], "admin");
        }

        #[tokio::test]
        async fn test_viewer_passes() {
            let _env = fixture();
            let token = access_token("viewer1@example.com", "viewer");
            let req = make_request(Some(&token), None);
            let payload = backend::auth_service::require_auth(&req).await.unwrap();
            assert_eq!(payload["role"], "viewer");
        }

        #[tokio::test]
        async fn test_cookie_token_passes() {
            let _env = fixture();
            let token = access_token("viewer1@example.com", "viewer");
            let req = make_request(None, Some(&token));
            let payload = backend::auth_service::require_auth(&req).await.unwrap();
            assert_eq!(payload["role"], "viewer");
        }

        #[tokio::test]
        async fn test_invalid_role_fails() {
            let _env = fixture();
            let token = access_token("nobody@example.com", "unknown");
            let req = make_request(Some(&token), None);
            let err = backend::auth_service::require_auth(&req).await.unwrap_err();
            assert_eq!(err.status_code, 403);
        }

        #[tokio::test]
        async fn test_revoked_viewer_fails() {
            let _env = fixture();
            let token = access_token("viewer1@example.com", "viewer");
            let req = make_request(Some(&token), None);
            let err = backend::auth_service::require_auth_with(&req, &FixedRole(None)).await.unwrap_err();
            assert_eq!(err.status_code, 403);
            assert_eq!(err.detail, "Access denied");
        }

        #[tokio::test]
        async fn test_role_mismatch_fails() {
            let _env = fixture();
            let token = access_token("viewer1@example.com", "viewer");
            let req = make_request(Some(&token), None);
            let err = backend::auth_service::require_auth_with(&req, &FixedRole(Some("admin".to_string()))).await.unwrap_err();
            assert_eq!(err.status_code, 403);
            assert_eq!(err.detail, "Access denied");
        }

        #[tokio::test]
        async fn test_no_token_fails() {
            let _env = fixture();
            let req = make_request(None, None);
            let err = backend::auth_service::require_auth(&req).await.unwrap_err();
            assert_eq!(err.status_code, 401);
        }
    }

    mod test_require_admin {
        use super::*;

        #[tokio::test]
        async fn test_admin_passes() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let req = make_request(Some(&token), None);
            let payload = backend::auth_service::require_admin(&req).await.unwrap();
            assert_eq!(payload["role"], "admin");
        }

        #[tokio::test]
        async fn test_admin_cookie_passes() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let req = make_request(None, Some(&token));
            let payload = backend::auth_service::require_admin(&req).await.unwrap();
            assert_eq!(payload["role"], "admin");
        }

        #[tokio::test]
        async fn test_viewer_blocked() {
            let _env = fixture();
            let token = access_token("viewer1@example.com", "viewer");
            let req = make_request(Some(&token), None);
            let err = backend::auth_service::require_admin(&req).await.unwrap_err();
            assert_eq!(err.status_code, 403);
            assert!(detail_of(&err).contains("admin"), "{}", err.detail);
        }

        #[tokio::test]
        async fn test_admin_revoked_fails() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let req = make_request(Some(&token), None);
            let err = backend::auth_service::require_admin_with(&req, &FixedRole(None)).await.unwrap_err();
            assert_eq!(err.status_code, 403);
            assert_eq!(err.detail, "Access denied");
        }

        #[tokio::test]
        async fn test_admin_demoted_to_viewer_fails() {
            let _env = fixture();
            let token = access_token("admin@example.com", "admin");
            let req = make_request(Some(&token), None);
            let err = backend::auth_service::require_admin_with(&req, &FixedRole(Some("viewer".to_string()))).await.unwrap_err();
            assert_eq!(err.status_code, 403);
            assert_eq!(err.detail, "Access denied");
        }

        #[tokio::test]
        async fn test_no_token_fails() {
            let _env = fixture();
            let req = make_request(None, None);
            let err = backend::auth_service::require_admin(&req).await.unwrap_err();
            assert_eq!(err.status_code, 401);
        }
    }

    // ========== 쿠키 설정 ==========

    /// `_get_cookie_settings`의 SameSite/Secure 검증 테스트.
    /// Python은 autouse fixture로 `BookManager` 등을 patch하고 `main` 모듈을 reload한다.
    /// Rust는 환경 변수를 호출 시점에 읽으므로 reload도 patch도 필요 없다.
    mod test_get_cookie_settings {
        use super::*;

        #[test]
        fn test_default_values() {
            let mut env = fixture();
            env.set("TM_FRONTEND_URL", "http://localhost:3000");
            env.remove("TM_COOKIE_SECURE");
            env.remove("TM_COOKIE_SAMESITE");
            let (secure, samesite) = backend::main::_get_cookie_settings();
            assert!(!secure);
            assert_eq!(samesite, "lax");
        }

        #[test]
        fn test_secure_true() {
            let mut env = fixture();
            env.set("TM_FRONTEND_URL", "https://app.example.com");
            env.set("TM_COOKIE_SECURE", "true");
            env.remove("TM_COOKIE_SAMESITE");
            let (secure, samesite) = backend::main::_get_cookie_settings();
            assert!(secure);
            assert_eq!(samesite, "lax");
        }

        #[test]
        fn test_samesite_strict() {
            let mut env = fixture();
            env.set("TM_FRONTEND_URL", "http://localhost:3000");
            env.remove("TM_COOKIE_SECURE");
            env.set("TM_COOKIE_SAMESITE", "strict");
            let (_secure, samesite) = backend::main::_get_cookie_settings();
            assert_eq!(samesite, "strict");
        }

        #[test]
        fn test_invalid_samesite_falls_back_to_lax() {
            let mut env = fixture();
            env.set("TM_FRONTEND_URL", "http://localhost:3000");
            env.remove("TM_COOKIE_SECURE");
            env.set("TM_COOKIE_SAMESITE", "invalid_value");
            let (_secure, samesite) = backend::main::_get_cookie_settings();
            assert_eq!(samesite, "lax");
        }

        #[test]
        fn test_samesite_none_with_secure_true() {
            let mut env = fixture();
            env.set("TM_FRONTEND_URL", "http://localhost:3000");
            env.set("TM_COOKIE_SECURE", "true");
            env.set("TM_COOKIE_SAMESITE", "none");
            let (secure, samesite) = backend::main::_get_cookie_settings();
            assert!(secure);
            assert_eq!(samesite, "none");
        }

        #[test]
        fn test_samesite_none_without_secure_falls_back_to_lax() {
            let mut env = fixture();
            env.set("TM_FRONTEND_URL", "http://localhost:3000");
            env.set("TM_COOKIE_SECURE", "false");
            env.set("TM_COOKIE_SAMESITE", "none");
            let (secure, samesite) = backend::main::_get_cookie_settings();
            assert!(!secure);
            assert_eq!(samesite, "lax");
        }

        #[test]
        fn test_non_local_origin_forces_secure_even_when_disabled() {
            let mut env = fixture();
            env.set("TM_FRONTEND_URL", "https://app.example.com");
            env.set("TM_COOKIE_SECURE", "false");
            env.remove("TM_COOKIE_SAMESITE");
            let (secure, samesite) = backend::main::_get_cookie_settings();
            assert!(secure);
            assert_eq!(samesite, "lax");
        }
    }

    #[tokio::test]
    async fn test_handlers_direct() {
        let _env = fixture();
        let logs = LogCapture::start(log::Level::Warn);
        let (parts, _) = axum::http::Request::builder().method("GET").uri("/x").body(()).unwrap().into_parts();

        let secret = "super-secret-token";
        let exc = RequestValidationError::new(vec![json!({"loc": ["body"], "msg": "bad", "type": "value_error"})], json!({"credential": secret}));
        let resp = backend::main::validation_exception_handler(&parts, &exc).await;
        assert_eq!(resp.status().as_u16(), 422);
        assert!(!logs.text().contains(secret));

        let http_exc = HTTPException::new(404, "no");
        let resp = backend::main::http_exception_handler(&parts, &http_exc).await;
        assert_eq!(resp.status().as_u16(), 404);

        let boom = std::io::Error::other("boom");
        let resp = backend::main::general_exception_handler(&parts, &boom).await;
        assert_eq!(resp.status().as_u16(), 500);
    }

    #[test]
    fn test_cookie_settings_defaults() {
        let mut env = fixture();
        env.set("TM_FRONTEND_URL", "http://localhost:3000");
        env.remove("TM_COOKIE_SECURE");
        env.remove("TM_COOKIE_SAMESITE");
        let (secure, samesite) = backend::main::_get_cookie_settings();
        assert!(!secure);
        assert_eq!(samesite, "lax");
    }

    #[test]
    fn test_cookie_settings_none_requires_secure() {
        let mut env = fixture();
        env.set("TM_FRONTEND_URL", "http://localhost:3000");
        env.set("TM_COOKIE_SECURE", "false");
        env.set("TM_COOKIE_SAMESITE", "none");
        let (secure, samesite) = backend::main::_get_cookie_settings();
        assert!(!secure);
        assert_eq!(samesite, "lax");
    }

    #[test]
    fn test_set_and_clear_auth_cookies() {
        let mut env = fixture();
        env.set("TM_COOKIE_SECURE", "true");
        env.set("TM_COOKIE_SAMESITE", "strict");
        let mut response = CustomJSONResponse::new(json!({"ok": true}));
        backend::main::_set_auth_cookies(&mut response, "access-token", "refresh-token");
        let cookies: Vec<String> = response.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
        assert!(cookies.iter().any(|cookie| cookie.contains(ACCESS_COOKIE_NAME)), "{cookies:?}");
        assert!(cookies.iter().any(|cookie| cookie.contains(REFRESH_COOKIE_NAME)), "{cookies:?}");

        let mut response2 = CustomJSONResponse::new(json!({"ok": true}));
        backend::main::_clear_auth_cookies(&mut response2);
        let cookies2: Vec<String> = response2.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
        assert!(cookies2.iter().any(|cookie| cookie.contains(&format!("{ACCESS_COOKIE_NAME}="))), "{cookies2:?}");
        assert!(cookies2.iter().any(|cookie| cookie.contains(&format!("{REFRESH_COOKIE_NAME}="))), "{cookies2:?}");
    }

    #[test]
    fn test_custom_jsonable_encoder_preserves_strings() {
        let _env = fixture();
        let payload = json!({"k": "한글", "list": ["테스트", {"x": "값"}]});
        let encoded = backend::main::custom_jsonable_encoder(&payload);
        assert_eq!(encoded["k"], "한글");
        assert_eq!(encoded["list"][0], "테스트");
    }

    // ---- merged from test_auth_env_guard.py ----

    #[test]
    fn test_auth_requires_jwt_secret_os_environ() {
        // Python은 os.environ에서 직접 지우고 모듈을 다시 import한다. Rust는 환경 변수를 지우고 설정 로더를 호출한다.
        let mut env = fixture();
        env.remove("TM_JWT_SECRET");
        let result = backend::auth_service::load_jwt_secret(&process_env);
        assert!(matches!(result, Err(SystemExit(_))), "{result:?}");
    }

    /// determine_role 이 "admin"/"viewer" 이외 역할을 반환할 때 require_auth 가 403 을 올린다 (line 101).
    #[tokio::test]
    async fn test_require_auth_non_standard_role_raises_403() {
        let _env = fixture();
        let token = access_token("admin@example.com", "superuser");
        let req = FakeRequest { headers: HashMap::from([("Authorization".to_string(), format!("Bearer {token}"))]), cookies: HashMap::new() };

        let err = backend::auth_service::require_auth_with(&req, &FixedRole(Some("superuser".to_string()))).await.unwrap_err();
        assert_eq!(err.status_code, 403);
        assert!(detail_of(&err).contains("permissions"), "{}", err.detail);
    }

    // ========== Phase 0: refresh 회전 실패 관측 로깅 ==========

    /// refresh 회전 거부 사건을 마스킹된 필드로만 남기는지 검증한다.
    ///
    /// 관측 목적은 `reuse-detected` 가 같은 브라우저의 멀티탭 동시성에서 오는지, 다른
    /// 환경에서 복사된 상태에서 오는지 구분하는 것이다.
    mod test_refresh_rotation_observation {
        use super::*;

        const RAW_EMAIL: &str = "observed-user@example.com";
        const RAW_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) ObservedBrowser/123.45";
        const RAW_IP: &str = "203.0.113.77";

        /// 거부된 refresh 응답, 로그 텍스트, 쿠키로 보낸 원문 토큰을 돌려준다.
        async fn reject_refresh(status: &str) -> (Response, String, String) {
            let store = FakeStore::rotating(status).with_observation(Ok(Some(json!({"replaced_by_present": true, "revoked_at": 1, "revoke_reason": "rotated"}))));
            let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(store);
            let app = auth_app(deps);
            let raw_cookie = crafted_refresh(RAW_EMAIL, "observed-family", "observed-jti");
            let logs = LogCapture::start(log::Level::Warn);
            let resp = post_empty(&refresh_client(&app, &raw_cookie).header("User-Agent", RAW_UA).header("X-Forwarded-For", &format!("{RAW_IP}, 10.0.0.1")), "/auth/refresh").await;
            (resp, logs.text(), raw_cookie)
        }

        #[tokio::test]
        async fn test_emits_structured_masked_event() {
            let _env = fixture();
            let (resp, text, _raw_cookie) = reject_refresh("reused").await;

            assert_eq!(resp.status, 401);
            assert!(text.contains("event=refresh-rotation-rejected"), "{text}");
            assert!(text.contains("status=reused"), "{text}");
            assert!(text.contains("replaced_by_present=true"), "{text}");
            // 모든 식별자는 HMAC 해시로만 남는다
            for (field, raw) in [("email_hash", RAW_EMAIL), ("family_hash", "observed-family"), ("jti_hash", "observed-jti"), ("request_user_agent_hash", RAW_UA), ("request_ip_prefix_hash", "203.0.113")]
            {
                let expected = format!("{field}={}", observation_hash(raw));
                assert!(text.contains(&expected), "{expected} not in {text}");
            }
        }

        #[tokio::test]
        async fn test_never_logs_raw_identifiers() {
            // 원문 토큰·쿠키·전체 이메일·전체 IP·raw user-agent 는 로그에 남지 않는다.
            let _env = fixture();
            let (_, text, raw_cookie) = reject_refresh("reused").await;

            assert!(!text.contains(RAW_EMAIL));
            assert!(!text.contains(RAW_UA));
            assert!(!text.contains(RAW_IP)); // /24 프리픽스도 해시로만 기록
            assert!(!text.contains(&raw_cookie));
            assert!(!text.contains("observed-family"));
            assert!(!text.contains("observed-jti"));
        }

        #[tokio::test]
        async fn test_observation_lookup_failure_does_not_break_response() {
            // 관측 조회가 실패해도 refresh 응답 경로는 그대로 401 을 반환한다.
            let _env = fixture();
            let store = FakeStore::rotating("reused").with_observation(Err("store down".to_string()));
            let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(store);
            let app = auth_app(deps);

            let logs = LogCapture::start(log::Level::Warn);
            let resp = post_empty(&refresh_client(&app, &crafted_refresh(RAW_EMAIL, "F", "J")), "/auth/refresh").await;

            assert_eq!(resp.status, 401);
            assert!(logs.text().contains("event=refresh-rotation-rejected"), "{}", logs.text());
            assert!(logs.text().contains("replaced_by_present=false"), "{}", logs.text());
        }

        #[tokio::test]
        async fn test_successful_refresh_emits_no_observation_event() {
            let _env = fixture();
            let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(FakeStore::rotating("ok"));
            let app = auth_app(deps);

            let logs = LogCapture::start(log::Level::Warn);
            let resp = post_empty(&refresh_client(&app, &crafted_refresh(RAW_EMAIL, "F", "J")), "/auth/refresh").await;

            assert_eq!(resp.status, 200);
            assert!(!logs.text().contains("event=refresh-rotation-rejected"), "{}", logs.text());
        }
    }

    // ========== 접속 IP ==========

    const DEFAULT_CLIENT_HOST: &str = "192.0.2.9";

    fn ip_headers(headers: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            map.insert(HeaderName::from_bytes(k.to_lowercase().as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        map
    }

    /// 실제 접속 IP 는 경유 프록시(Cloudflare edge, Traefik) IP 가 아니어야 한다.
    mod test_client_ip {
        use super::*;

        #[test]
        fn test_prefers_cf_connecting_ip_over_proxy_hops() {
            // 운영 경로에서 XFF 첫 항목은 Cloudflare edge IP 라 신뢰할 수 없다.
            let headers = ip_headers(&[("CF-Connecting-IP", "203.0.113.77"), ("X-Forwarded-For", "104.22.17.33")]);
            assert_eq!(backend::main::_client_ip(&headers, Some("10.1.202.80")), "203.0.113.77");
        }

        #[test]
        fn test_falls_back_to_true_client_ip() {
            let headers = ip_headers(&[("True-Client-IP", "203.0.113.78"), ("X-Forwarded-For", "104.22.17.33")]);
            assert_eq!(backend::main::_client_ip(&headers, Some(DEFAULT_CLIENT_HOST)), "203.0.113.78");
        }

        #[test]
        fn test_falls_back_to_forwarded_for_without_cloudflare() {
            // Cloudflare 를 거치지 않는 경로에서는 XFF 첫 항목을 쓴다.
            let headers = ip_headers(&[("X-Forwarded-For", "198.51.100.23, 10.1.2.3")]);
            assert_eq!(backend::main::_client_ip(&headers, Some(DEFAULT_CLIENT_HOST)), "198.51.100.23");
        }

        #[test]
        fn test_falls_back_to_peer_when_no_headers() {
            assert_eq!(backend::main::_client_ip(&ip_headers(&[]), Some(DEFAULT_CLIENT_HOST)), "192.0.2.9");
        }

        #[test]
        fn test_ignores_blank_headers() {
            let headers = ip_headers(&[("CF-Connecting-IP", "  "), ("X-Forwarded-For", " , 10.1.2.3")]);
            assert_eq!(backend::main::_client_ip(&headers, Some(DEFAULT_CLIENT_HOST)), "192.0.2.9");
        }

        #[test]
        fn test_returns_empty_when_no_client_info() {
            assert_eq!(backend::main::_client_ip(&ip_headers(&[]), None), "");
        }

        #[test]
        fn test_keeps_full_ipv6() {
            let headers = ip_headers(&[("CF-Connecting-IP", "2001:db8:abcd:1234::1")]);
            assert_eq!(backend::main::_client_ip(&headers, Some(DEFAULT_CLIENT_HOST)), "2001:db8:abcd:1234::1");
        }
    }

    /// 관측용 IP 프리픽스는 IPv4 /24, IPv6 /48 까지만 남긴다.
    mod test_request_ip_prefix {
        use super::*;

        #[test]
        fn test_masks_cf_connecting_ip() {
            // 프리픽스도 프록시 IP 가 아니라 실제 클라이언트 IP 를 기준으로 만든다.
            let headers = ip_headers(&[("CF-Connecting-IP", "203.0.113.77"), ("X-Forwarded-For", "104.22.17.33")]);
            assert_eq!(backend::main::_request_ip_prefix(&headers, Some(DEFAULT_CLIENT_HOST)), "203.0.113");
        }

        #[test]
        fn test_uses_forwarded_for_first_hop_ipv4() {
            let headers = ip_headers(&[("X-Forwarded-For", "198.51.100.23, 10.1.2.3")]);
            assert_eq!(backend::main::_request_ip_prefix(&headers, Some(DEFAULT_CLIENT_HOST)), "198.51.100");
        }

        #[test]
        fn test_truncates_ipv6_to_48_bits() {
            let headers = ip_headers(&[("X-Forwarded-For", "2001:db8:abcd:1234::1")]);
            assert_eq!(backend::main::_request_ip_prefix(&headers, Some(DEFAULT_CLIENT_HOST)), "2001:db8:abcd");
        }

        #[test]
        fn test_falls_back_to_client_host() {
            assert_eq!(backend::main::_request_ip_prefix(&ip_headers(&[]), Some(DEFAULT_CLIENT_HOST)), "192.0.2");
        }

        #[test]
        fn test_returns_empty_when_no_client_info() {
            assert_eq!(backend::main::_request_ip_prefix(&ip_headers(&[]), None), "");
        }
    }
}

// `tests/rust/backend/auth_bookstore_gaps.rs`에서 옮긴 테스트.
mod auth_bookstore_gaps {
    use super::{common};

    use backend::auth_service;
    use bookstore::store_clients::{AbstractBookstore, KyoboBookstore};

    #[test]
    fn test_observation_hash_of_empty_value_is_empty_string() {
        common::setup_env();
        assert_eq!(auth_service::observation_hash(""), "");
    }

    #[test]
    fn test_kyobo_fallback_links_stop_at_max_results() {
        common::setup_env();
        let store = KyoboBookstore::new(false);
        let anchors: String = (0..store.max_results + 5).map(|n| format!(r#"<a href="/detail/{n}">책 {n}</a>"#)).collect();
        let soup = format!("<html><body>{anchors}</body></html>");

        let links = store.extract_search_links(&soup);

        let expected: Vec<String> = (0..store.max_results).map(|n| format!("/detail/{n}")).collect();
        assert_eq!(links, expected);
    }
}

// `tests/rust/backend/book_admin_api.rs`에서 옮긴 테스트.
mod book_admin_api {
    use super::{common};

    use backend::main::{AppDeps, AuthMode, app_with};
    use common::TestClient;
    use serde_json::{Value, json};

    fn doc() -> Value {
        json!({"category":"A","title":"book","author":"author","file_path":"A/book.txt","file_type":"txt","file_size":4,"updated_time":"2026-10-25"})
    }

    #[tokio::test]
    async fn admin_operations_reject_real_viewer_token_before_touching_managers() {
        common::setup_env();
        // 같은 바이너리의 auth 테스트가 TM_JWT_SECRET 등을 잠깐 바꾼다. 바꾸는 동안 토큰을 만들지 않게 읽기 잠금을 쥔다.
        let _env = common::env::EnvGuard::read();
        let token = backend::auth_service::create_jwt_token("viewer@test.com", "viewer", None, None);
        let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies))).bearer(&token);
        for prefix in ["", "/comics"] {
            for (method, path, body) in [
                (axum::http::Method::PUT, "/books/1", doc()),
                (axum::http::Method::DELETE, "/books/1", Value::Null),
                (axum::http::Method::PUT, "/categories/rename", json!({"old_category":"A","new_category":"B"})),
                (axum::http::Method::POST, "/categories/delete", json!({"category":"A"})),
                (axum::http::Method::GET, "/category-delete-preview?category=A", Value::Null),
                (axum::http::Method::GET, "/category-pdf-stats?category=A", Value::Null),
            ] {
                assert_eq!(client.request(method, &format!("{prefix}{path}"), Some("application/json"), serde_json::to_vec(&body).unwrap()).await.status, 403, "{prefix}{path}");
            }
        }
    }
}
