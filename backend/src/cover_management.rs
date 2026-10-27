//! 관리자용 EPUB 표지 목록, 후보, 이미지 cache, 저장 API. Python `backend/cover_management.py`에 대응한다.
//!
//! - 서비스(`CoverManagement`)는 파일 시스템 위에서 동기로 동작한다. 라우트(`routes`)가 블로킹 작업으로 넘겨 부른다.
//! - 검색·내려받기·시계·썸네일은 `CoverHooks`로 바꾼다. 기본값은 실제 구현이다. 설계: docs/rust-test-seams.md 3.7, 3.8, 3.12.
//! - 취소(`check_cancelled`)는 Python `ContextVar`처럼 요청을 처리하는 스레드에 붙는다(`with_cancel_scope`).

pub mod routes;
mod text;

use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::alphabet::STANDARD as STANDARD_ALPHABET;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use corpus_layout::IGNORED_DIR_NAMES;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sysdeps::{Clock, RealFs, SystemClock};
use zip::ZipArchive;

use crate::book_manager::Thumbnailer;
use crate::cover_sources::urlx::{Netloc, urlsplit};
use crate::cover_sources::{CancelFn, GateError, STORES, SearchError, SearchHooks, StoreGate, blocked_status, build_search_url, search_candidates_with, store_hosts, store_lock_with};
use crate::direct_cover_url::{ImageFetcher, RealImageFetcher};
use crate::epub_cover::pathx;
use crate::epub_cover::{self, CoverError, EpubCoverHooks, MAX_IMAGE_BYTES};

pub use text::clean_title;

const BOOK_CHANGED: &str = "EPUB이 변경되었습니다. 목록을 새로 불러오세요";
const ALREADY_COVERED: &str = "EPUB에 이미 표지가 있습니다";
const UNREADABLE_COVER: &str = "저장한 표지를 읽을 수 없습니다";

// ---------------------------------------------------------------------------
// 취소
// ---------------------------------------------------------------------------

thread_local! {
    static REQUEST_CANCELLED: RefCell<Option<CancelFn>> = const { RefCell::new(None) };
}

struct CancelScope(Option<CancelFn>);

