use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use backend::main::{GoogleIdTokenVerifier, GoogleJwksFetcher, GoogleVerifier};
use serde_json::{Value, json};

const VALID: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5IiwidHlwIjoiSldUIn0.eyJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJhdWQiOiJ0ZXh0bWdtdC1jbGllbnQiLCJleHAiOjQxMDI0NDQ4MDAsImVtYWlsIjoicmVhZGVyQGV4YW1wbGUuY29tIiwiZW1haWxfdmVyaWZpZWQiOnRydWV9.w5063aZeVEDYkVBBzW3w1Yr28Xa27Fu2WM6A_unCvL4HgvRNIl6q2AQI7pvFbpwFtcKJQ5KkQ5gm4cMc39EhI76ogfnUAmX3ZNiU97Qg9SYxhiqo_2paeXhtbHaPyx6n80BRih9KUpvDJq-LgSD55GExKrFodFOs6W_jEJR3T7OFNdnBNmGrYxL8qHZJh494DMLcCppVhFdVp8ITW94wIc73PU3C4LBJjR4sDpFENBXzZIl5w4E9A-k9oZRpPwWoAlpNCfKLJO6qRyAvewoHgpEkR7Iek8IBRSTG2f2aBVy4m2SrsZ9a58kfAa2ZXhOT074_kYYTnsPNh0jG5Af9ug";
const WRONG_AUDIENCE: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5IiwidHlwIjoiSldUIn0.eyJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJhdWQiOiJvdGhlci1jbGllbnQiLCJleHAiOjQxMDI0NDQ4MDAsImVtYWlsIjoicmVhZGVyQGV4YW1wbGUuY29tIiwiZW1haWxfdmVyaWZpZWQiOnRydWV9.KtZ8cp_sUdoH5hfz-il7LiMXDNLllvBA8WZlmwoSf2HIb9jouotVreyyVzc1FXnjB6a0MM-YRz9uSi7zneKHbSwcfOOmaRLK0P49GQmeQ0txSqXC1uqCiZ7ztzS3IBCzmkMWUU11co75vsmU8R_uTNg1hxRcrMUVg1nP9tf9KMpyIAAxNY1hwm8XQ0HIYEVakQ-DxEudaJVp5hYxhVRQ39fMVwRlRY-1wYUw6LYKTMplSuXM_ipTvZkyTRz3m4P9YoRDqVEyPg9Nx7CzN1hbpk1iCC_vjaymwuVI9yWmwL6FOLyL-O20cNqOMEKJGvu4fphG7jVIJxCWALcoKkrCGA";
const WRONG_ISSUER: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5IiwidHlwIjoiSldUIn0.eyJpc3MiOiJhdHRhY2tlciIsImF1ZCI6InRleHRtZ210LWNsaWVudCIsImV4cCI6NDEwMjQ0NDgwMCwiZW1haWwiOiJyZWFkZXJAZXhhbXBsZS5jb20iLCJlbWFpbF92ZXJpZmllZCI6dHJ1ZX0.vY-tTshNgMfe_OeH-vU7wRKYCWItnQQqRXgWvLd4MigpwwcLP7UhdBnQ8EQ02Tr6OvM9CdI3ZwD1d1dF2fmQ5f_RtJ2EAHKuUxjEeP3U0aKk9KnaoXyhrwxchTbPL0NneawpV_IYJ5rpoCJvSBky0ba1A1G-LSBTOykrCuDoE209C1T5cp0aQFxkbTSqAPeb868jjDB81CPl58BZZlB8V7LaYSZg1PrRGWdRjY6-jUERKLPTCVe7WjDVWDUdHAdjLUuO33iL7BvHzwmSKKANzfntDY_kz24U09-M9e9Ws9kTu2d2tl6sn4yFJLG7yU9IgMyFTifZOkKOOs9hbB61rg";
const EXPIRED: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5IiwidHlwIjoiSldUIn0.eyJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJhdWQiOiJ0ZXh0bWdtdC1jbGllbnQiLCJleHAiOjEsImVtYWlsIjoicmVhZGVyQGV4YW1wbGUuY29tIiwiZW1haWxfdmVyaWZpZWQiOnRydWV9.D92jTaYhWP4pYMei-hAm1jsoh1OsAbKc4T0twxNpdVBuPgHmgiHrZzrxwnk7hFezeJRjPXlKJH5jLDFeVJKbKCu9skBWYOlbYwyStPiMMF6mszz6Ux6Jj9M2bpbaqw8khRBYGQc_3i4EQFg4MwcKfYe9x7f-7MS_wk_1_-nV9JzsFyMmk0xsOKCYPafGgxHzs9a99le1TtjR46Y8M_44tL2Rr4304IUDmB8zM7hroT41cQBHIIXE0k9kHgC17L-kOmW9RlTmEbbmlhOQTgzWy6TwwThn4zl3srsgnpb1Yjh3lvIRwgk8f2qESTypBQ3cwhA0DSx9G8eDBHFSyUYMBg";
const MODULUS: &str = "xrlP2WnarA6cGuagLcpYaql7E6abh5q2F9JD9-OjbnGtMg9EjcucmtsxZUp8CJyNCfjqZXY62aTRfAR_hqEarOXxD_tWgM-4q6t6Fm-jHpteMRdjVTb7Z9F-p5Q55C38NYf585Yz-W4CGdWZhPDUWTBokWmb85O_ITGVaG_1P60Ket2XW-vGz05XKzXWNP69RHGrOP-IBwxzJqgyS3L1iqBzIaBpUWI31sR4bhvuhfE4vXxADn_WXwGQ00LZIbOPQnFto95yaVJwviWQLO0qFphqu2d6Teps4RvPjXbjNqMvVt2PPMNLSgt4MGW43FFiwfuB3Ew6v2WW8VumRfm9SQ";

