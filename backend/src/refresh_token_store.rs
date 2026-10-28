//! refresh token 저장소. Python `backend/refresh_token_store.py`의 Rust 대응이다.
//!
//! 운영 factory와 저장소 API는 MySQL replica 공유 저장소만 사용한다.
//! 회전·재사용 탐지 판정(`plan_rotation`)과 세션 집계(`build_session_page`)는 저장소 구현이 공유한다.
//! 로그에는 토큰·family·이메일 원문을 남기지 않는다. 설계: docs/rust-test-seams.md 3.4, 3.10

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde_json::{Value, json};
use sysdeps::{Clock, SystemClock};

use crate::db::{ConnectionProvider, DbConnection, DbError, DbValue, MysqlConfig, Row, mysql_provider};

// 멀티탭/멀티기기 환경에서 여러 요청이 같은 refresh token 으로 거의 동시에 회전을 시도하면, 먼저 처리된 요청이
// 토큰을 회전(replaced_by 설정)시킨 직후 나머지 요청이 이미 회전된 토큰을 제출한다. 이를 재사용 공격으로 오판해
// 패밀리를 폐기하면 세션이 통째로 풀린다. 회전 직후 grace window 안의 재제출은 정상 동시 요청으로 보고,
// window 를 벗어난 재제출만 진짜 재사용으로 차단한다.
pub const DEFAULT_ROTATION_GRACE_SECONDS: i64 = 30;

pub const ADMIN_REVOKE_REASON: &str = "admin-revoked";
/// 관리자 화면에 family_id 전체를 노출하지 않기 위한 표시용 접두사 길이.
pub const SESSION_LABEL_LENGTH: usize = 8;
/// 세션 목록은 family 단위로 집계되지만 만료 row 정리(purge)가 없으므로 상한을 둔다.
pub const MAX_SESSION_GROUPS: usize = 2000;
/// IPv6 최대 표기 길이(45자). IPv4-mapped 표기까지 포함한다.
pub const MAX_CLIENT_IP_LENGTH: usize = 45;
/// User-Agent 는 상한이 없으므로 저장 전에 자른다.
pub const MAX_USER_AGENT_LENGTH: usize = 512;
/// 브라우저를 못 알아본 UA(스크립트·봇)는 앞쪽 제품 토큰만 남긴다.
pub const UA_FALLBACK_LENGTH: usize = 40;
/// 브라우저 자동 업데이트로 세션이 갈라져 보이지 않도록 병합을 허용하는 최대 버전 차이.
pub const SESSION_MERGE_VERSION_TOLERANCE: i64 = 1;

const TABLE_COLUMNS: &str = "jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason, client_ip, user_agent";

