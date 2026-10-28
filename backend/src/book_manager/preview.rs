//! 미리보기와 PDF 쪽 추출. Python `get_book_preview`, `get_pdf_pages`에 대응한다.
//!
//! 지원: PDF(앞 N쪽), EPUB(앞 N챕터 또는 전체), DOC/HWP(LibreOffice → HTML). 지원하지 않는 형식은 400이다.
//! HTML 미리보기와 리소스 경로는 `html.rs`에서 처리한다. HWP3은 LibreOffice 변환이 비면 네이티브 파서로 폴백한다.
//! PDF reader와 쪽수는 경로·수정 시각 cache를 공유한다.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
use std::time::SystemTime;

use regex::Regex;
use scraper::{Html, Selector};
use zip::ZipArchive;
use zip::result::ZipError;

use super::epub::{Edit, NCX_NS, OPF_NS, XML_DECLARATION, apply_edits, children_named, declare_unbound_prefixes, find_el, parse_xml, read_member, xml_text, zip_names};
use super::util::{clean_href, posix_dirname, zip_path};
use super::{BookManager, BookResponse};

const HTML_VIEWER_CSP: &str = "sandbox; default-src 'none'; script-src 'none'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; media-src 'self' blob:;";
/// 뷰어 이미지 축소를 시도하는 최소 크기(바이트).
const EPUB_IMAGE_MIN_BYTES: usize = 400 * 1024;
/// 이 크기를 넘는 폰트는 미리보기에서 뺀다(바이트).
const FONT_SIZE_LIMIT: u64 = 500 * 1024;
/// 이미지 축소 정책 버전. 캐시 파일 이름에 붙여 정책이 바뀌면 옛 산출물이 계속 나가지 않게 한다.
const FONT_EXTENSIONS: [&str; 4] = [".ttf", ".otf", ".woff", ".woff2"];
const FONT_MEDIA_TYPES: [&str; 8] = ["font/ttf", "font/otf", "font/woff", "font/woff2", "application/font-ttf", "application/font-woff", "application/font-woff2", "application/x-font-ttf"];
const XHTML_SUFFIXES: [&str; 3] = [".xhtml", ".html", ".htm"];
const EPUB_IMAGE_SUFFIXES: [&str; 3] = [".jpg", ".jpeg", ".png"];
const DEFAULT_PREVIEW_PAGES: u32 = 5;
const DEFAULT_PREVIEW_CHAPTERS: u32 = 10;

/// 본문이 문자열인 응답. FastAPI `Response(content=..., status_code=...)`.
pub(super) fn text_response(status_code: u16, content: impl Into<String>) -> BookResponse {
    BookResponse { status_code, media_type: None, headers: Vec::new(), body: content.into().into_bytes() }
}

/// `html.escape(text)`: `&`, `<`, `>`, `"`, `'`를 바꾼다.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

fn no_transform_headers() -> Vec<(String, String)> {
    vec![("Content-Encoding".to_string(), "identity".to_string()), ("Cache-Control".to_string(), "no-transform".to_string())]
}

/// 뷰어 HTML에 붙이는 보안 헤더. `BookManager._html_security_headers`.
pub fn html_security_headers() -> Vec<(String, String)> {
    [
        ("Content-Security-Policy", HTML_VIEWER_CSP),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
        ("Permissions-Policy", "camera=(), microphone=(), geolocation=()"),
        ("Cache-Control", "no-transform"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn css_url_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"url\(["']?([^"')\s]+)["']?\)"#).expect("정규식"))
}

fn font_face_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"@font-face\s*\{[^}]*\}").expect("정규식"))
}

/// XHTML에서는 `<title/>`이 유효하지만 HTML 파서는 title/script/style/textarea의 자기닫힘 표기를 인정하지 않는다. 미리보기 산출물에서만 쌍으로 편다.
fn expand_self_closing_rcdata(data: &[u8]) -> Vec<u8> {
    static RE: OnceLock<regex::bytes::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::bytes::Regex::new(r"(?i)<(title|script|style|textarea)(\s[^<>]*?)?\s*/>").expect("정규식"));
    re.replace_all(data, &b"<${1}${2}></${1}>"[..]).into_owned()
}

/// manifest 항목 하나.
struct ManifestItem {
    id: String,
    href: String,
    media_type: String,
}

/// 미리보기 EPUB을 만들다가 응답으로 바로 끝내야 하는 경우.
enum PreviewFailure {
    Response(BookResponse),
    Other(String),
}

impl From<String> for PreviewFailure {
    fn from(message: String) -> Self {
        PreviewFailure::Other(message)
    }
}

impl From<ZipError> for PreviewFailure {
    fn from(e: ZipError) -> Self {
        PreviewFailure::Other(e.to_string())
    }
}

/// `get_pdf_pages` 한 요청의 최대 쪽 수. 뷰어는 11쪽씩 요청한다.
pub const MAX_PDF_PAGE_RANGE: i64 = 50;