impl Drop for CancelScope {
    fn drop(&mut self) {
        REQUEST_CANCELLED.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// `cancel`이 true를 돌려주면 이 스레드에서 도는 `check_cancelled`가 실패한다. 끝나면 이전 값으로 돌린다.
pub fn with_cancel_scope<R>(cancel: CancelFn, f: impl FnOnce() -> R) -> R {
    let previous = REQUEST_CANCELLED.with(|slot| slot.borrow_mut().replace(cancel));
    let _restore = CancelScope(previous);
    f()
}

pub fn is_cancelled() -> bool {
    let cancel = REQUEST_CANCELLED.with(|slot| slot.borrow().clone());
    cancel.is_some_and(|f| f())
}

/// 화면에서 더 필요하지 않은 요청이면 `CoverError`(취소)를 돌려준다.
pub fn check_cancelled() -> Result<(), CoverError> {
    if is_cancelled() { Err(CoverError::cancelled()) } else { Ok(()) }
}

// ---------------------------------------------------------------------------
// 훅
// ---------------------------------------------------------------------------

type SearchFn = Arc<dyn Fn(&str, &Value) -> Result<Value, SearchError> + Send + Sync>;
type FetchBytesFn = Arc<dyn Fn(&str, &str) -> Result<Vec<u8>, String> + Send + Sync>;
type CachedSearchFn = Arc<dyn Fn(&str, &str) -> Option<Value> + Send + Sync>;
type InspectFn = Arc<dyn Fn(&Path) -> Result<Value, CoverError> + Send + Sync>;
type StoreLockFn = Arc<dyn Fn(&Path, &str) -> Box<dyn Any + Send> + Send + Sync>;
type DeleteCacheFn = Arc<dyn Fn(&Path) -> std::io::Result<()> + Send + Sync>;
type DirectoriesFn = Arc<dyn Fn(&str) -> std::io::Result<Vec<Value>> + Send + Sync>;
type CandidatesFn = Arc<dyn Fn(&str, &str) -> Result<Value, CoverError> + Send + Sync>;
type DisconnectedFn = Arc<dyn Fn() -> bool + Send + Sync>;
type ResolveFn = Arc<dyn Fn(&Path) -> PathBuf + Send + Sync>;
type ObserverFn = Arc<dyn Fn() + Send + Sync>;

/// 바깥 의존성을 바꾸는 지점. 기본값은 실제 구현이다.
#[derive(Clone, Default)]
pub struct CoverHooks {
    search: Option<SearchFn>,
    search_hooks: SearchHooks,
    fetch_bytes: Option<FetchBytesFn>,
    image_fetcher: Option<Arc<dyn ImageFetcher>>,
    clock: Option<Arc<dyn Clock>>,
    epub_hooks: EpubCoverHooks,
    cached_search: Option<CachedSearchFn>,
    thumbnailer: Option<Arc<dyn Thumbnailer>>,
    inspect_epub: Option<InspectFn>,
    max_image_bytes: Option<usize>,
    store_lock: Option<StoreLockFn>,
    delete_book_cache: Option<DeleteCacheFn>,
    directories: Option<DirectoriesFn>,
    candidates: Option<CandidatesFn>,
    is_disconnected: Option<DisconnectedFn>,
    resolve: Option<ResolveFn>,
    lock_wait_observer: Option<ObserverFn>,
}

impl CoverHooks {
    /// 서점 검색(`search_candidates`)을 바꾼다. 인자는 서점 이름과 책 메타데이터다.
    pub fn with_search(mut self, f: impl Fn(&str, &Value) -> Result<Value, SearchError> + Send + Sync + 'static) -> Self {
        self.search = Some(Arc::new(f));
        self
    }

    /// 실제 검색이 쓰는 세션·클라이언트·URL 검증(`with_search`로 바꾸지 않았을 때).
    pub fn with_search_hooks(mut self, hooks: SearchHooks) -> Self {
        self.search_hooks = hooks;
        self
    }

    /// 서점 이미지 내려받기(`fetch_bytes`)를 바꾼다. 인자는 서점 이름과 URL이다.
    pub fn with_fetch_bytes(mut self, f: impl Fn(&str, &str) -> Result<Vec<u8>, String> + Send + Sync + 'static) -> Self {
        self.fetch_bytes = Some(Arc::new(f));
        self
    }

    /// 공개 이미지 URL 내려받기(`fetch_url_image`)를 바꾼다.
    pub fn with_image_fetcher(mut self, fetcher: Arc<dyn ImageFetcher>) -> Self {
        self.image_fetcher = Some(fetcher);
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn with_epub_hooks(mut self, hooks: EpubCoverHooks) -> Self {
        self.epub_hooks = hooks;
        self
    }

    /// 캐시된 검색 결과 읽기(`_cached_search`)를 바꾼다. 인자는 서점 이름과 책 상대 경로다.
    pub fn with_cached_search(mut self, f: impl Fn(&str, &str) -> Option<Value> + Send + Sync + 'static) -> Self {
        self.cached_search = Some(Arc::new(f));
        self
    }

    pub fn with_thumbnailer(mut self, thumbnailer: Arc<dyn Thumbnailer>) -> Self {
        self.thumbnailer = Some(thumbnailer);
        self
    }

    /// 표지 검사(`inspect_epub`)를 바꾼다.
    pub fn with_inspect_epub(mut self, f: impl Fn(&Path) -> Result<Value, CoverError> + Send + Sync + 'static) -> Self {
        self.inspect_epub = Some(Arc::new(f));
        self
    }

    /// 원본 미리보기가 읽을 표지 크기 상한(기본 `MAX_IMAGE_BYTES`).
    pub fn with_max_image_bytes(mut self, bytes: usize) -> Self {
        self.max_image_bytes = Some(bytes);
        self
    }

    /// 서점별 잠금(`store_lock`)을 바꾼다. 돌려준 값을 쥐고 있는 동안이 잠금 구간이다.
    pub fn with_store_lock(mut self, f: impl Fn(&Path, &str) -> Box<dyn Any + Send> + Send + Sync + 'static) -> Self {
        self.store_lock = Some(Arc::new(f));
        self
    }

    /// 책 후보 캐시 삭제(`_delete_book_cache`)를 바꾼다. 인자는 책별 캐시 디렉터리다.
    pub fn with_delete_book_cache(mut self, f: impl Fn(&Path) -> std::io::Result<()> + Send + Sync + 'static) -> Self {
        self.delete_book_cache = Some(Arc::new(f));
        self
    }

    /// 디렉터리 목록(`directories`)을 바꾼다(라우트가 쓴다).
    pub fn with_directories(mut self, f: impl Fn(&str) -> std::io::Result<Vec<Value>> + Send + Sync + 'static) -> Self {
        self.directories = Some(Arc::new(f));
        self
    }

    /// 후보 검색(`candidates`)을 바꾼다(라우트가 쓴다). 인자는 서점 이름과 파일 경로다.
    pub fn with_candidates(mut self, f: impl Fn(&str, &str) -> Result<Value, CoverError> + Send + Sync + 'static) -> Self {
        self.candidates = Some(Arc::new(f));
        self
    }

    /// 요청한 쪽이 연결을 끊었는지(`Request.is_disconnected`)를 바꾼다(라우트가 쓴다).
    pub fn with_is_disconnected(mut self, f: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.is_disconnected = Some(Arc::new(f));
        self
    }

    /// 경로 해석(`Path.resolve`)을 바꾼다.
    pub fn with_resolve(mut self, f: impl Fn(&Path) -> PathBuf + Send + Sync + 'static) -> Self {
        self.resolve = Some(Arc::new(f));
        self
    }

    /// 책 잠금이 막혀 기다릴 때마다 부른다.
    pub fn with_lock_wait_observer(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.lock_wait_observer = Some(Arc::new(f));
        self
    }

    pub(crate) fn directories_hook(&self) -> Option<&DirectoriesFn> {
        self.directories.as_ref()
    }

    pub(crate) fn candidates_hook(&self) -> Option<&CandidatesFn> {
        self.candidates.as_ref()
    }

    pub(crate) fn is_disconnected(&self) -> bool {
        self.is_disconnected.as_ref().is_some_and(|f| f())
    }

    pub(crate) fn has_disconnect_probe(&self) -> bool {
        self.is_disconnected.is_some()
    }

    fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone().unwrap_or_else(|| Arc::new(SystemClock))
    }
}

/// 백업 위치와 이름. 기본은 `<root>/.cover_backups`에 편집마다 고유한 이름이다.
#[derive(Debug, Clone, Copy, Default)]
pub struct Backup<'a> {
    pub dir: Option<&'a Path>,
    pub name: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// 도우미
// ---------------------------------------------------------------------------

/// 파일 이름에서 검색어를 만든다.
pub fn filename_title(path: &Path) -> String {
    clean_title(&path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

/// `str(Path(relative))`: 빈 조각과 `.`을 없앤 경로 문자열.
fn norm_rel(relative: &str) -> String {
    let parts: Vec<&str> = relative.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    let joined = parts.join("/");
    if relative.starts_with('/') {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn book_key(relative: &str) -> String {
    sha256_hex(&norm_rel(relative))[..32].to_string()
}

fn base64_engine() -> GeneralPurpose {
    // Python `b64decode(validate=True)`: 알파벳 밖 글자와 틀린 padding은 거절하고 남는 비트는 받는다.
    GeneralPurpose::new(&STANDARD_ALPHABET, GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::RequireCanonical).with_decode_allow_trailing_bits(true))
}

fn decode_base64(data: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64_engine().decode(data)
}

fn now_seconds() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

fn is_epub_name(name: &str) -> bool {
    pathx::ext(name).eq_ignore_ascii_case(".epub")
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

fn host_of(url: &str) -> String {
    urlsplit(url).netloc.map(|netloc| Netloc::parse(&netloc).hostname.unwrap_or_default().to_lowercase()).unwrap_or_default()
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

const IMAGE_INFO_KEYS: [&str; 3] = ["width", "height", "byte_size"];

fn has_image_info(record: &Value) -> bool {
    IMAGE_INFO_KEYS.iter().all(|key| record.get(*key).is_some())
}

// ---------------------------------------------------------------------------
// 서비스
// ---------------------------------------------------------------------------

/// 책 잠금. 버리면 풀린다. 잠금 파일은 지우지 않는다(대기 요청과 새 요청이 같은 잠금을 써야 한다).
pub struct BookLock {
    _file: File,
}

pub struct CoverManagement {
    root: PathBuf,
    cache: PathBuf,
    hooks: CoverHooks,
}

impl CoverManagement {
    pub fn new(root: &Path) -> Self {
        Self::with_hooks(root, CoverHooks::default())
    }

    pub fn with_hooks(root: &Path, hooks: CoverHooks) -> Self {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let cache = root.join(".cover_management");
        CoverManagement { root, cache, hooks }
    }

    pub fn hooks(&self) -> &CoverHooks {
        &self.hooks
    }

    pub fn set_hooks(&mut self, hooks: CoverHooks) {
        self.hooks = hooks;
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn cache(&self) -> &Path {
        &self.cache
    }

    fn epub_hooks(&self) -> &EpubCoverHooks {
        &self.hooks.epub_hooks
    }

    fn inspect(&self, path: &Path) -> Result<Value, CoverError> {
        match &self.hooks.inspect_epub {
            Some(f) => f(path),
            None => epub_cover::inspect_epub_with(path, self.epub_hooks()),
        }
    }

    fn revision_of(&self, path: &Path) -> String {
        self.epub_hooks().revision_of(path)
    }

    fn relative_of(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string_lossy().into_owned())
    }

    fn backup_dir(&self, backup: Backup<'_>) -> PathBuf {
        backup.dir.map(Path::to_path_buf).unwrap_or_else(|| self.root.join(".cover_backups"))
    }

    /// 책 상대 경로를 실제 EPUB(또는 디렉터리) 경로로 푼다. 바깥 경로·내부 디렉터리·symlink는 거절한다.
    pub fn resolve(&self, relative: &str, directory: bool) -> Result<PathBuf, CoverError> {
        if relative.starts_with('/') || relative.contains('\\') || relative.contains('\0') {
            return Err(CoverError::new("올바른 책 상대 경로가 아닙니다"));
        }
        let parts: Vec<&str> = relative.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
        if parts.contains(&"..") {
            return Err(CoverError::new("올바른 책 상대 경로가 아닙니다"));
        }
        let mut current = self.root.clone();
        for part in parts {
            if part.starts_with('.') || IGNORED_DIR_NAMES.contains(&part) {
                return Err(CoverError::new("내부 관리 디렉토리는 사용할 수 없습니다"));
            }
            current.push(part);
            if is_symlink(&current) {
                return Err(CoverError::new("symlink는 표지 관리에서 제외합니다"));
            }
        }
        let resolved = match &self.hooks.resolve {
            Some(f) => f(&current),
            None => std::fs::canonicalize(&current).unwrap_or(current),
        };
        if !resolved.starts_with(&self.root) {
            return Err(CoverError::new("책 디렉토리 밖의 경로는 사용할 수 없습니다"));
        }
        if directory {
            if !resolved.is_dir() {
                return Err(CoverError::new("책 디렉토리를 찾을 수 없습니다"));
            }
        } else if !resolved.file_name().is_some_and(|n| is_epub_name(&n.to_string_lossy())) || !resolved.is_file() {
            return Err(CoverError::new("EPUB 파일을 찾을 수 없습니다"));
        }
        Ok(resolved)
    }

    pub fn directories(&self, path: &str) -> Result<Vec<Value>, CoverError> {
        let parent = self.resolve(path, true)?;
        let mut children: Vec<Value> = Vec::new();
        for entry in std::fs::read_dir(&parent)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || IGNORED_DIR_NAMES.contains(&name.as_str()) || !entry.file_type()?.is_dir() {
                continue;
            }
            children.push(json!({"id": self.relative_of(&entry.path()), "label": name, "fileType": "folder", "children": []}));
        }
        children.sort_by_key(|item| (pyfmt::py_casefold(str_field(item, "label")), str_field(item, "label").to_string()));
        Ok(children)
    }

    /// 표지가 없는(`covered`면 있는) EPUB 목록을 `limit`개씩 훑는다.
    pub fn missing(&self, directory: &str, cursor: &str, limit: usize, covered: bool) -> Result<Value, CoverError> {
        let parent = self.resolve(directory, true)?;
        if !cursor.is_empty() && (!cursor.bytes().all(|b| b.is_ascii_digit()) || cursor.len() > 10) {
            return Err(CoverError::new("목록 cursor가 올바르지 않습니다"));
        }
        let offset: usize = if cursor.is_empty() { 0 } else { cursor.parse().map_err(|_| CoverError::new("목록 cursor가 올바르지 않습니다"))? };
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&parent)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let plain_file = std::fs::symlink_metadata(&path).map(|m| m.is_file()).unwrap_or(false);
            if plain_file && !name.starts_with('.') && is_epub_name(&name) {
                files.push((name, path));
            }
        }
        files.sort_by_key(|(name, _)| (pyfmt::py_casefold(name), name.clone()));
        let total = files.len();
        let mut items: Vec<Value> = Vec::new();
        for (_, path) in files.iter().skip(offset).take(limit) {
            let relative = self.relative_of(path);
            let category = match path.parent() {
                Some(dir) if dir != self.root => self.relative_of(dir),
                _ => "_root".to_string(),
            };
            let identity = json!({"file_path": relative, "book_id": file_inode(path), "category": category, "title": filename_title(path)});
            match self.inspect(path) {
                Ok(metadata) => {
                    let has_cover = truthy(metadata.get("has_cover"));
                    if has_cover != covered && !truthy(metadata.get("cover_error")) {
                        continue;
                    }
                    let mut item = metadata;
                    merge(&mut item, &identity);
                    item["status"] = json!(if has_cover { "covered" } else { "missing" });
                    items.push(item);
                }
                Err(err) => {
                    let mut item = identity;
                    item["revision"] = json!(self.revision_of(path));
                    item["status"] = json!("error");
                    item["error"] = json!(err.to_string());
                    items.push(item);
                }
            }
        }
        let next_offset = (offset + limit).min(total);
        Ok(json!({"items": items, "next_cursor": if next_offset < total { json!(next_offset.to_string()) } else { Value::Null }, "scanned": next_offset, "total": total}))
    }

    // ---- 캐시 파일 ----

    /// 임시 파일에 쓴 뒤 바꿔 넣는다. 실패하면 오류를 돌려준다.
    pub fn try_write(&self, path: &Path, data: &[u8]) -> Result<(), CoverError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        for attempt in 0..16u32 {
            let temporary = parent.join(format!(".tmp-{}{attempt}", &epub_cover::random_hex()[..16]));
            let created = OpenOptions::new().write(true).create_new(true).open(&temporary);
            match created {
                Ok(mut output) => {
                    let outcome = output.write_all(data).and_then(|_| output.flush()).and_then(|_| std::fs::rename(&temporary, path));
                    if outcome.is_err() {
                        let _ = std::fs::remove_file(&temporary);
                    }
                    return outcome.map_err(CoverError::from);
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err.into()),
            }
        }
        Err(CoverError::io("임시 파일 이름을 만들 수 없습니다"))
    }

    /// `try_write`의 결과를 버리고 실패를 로그에 남긴다.
    pub fn write(&self, path: &Path, data: &[u8]) {
        if let Err(err) = self.try_write(path, data) {
            log::warn!("Cover cache write failed: {err}");
        }
    }

    fn read_json_within(&self, path: &Path, ttl: Option<f64>) -> Option<Value> {
        let text = {
            if let Some(ttl) = ttl {
                let modified = std::fs::metadata(path).ok()?.modified().ok()?;
                let age = now_seconds() - modified.duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                if age >= ttl {
                    return None;
                }
            }
            std::fs::read_to_string(path).ok()?
        };
        serde_json::from_str::<Value>(&text).ok().filter(Value::is_object)
    }

    pub fn read_json(&self, path: &Path) -> Option<Value> {
        self.read_json_within(path, None)
    }

    pub fn book_cache(&self, relative: &str) -> PathBuf {
        self.cache.join(format!("book-{}", book_key(relative)))
    }

    fn book_lock(&self, relative: &str, exclusive: bool) -> Result<BookLock, CoverError> {
        // 잠금 파일은 지우지 않는다. 대기 요청과 새 요청이 같은 잠금을 써야 한다.
        let lock_dir = self.cache.join(".book_locks");
        std::fs::create_dir_all(&lock_dir)?;
        let file = OpenOptions::new().read(true).append(true).create(true).open(lock_dir.join(format!("{}.lock", book_key(relative))))?;
        loop {
            check_cancelled()?;
            let locked = if exclusive { file.try_lock() } else { file.try_lock_shared() };
            match locked {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    if let Some(observer) = &self.hooks.lock_wait_observer {
                        observer();
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(std::fs::TryLockError::Error(err)) => return Err(err.into()),
            }
        }
        check_cancelled()?;
        Ok(BookLock { _file: file })
    }

    fn acquire_store_lock(&self, store: &str) -> Result<Box<dyn Any + Send>, CoverError> {
        if let Some(f) = &self.hooks.store_lock {
            return Ok(f(&self.cache, store));
        }
        let clock = self.hooks.clock();
        store_lock_with(&self.cache, store, &RealFs, clock.as_ref(), &is_cancelled).map(|guard| Box::new(guard) as Box<dyn Any + Send>).map_err(|err| match err {
            crate::cover_sources::StoreLockError::Cancelled => CoverError::cancelled(),
            crate::cover_sources::StoreLockError::UnknownStore(_) => CoverError::new("지원하지 않는 서점입니다"),
            other => CoverError::io(&other.to_string()),
        })
    }

    // ---- 검색 후보 ----

    fn candidate_file(&self, candidate_id: &str) -> Result<PathBuf, CoverError> {
        let valid = candidate_id.len() == 64 && candidate_id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid {
            return Err(CoverError::new("표지 후보 ID가 올바르지 않습니다"));
        }
        let grouped = self.cache.join(format!("book-{}", &candidate_id[..32])).join(format!("candidate-{candidate_id}.json"));
        Ok(if grouped.is_file() { grouped } else { self.cache.join(format!("candidate-{candidate_id}.json")) })
    }

    /// 후보 기록 파일 경로. ID가 올바르지 않으면 어떤 후보 파일과도 겹치지 않는 경로를 돌려준다.
    pub fn candidate_path(&self, candidate_id: &str) -> PathBuf {
        self.candidate_file(candidate_id).unwrap_or_else(|_| self.cache.join("candidate-invalid.json"))
    }

    pub fn candidate(&self, candidate_id: &str) -> Result<Value, CoverError> {
        self.read_json(&self.candidate_file(candidate_id)?).filter(has_entries).ok_or_else(|| CoverError::new("표지 후보를 찾을 수 없습니다. 후보를 다시 검색하세요"))
    }

    fn image_file(&self, candidate_id: &str) -> Result<PathBuf, CoverError> {
        Ok(self.candidate_file(candidate_id)?.parent().unwrap_or(&self.cache).join(format!("image-{candidate_id}.jpg")))
    }

    fn image_info(&self, candidate_id: &str, candidate: &mut Value, data: Vec<u8>) -> Result<Vec<u8>, CoverError> {
        if !has_image_info(candidate) {
            let (width, height) = image::ImageReader::new(std::io::Cursor::new(&data))
                .with_guessed_format()
                .and_then(|reader| reader.into_dimensions().map_err(std::io::Error::other))
                .map_err(|_| CoverError::io("이미지 형식을 읽을 수 없습니다"))?;
            merge(candidate, &json!({"width": width, "height": height, "byte_size": data.len()}));
            self.try_write(&self.candidate_file(candidate_id)?, &serde_json::to_vec(candidate).map_err(|e| CoverError::io(&e.to_string()))?)?;
        }
        Ok(data)
    }

    /// 캐시한 검색 결과. 성공 결과는 후보 기록이 모두 남아 있어야 쓴다. 오류는 2분, 그 밖은 1시간 뒤 버린다.
    fn cached_search(&self, store: &str, relative: &str, path: &Path) -> Result<Option<Value>, CoverError> {
        if let Some(f) = &self.hooks.cached_search {
            return Ok(f(store, relative));
        }
        let cached = self.read_json(path).filter(has_entries);
        if let Some(mut cached) = cached.clone().filter(|c| c.get("status").and_then(Value::as_str) == Some("success")) {
            let ids: Vec<String> = cached.get("candidates").and_then(Value::as_array).map(|list| list.iter().map(|c| str_field(c, "candidate_id").to_string()).collect()).unwrap_or_default();
            let mut merged: Vec<(usize, Value)> = Vec::new();
            for (index, id) in ids.iter().enumerate() {
                let record_path = self.candidate_file(id)?;
                let Some(mut record) = self.read_json(&record_path).filter(has_entries) else { return Ok(None) };
                let image_path = record_path.parent().unwrap_or(&self.cache).join(format!("image-{id}.jpg"));
                if image_path.is_file() && !has_image_info(&record) {
                    self.image_info(id, &mut record, std::fs::read(&image_path)?)?;
                }
                merged.push((index, record));
            }
            if let Some(list) = cached.get_mut("candidates").and_then(Value::as_array_mut) {
                for (index, record) in merged {
                    for key in IMAGE_INFO_KEYS {
                        if let Some(value) = record.get(key) {
                            list[index][key] = value.clone();
                        }
                    }
                }
            }
            return Ok(Some(cached));
        }
        let is_error = cached.as_ref().is_some_and(|c| c.get("status").and_then(Value::as_str) == Some("error"));
        Ok(self.read_json_within(path, Some(if is_error { 120.0 } else { 3600.0 })).filter(has_entries))
    }

    fn search(&self, store: &str, metadata: &Value) -> Result<Value, SearchError> {
        match &self.hooks.search {
            Some(f) => f(store, metadata),
            None => search_candidates_with(store, metadata, &self.hooks.search_hooks),
        }
    }

    fn fetch_store_bytes(&self, store: &str, url: &str) -> Result<Vec<u8>, CoverError> {
        match &self.hooks.fetch_bytes {
            Some(f) => f(store, url).map_err(|message| CoverError::io(&message)),
            None => self.hooks.search_hooks.fetch_bytes(store, url, Some(MAX_IMAGE_BYTES)).map_err(|err| match err {
                SearchError::Cancelled => CoverError::cancelled(),
                SearchError::Value(message) => CoverError::io(&message),
                other => CoverError::io(&other.to_string()),
            }),
        }
    }

    fn blocked_result(&self, store: &str, metadata: &Value, retry_after: f64) -> Value {
        let minutes = ((retry_after / 60.0).ceil() as i64).max(1);
        json!({
            "status": "error",
            "blocked": true,
            "retry_after": retry_after.trunc() as i64,
            "error": format!("서점이 일시적으로 요청을 제한하고 있습니다. 약 {minutes}분 뒤 다시 시도하세요"),
            "search_url": build_search_url(store, str_field(metadata, "title")),
            "candidates": [],
        })
    }

    pub fn candidates(&self, store: &str, relative: &str, include_covered: bool, refresh: bool) -> Result<Value, CoverError> {
        if !STORES.contains(&store) {
            return Err(CoverError::new("지원하지 않는 서점입니다"));
        }
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _book = self.book_lock(&relative, false)?;
        let mut metadata = self.inspect(&path)?;
        metadata["title"] = json!(filename_title(&path));
        if truthy(metadata.get("has_cover")) && !include_covered {
            return Err(CoverError::new(ALREADY_COVERED));
        }
        let cache_key = sha256_hex(&format!("{store}:{relative}:{}:filename-v1", str_field(&metadata, "revision")));
        let book_cache = self.book_cache(&relative);
        let result_path = book_cache.join(format!("search-{cache_key}.json"));
        let flat_path = self.cache.join(format!("search-{cache_key}.json"));
        if refresh {
            let _ = std::fs::remove_file(&result_path);
            let _ = std::fs::remove_file(&flat_path);
        }
        if let Some(cached) = self.cached_search(store, &relative, &result_path)? {
            return Ok(cached);
        }
        // 이전 배포의 flat cache도 재사용한다.
        if let Some(cached) = self.cached_search(store, &relative, &flat_path)? {
            return Ok(cached);
        }
        let _store = self.acquire_store_lock(store)?;
        if let Some(cached) = self.cached_search(store, &relative, &result_path)? {
            return Ok(cached);
        }
        let gate = StoreGate::with_clock(&self.cache, store, self.hooks.clock()).with_cancel(Arc::new(is_cancelled));
        match gate.before_request() {
            Ok(()) => {}
            Err(GateError::Cancelled) => return Err(CoverError::cancelled()),
            Err(GateError::Blocked(blocked)) => return Ok(self.blocked_result(store, &metadata, blocked.retry_after)),
        }
        let error_result = |metadata: &Value| {
            json!({
                "status": "error",
                "error": "서점에 접근하지 못했습니다. 잠시 후 다시 시도하세요",
                "search_url": build_search_url(store, str_field(metadata, "title")),
                "candidates": [],
            })
        };
        let result = match self.search(store, &metadata) {
            Ok(mut result) => {
                gate.record_success();
                match self.store_candidates(store, &relative, &cache_key, &book_cache, &metadata, &mut result) {
                    Ok(()) => result,
                    Err(err) => {
                        log::warn!("Cover candidate search failed ({store}): {err}");
                        error_result(&metadata)
                    }
                }
            }
            Err(SearchError::Cancelled) => return Err(CoverError::cancelled()),
            Err(err) if blocked_status(&err) => {
                let retry_after = gate.record_blocked();
                let status = if let SearchError::Http { status } = err { status } else { 0 };
                log::warn!("Cover store rate-limited ({store}): HTTP {status}, pausing {}s", retry_after.trunc());
                return Ok(self.blocked_result(store, &metadata, retry_after));
            }
            Err(err) => {
                match &err {
                    SearchError::Http { status } => log::warn!("Cover candidate search failed ({store}): HTTPError HTTP {status}"),
                    SearchError::Value(_) => log::warn!("Cover candidate search failed ({store}): ValueError"),
                    _ => log::warn!("Cover candidate search failed ({store}): RequestException"),
                }
                error_result(&metadata)
            }
        };
        self.try_write(&result_path, &serde_json::to_vec(&result).map_err(|e| CoverError::io(&e.to_string()))?)?;
        Ok(result)
    }

    /// 검색 후보마다 기록 파일을 쓰고 응답에는 `candidate_id`만 남긴다(`image_url`은 서버만 안다).
    fn store_candidates(&self, store: &str, relative: &str, cache_key: &str, book_cache: &Path, metadata: &Value, result: &mut Value) -> Result<(), CoverError> {
        let Some(list) = result.get_mut("candidates").and_then(Value::as_array_mut) else { return Ok(()) };
        for candidate in list.iter_mut() {
            let image_url = candidate.get("image_url").and_then(Value::as_str).ok_or_else(|| CoverError::new("후보에 image_url이 없습니다"))?.to_string();
            let suffix = &sha256_hex(&format!("{cache_key}:{image_url}"))[..32];
            let candidate_id = format!("{}{suffix}", book_key(relative));
            let mut record = candidate.clone();
            merge(&mut record, &json!({"store": store, "file_path": relative, "revision": str_field(metadata, "revision")}));
            self.try_write(&book_cache.join(format!("candidate-{candidate_id}.json")), &serde_json::to_vec(&record).map_err(|e| CoverError::io(&e.to_string()))?)?;
            candidate["candidate_id"] = json!(candidate_id);
            if let Some(object) = candidate.as_object_mut() {
                object.remove("image_url");
            }
        }
        Ok(())
    }

    /// 후보 이미지를 받아 캐시에 둔다. 이미 있으면 그대로 돌려준다.
    pub fn image(&self, candidate_id: &str) -> Result<Vec<u8>, CoverError> {
        let candidate = self.candidate(candidate_id)?;
        let _book = self.book_lock(str_field(&candidate, "file_path"), false)?;
        let mut candidate = self.candidate(candidate_id)?;
        if self.revision_of(&self.resolve(str_field(&candidate, "file_path"), false)?) != str_field(&candidate, "revision") {
            return Err(CoverError::new(BOOK_CHANGED));
        }
        let image_path = self.image_file(candidate_id)?;
        if image_path.exists() {
            return self.image_info(candidate_id, &mut candidate, std::fs::read(&image_path)?);
        }
        let store = str_field(&candidate, "store").to_string();
        let _store = self.acquire_store_lock(&store)?;
        if image_path.exists() {
            return self.image_info(candidate_id, &mut candidate, std::fs::read(&image_path)?);
        }
        let raw = self.fetch_store_bytes(&store, str_field(&candidate, "image_url"))?;
        let data = epub_cover::normalize_image_with(&raw, None, self.epub_hooks())?;
        self.try_write(&image_path, &data)?;
        self.image_info(candidate_id, &mut candidate, data)
    }

    // ---- 캐시 정리 ----

    fn delete_book_cache(&self, relative: &str, expected_revision: &str) -> std::io::Result<()> {
        let book_cache = self.book_cache(relative);
        if let Some(f) = &self.hooks.delete_book_cache {
            return f(&book_cache);
        }
        if book_cache.exists() {
            std::fs::remove_dir_all(&book_cache)?;
        }
        // 이전 flat cache의 모든 서점·revision 후보도 해당 책만 정리한다.
        let mut revisions: BTreeSet<String> = BTreeSet::from([expected_revision.to_string()]);
        let mut aliases: BTreeSet<String> = BTreeSet::from([relative.to_string()]);
        if let Ok(entries) = std::fs::read_dir(&self.cache) {
            let mut names: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            names.sort();
            for path in names {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !(name.starts_with("candidate-") && name.ends_with(".json")) {
                    continue;
                }
                let Some(candidate) = self.read_json(&path).filter(has_entries) else { continue };
                if norm_rel(str_field(&candidate, "file_path")) != norm_rel(relative) {
                    continue;
                }
                revisions.insert(str_field(&candidate, "revision").to_string());
                aliases.insert(str_field(&candidate, "file_path").to_string());
                let candidate_id = name.trim_start_matches("candidate-").trim_end_matches(".json");
                remove_if_exists(&self.cache.join(format!("image-{candidate_id}.jpg")))?;
                remove_if_exists(&path)?;
            }
        }
        for store in STORES {
            for alias in &aliases {
                for old_revision in &revisions {
                    remove_if_exists(&self.cache.join(format!("search-{}.json", sha256_hex(&format!("{store}:{alias}:{old_revision}")))))?;
                }
            }
        }
        Ok(())
    }

    /// mtime을 보존하는 표지 편집 뒤 해당 책 ID의 썸네일과 미리보기를 지운다.
    pub fn invalidate_view_cache(&self, book_id: u64) {
        for directory in [".cover_cache", ".preview_cache"] {
            let Ok(entries) = std::fs::read_dir(self.root.join(directory)) else { continue };
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().into_owned();
                let owned = name.starts_with(&format!("{book_id}.")) || name.starts_with(&format!("{book_id}_"));
                if owned && entry.path().is_file() {
                    let _ = remove_if_exists(&entry.path());
                }
            }
        }
    }

    // ---- 저장 ----

    pub fn apply(&self, relative: &str, candidate_id: &str, expected_revision: &str) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _lock = self.book_lock(&relative, true)?;
        let candidate = self.candidate(candidate_id)?;
        if norm_rel(str_field(&candidate, "file_path")) != norm_rel(&relative) || str_field(&candidate, "revision") != expected_revision {
            return Err(CoverError::new("표지 후보와 EPUB이 일치하지 않습니다. 목록을 새로 불러오세요"));
        }
        let image_path = self.image_file(candidate_id)?;
        if !image_path.is_file() {
            return Err(CoverError::new("표지 이미지를 먼저 확인하세요"));
        }
        let data = std::fs::read(&image_path)?;
        self.save_cover(&path, &relative, &data, expected_revision, false, Backup::default())
    }

    pub fn apply_url(&self, relative: &str, url: &str, expected_revision: &str, replace_existing: bool) -> Result<Value, CoverError> {
        self.apply_url_backup(relative, url, expected_revision, replace_existing, Backup::default())
    }

    pub fn apply_url_backup(&self, relative: &str, url: &str, expected_revision: &str, replace_existing: bool, backup: Backup<'_>) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _lock = self.book_lock(&relative, true)?;
        let metadata = self.inspect(&path)?;
        if str_field(&metadata, "revision") != expected_revision {
            return Err(CoverError::new(BOOK_CHANGED));
        }
        if truthy(metadata.get("has_cover")) && !replace_existing {
            return Err(CoverError::new(ALREADY_COVERED));
        }
        let data = self.download_image(url)?;
        self.save_cover(&path, &relative, &data, expected_revision, replace_existing, backup)
    }

    /// 공개 이미지 URL을 받는다. 서점 이미지 주소면 그 서점의 잠금 안에서 받는다.
    fn download_image(&self, url: &str) -> Result<Vec<u8>, CoverError> {
        let host = host_of(url);
        let store = STORES.iter().find(|name| store_hosts(name).iter().any(|domain| host == *domain || host.ends_with(&format!(".{domain}"))));
        let _guard = match store {
            Some(store) => Some(self.acquire_store_lock(store)?),
            None => None,
        };
        let fetcher: Arc<dyn ImageFetcher> = self.hooks.image_fetcher.clone().unwrap_or_else(|| Arc::new(RealImageFetcher));
        Ok(fetcher.fetch_url_image(url)?)
    }

    pub fn insert_illustration(&self, relative: &str, expected_revision: &str, before_idref: &str, image_data: Option<&str>) -> Result<Value, CoverError> {
        self.insert_illustration_backup(relative, expected_revision, before_idref, None, image_data, Backup::default())
    }

    /// `before_idref` 페이지 앞에 URL 또는 올린 이미지(base64)를 삽화로 넣는다. 표지 저장과 같은 잠금·백업·캐시 무효화를 쓴다.
    pub fn insert_illustration_backup(
        &self,
        relative: &str,
        expected_revision: &str,
        before_idref: &str,
        url: Option<&str>,
        image_data: Option<&str>,
        backup: Backup<'_>,
    ) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let decoded = match image_data.filter(|d| !d.is_empty()) {
            Some(data) => Some(decode_base64(data).map_err(|_| CoverError::new("삽화 이미지 데이터를 읽을 수 없습니다"))?),
            None => None,
        };
        let _lock = self.book_lock(&relative, true)?;
        let data = match decoded {
            Some(data) => data,
            None => self.download_image(url.unwrap_or(""))?,
        };
        let result = epub_cover::add_illustration_backup_with(&path, &data, expected_revision, &self.backup_dir(backup), before_idref, backup.name, self.epub_hooks())?;
        self.finish_save(&relative, result, expected_revision)
    }

    /// 쪽 이미지를 한 번에 크롭·분할·삭제·삽입한다. `inserts`는 `[{before_idref, url | image_data(base64)}]`다.
    pub fn edit_images(&self, relative: &str, expected_revision: &str, edits: epub_cover::EditImages, inserts: &[Value], backup: Backup<'_>) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let mut decoded: Vec<(String, Option<Vec<u8>>, String)> = Vec::new();
        for entry in inserts {
            let mut data = None;
            if let Some(encoded) = entry.get("image_data").and_then(Value::as_str).filter(|d| !d.is_empty()) {
                data = Some(decode_base64(encoded).map_err(|_| CoverError::new("삽화 이미지 데이터를 읽을 수 없습니다"))?);
            }
            let url = str_field(entry, "url").trim().to_string();
            if data.is_none() && url.is_empty() {
                return Err(CoverError::new("이미지 URL이 비어 있습니다"));
            }
            decoded.push((str_field(entry, "before_idref").to_string(), data, url));
        }
        let _lock = self.book_lock(&relative, true)?;
        let mut images: Vec<(String, Vec<u8>)> = Vec::new();
        for (before, data, url) in decoded {
            let bytes = match data {
                Some(data) => data,
                None => self.download_image(&url)?,
            };
            images.push((before, bytes));
        }
        let args = epub_cover::EditImages { inserts: images, backup_name: backup.name.map(str::to_string), ..edits };
        let result = epub_cover::edit_images_with(&path, expected_revision, &self.backup_dir(backup), args, self.epub_hooks())?;
        self.finish_save(&relative, result, expected_revision)
    }

    pub fn apply_image(&self, relative: &str, image_data: &str, expected_revision: &str) -> Result<Value, CoverError> {
        self.apply_image_backup(relative, image_data, expected_revision, Backup::default())
    }

    pub fn apply_image_backup(&self, relative: &str, image_data: &str, expected_revision: &str, backup: Backup<'_>) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let data = decode_base64(image_data).map_err(|_| CoverError::new("표지 이미지 데이터를 읽을 수 없습니다"))?;
        let _lock = self.book_lock(&relative, true)?;
        self.save_cover(&path, &relative, &data, expected_revision, true, backup)
    }

    pub fn clear_cache(&self, relative: &str, expected_revision: &str) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _lock = self.book_lock(&relative, true)?;
        if self.revision_of(&path) != expected_revision {
            return Err(CoverError::new(BOOK_CHANGED));
        }
        self.delete_book_cache(&relative, expected_revision)?;
        Ok(json!({"cleared": true, "file_path": relative}))
    }