// ---------------------------------------------------------------------------
// 오류
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum StoreError {
    Db(DbError),
    /// 지원하지 않는 저장소 설정.
    Unsupported(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Db(e) => write!(f, "{e}"),
            StoreError::Unsupported(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<DbError> for StoreError {
    fn from(e: DbError) -> Self {
        StoreError::Db(e)
    }
}

// ---------------------------------------------------------------------------
// 환경 / 시간 / 문자열 도우미
// ---------------------------------------------------------------------------

fn env_var(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// 정수가 아니면 기본값으로 폴백한다.
pub fn _rotation_grace_seconds() -> i64 {
    env_var("TM_REFRESH_ROTATION_GRACE_SECONDS").and_then(|v| v.trim().parse().ok()).unwrap_or(DEFAULT_ROTATION_GRACE_SECONDS)
}

/// 방금 정상 회전(reason='rotated')되었고 grace window 안이면 true.
fn is_within_rotation_grace(revoke_reason: Option<&str>, revoked_at: Option<i64>, now: i64) -> bool {
    revoke_reason == Some("rotated") && revoked_at.is_some_and(|at| now - at <= _rotation_grace_seconds())
}

/// 빈 값은 NULL 로, 긴 값은 컬럼 폭(문자 수)에 맞게 자른다.
fn truncate(value: Option<&str>, limit: usize) -> Option<String> {
    value.filter(|v| !v.is_empty()).map(|v| v.chars().take(limit).collect())
}

fn sql_with(template: &str, placeholder: &str) -> String {
    template.replace("{p}", placeholder)
}

// ---------------------------------------------------------------------------
// User-Agent 요약
// ---------------------------------------------------------------------------

type UaPatterns = Vec<(Regex, &'static str)>;

fn compile_patterns(defs: &[(&str, &'static str)]) -> UaPatterns {
    defs.iter().map(|(pattern, template)| (Regex::new(pattern).expect("UA 패턴이 올바르지 않다"), *template)).collect()
}

// 파생 브라우저가 Chrome/Safari 토큰을 함께 달고 다니므로 순서가 중요하다.
static UA_BROWSER_PATTERNS: LazyLock<UaPatterns> = LazyLock::new(|| {
    compile_patterns(&[
        (r"Edg(?:e|A|iOS)?/(\d+)", "Edge {0}"),
        (r"OPR/(\d+)", "Opera {0}"),
        (r"SamsungBrowser/(\d+)", "Samsung Internet {0}"),
        (r"Whale/(\d+)", "Whale {0}"),
        (r"(?:FxiOS|Firefox)/(\d+)", "Firefox {0}"),
        (r"(?:CriOS|Chrome)/(\d+)", "Chrome {0}"),
        (r"Version/(\d+)[\d.]* (?:Mobile/\S+ )?Safari", "Safari {0}"),
    ])
});

static UA_OS_PATTERNS: LazyLock<UaPatterns> = LazyLock::new(|| {
    compile_patterns(&[
        (r"iPhone OS (\d+)", "iOS {0}"),
        (r"CPU OS (\d+)", "iPadOS {0}"),
        (r"Mac OS X (\d+[._]\d+)", "macOS {0}"),
        (r"Android (\d+)", "Android {0}"),
        (r"Windows NT 10\.0", "Windows 10+"),
        (r"Windows NT ([\d.]+)", "Windows NT {0}"),
        (r"CrOS", "ChromeOS"),
        (r"Linux", "Linux"),
    ])
});

fn match_ua_pattern(user_agent: &str, patterns: &[(Regex, &str)]) -> String {
    for (pattern, template) in patterns {
        if let Some(caps) = pattern.captures(user_agent) {
            return match caps.get(1) {
                Some(group) => template.replace("{0}", &group.as_str().replace('_', ".")),
                None => template.to_string(),
            };
        }
    }
    String::new()
}

/// UA 원문을 '브라우저 / OS' 한 줄로 줄인다. 판별에 실패하면 원문 앞부분을 그대로 돌려줘 봇/스크립트도 식별하게 한다.
pub fn summarize_user_agent(user_agent: &str) -> String {
    if user_agent.is_empty() {
        return String::new();
    }
    let browser = match_ua_pattern(user_agent, &UA_BROWSER_PATTERNS);
    let os_name = match_ua_pattern(user_agent, &UA_OS_PATTERNS);
    match (browser.is_empty(), os_name.is_empty()) {
        (false, false) => format!("{browser} / {os_name}"),
        (false, true) => browser,
        (true, false) => os_name,
        (true, true) => user_agent.chars().take(UA_FALLBACK_LENGTH).collect(),
    }
}

/// 브라우저 이름과 메이저 버전을 분리해서 뽑는다. 세션 병합에서 버전 인접 여부(±1)를 비교하려면 정수 버전이 필요하다.
/// 첫 번째로 맞는 패턴에 버전 그룹이 없거나 정수가 아니면 `None`이다.
pub fn _parse_browser_name_version_with(user_agent: &str, patterns: &[(Regex, &str)]) -> Option<(String, i64)> {
    if user_agent.is_empty() {
        return None;
    }
    for (pattern, template) in patterns {
        if let Some(caps) = pattern.captures(user_agent) {
            let version = caps.get(1)?.as_str().parse::<i64>().ok()?;
            let name = template.split(" {0}").next().unwrap_or("");
            return Some((name.to_string(), version));
        }
    }
    None
}

fn parse_browser_name_version(user_agent: &str) -> Option<(String, i64)> {
    _parse_browser_name_version_with(user_agent, &UA_BROWSER_PATTERNS)
}

// ---------------------------------------------------------------------------
// 세션 집계
// ---------------------------------------------------------------------------

// family 단위 집계. `{p}` 는 저장소별 placeholder(?, %s)로 치환한다.
// 접속 IP/UA 는 family 안에서 가장 최근에 발급된 토큰(= 마지막 refresh 갱신) 값을 쓴다.
// `{p}` 두 개가 서브쿼리의 {where} 보다 앞서 나오므로 파라미터 순서는 (now, now, ...) 다.
const SESSION_AGGREGATE_SQL: &str = "
    SELECT family_id,
           email,
           MIN(issued_at) AS created_at,
           MAX(issued_at) AS last_seen_at,
           MAX(expires_at) AS max_expires_at,
           COUNT(*) AS token_count,
           SUM(CASE WHEN revoked_at IS NULL AND expires_at >= {p} THEN 1 ELSE 0 END) AS valid_token_count,
           MAX(CASE WHEN revoked_at IS NULL AND expires_at >= {p} THEN expires_at ELSE 0 END) AS active_expires_at,
           MAX(CASE WHEN revoke_reason IS NOT NULL AND revoke_reason <> 'rotated' THEN revoke_reason END) AS terminal_reason,
           MAX(revoked_at) AS revoked_at,
           MAX(CASE WHEN recency_rank = 1 THEN client_ip END) AS client_ip,
           MAX(CASE WHEN recency_rank = 1 THEN user_agent END) AS user_agent
    FROM (
        SELECT family_id,
               email,
               issued_at,
               expires_at,
               revoked_at,
               revoke_reason,
               client_ip,
               user_agent,
               ROW_NUMBER() OVER (PARTITION BY family_id ORDER BY issued_at DESC, jti DESC) AS recency_rank
        FROM refresh_tokens
        {where}
    ) ranked
    GROUP BY family_id, email
    ORDER BY MAX(issued_at) DESC
    LIMIT {limit}
";

fn session_aggregate_sql(placeholder: &str, by_email: bool, by_family: bool, limit: usize) -> String {
    let mut conditions = Vec::new();
    if by_email {
        conditions.push(format!("email = {placeholder}"));
    }
    if by_family {
        conditions.push(format!("family_id = {placeholder}"));
    }
    let where_clause = if conditions.is_empty() { String::new() } else { format!("WHERE {}", conditions.join(" AND ")) };
    sql_with(SESSION_AGGREGATE_SQL, placeholder).replace("{where}", &where_clause).replace("{limit}", &limit.to_string())
}

static NULL: DbValue = DbValue::Null;

fn cell<'a>(row: &'a Row, key: &str) -> &'a DbValue {
    row.get(key).unwrap_or(&NULL)
}

/// MySQL 의 `SUM()` 은 DECIMAL 이라 문자열로 읽힌다. 정수/문자열 모두 정수로 바꾼다. NULL 은 0 이다.
fn int_of(value: &DbValue) -> i64 {
    match value {
        DbValue::Int(n) => *n,
        DbValue::Text(s) => s.trim().parse::<i64>().or_else(|_| s.trim().parse::<f64>().map(|f| f as i64)).unwrap_or(0),
        _ => 0,
    }
}

fn opt_int(value: &DbValue) -> Option<i64> {
    (!value.is_null()).then(|| int_of(value))
}

fn text_of(value: &DbValue) -> String {
    match value {
        DbValue::Text(s) => s.clone(),
        DbValue::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

fn opt_text(value: &DbValue) -> Option<String> {
    (!value.is_null()).then(|| text_of(value))
}

#[derive(Clone)]
struct Session {
    session_id: String,
    email: String,
    status: &'static str,
    created_at: i64,
    last_seen_at: i64,
    expires_at: i64,
    revoked_at: Option<i64>,
    revoke_reason: Option<String>,
    token_count: i64,
    valid_token_count: i64,
    client_ip: String,
    user_agent: String,
    is_current: bool,
    merged_family_ids: Vec<String>,
}

impl Session {
    fn to_json(&self) -> Value {
        let label: String = self.session_id.chars().take(SESSION_LABEL_LENGTH).collect();
        json!({
            "session_id": self.session_id,
            "session_label": format!("{label}..."),
            "email": self.email,
            "status": self.status,
            "created_at": self.created_at,
            "last_seen_at": self.last_seen_at,
            "expires_at": self.expires_at,
            "revoked_at": self.revoked_at,
            "revoke_reason": self.revoke_reason,
            "token_count": self.token_count,
            "valid_token_count": self.valid_token_count,
            "client_ip": self.client_ip,
            "user_agent": self.user_agent,
            "user_agent_summary": summarize_user_agent(&self.user_agent),
            "is_current": self.is_current,
            "merged_family_ids": self.merged_family_ids,
        })
    }
}

/// family 집계 row를 관리자 화면용 세션으로 바꾼다. 상태 판정은 SQL dialect와 분리해 처리한다.
/// - active: 아직 살아 있는 토큰이 하나라도 있다.
/// - revoked: 살아 있는 토큰이 없고 종료 사유('rotated' 이외)가 있다.
/// - expired: 살아 있는 토큰이 없고 종료 사유도 없다(자연 만료).
fn derive_session(row: &Row) -> Session {
    let valid_token_count = int_of(cell(row, "valid_token_count"));
    let terminal_reason = opt_text(cell(row, "terminal_reason")).filter(|r| !r.is_empty());
    let status = if valid_token_count > 0 {
        "active"
    } else if terminal_reason.is_some() {
        "revoked"
    } else {
        "expired"
    };
    let active_expires_at = int_of(cell(row, "active_expires_at"));
    let family_id = text_of(cell(row, "family_id"));
    Session {
        merged_family_ids: vec![family_id.clone()],
        session_id: family_id,
        email: text_of(cell(row, "email")),
        status,
        created_at: int_of(cell(row, "created_at")),
        last_seen_at: int_of(cell(row, "last_seen_at")),
        // 활성 세션은 살아 있는 토큰의 만료를, 그 외에는 마지막 토큰의 만료를 보여준다.
        expires_at: if status == "active" && active_expires_at != 0 { active_expires_at } else { int_of(cell(row, "max_expires_at")) },
        revoked_at: if status != "active" { opt_int(cell(row, "revoked_at")) } else { None },
        revoke_reason: if status == "revoked" { terminal_reason } else { None },
        token_count: int_of(cell(row, "token_count")),
        valid_token_count,
        // 마지막 refresh 갱신 시점의 접속 정보. 컬럼 추가 이전 세션은 빈 문자열이다.
        client_ip: text_of(cell(row, "client_ip")),
        user_agent: text_of(cell(row, "user_agent")),
        is_current: false,
    }
}

/// 같은 기기의 브라우저 업데이트로 판단된 세션 묶음을 화면 표시용 한 행으로 합친다.
/// family_id 회전·재사용탐지 상태는 건드리지 않는다. revoke 는 여전히 최신(primary) family 하나만 대상으로 한다.
fn combine_sessions(members: Vec<Session>) -> Session {
    if members.len() == 1 {
        return members.into_iter().next().expect("길이를 확인했다");
    }
    // 표시 필드(client_ip/user_agent/expires_at 등)는 실제로 살아 있는 세션을 우선한다.
    // 더 최근에 만들어졌지만 이미 만료된 family 를 대표로 삼지 않는다.
    let any_active = members.iter().any(|m| m.status == "active");
    let mut primary: Option<&Session> = None;
    for member in members.iter().filter(|m| !any_active || m.status == "active") {
        // 동률이면 먼저 나온 쪽을 유지한다(Python max 와 같다).
        if primary.is_none_or(|p| member.last_seen_at > p.last_seen_at) {
            primary = Some(member);
        }
    }
    let mut combined = primary.expect("members 가 비어 있지 않다").clone();
    combined.status = if any_active { "active" } else { "expired" };
    combined.created_at = members.iter().map(|m| m.created_at).min().unwrap_or(combined.created_at);
    combined.last_seen_at = members.iter().map(|m| m.last_seen_at).max().unwrap_or(combined.last_seen_at);
    combined.token_count = members.iter().map(|m| m.token_count).sum();
    combined.valid_token_count = members.iter().map(|m| m.valid_token_count).sum();
    combined.is_current = members.iter().any(|m| m.is_current);
    combined.merged_family_ids = members.iter().map(|m| m.session_id.clone()).collect();
    combined
}

type MergeKey = (String, String, String, String);

/// 같은 계정·접속 IP·OS 에서 브라우저 메이저 버전만 인접(±1)하게 바뀐 세션들을 하나의 표시 행으로 묶는다.
/// - revoked 세션은 관리자의 명시적 폐기 이력을 가리지 않도록 병합하지 않는다.
/// - UA 를 브라우저/OS 로 파싱하지 못하거나 client_ip 가 없는 세션도 병합하지 않는다.
/// - 병합은 표시 전용이며 family_id/rotate()/재사용탐지에는 영향을 주지 않는다.
fn merge_similar_sessions(sessions: Vec<Session>) -> Vec<Session> {
    let mut bucket_index: HashMap<MergeKey, usize> = HashMap::new();
    let mut buckets: Vec<Vec<(i64, Session)>> = Vec::new();
    let mut merged: Vec<Session> = Vec::new();
    for session in sessions {
        if session.status == "revoked" {
            merged.push(session);
            continue;
        }
        let browser = parse_browser_name_version(&session.user_agent);
        let os_name = match_ua_pattern(&session.user_agent, &UA_OS_PATTERNS);
        let Some((browser_name, version)) = browser.filter(|_| !os_name.is_empty() && !session.client_ip.is_empty()) else {
            merged.push(session);
            continue;
        };
        let key = (session.email.clone(), session.client_ip.clone(), os_name, browser_name);
        let index = *bucket_index.entry(key).or_insert_with(|| {
            buckets.push(Vec::new());
            buckets.len() - 1
        });
        buckets[index].push((version, session));
    }

    for mut entries in buckets {
        entries.sort_by_key(|(_, s)| s.last_seen_at);
        let mut iter = entries.into_iter();
        let (mut chain_version, first) = iter.next().expect("버킷은 비어 있지 않다");
        let mut chain = vec![first];
        for (version, session) in iter {
            if (version - chain_version).abs() <= SESSION_MERGE_VERSION_TOLERANCE {
                chain.push(session);
            } else {
                merged.push(combine_sessions(std::mem::replace(&mut chain, vec![session])));
            }
            chain_version = version;
        }
        merged.push(combine_sessions(chain));
    }
    merged
}

/// 집계 row 들을 병합·상태 요약·필터·페이징이 적용된 응답으로 만든다.
fn build_session_page(rows: &[Row], q: &SessionQuery) -> Value {
    let mut sessions: Vec<Session> = rows.iter().map(derive_session).collect();
    if let Some(current) = q.current_family_id.as_deref().filter(|c| !c.is_empty()) {
        for session in sessions.iter_mut().filter(|s| s.session_id == current) {
            session.is_current = true;
        }
    }
    let sessions = merge_similar_sessions(sessions);

    let mut summary = json!({"active": 0, "expired": 0, "revoked": 0, "total": sessions.len()});
    for session in &sessions {
        summary[session.status] = json!(summary[session.status].as_i64().unwrap_or(0) + 1);
    }

    let mut filtered: Vec<&Session> = sessions.iter().filter(|s| q.status == "all" || s.status == q.status).collect();
    filtered.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at));
    let total_items = filtered.len();
    let page_size = q.page_size as usize;
    // Python 슬라이스처럼 page 0 은 빈 목록이다.
    let items: Vec<Value> = if q.page == 0 { Vec::new() } else { filtered.iter().skip((q.page as usize - 1) * page_size).take(page_size).map(|s| s.to_json()).collect() };
    json!({
        "items": items,
        "pagination": {"page": q.page, "pageSize": q.page_size, "totalItems": total_items, "totalPages": total_items.div_ceil(page_size.max(1)).max(1)},
        "summary": summary,
    })
}

/// `list_sessions` 인자. Python 키워드 인자의 기본값에 대응한다.
#[derive(Debug, Clone)]
pub struct SessionQuery {
    pub status: String,
    pub current_family_id: Option<String>,
    pub page: u32,
    pub page_size: u32,
    pub email: Option<String>,
}

impl Default for SessionQuery {
    fn default() -> Self {
        Self { status: "active".to_string(), current_family_id: None, page: 1, page_size: 50, email: None }
    }
}

// ---------------------------------------------------------------------------
// 회전 판정 (두 저장소 공통)
// ---------------------------------------------------------------------------

const INSERT_TOKEN_SQL: &str = "INSERT INTO refresh_tokens
    (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason, client_ip, user_agent)
VALUES ({p}, {p}, {p}, {p}, {p}, NULL, NULL, NULL, {p}, {p})";

const SELECT_TOKEN_SQL: &str = "SELECT family_id, email, expires_at, replaced_by, revoked_at, revoke_reason FROM refresh_tokens WHERE jti = {p}";

const MARK_ROTATED_SQL: &str = "UPDATE refresh_tokens SET replaced_by = {p}, revoked_at = {p}, revoke_reason = {p} WHERE jti = {p}";

const REVOKE_FAMILY_SQL: &str = "UPDATE refresh_tokens
SET revoked_at = COALESCE(revoked_at, {p}),
    revoke_reason = CASE
        WHEN revoke_reason IS NULL OR revoke_reason = 'rotated' THEN {p}
        ELSE revoke_reason
    END
WHERE family_id = {p}";

const OBSERVATION_SQL: &str = "SELECT replaced_by, revoked_at, revoke_reason FROM refresh_tokens WHERE jti = {p}";

struct TokenRow {
    family_id: String,
    email: String,
    expires_at: i64,
    replaced_by: Option<String>,
    revoked_at: Option<i64>,
    revoke_reason: Option<String>,
}

impl TokenRow {
    fn from_db_row(row: &Row) -> Self {
        Self {
            family_id: text_of(cell(row, "family_id")),
            email: text_of(cell(row, "email")),
            expires_at: int_of(cell(row, "expires_at")),
            replaced_by: opt_text(cell(row, "replaced_by")),
            revoked_at: opt_int(cell(row, "revoked_at")),
            revoke_reason: opt_text(cell(row, "revoke_reason")),
        }
    }
}

/// `rotate` 결과. `Ok`일 때만 새 토큰을 발급한다. 나머지는 거절 사유다.
/// 문자열 비교 오타 하나로 인가 판정이 바뀌지 않게 enum으로 둔다. 로그와 테스트에는 `as_str`을 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    Ok,
    Missing,
    Mismatch,
    Expired,
    Reused,
    Revoked,
}

impl Rotation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::Mismatch => "mismatch",
            Self::Expired => "expired",
            Self::Reused => "reused",
            Self::Revoked => "revoked",
        }
    }
}

impl std::str::FromStr for Rotation {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        [Self::Ok, Self::Missing, Self::Mismatch, Self::Expired, Self::Reused, Self::Revoked].into_iter().find(|r| r.as_str() == text).ok_or_else(|| format!("unknown rotation result: {text}"))
    }
}