impl BookManager {
    /// Python과 같은 공개 검증 지점. EPUB 미리보기의 HTML 호환 변환을 수행한다.
    pub fn _expand_self_closing_rcdata(data: &[u8]) -> Vec<u8> {
        expand_self_closing_rcdata(data)
    }

    fn mtime_of(&self, path: &Path) -> Option<SystemTime> {
        self.fs.metadata(path).ok().and_then(|m| m.modified().ok())
    }

    /// 캐시 파일이 있고 원본보다 새것인가.
    /// 캐시를 임시 파일에 다 쓴 뒤 이름을 바꾼다. 동시 요청이 쓰는 중인 잘린 파일을 캐시 적중으로 내보내지 않게 한다.
    fn write_cache(&self, cache_file: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let tmp = super::util::temp_sibling(cache_file);
        self.fs.write(&tmp, bytes).and_then(|()| self.fs.rename(&tmp, cache_file)).inspect_err(|_| {
            let _ = self.fs.remove_file(&tmp);
        })
    }

    fn cache_is_fresh(&self, cache_file: &Path, original_mtime: SystemTime) -> bool {
        self.mtime_of(cache_file).is_some_and(|t| t >= original_mtime)
    }

    fn file_response(&self, path: &Path, media_type: &str, headers: Vec<(String, String)>) -> BookResponse {
        match std::fs::read(path) {
            Ok(body) => BookResponse { status_code: 200, media_type: Some(media_type.to_string()), headers, body },
            Err(e) => text_response(500, format!("File read failed: {e}")),
        }
    }

    /// 기본값(5쪽, 10챕터)과 빈 `resource_base_url`로 미리보기를 만든다.
    pub async fn get_book_preview(&self, book_id: i64, pages: Option<u32>, chapters: Option<u32>) -> BookResponse {
        self.get_book_preview_with_base(book_id, pages, chapters, "").await
    }

    /// 미리보기를 만든다. `chapters`가 0이면 EPUB 전체다. `resource_base_url`은 HTML 미리보기가 리소스 주소를 만들 때 쓴다.
    pub async fn get_book_preview_with_base(&self, book_id: i64, pages: Option<u32>, chapters: Option<u32>, resource_base_url: &str) -> BookResponse {
        let (pages, chapters) = (pages.unwrap_or(DEFAULT_PREVIEW_PAGES), chapters.unwrap_or(DEFAULT_PREVIEW_CHAPTERS));
        log::debug!("# get_book_preview(book_id={book_id}, pages={pages}, chapters={chapters}, resource_base_url='{resource_base_url}')");
        let doc = match self.find_doc(book_id) {
            Ok(Some(doc)) => doc,
            Ok(None) => {
                log::warn!("get_book_preview: book_id={book_id} not found in ES");
                return text_response(404, format!("Book not found: {book_id}"));
            }
            Err(e) => return text_response(500, format!("Book lookup failed: {e}")),
        };
        let book = match self.try_make_book(book_id, &doc, 0.0) {
            Ok(book) => book,
            Err(e) => return text_response(500, format!("Book conversion failed: {e}")),
        };
        if !self.fs.is_file(&book.file_path).unwrap_or(false) {
            log::warn!("get_book_preview: file not found: '{}' (book_id={book_id})", book.file_path.display());
            return text_response(404, format!("File not found: {}", book.file_path.display()));
        }

        let suffix = book.file_path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
        let cache_dir = self.path_prefix.join(".preview_cache");
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            return text_response(500, format!("Preview cache unavailable: {e}"));
        }
        let Some(original_mtime) = self.mtime_of(&book.file_path) else { return text_response(500, "Preview failed") };