struct FixtureFetcher {
    calls: AtomicUsize,
    kid: String,
}

impl GoogleJwksFetcher for FixtureFetcher {
    fn fetch(&self) -> Result<(Value, Duration), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok((json!({"keys": [{"kid": self.kid, "kty": "RSA", "alg": "RS256", "use": "sig", "n": MODULUS, "e": "AQAB"}]}), Duration::from_secs(300)))
    }
}

fn verifier() -> (GoogleIdTokenVerifier, Arc<FixtureFetcher>) {
    let fetcher = Arc::new(FixtureFetcher { calls: AtomicUsize::new(0), kid: "test-key".into() });
    (GoogleIdTokenVerifier::with_fetcher(fetcher.clone()), fetcher)
}

#[test]
fn verifies_signature_audience_issuer_and_expiration() {
    let (verifier, fetcher) = verifier();
    let claims = verifier.verify(VALID, "textmgmt-client").unwrap();
    assert_eq!(claims["email"], "reader@example.com");
    assert!(verifier.verify(WRONG_AUDIENCE, "textmgmt-client").is_err());
    assert!(verifier.verify(WRONG_ISSUER, "textmgmt-client").is_err());
    assert!(verifier.verify(EXPIRED, "textmgmt-client").is_err());
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn rejects_tampered_signature_and_unknown_key() {
    let (verifier, _) = verifier();
    let tampered = format!("{}A", &VALID[..VALID.len() - 1]);
    assert!(verifier.verify(&tampered, "textmgmt-client").is_err());
    let fetcher = Arc::new(FixtureFetcher { calls: AtomicUsize::new(0), kid: "rotated-key".into() });
    let verifier = GoogleIdTokenVerifier::with_fetcher(fetcher.clone());
    assert!(verifier.verify(VALID, "textmgmt-client").is_err());
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn rejects_oversized_token_before_fetching_keys() {
    let (verifier, fetcher) = verifier();
    let oversized = "x".repeat(64 * 1024 + 1);
    assert!(verifier.verify(&oversized, "textmgmt-client").is_err());
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 0);
}