enum Plan {
    /// 변경 없이 rollback 하고 결과를 돌려준다.
    Reject(Rotation),
    /// grace window 내 동시 요청: 패밀리를 살린 채 후속 토큰만 재발급한다.
    InsertOnly,
    /// 패밀리를 폐기하고 commit 한 뒤 결과를 돌려준다.
    RevokeFamily { reason: &'static str, result: Rotation },
    /// 새 토큰을 넣고 이전 토큰을 'rotated' 로 표시한다.
    Rotate,
}

fn plan_rotation(row: Option<&TokenRow>, family_id: &str, email: &str, now: i64) -> Plan {
    let Some(row) = row else { return Plan::Reject(Rotation::Missing) };
    if row.family_id != family_id || row.email != email {
        return Plan::Reject(Rotation::Mismatch);
    }
    if row.expires_at < now {
        return Plan::Reject(Rotation::Expired);
    }
    if row.replaced_by.is_some() {
        if is_within_rotation_grace(row.revoke_reason.as_deref(), row.revoked_at, now) {
            return Plan::InsertOnly;
        }
        return Plan::RevokeFamily { reason: "reuse-detected", result: Rotation::Reused };
    }
    if row.revoked_at.is_some() {
        return Plan::RevokeFamily { reason: "revoked-token-reuse", result: Rotation::Revoked };
    }
    Plan::Rotate
}

// ---------------------------------------------------------------------------
// 공통 trait
// ---------------------------------------------------------------------------

/// refresh token 저장소 인터페이스.
#[allow(clippy::too_many_arguments)]
pub trait RefreshTokenStoreApi: Send + Sync {
    fn store_issued(&self, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError>;

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
    ) -> Result<Rotation, StoreError>;