    fn save_cover(&self, path: &Path, relative: &str, image: &[u8], expected_revision: &str, replace_existing: bool, backup: Backup<'_>) -> Result<Value, CoverError> {
        let result = epub_cover::add_cover_backup_with(path, image, expected_revision, &self.backup_dir(backup), replace_existing, backup.name, self.epub_hooks())?;
        self.finish_save(relative, result, expected_revision)
    }

    /// 이 도구가 넣은 표지를 EPUB에서 빼고, 저장 때와 같이 캐시를 비운다.
    pub fn remove(&self, relative: &str, expected_revision: &str, backup: Backup<'_>) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _lock = self.book_lock(&relative, true)?;
        let result = epub_cover::remove_cover_backup_with(&path, expected_revision, &self.backup_dir(backup), backup.name, self.epub_hooks())?;
        self.finish_save(&relative, result, expected_revision)
    }

    fn finish_save(&self, relative: &str, mut result: Value, expected_revision: &str) -> Result<Value, CoverError> {
        if let Err(err) = self.delete_book_cache(relative, expected_revision) {
            log::warn!("Cover saved; candidate cache cleanup failed: {:?}", err.kind());
            result["warning"] = json!("EPUB 표지를 저장했습니다. 후보 캐시를 삭제하지 못했습니다");
        }
        let book_id = result.get("book_id").and_then(Value::as_u64).unwrap_or(0);
        self.invalidate_view_cache(book_id);
        let backup_path = Path::new(str_field(&result, "backup_path"));
        let backup_relative = backup_path.strip_prefix(&self.root).map_err(|_| CoverError::io("백업 경로가 책 디렉토리 밖입니다"))?.to_string_lossy().into_owned();
        let mut response = Map::new();
        response.insert("file_path".to_string(), json!(relative));
        for key in ["revision", "book_id", "file_size", "cover_image"] {
            response.insert(key.to_string(), result.get(key).cloned().unwrap_or(Value::Null));
        }
        response.insert("backup_path".to_string(), json!(backup_relative));
        for key in ["warning", "idref", "idrefs"] {
            if truthy(result.get(key)) {
                response.insert(key.to_string(), result[key].clone());
            }
        }
        Ok(Value::Object(response))
    }

