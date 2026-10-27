//! EPUB 표지 검사와 inode를 보존하는 표지 저장. Python `backend/epub_cover.py`에 대응한다.
//!
//! 설계: docs/rust-test-seams.md 3.5, 3.7, 3.11, 3.12.
//! - 한도(`MAX_*`)와 `fsync`/`flock`, 개정 번호, 무결성 검사는 `EpubCoverHooks`로 바꾼다. 기본값은 운영 값이다.
//! - `lxml`은 `xml`(고칠 수 있는 작은 트리), PIL은 `image`, `zipfile`은 `zip`으로 옮겼다.
//! - 쓰기는 `stage`(OPF·XHTML 편집), `rewrite`(잠금·백업·제자리 쓰기·되돌리기), `edit`(쪽 이미지 편집)에 나눴다.
//!
//! Python과 달라진 점:
//! - JPEG 부분 디코딩(`draft`)이 없어 `max_image_pixels`를 원본 크기로 확인한다. 큰 JPEG는 Python보다 일찍 거절한다.
//! - XML 문자 인코딩은 UTF-8과 UTF-16 BOM만 읽는다(`encoding=` 선언을 따르지 않는다).
//! - 목록 검사에서 `flock(LOCK_SH)`는 실제 파일 시스템이면 공유 잠금이고, `with_fs`로 대역을 넣으면 `FsOps::lock_exclusive`를 쓴다(공유 잠금 seam이 없다).

use std::fmt;
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use sysdeps::FsOps;
use zip::ZipArchive;

pub mod edit;
pub mod pathx;
mod rewrite;
mod stage;
pub mod xml;

pub use edit::{EditImages, edit_images, edit_images_with, edit_page_images};
pub use pathx::member_path;
pub use rewrite::{add_cover, add_cover_backup_with, add_cover_with, add_illustration, add_illustration_backup_with, add_illustration_with, remove_cover, remove_cover_backup_with, remove_cover_with};

use xml::Element;

pub const OPF_NS: &str = "http://www.idpf.org/2007/opf";
pub const MAX_XML_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EPUB_BYTES: usize = 1024 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 15 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: usize = 80_000_000;
/// ZIP 항목 수 상한(Python `_read_package`).
pub const MAX_ENTRIES: usize = 20_000;

// ---------------------------------------------------------------------------
// 오류
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// 사용자에게 보여 줄 수 있는 오류(Python `CoverError`).
    Message,
    /// 운영체제 오류(Python `OSError`).
    Io,
    /// 화면에서 더 필요하지 않은 요청(Python `FetchCancelled`).
    Cancelled,
}

/// 표지 처리 오류. `Display`는 사용자에게 보여 줄 문장이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverError {
    kind: Kind,
    message: String,
    io_kind: Option<std::io::ErrorKind>,
}

impl CoverError {
    pub fn new(message: &str) -> Self {
        CoverError { kind: Kind::Message, message: message.to_string(), io_kind: None }
    }

    pub fn io(message: &str) -> Self {
        CoverError { kind: Kind::Io, message: message.to_string(), io_kind: None }
    }

    pub fn cancelled() -> Self {
        CoverError { kind: Kind::Cancelled, message: "표지 요청을 취소했습니다".to_string(), io_kind: None }
    }

    pub fn is_cancelled(&self) -> bool {
        self.kind == Kind::Cancelled
    }

    /// 운영체제 오류인가. 라우트는 이 경우 409 대신 503을 돌려준다.
    pub fn is_io(&self) -> bool {
        self.kind == Kind::Io
    }

