//! 표지 관련 보조. EPUB 표지 멤버 찾기와 썸네일은 `epub_cover`가 구현하고 여기서는 `BookManager`의 이름으로 이어 준다.
//! Python `_find_epub_cover_member`, `_extract_cover_thumbnail`(EPUB 부분).

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use zip::ZipArchive;

use super::{BookManager, BookManagerDeps};
use crate::cover_management::{Backup, CoverHooks, CoverManagement};
use crate::direct_cover_url::ImageFetcher;
use crate::epub_cover::{self, CoverError, EpubCoverHooks};

/// 표지 편집이 저장 위치를 정할 때 쓰는 기본 휴지통 디렉터리 이름(Python `TRASH_DIRNAME`).
pub const TRASH_DIRNAME: &str = "0_trash";

const NOT_EPUB: &str = "EPUB 파일이 아닙니다";
const EMPTY_URL: &str = "이미지 URL이 비어 있습니다";
const SAVE_FAILED: &str = "표지를 저장하지 못했습니다. 다시 시도하세요";
const INDEX_WARNING: &str = "EPUB 표지를 저장했습니다. 검색 색인의 파일 크기는 갱신하지 못했습니다";

/// 표지 편집 대상 책. `BookLookup`이 돌려준다.
#[derive(Debug, Clone)]
pub struct BookRef {
    pub book_id: i64,
    pub file_path: PathBuf,
    pub file_type: String,
}

/// 책 ID로 책을 찾는 능력. 꽂지 않으면 ES(`BookManager::get_book`)로 찾는다.
#[async_trait]
pub trait BookLookup: Send + Sync {
    async fn get_book(&self, book_id: i64) -> (Option<BookRef>, Option<String>);
}

/// 표지 편집이 쓰는 설정과 대역. `BookManagerDeps::with_ext`로 꽂는다.
#[derive(Clone, Default)]
struct CoverExt {
    trash_dirname: Option<String>,
    lookup: Option<Arc<dyn BookLookup>>,
    image_fetcher: Option<Arc<dyn ImageFetcher>>,
    epub_hooks: Option<EpubCoverHooks>,
    thumbnailer: Option<Arc<dyn Thumbnailer>>,
}

impl BookManagerDeps {
    fn cover_ext(&self) -> CoverExt {
        self.ext.get::<CoverExt>().map(|ext| (*ext).clone()).unwrap_or_default()
    }

    /// 표지 편집 때 원본을 두는 디렉터리 이름(기본 `0_trash`).
    pub fn with_trash_dirname(self, name: &str) -> Self {
        let ext = CoverExt { trash_dirname: Some(name.to_string()), ..self.cover_ext() };
        self.with_ext(ext)
    }

    pub fn with_book_lookup(self, lookup: Arc<dyn BookLookup>) -> Self {
        let ext = CoverExt { lookup: Some(lookup), ..self.cover_ext() };
        self.with_ext(ext)
    }

    /// 표지 URL 내려받기(`fetch_url_image`) 대역.
    pub fn with_image_fetcher(self, fetcher: Arc<dyn ImageFetcher>) -> Self {
        let ext = CoverExt { image_fetcher: Some(fetcher), ..self.cover_ext() };
        self.with_ext(ext)
    }

    pub fn with_epub_hooks(self, hooks: EpubCoverHooks) -> Self {
        let ext = CoverExt { epub_hooks: Some(hooks), ..self.cover_ext() };
        self.with_ext(ext)
    }

    pub fn with_thumbnailer(self, thumbnailer: Arc<dyn Thumbnailer>) -> Self {
        let ext = CoverExt { thumbnailer: Some(thumbnailer), ..self.cover_ext() };
        self.with_ext(ext)
    }
}