    // ---- 미리보기 ----

    pub fn preview(&self, relative: &str, original: bool) -> Result<Vec<u8>, CoverError> {
        self.preview_width(relative, original, None)
    }

    /// 표지 미리보기 JPEG. `original`이면 EPUB에 저장한 600px 표지를, 아니면 `width`(150/300)에 맞춘 썸네일을 돌려준다.
    pub fn preview_width(&self, relative: &str, original: bool, width: Option<u32>) -> Result<Vec<u8>, CoverError> {
        let path = self.resolve(relative, false)?;
        // 저장 중인 파일을 읽지 않도록 표지 검사에서 공유 잠금을 확인한다.
        let info = self.inspect(&path)?;
        if !truthy(info.get("has_cover")) {
            return Err(CoverError::new("EPUB에 표지가 없습니다"));
        }
        // 검사가 표지 이미지를 읽지 못했다고 보고하면(크기 상한 초과 등) 썸네일도 만들지 않는다.
        if truthy(info.get("cover_error")) {
            return Err(CoverError::new(UNREADABLE_COVER));
        }
        if original {
            return self.original_cover(&path);
        }
        let thumbnail = match &self.hooks.thumbnailer {
            Some(thumbnailer) => thumbnailer.extract_cover_thumbnail(&path, "epub", width),
            None => epub_cover::extract_epub_thumbnail_with(&path, width, self.epub_hooks()),
        };
        thumbnail.ok_or_else(|| CoverError::new(UNREADABLE_COVER))
    }