    /// 운영체제 오류였다면 그 종류(예: `AlreadyExists`). 그 밖에는 `None`이다.
    pub fn io_kind(&self) -> Option<std::io::ErrorKind> {
        self.io_kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CoverError {}

impl From<std::io::Error> for CoverError {
    fn from(err: std::io::Error) -> Self {
        CoverError { io_kind: Some(err.kind()), ..CoverError::io(&err.to_string()) }
    }
}

impl From<crate::direct_cover_url::FetchError> for CoverError {
    fn from(err: crate::direct_cover_url::FetchError) -> Self {
        use crate::direct_cover_url::FetchError;
        match err {
            FetchError::Value(message) => CoverError::new(&message),
            FetchError::Os(message) | FetchError::Other(message) => CoverError::io(&message),
        }
    }
}

const INSPECT_FAILED: &str = "EPUB 또는 기존 표지를 검사할 수 없습니다";
const SAVING: &str = "이 EPUB은 현재 저장 중입니다";
const CHANGED: &str = "EPUB이 변경되었습니다. 목록을 새로 불러오세요";

/// 내부 오류. Python에서 `CoverError`, `OSError`, 그 밖의 예외(`KeyError`, `BadZipFile`, `XMLSyntaxError` 등)를 가르던 것을 옮겼다.
#[derive(Debug)]
pub(crate) enum Fail {
    Cover(CoverError),
    Io(std::io::Error),
    Lib(String),
}

pub(crate) type FResult<T> = Result<T, Fail>;

impl From<CoverError> for Fail {
    fn from(err: CoverError) -> Self {
        Fail::Cover(err)
    }
}

impl From<std::io::Error> for Fail {
    fn from(err: std::io::Error) -> Self {
        Fail::Io(err)
    }
}

impl From<zip::result::ZipError> for Fail {
    fn from(err: zip::result::ZipError) -> Self {
        match err {
            zip::result::ZipError::Io(io) => Fail::Io(io),
            other => Fail::Lib(other.to_string()),
        }
    }
}

impl From<xml::XmlError> for Fail {
    fn from(err: xml::XmlError) -> Self {
        Fail::Lib(err.to_string())
    }
}

impl Fail {
    pub(crate) fn lib(message: &str) -> Self {
        Fail::Lib(message.to_string())
    }

    /// 저장 경로에서 밖으로 내보낼 때. 사용자 문장은 그대로, 운영체제 오류는 `is_io`, 나머지는 검사 실패다.
    pub(crate) fn into_cover(self) -> CoverError {
        match self {
            Fail::Cover(err) => err,
            Fail::Io(err) => CoverError::from(err),
            Fail::Lib(detail) => {
                log::debug!("EPUB 처리 실패: {detail}");
                CoverError::new(INSPECT_FAILED)
            }
        }
    }

    fn would_block(&self) -> bool {
        matches!(self, Fail::Io(err) if err.kind() == std::io::ErrorKind::WouldBlock)
    }
}

// ---------------------------------------------------------------------------
// 한도와 훅
// ---------------------------------------------------------------------------

/// 검사 한도. 기본값은 운영 값이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_epub_bytes: usize,
    pub max_xml_bytes: usize,
    pub max_image_pixels: usize,
    pub max_image_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_epub_bytes: MAX_EPUB_BYTES, max_xml_bytes: MAX_XML_BYTES, max_image_pixels: MAX_IMAGE_PIXELS, max_image_bytes: MAX_IMAGE_BYTES }
    }
}

type FindOpfPath = Arc<dyn Fn() -> String + Send + Sync>;
type RevisionFn = Arc<dyn Fn(&Path) -> String + Send + Sync>;
pub type TestzipFn = Arc<dyn Fn(&Path) -> Option<String> + Send + Sync>;
type InspectArchiveFn = Arc<dyn Fn(Value) -> Value + Send + Sync>;
type UuidFn = Arc<dyn Fn() -> String + Send + Sync>;
type Observer = Arc<dyn Fn() + Send + Sync>;

/// 바깥 의존성을 바꾸는 지점. 기본값은 실제 구현이다. 설계: docs/rust-test-seams.md 3.12.
#[derive(Clone, Default)]
pub struct EpubCoverHooks {
    limits: Limits,
    fs: Option<Arc<dyn FsOps>>,
    find_opf_path: Option<FindOpfPath>,
    revision: Option<RevisionFn>,
    testzip: Option<TestzipFn>,
    inspect_archive: Option<InspectArchiveFn>,
    uuid_hex: Option<UuidFn>,
    pixel_decode_observer: Option<Observer>,
}

impl EpubCoverHooks {
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// `fsync`와 `flock`을 이 `FsOps`로 한다. 목록 검사의 공유 잠금도 `lock_exclusive`로 대신한다.
    pub fn with_fs(mut self, fs: Arc<dyn FsOps>) -> Self {
        self.fs = Some(fs);
        self
    }

    pub fn with_find_opf_path(mut self, f: Arc<dyn Fn() -> String + Send + Sync>) -> Self {
        self.find_opf_path = Some(f);
        self
    }

    pub fn with_revision(mut self, f: Arc<dyn Fn(&Path) -> String + Send + Sync>) -> Self {
        self.revision = Some(f);
        self
    }

    /// `ZipFile.testzip()` 대신 쓴다. 손상된 멤버 이름을 돌려주면 검증 실패다.
    pub fn with_testzip(mut self, f: TestzipFn) -> Self {
        self.testzip = Some(f);
        self
    }