type Change = Box<dyn FnOnce(&CoverManagement, &str, &str, Backup<'_>) -> Result<Value, CoverError> + Send>;

/// 표지 썸네일 추출 능력. 구 `ThumbnailExtractor`.
pub trait Thumbnailer: Send + Sync {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>>;
}

impl BookManager {
    pub(super) fn thumbnailer(&self) -> Option<Arc<dyn Thumbnailer>> {
        self.cover_ext().thumbnailer
    }

    /// EPUB 표지 이미지 멤버 경로. 명시한 표지가 없으면 guide·manifest의 표지 XHTML에서 이미지를 찾는다.
    pub fn find_epub_cover_member<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Option<String> {
        epub_cover::find_epub_cover_member(archive)
    }

    /// `file_size`가 값을 돌려주면 ZIP 헤더 크기 대신 쓴다(Python이 `ZipInfo.file_size`를 덮어쓰는 것에 대응).
    pub fn find_epub_cover_member_with<R: Read + Seek>(archive: &mut ZipArchive<R>, file_size: &dyn Fn(&str) -> Option<u64>) -> Option<String> {
        epub_cover::find_epub_cover_member_with(archive, file_size)
    }

    /// EPUB 표지를 JPEG 썸네일로 만든다. PDF는 아직 연결하지 않았다.
    pub fn extract_cover_thumbnail(path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        if file_type == "epub" { epub_cover::extract_epub_thumbnail(path, width) } else { None }
    }
}

impl BookManager {
    pub(super) fn pdf_edit_settings(&self) -> (String, Arc<dyn ImageFetcher>) {
        let ext = self.cover_ext();
        (ext.trash_dirname.unwrap_or_else(|| TRASH_DIRNAME.into()), ext.image_fetcher.unwrap_or_else(|| Arc::new(crate::direct_cover_url::RealImageFetcher)))
    }

    pub(super) fn image_pixel_limit(&self) -> usize {
        self.cover_ext().epub_hooks.as_ref().map(|hooks| hooks.limits().max_image_pixels).unwrap_or(crate::epub_cover::MAX_IMAGE_PIXELS)
    }

    /// 훅이 정한 OPF 경로(구 `override_find_opf_path`). 훅이 없으면 `None`.
    pub(super) fn opf_path_override(&self) -> Option<String> {
        self.ext::<CoverExt>().and_then(|ext| ext.epub_hooks.as_ref().and_then(|h| h.opf_path_override()))
    }

    fn cover_ext(&self) -> CoverExt {
        self.ext::<CoverExt>().map(|ext| (*ext).clone()).unwrap_or_default()
    }

    fn cover_hooks(&self) -> CoverHooks {
        let ext = self.cover_ext();
        let mut hooks = CoverHooks::default();
        if let Some(fetcher) = ext.image_fetcher {
            hooks = hooks.with_image_fetcher(fetcher);
        }
        if let Some(epub) = ext.epub_hooks {
            hooks = hooks.with_epub_hooks(epub);
        }
        if let Some(thumbnailer) = ext.thumbnailer {
            hooks = hooks.with_thumbnailer(thumbnailer);
        }
        hooks
    }

    async fn cover_book(&self, book_id: i64) -> (Option<BookRef>, Option<String>) {
        if let Some(lookup) = self.cover_ext().lookup {
            return lookup.get_book(book_id).await;
        }
        match self.get_book(book_id).await {
            Ok(book) => (Some(BookRef { book_id: book.book_id, file_path: book.file_path, file_type: book.file_type }), None),
            Err(error) => (None, Some(error.to_string())),
        }
    }

    /// EPUB 표지를 URL의 이미지(`url`) 또는 올린 이미지(`image_data`, base64)로 넣거나 바꾼다.
    ///
    /// 표지 관리와 같은 경로(`CoverManagement::apply_url`/`apply_image`)를 쓰므로 검증과 썸네일·미리보기 캐시 무효화가 같다.
    pub async fn set_epub_cover(&self, book_id: i64, url: Option<&str>, image_data: Option<&str>) -> Result<Value, String> {
        if let Some(data) = image_data.filter(|d| !d.is_empty()) {
            let data = data.to_string();
            return self.change_epub_cover(book_id, Box::new(move |service, relative, revision, backup| service.apply_image_backup(relative, &data, revision, backup))).await;
        }
        let url = url.unwrap_or("").trim().to_string();
        if url.is_empty() {
            return Err(EMPTY_URL.to_string());
        }
        self.change_epub_cover(book_id, Box::new(move |service, relative, revision, backup| service.apply_url_backup(relative, &url, revision, true, backup))).await
    }

    /// `before_idref` 페이지 앞에 삽화를 넣는다. 표지는 바꾸지 않는다.
    pub async fn insert_epub_illustration(&self, book_id: i64, before_idref: &str, url: Option<&str>, image_data: Option<&str>) -> Result<Value, String> {
        let url = url.unwrap_or("").trim().to_string();
        let data = image_data.filter(|d| !d.is_empty()).map(str::to_string);
        if data.is_none() && url.is_empty() {
            return Err(EMPTY_URL.to_string());
        }
        let before = before_idref.to_string();
        self.change_epub_cover(book_id, Box::new(move |service, relative, revision, backup| service.insert_illustration_backup(relative, revision, &before, Some(&url), data.as_deref(), backup))).await
    }

    /// EPUB 쪽 이미지의 크롭·분할·삭제·삽입을 한 번에 저장한다.
    ///
    /// crops: `[{idref, index, x0, y0, x1, y1, angle}]`, splits: `[{idref, index, first, second}]`, deletes: `[{idref, index}]`
    /// (index는 쪽 안 이미지의 문서 순서, 기본 0), inserts: `[{before_idref, url | image_data(base64)}]`.
    /// 저장 절차는 표지와 같다(백업·색인 갱신·캐시 무효화).
    pub async fn edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> Result<Value, String> {
        fn number(entry: &Value, key: &str) -> Result<f64, CoverError> {
            entry.get(key).and_then(Value::as_f64).filter(|f| f.is_finite()).ok_or_else(|| CoverError::new("크롭 영역이 올바르지 않습니다"))
        }
        fn rect(entry: &Value) -> Result<Value, CoverError> {
            let angle = match entry.get("angle") {
                None | Some(Value::Null) => 0.0,
                Some(_) => number(entry, "angle")?,
            };
            Ok(json!([number(entry, "x0")?, number(entry, "y0")?, number(entry, "x1")?, number(entry, "y1")?, angle]))
        }
        let index = |entry: &Value| entry.get("index").cloned().filter(|v| !v.is_null()).unwrap_or(json!(0));
        let converted = (|| -> Result<(Vec<Value>, Vec<Value>), CoverError> {
            let crop_args = crops.iter().map(|c| Ok(json!({"idref": c.get("idref"), "index": index(c), "rect": rect(c)?}))).collect::<Result<Vec<_>, CoverError>>()?;
            let split_args = splits
                .iter()
                .map(|s| Ok(json!({"idref": s.get("idref"), "index": index(s), "first": rect(s.get("first").unwrap_or(&Value::Null))?, "second": rect(s.get("second").unwrap_or(&Value::Null))?})))
                .collect::<Result<Vec<_>, CoverError>>()?;
            Ok((crop_args, split_args))
        })();
        let (crop_args, split_args) = match converted {
            Ok(args) => args,
            Err(err) => return Err(err.to_string()),
        };
        let (deletes, inserts) = (deletes.to_vec(), inserts.to_vec());
        self.change_epub_cover(
            book_id,
            Box::new(move |service, relative, revision, backup| {
                let edits = epub_cover::EditImages { crops: crop_args, splits: split_args, deletes, ..Default::default() };
                service.edit_images(relative, revision, edits, &inserts, backup)
            }),
        )
        .await
    }

    /// EPUB 표지를 읽기 순서에서 뺀다. 본문과 공유하는 이미지는 보존한다.
    pub async fn remove_epub_cover(&self, book_id: i64) -> Result<Value, String> {
        self.change_epub_cover(book_id, Box::new(|service, relative, revision, backup| service.remove(relative, revision, backup))).await
    }

    /// 표지를 바꾸는 공통 절차: 책 확인, 백업과 함께 변경, 색인 갱신.
    ///
    /// 뷰어는 파일의 현재 revision을 모르므로 여기서 읽어 넘기고, 저장 직전에 잠금 안에서 다시 확인한다.
    /// 원본은 PDF 편집과 같이 휴지통에 원래 이름으로 보존한다.
    async fn change_epub_cover(&self, book_id: i64, change: Change) -> Result<Value, String> {
        let (book, error) = self.cover_book(book_id).await;
        let Some(book) = book else { return Err(error.unwrap_or_else(|| format!("Book not found: {book_id}"))) };
        if book.file_type != "epub" {
            return Err(NOT_EPUB.to_string());
        }
        let root = self.path_prefix.clone();
        let trash = root.join(self.cover_ext().trash_dirname.as_deref().unwrap_or(TRASH_DIRNAME));
        let hooks = self.cover_hooks();
        let epub_hooks = self.cover_ext().epub_hooks.unwrap_or_default();
        let work = move || -> Result<Value, CoverError> {
            let service = CoverManagement::with_hooks(&root, hooks);
            let relative = book
                .file_path
                .strip_prefix(&root)
                .or_else(|_| book.file_path.strip_prefix(service.root()))
                .map_err(|_| CoverError::io("책 경로가 책 디렉토리 밖입니다"))?
                .to_string_lossy()
                .into_owned();
            let path = service.resolve(&relative, false)?;
            let expected_revision = epub_cover::inspect_epub_with(&path, &epub_hooks)?["revision"].as_str().unwrap_or("").to_string();
            let name = book.file_path.file_name().map(|n| n.to_string_lossy().into_owned());
            let backup = Backup { dir: Some(&trash), name: name.as_deref() };
            let result = change(&service, &relative, &expected_revision, backup)?;
            // 외부 파일 교체 뒤에도 ES의 책 ID는 이전 inode일 수 있다.
            // 편집 화면은 ES의 책 ID로 미리보기를 요청하므로 이 캐시도 지운다.
            if result["book_id"].as_i64() != Some(book_id) {
                service.invalidate_view_cache(u64::try_from(book_id).unwrap_or(0));
            }
            Ok(result)
        };
        let mut result = match tokio::task::spawn_blocking(work).await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) if !err.is_io() && !err.is_cancelled() => return Err(err.to_string()),
            Ok(Err(err)) => {
                log::error!("change_epub_cover failed: {err}");
                return Err(SAVE_FAILED.to_string());
            }
            Err(err) => {
                log::error!("change_epub_cover failed: {err}");
                return Err(SAVE_FAILED.to_string());
            }
        };
        // 최신 책 목록은 updated_time 순이다. 표지 저장은 EPUB의 mtime을 되돌리므로(캐시·재적재 판정 보존) 색인에 직접 반영한다.
        // loader는 파일 mtime이 저장된 updated_time 이하이면 이 값을 유지하므로 재적재해도 되돌아가지 않는다.
        let updated_time = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f").to_string();
        let fields = json!({"file_size": result["file_size"], "updated_time": updated_time});
        let es = self.es.clone();
        let updated = tokio::task::spawn_blocking(move || es.update(book_id, &fields)).await;
        if !matches!(updated, Ok(Ok(true))) {
            log::warn!("Cover saved; ES metadata update failed");
            result["warning"] = json!(INDEX_WARNING);
        }
        Ok(result)
    }
}
