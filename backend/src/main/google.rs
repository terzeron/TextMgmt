use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde_json::Value;

use super::traits::GoogleVerifier;

const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
pub(super) const GOOGLE_ISSUERS: [&str; 2] = ["accounts.google.com", "https://accounts.google.com"];
const UNKNOWN_KID_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Google 공개 서명 키를 가져온다. 테스트는 고정 JWKS를 주입한다.
pub trait GoogleJwksFetcher: Send + Sync {
    fn fetch(&self) -> Result<(Value, Duration), String>;
}

struct HttpGoogleJwksFetcher {
    agent: ureq::Agent,
}

impl Default for HttpGoogleJwksFetcher {
    fn default() -> Self {
        Self { agent: ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into() }
    }
}

impl GoogleJwksFetcher for HttpGoogleJwksFetcher {
    fn fetch(&self) -> Result<(Value, Duration), String> {
        let mut response = self.agent.get(GOOGLE_JWKS_URL).call().map_err(|e| e.to_string())?;
        let max_age = response.headers().get("cache-control").and_then(|value| value.to_str().ok()).and_then(parse_max_age).unwrap_or(Duration::from_secs(300));
        let bytes = response.body_mut().read_to_vec().map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("Google JWKS response exceeded 1 MiB".to_string());
        }
        let jwks = serde_json::from_slice(&bytes).map_err(|e| format!("invalid Google JWKS: {e}"))?;
        Ok((jwks, max_age))
    }
}

fn parse_max_age(cache_control: &str) -> Option<Duration> {
    let directives: Vec<&str> = cache_control.split(',').map(str::trim).collect();
    if directives.iter().any(|directive| directive.eq_ignore_ascii_case("no-cache") || directive.eq_ignore_ascii_case("no-store")) {
        return Some(Duration::ZERO);
    }
    directives.iter().find_map(|directive| {
        let (name, age) = directive.split_once('=')?;
        name.trim().eq_ignore_ascii_case("max-age").then(|| age.trim().trim_matches('"').parse::<u64>().ok()).flatten().map(Duration::from_secs)
    })
}

struct CachedJwks {
    value: Value,
    expires_at: Instant,
    last_unknown_kid_refresh: Option<Instant>,
}

/// Google OAuth2 ID token verifier. Google의 cache-control 만료까지 JWKS를 보관한다.
pub struct GoogleIdTokenVerifier {
    fetcher: Arc<dyn GoogleJwksFetcher>,
    cache: Mutex<Option<CachedJwks>>,
}

impl GoogleIdTokenVerifier {
    pub fn with_fetcher(fetcher: Arc<dyn GoogleJwksFetcher>) -> Self {
        Self { fetcher, cache: Mutex::new(None) }
    }

    fn jwks(&self, unknown_kid: bool) -> Result<Value, String> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if let Some(current) = cache.as_ref() {
            let fresh = now < current.expires_at;
            let forced_recently = current.last_unknown_kid_refresh.is_some_and(|last| now.duration_since(last) < UNKNOWN_KID_REFRESH_INTERVAL);
            if fresh && (!unknown_kid || forced_recently) {
                return Ok(current.value.clone());
            }
        }

        let (value, max_age) = self.fetcher.fetch()?;
        let last_unknown_kid_refresh = if unknown_kid { Some(now) } else { cache.as_ref().and_then(|c| c.last_unknown_kid_refresh) };
        let expires_at = now.checked_add(max_age).unwrap_or_else(|| now + Duration::from_secs(24 * 60 * 60));
        *cache = Some(CachedJwks { value: value.clone(), expires_at, last_unknown_kid_refresh });
        Ok(value)
    }

    fn find_key<'a>(jwks: &'a Value, kid: &str) -> Option<&'a Value> {
        jwks.get("keys").and_then(Value::as_array).and_then(|keys| keys.iter().find(|key| key.get("kid").and_then(Value::as_str) == Some(kid)))
    }

    fn verify_with_key(credential: &str, audience: &str, key: &Value) -> Result<Value, String> {
        if key.get("kty").and_then(Value::as_str) != Some("RSA")
            || key.get("use").and_then(Value::as_str).is_some_and(|usage| usage != "sig")
            || key.get("alg").and_then(Value::as_str).is_some_and(|alg| alg != "RS256")
        {
            return Err("Google signing key has an unsupported type or algorithm".to_string());
        }
        let modulus = key.get("n").and_then(Value::as_str).ok_or_else(|| "Google signing key has no modulus".to_string())?;
        let exponent = key.get("e").and_then(Value::as_str).ok_or_else(|| "Google signing key has no exponent".to_string())?;
        let decoding_key = DecodingKey::from_rsa_components(modulus, exponent).map_err(|e| e.to_string())?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 0;
        validation.set_audience(&[audience]);
        validation.set_issuer(&GOOGLE_ISSUERS);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        decode::<Value>(credential, &decoding_key, &validation).map(|token| token.claims).map_err(|e| e.to_string())
    }
}

impl Default for GoogleIdTokenVerifier {
    fn default() -> Self {
        Self::with_fetcher(Arc::new(HttpGoogleJwksFetcher::default()))
    }
}

impl GoogleVerifier for GoogleIdTokenVerifier {
    fn verify(&self, credential: &str, audience: &str) -> Result<Value, String> {
        if credential.len() > 64 * 1024 {
            return Err("Google ID token exceeds 64 KiB".to_string());
        }
        let mut jwks = self.jwks(false)?;
        let header = decode_header(credential).map_err(|e| e.to_string())?;
        if header.alg != Algorithm::RS256 {
            return Err("Google ID token must use RS256".to_string());
        }
        let kid = header.kid.ok_or_else(|| "Google ID token has no key id".to_string())?;
        // 캐시에 없는 kid는 Google 키 교체일 수 있으므로 한 번 다시 받는다.
        if Self::find_key(&jwks, &kid).is_none() {
            jwks = self.jwks(true)?;
        }
        let key = Self::find_key(&jwks, &kid).ok_or_else(|| format!("Google signing key not found: {kid}"))?;
        Self::verify_with_key(credential, audience, key)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::parse_max_age;

    #[test]
    fn cache_control_max_age_is_case_insensitive_and_honors_no_cache() {
        assert_eq!(parse_max_age("public, Max-Age=3600"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_max_age("max-age=\"120\""), Some(Duration::from_secs(120)));
        assert_eq!(parse_max_age("public, NO-STORE"), Some(Duration::ZERO));
        assert_eq!(parse_max_age("no-cache, max-age=300"), Some(Duration::ZERO));
        assert_eq!(parse_max_age("public"), None);
    }
}