    fn revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError>;

    /// 관측 로깅 전용 읽기 조회. 토큰 상태 판정이나 인가에 사용하지 않는다.
    fn get_token_observation(&self, token_id: &str) -> Result<Option<Value>, StoreError>;

    fn _session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError>;

    fn list_sessions(&self, q: &SessionQuery) -> Result<Value, StoreError> {
        let rows = self._session_rows(q.email.as_deref(), None)?;
        Ok(build_session_page(&rows, q))
    }

    /// 관리자 요청으로 family 하나를 폐기한다. 존재하지 않으면 found=false.
    fn revoke_session(&self, family_id: &str) -> Result<Value, StoreError> {
        let before = self._session_rows(None, Some(family_id))?;
        let Some(first) = before.first() else {
            return Ok(json!({"found": false, "revoked": false, "session": null}));
        };
        let had_active = int_of(cell(first, "valid_token_count")) > 0;
        self.revoke_family(family_id, ADMIN_REVOKE_REASON)?;
        let after = self._session_rows(None, Some(family_id))?;
        Ok(json!({"found": true, "revoked": had_active, "session": after.first().map(|r| derive_session(r).to_json())}))
    }
}

fn observation_json(replaced_by: &DbValue, revoked_at: &DbValue, revoke_reason: &DbValue) -> Value {
    json!({
        "replaced_by_present": !replaced_by.is_null(),
        "revoked_at": Value::from(revoked_at),
        "revoke_reason": Value::from(revoke_reason),
    })
}

fn warn_if_truncated(row_count: usize, max_session_groups: usize) {
    if row_count >= max_session_groups {
        log::warn!("세션 목록이 상한({max_session_groups})에 도달해 잘렸습니다. 오래된 refresh token row 정리가 필요합니다.");
    }
}

// ---------------------------------------------------------------------------
// MySQL 저장소
// ---------------------------------------------------------------------------

/// MySQL 백엔드 refresh token 저장소.
///
/// 여러 replica/worker가 같은 테이블을 공유한다. 회전은 InnoDB 트랜잭션과 `SELECT ... FOR UPDATE` 행 잠금으로 원자성을 보장한다.
pub struct MySQLRefreshTokenStore {
    provider: ConnectionProvider,
    max_session_groups: usize,
    clock: Arc<dyn Clock>,
}

impl MySQLRefreshTokenStore {
    /// `TM_MYSQL_*` 환경 변수로 접속한다. 테이블이 없으면 만든다.
    pub fn new() -> Result<Self, StoreError> {
        let config = MysqlConfig::from_env(&env_var, "textmanager");
        Self::with_connector(mysql_provider(config))
    }