    fn original_cover(&self, path: &Path) -> Result<Vec<u8>, CoverError> {
        let unreadable = || CoverError::new(UNREADABLE_COVER);
        let file = File::open(path)?;
        if let Err(err) = self.epub_hooks().lock_shared(&file) {
            return Err(if err.kind() == std::io::ErrorKind::WouldBlock { CoverError::new("이 EPUB은 현재 저장 중입니다") } else { err.into() });
        }
        let mut archive = ZipArchive::new(&file).map_err(|_| unreadable())?;
        let member = epub_cover::find_epub_cover_member(&mut archive).ok_or_else(unreadable)?;
        let limit = self.hooks.max_image_bytes.unwrap_or(MAX_IMAGE_BYTES) as u64;
        let size = epub_cover::entry_size(&mut archive, &member).map_err(|_| unreadable())?;
        if size > limit {
            return Err(unreadable());
        }
        let data = epub_cover::read_entry(&mut archive, &member).map_err(|_| unreadable())?;
        epub_cover::normalize_image_with(&data, Some(600), self.epub_hooks())
    }
}

fn has_entries(value: &Value) -> bool {
    value.as_object().is_some_and(|o| !o.is_empty())
}

fn merge(target: &mut Value, extra: &Value) {
    if let (Some(target), Some(extra)) = (target.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

fn file_inode(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).map(|m| m.ino()).unwrap_or(0)
}

/// 기본 썸네일 추출기. 표지 EPUB 이미지를 150/300px JPEG로 줄인다.
pub struct EpubThumbnailer;

impl Thumbnailer for EpubThumbnailer {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        if file_type == "epub" { epub_cover::extract_epub_thumbnail(path, width) } else { None }
    }
}