        match suffix.as_str() {
            ".pdf" => self.preview_pdf(book_id, &book.file_path, &cache_dir, original_mtime, pages),
            ".epub" => self.preview_epub(book_id, &book.file_path, &cache_dir, original_mtime, chapters),
            ".html" => self.preview_html(book_id, &book.file_path, resource_base_url),
            ".doc" | ".hwp" => self.preview_office(book_id, &book.file_path, &cache_dir, original_mtime, &suffix),
            _ => text_response(400, format!("Unsupported file type: {suffix}")),
        }
    }

    /// `prefix`로 시작하고 `suffix`로 끝나는 캐시 중 `keep`이 아닌 것을 지운다. 책마다 캐시를 하나만 남긴다.
    fn remove_other_caches(&self, cache_dir: &Path, prefix: &str, suffix: &str, keep: &str) {
        let Ok(entries) = self.fs.read_dir(cache_dir) else { return };
        for old in entries {
            let name = old.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.starts_with(prefix)
                && name.ends_with(suffix)
                && name != keep
                && let Err(e) = self.fs.remove_file(&old)
            {
                log::warn!("Failed to remove old preview cache {name}: {e}");
            }
        }
    }

    fn preview_pdf(&self, book_id: i64, file_path: &Path, cache_dir: &Path, original_mtime: SystemTime, pages: u32) -> BookResponse {
        // 쪽 수를 이름에 넣는다. 빼면 큰 pages 요청 하나가 모든 사용자의 미리보기를 전체 책으로 바꾼다.
        let cache_name = format!("{book_id}_n{pages}.pdf");
        let cache_file = cache_dir.join(&cache_name);
        self.remove_other_caches(cache_dir, &format!("{book_id}_n"), ".pdf", &cache_name);
        let legacy = cache_dir.join(format!("{book_id}.pdf"));
        if self.path_exists(&legacy)
            && let Err(e) = self.fs.remove_file(&legacy)
        {
            log::warn!("Failed to remove old preview cache {}: {e}", legacy.display());
        }
        if self.cache_is_fresh(&cache_file, original_mtime) {
            log::debug!("Preview cache hit for book_id={book_id} (PDF)");
            return self.file_response(&cache_file, "application/pdf", no_transform_headers());
        }
        let generate = || -> Result<Vec<u8>, String> {
            let (reader, total) = self.try_get_cached_pdf_reader(file_path).map_err(|e| e.to_string())?;
            let take = (total as usize).min(pages as usize);
            let bytes = self.pdf_pages.extract_pages_from_reader(&reader, 0, take).map_err(|e| e.to_string())?;
            self.write_cache(&cache_file, &bytes).map_err(|e| e.to_string())?;
            log::debug!("Preview generated for book_id={book_id} (PDF, {take} pages)");
            Ok(bytes)
        };
        match generate() {
            Ok(bytes) => {
                Self::_evict_old_cache_with(&*self.fs, cache_dir);
                BookResponse { status_code: 200, media_type: Some("application/pdf".to_string()), headers: no_transform_headers(), body: bytes }
            }
            Err(e) => {
                log::error!("PDF preview generation failed for book_id={book_id}: {e}");
                text_response(500, "PDF preview failed")
            }
        }
    }

    fn preview_office(&self, book_id: i64, file_path: &Path, cache_dir: &Path, original_mtime: SystemTime, suffix: &str) -> BookResponse {
        let cache_file = cache_dir.join(format!("{book_id}.html"));
        if self.cache_is_fresh(&cache_file, original_mtime) && self.fs.metadata(&cache_file).is_ok_and(|m| m.len() > 0) {
            log::debug!("Preview cache hit for book_id={book_id} ({suffix})");
            return self.file_response(&cache_file, "text/html", html_security_headers());
        }
        let converted = Self::try_convert_with_libreoffice(&*self.runner, &self.find_libreoffice(), None, file_path, "html");
        let html_content = match converted {
            Ok(html) => html,
            Err(e) => {
                log::error!("{} preview generation failed for book_id={book_id}: {e}", suffix.to_uppercase());
                return text_response(500, format!("{} preview failed", suffix.to_uppercase()));
            }
        };
        let mut html_content = html_content;
        if html_content.trim().is_empty() && suffix == ".hwp" {
            // LibreOffice 변환 실패 - 네이티브 HWP3 파서로 폴백한다.
            let plain_text = hwp3_parser::extract_text_from_hwp3(file_path);
            if !plain_text.is_empty() {
                let paragraphs: String = escape_html(&plain_text).split('\n').filter(|line| !line.trim().is_empty()).map(|line| format!("<p>{line}</p>")).collect::<Vec<_>>().join("\n");
                html_content = format!("<html><body style=\"font-family:sans-serif;padding:1em;line-height:1.8\">{paragraphs}</body></html>");
            }
        }
        if !html_content.is_empty() {
            if let Err(e) = self.write_cache(&cache_file, html_content.as_bytes()) {
                log::error!("{} preview cache write failed for book_id={book_id}: {e}", suffix.to_uppercase());
                return text_response(500, format!("{} preview failed", suffix.to_uppercase()));
            }
            Self::_evict_old_cache_with(&*self.fs, cache_dir);
            log::debug!("Preview generated for book_id={book_id} ({suffix})");
            return BookResponse { status_code: 200, media_type: Some("text/html".to_string()), headers: html_security_headers(), body: html_content.into_bytes() };
        }
        // 변환 실패 시 빈 HTML을 돌려준다.
        BookResponse {
            status_code: 200,
            media_type: Some("text/html".to_string()),
            headers: html_security_headers(),
            body: "<html><body><p>미리보기를 생성할 수 없습니다.</p></body></html>".as_bytes().to_vec(),
        }
    }

    fn preview_epub(&self, book_id: i64, file_path: &Path, cache_dir: &Path, original_mtime: SystemTime, requested_chapters: u32) -> BookResponse {
        let total_chapters = Self::_get_epub_total_chapters(file_path);
        // chapters<=0: 전체 챕터 포함(대용량 폰트 제거, 대형 이미지 축소)
        let chapters = if requested_chapters == 0 { total_chapters } else { i64::from(requested_chapters) };
        let cache_name = format!("{book_id}_ch{chapters}_v{}.epub", BookManager::EPUB_IMAGE_POLICY_VERSION);
        let cache_file = cache_dir.join(&cache_name);

        // 구 형식 캐시 정리(book_id.epub, book_id.html, 정책 버전 없는 ch 캐시)
        self.remove_other_caches(cache_dir, &format!("{book_id}_ch"), ".epub", &cache_name);
        for old_name in [format!("{book_id}.epub"), format!("{book_id}.html")] {
            let old = cache_dir.join(old_name);
            if self.path_exists(&old)
                && let Err(e) = self.fs.remove_file(&old)
            {
                log::warn!("Failed to remove old preview cache {}: {e}", old.display());
            }
        }
        let mut extra_headers = no_transform_headers();
        extra_headers.push(("X-Total-Chapters".to_string(), total_chapters.to_string()));
        if self.cache_is_fresh(&cache_file, original_mtime) {
            log::debug!("Preview cache hit for book_id={book_id} (EPUB, ch{chapters})");
            return self.file_response(&cache_file, "application/epub+zip", extra_headers);
        }

        // 임시 파일에 만들고 검증까지 마친 뒤 이름을 바꾼다. 동시 요청이 만드는 중인 파일을 캐시 적중으로 보지 않게 한다.
        let tmp_file = super::util::temp_sibling(&cache_file);
        // 훅이나 파서가 panic해도 요청 하나의 500으로 끝낸다.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.build_epub_preview(book_id, file_path, &tmp_file, requested_chapters == 0, chapters)))
            .unwrap_or_else(|_| Err(PreviewFailure::Other("panic during EPUB preview".to_string())));
        if built.is_err() {
            let _ = self.fs.remove_file(&tmp_file);
        }
        match built {
            Ok(chapter_count) => {
                // 생성된 EPUB 유효성 검증
                let (valid, err) = match self.seams().preview_epub_validator {
                    Some(validator) => {
                        let (valid, reason) = validator(&tmp_file);
                        (valid, (!reason.is_empty()).then_some(reason))
                    }
                    None => Self::_validate_preview_epub_with(&*self.fs, &tmp_file),
                };
                if !valid {
                    let _ = self.fs.remove_file(&tmp_file);
                    let err = err.unwrap_or_default();
                    log::error!("EPUB preview validation failed for book_id={book_id}: {err}");
                    return text_response(422, format!("EPUB preview validation failed: {err}"));
                }
                if let Err(e) = self.fs.rename(&tmp_file, &cache_file) {
                    let _ = self.fs.remove_file(&tmp_file);
                    log::error!("EPUB preview cache write failed for book_id={book_id}: {e}");
                    return text_response(500, "EPUB preview failed");
                }
                Self::_evict_old_cache_with(&*self.fs, cache_dir);
                log::debug!("Preview generated for book_id={book_id} (EPUB, {chapter_count} chapters)");
                self.file_response(&cache_file, "application/epub+zip", extra_headers)
            }
            Err(PreviewFailure::Response(response)) => response,
            Err(PreviewFailure::Other(message)) => {
                log::error!("EPUB preview generation failed for book_id={book_id}: {message}");
                if message.contains("corrupted") {
                    return text_response(422, "EPUB file is corrupted or not a valid ZIP");
                }
                text_response(500, "EPUB preview failed")
            }
        }
    }

    /// 원본 EPUB에서 선택한 챕터와 연관 리소스만 담은 미리보기 EPUB을 `cache_file`에 쓴다. 담은 챕터 수를 돌려준다.
    fn build_epub_preview(&self, book_id: i64, file_path: &Path, cache_file: &Path, is_full_view: bool, chapters: i64) -> Result<usize, PreviewFailure> {
        let file = self.fs.open(file_path).map_err(|e| e.to_string())?;
        let mut zin = match ZipArchive::new(file) {
            Ok(zin) => zin,
            Err(ZipError::Io(e)) if e.kind() != std::io::ErrorKind::UnexpectedEof => return Err(PreviewFailure::Other(e.to_string())),
            Err(_) => return Err(PreviewFailure::Response(text_response(422, "EPUB file is corrupted or not a valid ZIP"))),
        };
        let names = zip_names(&mut zin);

        // OPF 경로 찾기(container.xml -> regex -> 직접 탐색)
        let opf_override = self.opf_path_override().filter(|p| !p.is_empty());
        let Some(opf_path) = opf_override.or_else(|| Self::_find_opf_path(&mut zin)) else {
            log::warn!("EPUB preview: OPF file not found for book_id={book_id}");
            return Err(PreviewFailure::Response(text_response(422, "EPUB structure error: OPF file not found")));
        };
        let opf_dir = posix_dirname(&opf_path);
        let opf_bytes = match read_member(&mut zin, &opf_path) {
            Ok(bytes) => bytes,
            Err(ZipError::FileNotFound) => {
                log::warn!("EPUB preview: OPF file missing in archive: {opf_path} (book_id={book_id})");
                return Err(PreviewFailure::Response(text_response(422, format!("EPUB structure error: OPF file missing: {opf_path}"))));
            }
            Err(e) => return Err(e.into()),
        };
        // 선언 없이 쓰인 접두사(opf:, xsi: 등)를 선언한다. 재직렬화 때 xmlns가 빠지면 브라우저가 파싱하지 못한다.
        let opf_text = declare_unbound_prefixes(&xml_text(&opf_bytes));
        let doc = parse_xml(&opf_text).map_err(|e| format!("OPF parse error: {e}"))?;
        let root_range = doc.root_element().range();

        // manifest: id -> href, media-type (문서 순서 유지, 같은 id는 나중 값이 이긴다)
        let mut manifest: Vec<ManifestItem> = Vec::new();
        let mut href_to_id: HashMap<String, String> = HashMap::new();
        for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().namespace() == Some(OPF_NS) && n.tag_name().name() == "item") {
            let (id, href, media_type) = (item.attribute("id").unwrap_or("").to_string(), item.attribute("href").unwrap_or("").to_string(), item.attribute("media-type").unwrap_or("").to_string());
            href_to_id.insert(zip_path(&opf_dir, &clean_href(&href)), id.clone());
            href_to_id.entry(zip_path(&opf_dir, &href)).or_insert_with(|| id.clone());
            match manifest.iter_mut().find(|m| m.id == id) {
                Some(existing) => {
                    existing.href = href;
                    existing.media_type = media_type;
                }
                None => manifest.push(ManifestItem { id, href, media_type }),
            }
        }
        let item_of = |id: &str| manifest.iter().find(|m| m.id == id);

        let mut edits: Vec<Edit> = Vec::new();
        let spine_el = find_el(&doc, OPF_NS, "spine");
        let take = chapters.max(0) as usize;
        let (chapter_idrefs, spine_refs): (Vec<String>, Vec<roxmltree::Node>) = match spine_el {
            None => {
                log::warn!("EPUB preview: spine not found for book_id={book_id}, trying manifest order");
                let idrefs: Vec<String> = manifest.iter().filter(|m| m.media_type == "application/xhtml+xml").map(|m| m.id.clone()).take(take).collect();
                // spine을 새로 만들어 루트 끝에 붙인다.
                let root_text = &opf_text[root_range.clone()];
                if let Some(close) = root_text.rfind("</") {
                    let prefix = doc.root_element().lookup_prefix(OPF_NS).filter(|p| !p.is_empty()).map(|p| format!("{p}:")).unwrap_or_default();
                    let refs: String = idrefs.iter().map(|id| format!("<{prefix}itemref idref=\"{}\"/>", id.replace('&', "&amp;").replace('"', "&quot;"))).collect();
                    let at = root_range.start + close;
                    edits.push(Edit { range: at..at, replacement: format!("<{prefix}spine>{refs}</{prefix}spine>") });
                }
                (idrefs, Vec::new())
            }
            Some(spine) => {
                let refs: Vec<roxmltree::Node> = children_named(spine, OPF_NS, "itemref").collect();
                let selected: Vec<&roxmltree::Node> = if 0 < chapters && (chapters as usize) < refs.len() {
                    refs.iter().filter(|r| r.attribute("linear").filter(|l| !l.is_empty()).unwrap_or("yes") != "no").take(take).collect()
                } else {
                    refs.iter().take(take).collect()
                };
                let idrefs = selected.iter().filter_map(|r| r.attribute("idref")).filter(|id| item_of(id).is_some()).map(str::to_string).collect();
                (idrefs, refs)
            }
        };

        let toc_id = spine_el.and_then(|s| s.attribute("toc")).unwrap_or("").to_string();
        let mut files_to_include: BTreeSet<String>;
        let mut ids_to_keep: HashSet<String>;
        if is_full_view {
            // 전체보기: ZIP 파일과 manifest 항목 전체를 기본으로 포함한다(EPUB 구조 보존).
            files_to_include = names.iter().cloned().collect();
            ids_to_keep = manifest.iter().map(|m| m.id.clone()).collect();
        } else {
            // 미리보기(부분 챕터): 선택된 챕터와 연관 리소스만 포함한다.
            files_to_include = BTreeSet::from([opf_path.clone()]);
            if names.iter().any(|n| n == "META-INF/container.xml") {
                files_to_include.insert("META-INF/container.xml".to_string());
            }
            ids_to_keep = chapter_idrefs.iter().cloned().collect();

            // 커버는 meta cover/cover-image로만 선언되고 챕터가 참조하지 않는 EPUB이 있다. 항상 남긴다.
            let meta_cover_id = doc
                .descendants()
                .filter(|n| n.is_element() && n.tag_name().namespace() == Some(OPF_NS) && n.tag_name().name() == "meta")
                .find(|m| m.attribute("name") == Some("cover"))
                .and_then(|m| m.attribute("content"))
                .unwrap_or("")
                .to_string();
            for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().namespace() == Some(OPF_NS) && n.tag_name().name() == "item") {
                let item_id = item.attribute("id").unwrap_or("");
                let is_cover = item.attribute("properties").unwrap_or("").split_whitespace().any(|p| p == "cover-image") || (!meta_cover_id.is_empty() && item_id == meta_cover_id);
                if let Some(entry) = item_of(item_id).filter(|m| is_cover && m.media_type.starts_with("image/")) {
                    files_to_include.insert(zip_path(&opf_dir, &clean_href(&entry.href)));
                    ids_to_keep.insert(item_id.to_string());
                }
            }

            if let Some(toc) = item_of(&toc_id).filter(|_| !toc_id.is_empty()) {
                ids_to_keep.insert(toc_id.clone());
                files_to_include.insert(zip_path(&opf_dir, &clean_href(&toc.href)));
            }

            let mut chapter_zip_paths: Vec<String> = Vec::new();
            for idref in &chapter_idrefs {
                if let Some(entry) = item_of(idref) {
                    let zp = zip_path(&opf_dir, &clean_href(&entry.href));
                    files_to_include.insert(zp.clone());
                    chapter_zip_paths.push(zp);
                }
            }

            // 챕터 HTML에서 참조 리소스 수집
            let mut referenced: HashSet<String> = HashSet::new();
            let selector = Selector::parse("img, image, link, script, source, embed, object, video, a").expect("선택자");
            for zp in &chapter_zip_paths {
                let content = match read_member(&mut zin, zp) {
                    // HTML 파서는 `<title/>`을 빈 요소로 보지 않아 문서 끝까지 삼킨다. BeautifulSoup `html.parser`처럼 읽도록 쌍으로 편다.
                    Ok(bytes) => String::from_utf8_lossy(&expand_self_closing_rcdata(&bytes)).into_owned(),
                    Err(e) => {
                        // 없거나 손상돼 읽을 수 없는 챕터는 건너뛴다(책 전체를 거부하지 않는다).
                        log::warn!("EPUB preview: chapter file missing or unreadable in archive: {zp} ({e})");
                        continue;
                    }
                };
                let item_dir = posix_dirname(zp);
                let html = Html::parse_document(&content);
                for element in html.select(&selector) {
                    let value = element.value();
                    // attr("href")는 빈 namespace만 찾는다. SVG의 xlink:href는 attrs()의 로컬 이름으로 읽는다.
                    let url =
                        ["src", "href", "xlink:href", "data", "poster"].iter().filter_map(|a| value.attrs().find(|(name, _)| name == a).map(|(_, url)| url)).find(|u| !u.is_empty()).unwrap_or("");
                    if !url.is_empty() && !url.starts_with("data:") {
                        let clean = clean_href(url);
                        if !clean.is_empty() {
                            referenced.insert(zip_path(&item_dir, &clean));
                        }
                    }
                }
            }

            // CSS 수집 및 CSS 안 url() 참조 수집
            let css_refs: Vec<String> = referenced.iter().filter(|r| href_to_id.get(*r).and_then(|id| item_of(id)).is_some_and(|m| m.media_type.contains("css"))).cloned().collect();
            for r in &css_refs {
                referenced.remove(r);
            }
            for zp in &css_refs {
                if let Some(id) = href_to_id.get(zp) {
                    ids_to_keep.insert(id.clone());
                }
                files_to_include.insert(zp.clone());
                match read_member(&mut zin, zp) {
                    Ok(bytes) => {
                        let css = String::from_utf8_lossy(&bytes).into_owned();
                        let css_dir = posix_dirname(zp);
                        for m in css_url_pattern().captures_iter(&css) {
                            if !m[1].starts_with("data:") {
                                let clean = clean_href(&m[1]);
                                if !clean.is_empty() {
                                    referenced.insert(zip_path(&css_dir, &clean));
                                }
                            }
                        }
                    }
                    Err(_) => log::warn!("EPUB preview: CSS file missing or unreadable in archive: {zp}"),
                }
            }
            for ref_path in &referenced {
                if let Some(id) = href_to_id.get(ref_path) {
                    files_to_include.insert(ref_path.clone());
                    ids_to_keep.insert(id.clone());
                }
            }
        }

        // 대용량 폰트 제거(전체보기와 미리보기 공통)
        let mut excluded_fonts: HashSet<String> = HashSet::new();
        for entry in &manifest {
            let ref_path = zip_path(&opf_dir, &clean_href(&entry.href));
            let lower = ref_path.to_lowercase();
            if (FONT_EXTENSIONS.iter().any(|e| lower.ends_with(e)) || FONT_MEDIA_TYPES.contains(&entry.media_type.as_str()))
                && let Ok(font) = zin.by_name(&ref_path)
                && font.size() > FONT_SIZE_LIMIT
            {
                log::debug!("EPUB preview: skipping large font {ref_path} ({} bytes)", font.size());
                files_to_include.remove(&ref_path);
                ids_to_keep.remove(&entry.id);
                excluded_fonts.insert(ref_path);
            }
        }

        // OPF 수정: manifest에서 불필요한 항목 제거
        if let Some(manifest_el) = find_el(&doc, OPF_NS, "manifest") {
            for item in children_named(manifest_el, OPF_NS, "item") {
                if !ids_to_keep.contains(item.attribute("id").unwrap_or("")) {
                    edits.push(Edit::remove(item.range()));
                }
            }
        }
        // 미리보기(부분 챕터) 모드일 때만 spine, guide를 고친다.
        if !is_full_view {
            for itemref in &spine_refs {
                if !chapter_idrefs.iter().any(|id| Some(id.as_str()) == itemref.attribute("idref")) {
                    edits.push(Edit::remove(itemref.range()));
                }
            }
            if let Some(guide) = find_el(&doc, OPF_NS, "guide") {
                for reference in children_named(guide, OPF_NS, "reference") {
                    let ref_zp = zip_path(&opf_dir, &clean_href(reference.attribute("href").unwrap_or("")));
                    if !files_to_include.contains(&ref_zp) {
                        edits.push(Edit::remove(reference.range()));
                    }
                }
            }
        }
        let modified_opf = format!("{XML_DECLARATION}{}", apply_edits(&opf_text, root_range, edits));
        let ncx_zp = item_of(&toc_id).filter(|_| !toc_id.is_empty()).map(|m| zip_path(&opf_dir, &clean_href(&m.href)));

        let seams = self.seams();
        let out_file = std::fs::File::create(cache_file).map_err(|e| e.to_string())?;
        let mut zout = zip::ZipWriter::new(out_file);
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zout.start_file("mimetype", stored).map_err(|e| e.to_string())?;
        zout.write_all(b"application/epub+zip").map_err(|e| e.to_string())?;
        for zp in &files_to_include {
            if zp == "mimetype" {
                continue;
            }
            let data: Vec<u8> = if *zp == opf_path {
                modified_opf.clone().into_bytes()
            } else if Some(zp) == ncx_zp.as_ref() && !is_full_view {
                // NCX 파일: 미리보기에 없는 파일을 가리키는 navPoint를 지운다.
                match read_member(&mut zin, zp) {
                    Ok(raw) => filter_ncx(&raw, zp, &files_to_include).unwrap_or(raw),
                    Err(e) => {
                        log::warn!("EPUB preview: missing or unreadable file in archive: {zp} ({e})");
                        continue;
                    }
                }
            } else {
                let mut data = match read_member(&mut zin, zp) {
                    Ok(data) => data,
                    Err(e) => {
                        log::warn!("EPUB preview: missing or unreadable file in archive: {zp} ({e})");
                        continue;
                    }
                };
                let lower = zp.to_lowercase();
                if zp.ends_with(".css") && !excluded_fonts.is_empty() {
                    let css = String::from_utf8_lossy(&data).into_owned();
                    let css_dir = posix_dirname(zp);
                    let stripped = font_face_pattern().replace_all(&css, |m: &regex::Captures| {
                        let missing = css_url_pattern().captures_iter(&m[0]).any(|u| !files_to_include.contains(&zip_path(&css_dir, &clean_href(&u[1]))));
                        if missing { String::new() } else { m[0].to_string() }
                    });
                    data = stripped.into_owned().into_bytes();
                } else if XHTML_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
                    data = expand_self_closing_rcdata(&data);
                } else if EPUB_IMAGE_SUFFIXES.iter().any(|s| lower.ends_with(s)) && data.len() > seams.epub_image_min_bytes.unwrap_or(EPUB_IMAGE_MIN_BYTES) {
                    data = match &seams.downscale_hook {
                        Some(hook) => hook(&data),
                        None => Self::_downscale_epub_image(&data),
                    };
                }
                data
            };
            zout.start_file(zp.as_str(), deflated).map_err(|e| e.to_string())?;
            zout.write_all(&data).map_err(|e| e.to_string())?;
        }
        zout.finish().map_err(|e| e.to_string())?;
        Ok(chapter_idrefs.len())
    }

    /// PDF 쪽 추출 실패 응답. 저장소 오류(`OSError`)는 503, 그 밖은 500이다. 원인을 알 수 있게 오류 종류와 메시지를 본문에 담는다.
    fn pdf_pages_failure(&self, book_id: i64, path: &Path, e: &crate::pdf::PdfError) -> BookResponse {
        log::error!("PDF pages extraction failed for book_id={book_id} ({}): {e}", path.display());
        if e.kind == "OSError" {
            // "[Errno 28] No space left on device"에서 사람이 읽을 부분만 쓴다.
            let reason = e.message.split_once("] ").map_or(e.message.as_str(), |(_, rest)| rest);
            return text_response(503, format!("Storage access error ({reason}): {}", path.display()));
        }
        text_response(500, format!("PDF pages extraction failed ({}): {}", e.kind, e.message))
    }

    /// PDF에서 `start`~`end`쪽(1부터, 양끝 포함)만 추출해 돌려준다. 한 번에 `MAX_PDF_PAGE_RANGE`쪽까지다.
    pub async fn get_pdf_pages(&self, book_id: i64, start: i64, end: i64) -> BookResponse {
        log::debug!("# get_pdf_pages(book_id={book_id}, start={start}, end={end})");
        let doc = match self.find_doc(book_id) {
            Ok(Some(doc)) => doc,
            Ok(None) => return text_response(404, format!("Book not found: {book_id}")),
            Err(e) => return text_response(500, format!("PDF pages extraction failed (RuntimeError): {e}")),
        };
        let book = match self.try_make_book(book_id, &doc, 0.0) {
            Ok(book) => book,
            Err(e) => return text_response(500, format!("Book conversion failed: {e}")),
        };
        // stat() 자체가 실패할 수 있다(예: 스토리지 마운트가 끊겨 EIO). 잡지 않으면 CORS 헤더 없는 500이 나가 브라우저에는 "Failed to fetch"로만 보인다.
        match self.fs.is_file(&book.file_path) {
            Err(e) => {
                log::error!("PDF file access failed for book_id={book_id} ({}): {e}", book.file_path.display());
                return text_response(503, format!("Storage access error ({}): {}", super::util::strerror(&e), book.file_path.display()));
            }
            Ok(false) => return text_response(404, format!("File not found: {}", book.file_path.display())),
            Ok(true) => {}
        }
        if book.file_path.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("pdf".to_string()) {
            return text_response(400, format!("Not a PDF file: {}", book.file_path.display()));
        }

        // 페이지 수만 먼저 구한다. 디스크 캐시 히트면 페이지 추출이 필요 없으므로 리더를 만들 필요가 없다.
        let (total_pages, reader) = match self.try_get_cached_page_count(&book.file_path) {
            Ok(result) => result,
            Err(e) => return self.pdf_pages_failure(book_id, &book.file_path, &e),
        };
        // 범위 검증
        let start = start.max(1);
        let end = end.min(total_pages);
        if start > total_pages {
            return text_response(400, format!("Start page {start} exceeds total pages {total_pages}"));
        }
        // 범위마다 캐시 파일이 생기므로 거꾸로 된 범위와 너무 넓은 범위는 받지 않는다.
        if end < start {
            return text_response(400, format!("End page {end} is before start page {start}"));
        }
        if end - start + 1 > MAX_PDF_PAGE_RANGE {
            return text_response(400, format!("Page range exceeds {MAX_PDF_PAGE_RANGE} pages"));
        }

        let cache_dir = self.path_prefix.join(".preview_cache");
        let storage_error = |e: &std::io::Error| text_response(503, format!("Storage access error ({}): {}", super::util::strerror(e), book.file_path.display()));
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            return storage_error(&e);
        }
        let cache_file = cache_dir.join(format!("{book_id}_p{start}-{end}.pdf"));
        let original_mtime = match self.fs.metadata(&book.file_path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(e) => return storage_error(&e),
        };
        let mut headers = no_transform_headers();
        headers.push(("X-Total-Pages".to_string(), total_pages.to_string()));
        let pdf_response = |body: Vec<u8>| BookResponse { status_code: 200, media_type: Some("application/pdf".to_string()), headers: headers.clone(), body };

        if self.cache_is_fresh(&cache_file, original_mtime)
            && let Ok(bytes) = std::fs::read(&cache_file)
        {
            log::debug!("PDF pages cache hit for book_id={book_id} (p{start}-{end})");
            return pdf_response(bytes);
        }

        // 쪽 추출. Python `range(start - 1, end)`와 같다.
        let reader = match reader {
            Some(reader) => reader,
            None => match self.try_get_cached_pdf_reader(&book.file_path) {
                Ok((reader, _)) => reader,
                Err(e) => return self.pdf_pages_failure(book_id, &book.file_path, &e),
            },
        };
        let pdf_bytes = match self.pdf_pages.extract_pages_from_reader(&reader, (start - 1) as usize, end.max(0) as usize) {
            Ok(bytes) => bytes,
            Err(e) => return self.pdf_pages_failure(book_id, &book.file_path, &e),
        };
        if let Err(e) = self.write_cache(&cache_file, &pdf_bytes) {
            log::error!("PDF pages extraction failed for book_id={book_id} ({}): {e}", book.file_path.display());
            return storage_error(&e);
        }
        Self::_evict_old_cache_with(&*self.fs, &cache_dir);
        log::debug!("PDF pages extracted for book_id={book_id} (p{start}-{end}, total={total_pages})");
        pdf_response(pdf_bytes)
    }
}

/// 미리보기에 없는 파일을 가리키는 NCX navPoint를 지운다(자식도 함께). 파싱할 수 없으면 `None`.
fn filter_ncx(raw: &[u8], zp: &str, files_to_include: &BTreeSet<String>) -> Option<Vec<u8>> {
    let text = xml_text(raw);
    let doc = parse_xml(&text).ok()?;
    let ncx_dir = posix_dirname(zp);
    let mut edits: Vec<Edit> = Vec::new();
    for nav in doc.descendants().filter(|n| n.is_element() && n.tag_name().namespace() == Some(NCX_NS) && n.tag_name().name() == "navPoint") {
        if let Some(content) = children_named(nav, NCX_NS, "content").next() {
            let src = clean_href(content.attribute("src").unwrap_or(""));
            if !src.is_empty() && !files_to_include.contains(&zip_path(&ncx_dir, &src)) {
                edits.push(Edit::remove(nav.range()));
            }
        }
    }
    Some(format!("{XML_DECLARATION}{}", apply_edits(&text, doc.root_element().range(), edits)).into_bytes())
}

#[cfg(test)]
#[path = "../../../tests/rust/backend/book_manager_preview_unit.rs"]
mod tests;