    /// 연결 제공자를 주입하고 `_init_db`(테이블 생성·컬럼 마이그레이션)를 실행한다.
    pub fn with_connector(provider: ConnectionProvider) -> Result<Self, StoreError> {
        let store = Self::with_connection_provider(provider);
        store.init_db()?;
        Ok(store)
    }

    /// 연결 제공자만 주입한다. `_init_db`는 실행하지 않는다.
    pub fn with_connection_provider(provider: ConnectionProvider) -> Self {
        Self { provider, max_session_groups: MAX_SESSION_GROUPS, clock: Arc::new(SystemClock) }
    }

    /// 세션 목록 상한을 바꾼다. Python 모듈 전역 `MAX_SESSION_GROUPS` 대체.
    pub fn with_max_session_groups(mut self, max_session_groups: usize) -> Self {
        self.max_session_groups = max_session_groups;
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn _connect(&self) -> Result<Box<dyn DbConnection>, DbError> {
        (self.provider)()
    }

    fn now(&self) -> i64 {
        self.clock.now() as i64
    }

    /// 연결을 열어 `f`를 실행하고 항상 닫는다(Python `_connect` contextmanager).
    fn with_conn<T>(&self, f: impl FnOnce(&mut dyn DbConnection) -> Result<T, DbError>) -> Result<T, StoreError> {
        let mut conn = self._connect()?;
        let result = f(conn.as_mut());
        conn.close();
        Ok(result?)
    }

    /// `with_conn`에 오류 시 rollback 을 더한다(Python `except Exception: conn.rollback(); raise`).
    fn in_transaction<T>(&self, f: impl FnOnce(&mut dyn DbConnection) -> Result<T, DbError>) -> Result<T, StoreError> {
        self.with_conn(|conn| {
            let result = f(&mut *conn);
            if result.is_err() {
                // 원래 오류를 우선한다. rollback 실패는 연결이 이미 끊긴 경우라 의미가 없다.
                let _ = conn.rollback();
            }
            result
        })
    }

    fn init_db(&self) -> Result<(), StoreError> {
        self.with_conn(|conn| {
            let mut cur = conn.cursor();
            cur.execute(
                "CREATE TABLE IF NOT EXISTS refresh_tokens (
                    jti VARCHAR(64) PRIMARY KEY,
                    family_id VARCHAR(64) NOT NULL,
                    email VARCHAR(320) NOT NULL,
                    issued_at BIGINT NOT NULL,
                    expires_at BIGINT NOT NULL,
                    replaced_by VARCHAR(64),
                    revoked_at BIGINT,
                    revoke_reason VARCHAR(64),
                    client_ip VARCHAR(45),
                    user_agent VARCHAR(512),
                    INDEX idx_refresh_tokens_family (family_id)
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
                &[],
            )?;
            Self::migrate_add_client_columns(cur.as_mut())?;
            drop(cur);
            conn.commit()
        })
    }

    /// 컬럼 추가 이전에 만들어진 테이블에 client_ip/user_agent 를 붙인다.
    fn migrate_add_client_columns(cur: &mut dyn crate::db::DbCursor) -> Result<(), DbError> {
        for (column, definition) in [("client_ip", "VARCHAR(45)"), ("user_agent", "VARCHAR(512)")] {
            cur.execute("SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'refresh_tokens' AND column_name = %s", &[column.into()])?;
            if cur.fetchone().is_some_and(|row| int_of(cell(&row, "cnt")) == 0) {
                log::info!("Migrating refresh_tokens: adding {column} column");
                cur.execute(&format!("ALTER TABLE refresh_tokens ADD COLUMN {column} {definition}"), &[])?;
            }
        }
        Ok(())
    }

    fn revoke_family_in(cur: &mut dyn crate::db::DbCursor, family_id: &str, reason: &str, revoked_at: i64) -> Result<(), DbError> {
        cur.execute(&sql_with(REVOKE_FAMILY_SQL, "%s"), &[revoked_at.into(), reason.into(), family_id.into()])
    }
}

#[allow(clippy::too_many_arguments)]
impl RefreshTokenStoreApi for MySQLRefreshTokenStore {
    fn store_issued(&self, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError> {
        let params: [DbValue; 7] = [
            token_id.into(),
            family_id.into(),
            email.into(),
            issued_at.into(),
            expires_at.into(),
            truncate(client_ip, MAX_CLIENT_IP_LENGTH).into(),
            truncate(user_agent, MAX_USER_AGENT_LENGTH).into(),
        ];
        self.with_conn(|conn| {
            conn.cursor().execute(
                &format!(
                    "INSERT INTO refresh_tokens ({TABLE_COLUMNS})
                    VALUES (%s, %s, %s, %s, %s, NULL, NULL, NULL, %s, %s)
                    ON DUPLICATE KEY UPDATE
                        family_id = VALUES(family_id), email = VALUES(email),
                        issued_at = VALUES(issued_at), expires_at = VALUES(expires_at),
                        replaced_by = NULL, revoked_at = NULL, revoke_reason = NULL,
                        client_ip = VALUES(client_ip), user_agent = VALUES(user_agent)"
                ),
                &params,
            )?;
            conn.commit()
        })
    }

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
        let now = self.now();
        let insert_params: [DbValue; 7] = [
            new_token_id.into(),
            family_id.into(),
            email.into(),
            issued_at.into(),
            expires_at.into(),
            truncate(client_ip, MAX_CLIENT_IP_LENGTH).into(),
            truncate(user_agent, MAX_USER_AGENT_LENGTH).into(),
        ];
        let insert_sql = sql_with(INSERT_TOKEN_SQL, "%s");
        // autocommit=0 연결이라 첫 문장(SELECT ... FOR UPDATE)이 트랜잭션을 연다.
        self.in_transaction(|conn| {
            let mut cur = conn.cursor();
            cur.execute(&format!("{} FOR UPDATE", sql_with(SELECT_TOKEN_SQL, "%s")), &[current_token_id.into()])?;
            let row = cur.fetchone().map(|r| TokenRow::from_db_row(&r));
            match plan_rotation(row.as_ref(), family_id, email, now) {
                Plan::Reject(result) => {
                    conn.rollback()?;
                    Ok(result)
                }
                Plan::InsertOnly => {
                    cur.execute(&insert_sql, &insert_params)?;
                    conn.commit()?;
                    Ok(Rotation::Ok)
                }
                Plan::RevokeFamily { reason, result } => {
                    Self::revoke_family_in(cur.as_mut(), family_id, reason, now)?;
                    conn.commit()?;
                    Ok(result)
                }
                Plan::Rotate => {
                    cur.execute(&insert_sql, &insert_params)?;
                    cur.execute(&sql_with(MARK_ROTATED_SQL, "%s"), &[new_token_id.into(), issued_at.into(), "rotated".into(), current_token_id.into()])?;
                    conn.commit()?;
                    Ok(Rotation::Ok)
                }
            }
        })
    }

