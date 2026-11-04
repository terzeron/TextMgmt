//! 관리자용 EPUB 표지 목록, 후보, 이미지 cache, 저장 API.
//!
//! - 서비스(`CoverManagement`)는 파일 시스템 위에서 동기로 동작한다. 라우트(`routes`)가 블로킹 작업으로 넘겨 부른다.
//! - 검색·내려받기·시계·썸네일은 `CoverHooks`로 바꾼다. 기본값은 실제 구현이다. 설계: docs/rust-test-seams.md 3.7, 3.8, 3.12.
//! - 취소(`check_cancelled`)는 요청을 처리하는 스레드에 붙는다(`with_cancel_scope`).

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
use file_record::ignore_rules::IGNORED_DIR_NAMES;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sysdeps::{Clock, RealFs, SystemClock};
use zip::ZipArchive;

use crate::cover_sources::{CancelFn, GateError, STORES, SearchError, SearchHooks, StoreGate, blocked_status, build_search_url, search_candidates_with, store_hosts, store_lock_with};
use crate::direct_cover_url::{ImageFetcher, RealImageFetcher};
use crate::epub_cover::{self, CoverError, EpubCoverHooks, MAX_IMAGE_BYTES};
use htmlkit::url_parser::{Netloc, urlsplit};

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
    // 엄격한 base64: 알파벳 밖 글자와 틀린 padding은 거절하고 남는 비트는 받는다.
    GeneralPurpose::new(&STANDARD_ALPHABET, GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::RequireCanonical).with_decode_allow_trailing_bits(true))
}

fn decode_base64(data: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64_engine().decode(data)
}

fn now_seconds() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

use crate::has_content;

fn is_epub_name(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("epub"))
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
        children.sort_by_key(|item| (str_field(item, "label").to_lowercase(), str_field(item, "label").to_string()));
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
        files.sort_by_key(|(name, _)| (name.to_lowercase(), name.clone()));
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
                    let has_cover = metadata.get("has_cover").and_then(Value::as_bool) == Some(true);
                    if has_cover != covered && !has_content(metadata.get("cover_error")) {
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

    // ---- 미리보기 ----

    pub fn preview(&self, relative: &str, original: bool) -> Result<Vec<u8>, CoverError> {
        self.preview_width(relative, original, None)
    }

    /// 표지 미리보기 JPEG. `original`이면 EPUB에 저장한 600px 표지를, 아니면 `width`(150/300)에 맞춘 썸네일을 돌려준다.
    pub fn preview_width(&self, relative: &str, original: bool, width: Option<u32>) -> Result<Vec<u8>, CoverError> {
        let path = self.resolve(relative, false)?;
        // 저장 중인 파일을 읽지 않도록 표지 검사에서 공유 잠금을 확인한다.
        let info = self.inspect(&path)?;
        if info.get("has_cover").and_then(Value::as_bool) != Some(true) {
            return Err(CoverError::new("EPUB에 표지가 없습니다"));
        }
        // 검사가 표지 이미지를 읽지 못했다고 보고하면(크기 상한 초과 등) 썸네일도 만들지 않는다.
        if has_content(info.get("cover_error")) {
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

/// 책 파일에서 표지 썸네일(JPEG 바이트)을 만든다. `book_manager`와 표지 관리가 함께 쓴다.
pub trait Thumbnailer: Send + Sync {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>>;
}

/// 기본 썸네일 추출기. 표지 EPUB 이미지를 150/300px JPEG로 줄인다.
pub struct EpubThumbnailer;

impl Thumbnailer for EpubThumbnailer {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        if file_type == "epub" { epub_cover::extract_epub_thumbnail(path, width) } else { None }
    }
}

mod cover_candidate_finder;
mod cover_request_models;
mod cover_writer;

pub use cover_request_models::{ApplyCoverModel, ApplyImageModel, ApplyUrlModel, BookCoverModel, ValidationError};

// ---- 파일 이름으로 표지 검색어를 만드는 도우미. 유니코드 처리를 외부 crate 없이 한다. ----
//
// NFC로 정규화한 뒤 일반 범주가 P(구두점)나 S(기호)인 글자를 공백으로 바꾸는 것이 목표다.
// 이 crate 의존성에는 유니코드 표가 없어서 다음과 같이 근사한다.
// - NFC: 한글 자모(L+V(+T))만 음절로 합친다(macOS가 만든 NFD 이름이 대표적이다). 라틴 결합 문자는 합치지 않는다.
// - P/S: 글자·숫자·공백·제어 문자·결합 부호가 아니면 구두점·기호로 본다.
fn compose_hangul(text: &str) -> String {
    const S_BASE: u32 = 0xAC00;
    const L_BASE: u32 = 0x1100;
    const V_BASE: u32 = 0x1161;
    const T_BASE: u32 = 0x11A7;
    const L_COUNT: u32 = 19;
    const V_COUNT: u32 = 21;
    const T_COUNT: u32 = 28;
    let mut out = String::with_capacity(text.len());
    let mut last: Option<u32> = None;
    let flush = |out: &mut String, last: &mut Option<u32>| {
        if let Some(code) = last.take()
            && let Some(c) = char::from_u32(code)
        {
            out.push(c);
        }
    };
    for c in text.chars() {
        let code = c as u32;
        if let Some(prev) = last {
            // L + V
            if (L_BASE..L_BASE + L_COUNT).contains(&prev) && (V_BASE..V_BASE + V_COUNT).contains(&code) {
                last = Some(S_BASE + ((prev - L_BASE) * V_COUNT + (code - V_BASE)) * T_COUNT);
                continue;
            }
            // LV + T
            if (S_BASE..S_BASE + L_COUNT * V_COUNT * T_COUNT).contains(&prev) && (prev - S_BASE).is_multiple_of(T_COUNT) && (T_BASE + 1..T_BASE + T_COUNT).contains(&code) {
                last = Some(prev + (code - T_BASE));
                continue;
            }
        }
        flush(&mut out, &mut last);
        last = Some(code);
    }
    flush(&mut out, &mut last);
    out
}

fn is_combining_mark(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x064B..=0x065F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0x302A..=0x302F | 0x3099..=0x309A | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F)
}

fn is_punctuation_or_symbol(c: char) -> bool {
    !(c.is_alphanumeric() || c.is_whitespace() || c.is_control() || is_combining_mark(c))
}

/// 파일 이름(확장자 제외)을 검색어로 쓴다. 구두점과 기호는 공백으로, 연속 공백은 하나로 줄인다. 비면 "제목 없음"이다.
pub fn clean_title(stem: &str) -> String {
    let replaced: String = compose_hangul(stem).chars().map(|c| if is_punctuation_or_symbol(c) { ' ' } else { c }).collect();
    let title = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() { "제목 없음".to_string() } else { title }
}