    /// 압축 파일 검사 결과(`_inspect_archive`)를 바꾼다.
    pub fn with_inspect_archive(mut self, f: Arc<dyn Fn(Value) -> Value + Send + Sync>) -> Self {
        self.inspect_archive = Some(f);
        self
    }

    pub fn with_uuid_hex(mut self, f: Arc<dyn Fn() -> String + Send + Sync>) -> Self {
        self.uuid_hex = Some(f);
        self
    }

    /// 이미지 전체를 디코딩할 때마다 부른다(Python `Image.Image.load` 감시).
    pub fn with_pixel_decode_observer(mut self, f: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.pixel_decode_observer = Some(f);
        self
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub(crate) fn revision_of(&self, path: &Path) -> String {
        match &self.revision {
            Some(f) => f(path),
            None => revision(path),
        }
    }

    pub(crate) fn sync(&self, file: &File) -> std::io::Result<()> {
        match &self.fs {
            Some(fs) => fs.sync_file(file),
            None => file.sync_all(),
        }
    }

    /// `flock(LOCK_EX | LOCK_NB)`. 이미 잠겨 있으면 `WouldBlock`이다.
    pub(crate) fn lock_exclusive(&self, file: &File) -> std::io::Result<()> {
        match &self.fs {
            Some(fs) => fs.lock_exclusive(file, true),
            None => file.try_lock().map_err(from_try_lock),
        }
    }

    /// `flock(LOCK_SH | LOCK_NB)`.
    pub(crate) fn lock_shared(&self, file: &File) -> std::io::Result<()> {
        match &self.fs {
            Some(fs) => fs.lock_exclusive(file, true),
            None => file.try_lock_shared().map_err(from_try_lock),
        }
    }

    pub(crate) fn opf_path_override(&self) -> Option<String> {
        self.find_opf_path.as_ref().map(|f| f())
    }

    pub(crate) fn uuid_hex(&self) -> String {
        match &self.uuid_hex {
            Some(f) => f(),
            None => random_hex(),
        }
    }

    /// `ZipFile.testzip()`: 손상된 첫 멤버 이름. 없으면 `None`.
    pub(crate) fn testzip(&self, path: &Path) -> FResult<Option<String>> {
        if let Some(f) = &self.testzip {
            return Ok(f(path));
        }
        let mut archive = ZipArchive::new(File::open(path)?)?;
        for index in 0..archive.len() {
            let mut entry = match archive.by_index(index) {
                Ok(entry) => entry,
                Err(_) => return Ok(Some(index.to_string())),
            };
            let name = entry.name().map(|n| n.into_owned()).unwrap_or_else(|_| index.to_string());
            if std::io::copy(&mut entry, &mut std::io::sink()).is_err() {
                return Ok(Some(name));
            }
        }
        Ok(None)
    }

    pub(crate) fn finish_inspect(&self, value: Value) -> Value {
        match &self.inspect_archive {
            Some(f) => f(value),
            None => value,
        }
    }

    fn observe_decode(&self) {
        if let Some(f) = &self.pixel_decode_observer {
            f();
        }
    }
}

fn from_try_lock(err: std::fs::TryLockError) -> std::io::Error {
    match err {
        std::fs::TryLockError::WouldBlock => std::io::ErrorKind::WouldBlock.into(),
        std::fs::TryLockError::Error(io) => io,
    }
}

/// `uuid.uuid4().hex`처럼 보이는 32자리 16진수. 운영체제 난수를 쓴다.
pub(crate) fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    let filled = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_ok();
    if !filled {
        use sha2::{Digest, Sha256};
        use std::time::{SystemTime, UNIX_EPOCH};
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let seed = format!("{now}:{}:{}", std::process::id(), COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        bytes.copy_from_slice(&Sha256::digest(seed.as_bytes())[..16]);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// 파일 개정 번호
// ---------------------------------------------------------------------------

/// `inode:크기:mtime_ns:ctime_ns`. 파일이 바뀌었는지 확인하는 값이다. 읽을 수 없으면 빈 문자열이다.
pub fn revision(path: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    match std::fs::metadata(path) {
        Ok(m) => format!("{}:{}:{}:{}", m.ino(), m.size(), m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128, m.ctime() as i128 * 1_000_000_000 + m.ctime_nsec() as i128),
        Err(_) => String::new(),
    }
}

/// ZIP 항목 수 상한. Python `_read_package`의 첫 검사.
pub fn check_entry_count(count: usize) -> Result<(), CoverError> {
    if count > MAX_ENTRIES { Err(CoverError::new("EPUB이 검사 크기 제한을 초과했습니다")) } else { Ok(()) }
}

// ---------------------------------------------------------------------------
// ZIP 도우미
// ---------------------------------------------------------------------------

pub(crate) fn has_entry<R: Read + Seek>(archive: &ZipArchive<R>, name: &str) -> bool {
    archive.index_for_name(name).is_some()
}

pub(crate) fn entry_names<R: Read + Seek>(archive: &ZipArchive<R>) -> Vec<String> {
    (0..archive.len()).filter_map(|i| archive.name_for_index(i)).filter_map(|n| n.ok()).map(|n| n.into_owned()).collect()
}

/// `archive.getinfo(name).file_size`. 없으면 `KeyError`다.
pub(crate) fn entry_size<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> FResult<u64> {
    let index = archive.index_for_name(name).ok_or_else(|| Fail::Lib(format!("KeyError: {name}")))?;
    Ok(archive.by_index_raw(index)?.size())
}

/// `archive.read(name)`. 헤더가 밝힌 크기보다 많이 풀리면 거절한다(zip bomb).
pub(crate) fn read_entry<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> FResult<Vec<u8>> {
    let index = archive.index_for_name(name).ok_or_else(|| Fail::Lib(format!("KeyError: {name}")))?;
    let mut entry = archive.by_index(index)?;
    let declared = entry.size();
    let mut data = Vec::with_capacity(declared.min(MAX_IMAGE_BYTES as u64 * 2) as usize);
    (&mut entry).take(declared + 1).read_to_end(&mut data)?;
    if data.len() as u64 > declared {
        return Err(Fail::lib("압축을 푼 크기가 헤더와 다릅니다"));
    }
    Ok(data)
}

/// `container.xml`에서 OPF 경로를 찾는다. 안 되면 정규식, 그래도 안 되면 `.opf` 파일 이름을 찾는다. 없으면 빈 문자열이다.
pub fn find_opf_path<R: Read + Seek>(archive: &mut ZipArchive<R>) -> String {
    const CONTAINER_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:container";
    if let Ok(data) = read_entry(archive, "META-INF/container.xml") {
        if let Some(container) = xml::parse_lenient(&data) {
            let found = container.descendants().into_iter().find(|e| e.is(CONTAINER_NS, "rootfile")).map(|e| e.attr_or("full-path", "").to_string()).unwrap_or_default();
            if !found.is_empty() {
                return found;
            }
        }
        static FULL_PATH: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| regex::bytes::Regex::new(r#"full-path=["']([^"']+)"#).expect("정규식"));
        if let Some(m) = FULL_PATH.captures(&data).and_then(|c| c.get(1)) {
            return String::from_utf8_lossy(m.as_bytes()).into_owned();
        }
    }
    entry_names(archive).into_iter().find(|n| n.to_lowercase().ends_with(".opf")).unwrap_or_default()
}

fn resolve_opf_path<R: Read + Seek>(archive: &mut ZipArchive<R>, hooks: &EpubCoverHooks) -> String {
    hooks.opf_path_override().unwrap_or_else(|| find_opf_path(archive))
}

// ---------------------------------------------------------------------------
// 패키지 읽기
// ---------------------------------------------------------------------------

/// OPF 경로와 읽은 `package` 요소.
pub struct Package {
    pub opf_path: String,
    pub root: Element,
}

pub(crate) fn read_package_with<R: Read + Seek>(archive: &mut ZipArchive<R>, hooks: &EpubCoverHooks) -> FResult<(String, Element)> {
    let limits = hooks.limits();
    check_entry_count(archive.len())?;
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        total = total.saturating_add(archive.by_index_raw(index)?.size());
    }
    if total > limits.max_epub_bytes as u64 {
        return Err(CoverError::new("EPUB이 검사 크기 제한을 초과했습니다").into());
    }
    if entry_size(archive, "META-INF/container.xml")? > limits.max_xml_bytes as u64 {
        return Err(CoverError::new("EPUB container가 너무 큽니다").into());
    }
    let opf_path = resolve_opf_path(archive, hooks);
    let unsafe_path = opf_path.is_empty() || opf_path.starts_with('/') || pathx::normpath(&opf_path).starts_with("../");
    if unsafe_path || entry_size(archive, &opf_path)? > limits.max_xml_bytes as u64 {
        return Err(CoverError::new("EPUB package를 찾거나 읽을 수 없습니다").into());
    }
    let data = read_entry(archive, &opf_path)?;
    let package = xml::parse_package(&data)?;
    if package.name != "package" {
        return Err(CoverError::new("올바른 EPUB package가 아닙니다").into());
    }
    Ok((opf_path, package))
}

/// Python `_read_package`.
pub fn read_package<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Package, CoverError> {
    read_package_with(archive, &EpubCoverHooks::default()).map(|(opf_path, root)| Package { opf_path, root }).map_err(Fail::into_cover)
}

// ---------------------------------------------------------------------------
// 이미지
// ---------------------------------------------------------------------------

fn read_dimensions(data: &[u8]) -> Result<(u32, u32), image::ImageError> {
    image::ImageReader::new(Cursor::new(data)).with_guessed_format()?.into_dimensions()
}

/// PIL `Image.thumbnail`이 고르는 크기. 확대하지 않고 비율을 지킨다.
fn thumbnail_size(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if max_width >= width && max_height >= height {
        return (width, height);
    }
    let aspect = f64::from(width) / f64::from(height);
    // 같은 오차면 먼저 나온 값(바닥)을 고른다. Python `min(floor, ceil, key=...)`와 같다.
    let pick = |number: f64, error: &dyn Fn(f64) -> f64| -> u32 {
        let (floor, ceil) = (number.floor(), number.ceil());
        let best = if error(ceil) < error(floor) { ceil } else { floor };
        best.max(1.0) as u32
    };
    let (x, y) = (f64::from(max_width), f64::from(max_height));
    if x / y >= aspect { (pick(y * aspect, &|n| (aspect - n / y).abs()), max_height) } else { (max_width, pick(x / aspect, &|n| if n == 0.0 { 0.0 } else { (aspect - x / n).abs() })) }
}

fn encode_jpeg(image: &image::RgbImage, quality: u8) -> Result<Vec<u8>, image::ImageError> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(image)?;
    Ok(out)
}

/// 이미지를 RGB JPEG(품질 92)로 바꾸고 `max_width`(기본 2000)와 높이 3000 안으로 줄인다.
pub fn normalize_image(data: &[u8], max_width: Option<u32>) -> Result<Vec<u8>, CoverError> {
    normalize_image_with(data, max_width, &EpubCoverHooks::default())
}

pub fn normalize_image_with(data: &[u8], max_width: Option<u32>, hooks: &EpubCoverHooks) -> Result<Vec<u8>, CoverError> {
    let limits = hooks.limits();
    if data.is_empty() || data.len() > limits.max_image_bytes {
        return Err(CoverError::new("표지 이미지가 비어 있거나 너무 큽니다"));
    }
    let unreadable = || CoverError::new("표지 이미지 형식을 읽을 수 없습니다");
    let max_width = max_width.unwrap_or(2000);
    let (width, height) = read_dimensions(data).map_err(|_| unreadable())?;
    if width == 0 || height == 0 {
        return Err(unreadable());
    }
    if (width as usize).saturating_mul(height as usize) > limits.max_image_pixels {
        return Err(CoverError::new("표지 이미지 해상도가 너무 큽니다"));
    }
    hooks.observe_decode();
    let decoded = image::ImageReader::new(Cursor::new(data)).with_guessed_format().map_err(|_| unreadable())?.decode().map_err(|_| unreadable())?;
    let rgb = decoded.to_rgb8();
    let (new_width, new_height) = thumbnail_size(width, height, max_width, 3000);
    let rgb = if (new_width, new_height) == (width, height) { rgb } else { image::imageops::resize(&rgb, new_width, new_height, image::imageops::FilterType::Lanczos3) };
    encode_jpeg(&rgb, 92).map_err(|_| unreadable())
}

// ---------------------------------------------------------------------------
// 표지 멤버 찾기
// ---------------------------------------------------------------------------

/// 표지 이미지 멤버 경로. 명시한 표지 이미지가 없으면 guide나 manifest의 표지 XHTML에서 이미지를 찾는다.
pub fn find_epub_cover_member<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Option<String> {
    find_epub_cover_member_with(archive, &|_| None)
}

/// `file_size`가 값을 돌려주면 ZIP 헤더의 크기 대신 쓴다(Python이 `ZipInfo.file_size`를 덮어쓰는 것에 대응한다).
pub fn find_epub_cover_member_with<R: Read + Seek>(archive: &mut ZipArchive<R>, file_size: &dyn Fn(&str) -> Option<u64>) -> Option<String> {
    find_cover_member_in(archive, &EpubCoverHooks::default(), file_size)
}

pub(crate) fn find_cover_member_in<R: Read + Seek>(archive: &mut ZipArchive<R>, hooks: &EpubCoverHooks, file_size: &dyn Fn(&str) -> Option<u64>) -> Option<String> {
    let opf_path = resolve_opf_path(archive, hooks);
    if opf_path.is_empty() {
        return None;
    }
    let opf = xml::parse_lenient(&read_entry(archive, &opf_path).ok()?)?;
    let (mut image_items, mut page_items, mut cover_pages) = (Vec::new(), Vec::new(), Vec::<String>::new());
    let (mut meta_cover_id, mut first_spine_id) = (String::new(), String::new());
    for element in opf.descendants() {
        let media_type = element.attr_or("media-type", "");
        match element.name.as_str() {
            "item" if media_type.starts_with("image/") => image_items.push(element),
            "item" if media_type == "application/xhtml+xml" => page_items.push(element),
            "meta" if element.attr("name") == Some("cover") => meta_cover_id = element.attr_or("content", "").to_string(),
            "reference" if element.attr_or("type", "").split_whitespace().any(|t| t == "cover") => cover_pages.push(element.attr_or("href", "").to_string()),
            "itemref" if first_spine_id.is_empty() => first_spine_id = element.attr_or("idref", "").to_string(),
            _ => {}
        }
    }
    let id_href = |el: &Element| format!("{} {}", el.attr_or("id", ""), el.attr_or("href", "")).to_lowercase();
    let chosen = image_items
        .iter()
        .find(|el| el.attr_or("properties", "").split_whitespace().any(|p| p == "cover-image"))
        .or_else(|| if meta_cover_id.is_empty() { None } else { image_items.iter().find(|el| el.attr("id") == Some(meta_cover_id.as_str())) })
        .or_else(|| image_items.iter().find(|el| id_href(el).contains("cover")));
    let opf_dir = pathx::dirname(&opf_path);
    if let Some(chosen) = chosen {
        let unquoted = pathx::unquote(chosen.attr_or("href", ""));
        let href = unquoted.split('#').next().unwrap_or("").split('?').next().unwrap_or("");
        return Some(if opf_dir.is_empty() { pathx::normpath(href) } else { pathx::normpath(&pathx::join(&opf_dir, href)) });
    }
    cover_pages.extend(
        page_items.iter().filter(|el| (!meta_cover_id.is_empty() && el.attr("id") == Some(meta_cover_id.as_str())) || id_href(el).contains("cover")).map(|el| el.attr_or("href", "").to_string()),
    );
    // 잘못된 표지 ID는 첫 페이지로 보완하되, 삭제한 표지를 본문 이미지로 대체하지 않는다.
    if !meta_cover_id.is_empty() {
        cover_pages.extend(page_items.iter().filter(|el| !first_spine_id.is_empty() && el.attr("id") == Some(first_spine_id.as_str())).map(|el| el.attr_or("href", "").to_string()));
    }
    let mut seen: Vec<String> = Vec::new();
    for page_href in cover_pages {
        if seen.contains(&page_href) {
            continue;
        }
        seen.push(page_href.clone());
        let Some(page_member) = existing_member_path(archive, &opf_dir, &page_href) else { continue };
        let size = file_size(&page_member).or_else(|| entry_size(archive, &page_member).ok());
        if size.is_none_or(|s| s > MAX_XML_BYTES as u64) {
            continue;
        }
        let Some(page) = read_entry(archive, &page_member).ok().and_then(|data| xml::parse_lenient(&data)) else { continue };
        for el in page.descendants() {
            let href = match el.name.as_str() {
                "img" => el.attr_or("src", "").to_string(),
                "image" => el.attr("href").filter(|h| !h.is_empty()).or_else(|| el.attr_ns(xml::XLINK_NS, "href")).unwrap_or("").to_string(),
                _ => continue,
            };
            if let Some(member) = existing_member_path(archive, &pathx::dirname(&page_member), &href) {
                return Some(member);
            }
        }
    }
    None
}

/// `member_path`에 더해 ZIP에 실제로 있는 멤버만 돌려준다.
fn existing_member_path<R: Read + Seek>(archive: &ZipArchive<R>, base: &str, href: &str) -> Option<String> {
    let member = member_path(base, href);
    (!member.is_empty() && has_entry(archive, &member)).then_some(member)
}

/// EPUB 표지 이미지를 `width`(150 또는 300, 그 밖에는 300) 안에 들어가는 JPEG 썸네일로 만든다. 표지가 없으면 `None`이다.
pub fn extract_epub_thumbnail(path: &Path, width: Option<u32>) -> Option<Vec<u8>> {
    extract_epub_thumbnail_with(path, width, &EpubCoverHooks::default())
}

/// `extract_epub_thumbnail`에 크기 상한을 주입한다. 악의적인 EPUB의 거대한 표지로 메모리가 고갈되지 않게,
/// 읽기 전에 ZIP 항목 크기를, 디코딩 전에 헤더의 화소 수를 상한과 비교한다. 넘으면 `None`이다.
pub fn extract_epub_thumbnail_with(path: &Path, width: Option<u32>, hooks: &EpubCoverHooks) -> Option<Vec<u8>> {
    let width = if matches!(width, Some(150 | 300)) { width.unwrap_or(300) } else { 300 };
    let limits = hooks.limits();
    let mut archive = ZipArchive::new(File::open(path).ok()?).ok()?;
    let member = find_epub_cover_member(&mut archive)?;
    if entry_size(&mut archive, &member).ok()? > limits.max_image_bytes as u64 {
        return None;
    }
    let data = read_entry(&mut archive, &member).ok()?;
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format().ok()?.into_dimensions().ok()?;
    if (w as u64) * (h as u64) > limits.max_image_pixels as u64 {
        return None;
    }
    let decoded = image::load_from_memory(&data).ok()?;
    let rgb = decoded.to_rgb8();
    let (nw, nh) = thumbnail_size(w, h, width, width * 10);
    let rgb = if (nw, nh) == (w, h) { rgb } else { image::imageops::resize(&rgb, nw, nh, image::imageops::FilterType::Lanczos3) };
    encode_jpeg(&rgb, 85).ok()
}

// ---------------------------------------------------------------------------
// 검사
// ---------------------------------------------------------------------------

/// `//*[local-name()="name"]/text()`의 첫 글. 문서 순서로 훑어 글이 있는 첫 요소를 쓴다.
fn first_text(package: &Element, local: &str) -> String {
    for element in package.descendants() {
        if element.name != local {
            continue;
        }
        if let Some(xml::Node::Text(text)) = element.children.iter().find(|n| matches!(n, xml::Node::Text(_))) {
            return text.trim().to_string();
        }
    }
    String::new()
}

/// 기존 표지 이미지의 크기와 형식을 머리글만 읽어 확인한다. 실패하면 `cover_error` 문장이다.
fn inspect_cover_image<R: Read + Seek>(archive: &mut ZipArchive<R>, member: &str, hooks: &EpubCoverHooks) -> Result<Value, String> {
    let limits = hooks.limits();
    let unreadable = || "기존 표지 이미지를 읽을 수 없습니다".to_string();
    let byte_size = entry_size(archive, member).map_err(|_| unreadable())?;
    if byte_size > limits.max_image_bytes as u64 {
        return Err("기존 표지가 검사 크기 제한을 초과했습니다".to_string());
    }
    let mut info = Map::new();
    info.insert("byte_size".to_string(), json!(byte_size));
    if member.to_lowercase().ends_with(".svg") {
        let data = read_entry(archive, member).map_err(|_| unreadable())?;
        xml::parse(&data).map_err(|_| unreadable())?;
        return Ok(Value::Object(info));
    }
    // 목록에서는 머리글만 읽는다. 픽셀 디코딩과 변환은 미리보기·저장에서 검증한다.
    const HEAD: u64 = 512 * 1024;
    let index = archive.index_for_name(member).ok_or_else(unreadable)?;
    let mut head = Vec::new();
    {
        let mut entry = archive.by_index(index).map_err(|_| unreadable())?;
        (&mut entry).take(HEAD).read_to_end(&mut head).map_err(|_| unreadable())?;
    }
    let dimensions = match read_dimensions(&head) {
        Ok(d) => d,
        Err(_) if byte_size > HEAD => read_dimensions(&read_entry(archive, member).map_err(|_| unreadable())?).map_err(|_| unreadable())?,
        Err(_) => return Err(unreadable()),
    };
    if (dimensions.0 as usize).saturating_mul(dimensions.1 as usize) > limits.max_image_pixels {
        return Err("기존 표지 이미지 해상도가 너무 큽니다".to_string());
    }
    info.insert("width".to_string(), json!(dimensions.0));
    info.insert("height".to_string(), json!(dimensions.1));
    Ok(Value::Object(info))
}

/// Python `_inspect_archive`.
pub(crate) fn inspect_archive<R: Read + Seek>(archive: &mut ZipArchive<R>, hooks: &EpubCoverHooks) -> FResult<Value> {
    let (_, package) = read_package_with(archive, hooks)?;
    let member = find_cover_member_in(archive, hooks, &|_| None);
    let has_cover = member.as_deref().is_some_and(|m| has_entry(archive, m));
    let (mut cover_image, mut cover_error) = (Value::Null, String::new());
    if let (true, Some(member)) = (has_cover, member.as_deref()) {
        match inspect_cover_image(archive, member, hooks) {
            Ok(info) => cover_image = info,
            // 기존 이미지의 손상이 검색과 교체를 막지 않도록 EPUB 구조 오류와 구분한다.
            Err(message) => cover_error = message,
        }
    }
    let mut metadata = Map::new();
    for (key, local) in [("title", "title"), ("author", "creator"), ("isbn", "identifier")] {
        metadata.insert(key.to_string(), json!(first_text(&package, local)));
    }
    let isbn = metadata["isbn"].as_str().unwrap_or("").to_string();
    if !isbn.is_empty() {
        let value = isbn.strip_prefix("urn:isbn:").unwrap_or(&isbn).replace('-', "");
        let valid = value.chars().all(|c| c.is_ascii_digit()) && matches!(value.chars().count(), 10 | 13);
        metadata.insert("isbn".to_string(), json!(if valid { value } else { String::new() }));
    }
    metadata.insert("has_cover".to_string(), json!(has_cover));
    metadata.insert("cover_image".to_string(), cover_image);
    if !cover_error.is_empty() {
        metadata.insert("cover_error".to_string(), json!(cover_error));
    }
    Ok(hooks.finish_inspect(Value::Object(metadata)))
}

/// 임시 경로에 도판을 끼운 EPUB를 만든다(원본은 건드리지 않는다). 새 쪽의 idref를 돌려준다.
pub fn stage_illustration(source: &File, destination: &Path, image: &[u8], before_idref: &str) -> Result<String, CoverError> {
    stage::stage_illustration(source, destination, image, before_idref, &EpubCoverHooks::default()).map_err(Fail::into_cover)
}

/// 임시 경로에 표지를 뺀 EPUB를 만든다(원본은 건드리지 않는다).
pub fn stage_cover_removal(source: &File, destination: &Path) -> Result<(), CoverError> {
    stage::stage_cover_removal(source, destination, &EpubCoverHooks::default()).map_err(Fail::into_cover)
}

pub fn inspect_epub(path: &Path) -> Result<Value, CoverError> {
    inspect_epub_with(path, &EpubCoverHooks::default())
}

/// EPUB의 제목·저자·ISBN·표지 상태를 읽는다. 파일은 바꾸지 않는다. 저장 중이면 거절한다.
pub fn inspect_epub_with(path: &Path, hooks: &EpubCoverHooks) -> Result<Value, CoverError> {
    let run = || -> FResult<Value> {
        let file = File::open(path)?;
        hooks.lock_shared(&file)?;
        let mut archive = ZipArchive::new(&file)?;
        let mut metadata = inspect_archive(&mut archive, hooks)?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let title = metadata["title"].as_str().unwrap_or("");
        let title = if title.is_empty() { stem } else { title.to_string() };
        metadata["title"] = json!(title);
        metadata["revision"] = json!(hooks.revision_of(path));
        Ok(metadata)
    };
    run().map_err(|fail| match fail {
        Fail::Cover(err) => err,
        fail if fail.would_block() => CoverError::new(SAVING),
        _ => CoverError::new(INSPECT_FAILED),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_size_matches_pil() {
        assert_eq!(thumbnail_size(1800, 2700, 600, 3000), (600, 900));
        assert_eq!(thumbnail_size(1200, 600, 600, 3000), (600, 300));
        assert_eq!(thumbnail_size(300, 450, 600, 3000), (300, 450));
        assert_eq!(thumbnail_size(6000, 8000, 2000, 3000), (2000, 2667));
        assert_eq!(thumbnail_size(100, 5000, 600, 3000), (60, 3000));
    }

    #[test]
    fn entry_count_limit() {
        assert!(check_entry_count(20_000).is_ok());
        assert!(check_entry_count(20_001).is_err());
    }
}