    fn revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError> {
        let now = self.now();
        self.in_transaction(|conn| {
            Self::revoke_family_in(conn.cursor().as_mut(), family_id, reason, now)?;
            conn.commit()
        })
    }

    fn get_token_observation(&self, token_id: &str) -> Result<Option<Value>, StoreError> {
        let row = self.with_conn(|conn| {
            let mut cur = conn.cursor();
            cur.execute(&sql_with(OBSERVATION_SQL, "%s"), &[token_id.into()])?;
            Ok(cur.fetchone())
        })?;
        Ok(row.map(|r| observation_json(cell(&r, "replaced_by"), cell(&r, "revoked_at"), cell(&r, "revoke_reason"))))
    }

    fn _session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
        let now = self.now();
        let sql = session_aggregate_sql("%s", email.is_some(), family_id.is_some(), self.max_session_groups);
        // SQL 안에서 `now` placeholder 는 두 CASE 식에 각각 한 번씩 쓰인다.
        let mut params: Vec<DbValue> = vec![now.into(), now.into()];
        params.extend(email.map(DbValue::from));
        params.extend(family_id.map(DbValue::from));
        let rows = self.with_conn(|conn| {
            let mut cur = conn.cursor();
            cur.execute(&sql, &params)?;
            Ok(cur.fetchall().unwrap_or_default())
        })?;
        warn_if_truncated(rows.len(), self.max_session_groups);
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// 팩토리
// ---------------------------------------------------------------------------
/// 팩토리가 고른 저장소.
pub enum AnyRefreshTokenStore {
    MySql(MySQLRefreshTokenStore),
}

impl AnyRefreshTokenStore {
    fn inner(&self) -> &dyn RefreshTokenStoreApi {
        match self {
            AnyRefreshTokenStore::MySql(s) => s,
        }
    }
}

#[allow(clippy::too_many_arguments)]
impl RefreshTokenStoreApi for AnyRefreshTokenStore {
    fn store_issued(&self, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError> {
        self.inner().store_issued(token_id, family_id, email, issued_at, expires_at, client_ip, user_agent)
    }

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
        self.inner().rotate(current_token_id, new_token_id, family_id, email, issued_at, expires_at, client_ip, user_agent)
    }

