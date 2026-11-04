//! `backend::main::request_extractors` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::sync::{Arc, Mutex};
use db::Row;
use backend::refresh_token_store::{RefreshTokenStoreApi, Rotation, SessionQuery, StoreError};
use serde_json::{json, Value};
use crate::common::shared_fixtures::{ip_headers};

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

// ========== 접속 IP ==========

const DEFAULT_CLIENT_HOST: &str = "192.0.2.9";

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

mod request_ip_prefix {
    use crate::common::shared_fixtures::{ip_headers};

    use std::sync::{Arc, Mutex};


    use db::Row;

    use backend::refresh_token_store::{RefreshTokenStoreApi, Rotation, SessionQuery, StoreError};

    use serde_json::{json, Value};

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

    // ========== 접속 IP ==========

    const DEFAULT_CLIENT_HOST: &str = "192.0.2.9";

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