// ---------------------------------------------------------------------------
// 요청 모델
// ---------------------------------------------------------------------------

/// 요청 본문 검증 오류. `Display`는 pydantic의 오류 종류(`string_too_long` 등)를 담는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub field: &'static str,
    pub kind: &'static str,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} ({})", self.field, self.message, self.kind)
    }
}

impl std::error::Error for ValidationError {}

fn check_length(field: &'static str, value: &str, min: usize, max: usize) -> Result<(), ValidationError> {
    let count = value.chars().count();
    if count < min {
        return Err(ValidationError { field, kind: "string_too_short", message: format!("String should have at least {min} characters") });
    }
    if count > max {
        return Err(ValidationError { field, kind: "string_too_long", message: format!("String should have at most {max} characters") });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RawBookCover")]
pub struct BookCoverModel {
    pub file_path: String,
    pub revision: String,
}

#[derive(serde::Deserialize)]
struct RawBookCover {
    file_path: String,
    revision: String,
}

impl BookCoverModel {
    pub fn new(file_path: &str, revision: &str) -> Result<Self, ValidationError> {
        check_length("file_path", file_path, 1, 4096)?;
        check_length("revision", revision, 1, 200)?;
        Ok(Self { file_path: file_path.to_string(), revision: revision.to_string() })
    }
}

impl TryFrom<RawBookCover> for BookCoverModel {
    type Error = ValidationError;

    fn try_from(raw: RawBookCover) -> Result<Self, Self::Error> {
        Self::new(&raw.file_path, &raw.revision)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RawApplyCover")]
pub struct ApplyCoverModel {
    pub file_path: String,
    pub revision: String,
    pub candidate_id: String,
}

#[derive(serde::Deserialize)]
struct RawApplyCover {
    file_path: String,
    revision: String,
    candidate_id: String,
}

impl ApplyCoverModel {
    pub fn new(file_path: &str, revision: &str, candidate_id: &str) -> Result<Self, ValidationError> {
        BookCoverModel::new(file_path, revision)?;
        let valid = candidate_id.len() == 64 && candidate_id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid {
            return Err(ValidationError { field: "candidate_id", kind: "string_pattern_mismatch", message: "String should match pattern '^[a-f0-9]{64}$'".to_string() });
        }
        Ok(Self { file_path: file_path.to_string(), revision: revision.to_string(), candidate_id: candidate_id.to_string() })
    }
}

impl TryFrom<RawApplyCover> for ApplyCoverModel {
    type Error = ValidationError;

    fn try_from(raw: RawApplyCover) -> Result<Self, Self::Error> {
        Self::new(&raw.file_path, &raw.revision, &raw.candidate_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RawApplyUrl")]
pub struct ApplyUrlModel {
    pub file_path: String,
    pub revision: String,
    pub url: String,
    pub replace_existing: bool,
}

#[derive(serde::Deserialize)]
struct RawApplyUrl {
    file_path: String,
    revision: String,
    url: String,
    #[serde(default)]
    replace_existing: bool,
}

impl ApplyUrlModel {
    pub fn new(file_path: &str, revision: &str, url: &str, replace_existing: bool) -> Result<Self, ValidationError> {
        BookCoverModel::new(file_path, revision)?;
        check_length("url", url, 1, 4096)?;
        Ok(Self { file_path: file_path.to_string(), revision: revision.to_string(), url: url.to_string(), replace_existing })
    }
}

impl TryFrom<RawApplyUrl> for ApplyUrlModel {
    type Error = ValidationError;

    fn try_from(raw: RawApplyUrl) -> Result<Self, Self::Error> {
        Self::new(&raw.file_path, &raw.revision, &raw.url, raw.replace_existing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RawApplyImage")]
pub struct ApplyImageModel {
    pub file_path: String,
    pub revision: String,
    pub image_data: String,
}

#[derive(serde::Deserialize)]
struct RawApplyImage {
    file_path: String,
    revision: String,
    image_data: String,
}

impl ApplyImageModel {
    /// 디코딩하기 전에 base64 문자열 길이를 확인한다(15MB 이미지의 base64 길이가 상한이다).
    pub fn new(file_path: &str, revision: &str, image_data: &str) -> Result<Self, ValidationError> {
        BookCoverModel::new(file_path, revision)?;
        check_length("image_data", image_data, 1, MAX_IMAGE_BYTES.div_ceil(3) * 4)?;
        Ok(Self { file_path: file_path.to_string(), revision: revision.to_string(), image_data: image_data.to_string() })
    }
}

impl TryFrom<RawApplyImage> for ApplyImageModel {
    type Error = ValidationError;

    fn try_from(raw: RawApplyImage) -> Result<Self, Self::Error> {
        Self::new(&raw.file_path, &raw.revision, &raw.image_data)
    }
}