    fn revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError> {
        self.inner().revoke_family(family_id, reason)
    }

    fn get_token_observation(&self, token_id: &str) -> Result<Option<Value>, StoreError> {
        self.inner().get_token_observation(token_id)
    }

    fn _session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
        self.inner()._session_rows(email, family_id)
    }
}

/// 테스트용 MySQL connector factory에서 backend 환경 변수를 확인한다.
fn require_mysql_backend() -> Result<(), StoreError> {
    let backend = env_var("TM_REFRESH_TOKEN_BACKEND").unwrap_or_else(|| "mysql".to_string()).to_lowercase();
    if backend == "mysql" {
        return Ok(());
    }
    Err(StoreError::Unsupported(format!("Rust backend는 TM_REFRESH_TOKEN_BACKEND=mysql만 지원한다 (현재 값: {backend:?})")))
}

/// Rust backend는 replica 간 공유를 위해 MySQL 저장소만 사용한다.
pub fn create_refresh_token_store() -> Result<AnyRefreshTokenStore, StoreError> {
    require_mysql_backend()?;
    MySQLRefreshTokenStore::new().map(AnyRefreshTokenStore::MySql)
}

/// `create_refresh_token_store`와 같지만 MySQL 연결 제공자를 주입한다.
pub fn create_refresh_token_store_with(provider: ConnectionProvider) -> Result<AnyRefreshTokenStore, StoreError> {
    require_mysql_backend()?;
    MySQLRefreshTokenStore::with_connector(provider).map(AnyRefreshTokenStore::MySql)
}
