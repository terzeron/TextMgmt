//! HTML 책 미리보기와 리소스. Python `_sanitize_html_for_viewer`, `_build_html_resource_url`, `_html_security_headers`,
//! `get_html_resource`, 그리고 `get_book_content`의 HTML 첨부 헤더에 대응한다.
//!
//! 보안: 뷰어에 내보내는 HTML에서 활성 콘텐츠(script, iframe, form 등)와 이벤트 속성, 외부 URL을 모두 걷어낸다.
//! 응답에는 `sandbox` CSP를 붙여 한 번 더 막는다. 읽기 전에 크기 상한을 확인한다.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use scraper::{Html, Node, StrTendril};

use super::preview::{html_security_headers, text_response};
use super::util::posix_normpath;
use super::{BookManager, BookResponse};

/// 뷰어가 내려줄 수 있는 리소스 확장자.
pub const HTML_VIEWER_RESOURCE_EXTENSIONS: [&str; 21] = [".css", ".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".svg", ".tiff", ".ico", ".avif", ".woff", ".woff2", ".ttf", ".otf", ".eot", ".mp3", ".ogg", ".wav", ".mp4", ".webm"];
/// HTML 본문 하나를 읽는 상한(바이트).
pub const MAX_HTML_BYTES: u64 = 32 * 1024 * 1024;
/// HTML 리소스 하나를 읽는 상한(바이트).
pub const MAX_HTML_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;

const REMOVED_TAGS: [&str; 7] = ["script", "iframe", "frame", "object", "embed", "form", "base"];
const REMOVED_ATTRS: [&str; 5] = ["srcdoc", "integrity", "crossorigin", "nonce", "formaction"];
const URL_ATTRS: [&str; 3] = ["src", "href", "poster"];

/// 확장자(점 포함)로 미디어 타입을 찾는다. 없으면 `application/octet-stream`.
pub fn media_type_for_suffix(suffix: &str) -> &'static str {
    match suffix {
        ".txt" => "text/plain",
        ".pdf" => "application/pdf",
        ".epub" => "application/epub+zip",
        ".doc" => "application/msword",
        ".docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".hwp" => "application/x-hwp",
        ".html" => "text/html",
        ".jpg" | ".jpeg" => "image/jpeg",
        ".png" => "image/png",
        ".gif" => "image/gif",
        ".webp" => "image/webp",
        ".bmp" => "image/bmp",
        ".tiff" => "image/tiff",
        ".svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// `urllib.parse.urlparse(url).scheme`. 탭, 줄바꿈 문자는 지우고 앞쪽 제어 문자와 공백은 뗀다. 스킴이 없으면 빈 문자열이다.
fn url_scheme(url: &str) -> String {
    let cleaned: String = url.chars().filter(|c| !matches!(c, '\t' | '\r' | '\n')).collect();
    let cleaned = cleaned.trim_start_matches(|c: char| c <= ' ');
    let Some((head, _)) = cleaned.split_once(':') else { return String::new() };
    let mut chars = head.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if valid { head.to_lowercase() } else { String::new() }
}

/// `urllib.parse.quote(text, safe="")`.
fn quote_all(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl BookManager {
    /// 뷰어 HTML에 붙이는 보안 헤더.
    pub fn _html_security_headers() -> Vec<(String, String)> {
        html_security_headers()
    }

    /// HTML 안의 URL을 뷰어가 안전하게 열 수 있는 주소로 바꾼다. 쪽 안 이동(`#`)과 `data:`, `blob:`은 그대로 두고,
    /// 상대 경로는 리소스 엔드포인트로 돌리고, 외부 주소(스킴이나 `//`로 시작)는 `None`이다.
    pub fn _build_html_resource_url(resource_base_url: &str, raw_path: &str) -> Option<String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.starts_with('#') {
            return Some(trimmed.to_string());
        }
        let scheme = url_scheme(trimmed);
        if scheme == "data" || scheme == "blob" {
            return Some(trimmed.to_string());
        }
        if !scheme.is_empty() || trimmed.starts_with("//") {
            return None;
        }
        Some(format!("{resource_base_url}?path={}", quote_all(trimmed)))
    }

    /// 뷰어에 내보낼 HTML에서 활성 콘텐츠를 걷어내고 리소스 주소를 엔드포인트로 돌린다.
    pub fn _sanitize_html_for_viewer(html_content: &str, resource_base_url: &str) -> String {
        let mut html = Html::parse_document(html_content);

        // 1) 지울 노드: 활성 콘텐츠 태그, http-equiv가 있는 meta, 스타일시트가 아닌 link
        let mut detach = Vec::new();
        for node in html.tree.nodes() {
            let Node::Element(element) = node.value() else { continue };
            let name = element.name().to_ascii_lowercase();
            let has = |attr: &str| element.attrs().any(|(k, v)| k.eq_ignore_ascii_case(attr) && !v.is_empty());
            let remove = REMOVED_TAGS.contains(&name.as_str())
                || (name == "meta" && has("http-equiv"))
                || (name == "link" && !element.attr("rel").is_some_and(|rel| rel.split_whitespace().any(|r| r.eq_ignore_ascii_case("stylesheet"))));
            if remove {
                detach.push(node.id());
            }
        }
        for id in detach {
            if let Some(mut node) = html.tree.get_mut(id) {
                node.detach();
            }
        }

        // 2) 남은 요소: 위험한 속성을 지우고 URL을 고친다. rel 속성 이름표는 같은 파서가 만든 것을 빌려 쓴다.
        let rel_name = Html::parse_fragment("<a rel=\"x\"></a>")
            .tree
            .nodes()
            .find_map(|n| if let Node::Element(e) = n.value() { e.attrs.iter().map(|(k, _)| k.clone()).next() } else { None });
        let ids: Vec<_> = html.tree.nodes().filter(|n| n.value().is_element()).map(|n| n.id()).collect();
        for id in ids {
            let Some(mut node) = html.tree.get_mut(id) else { continue };
            let Node::Element(element) = node.value() else { continue };
            let is_anchor = element.name().eq_ignore_ascii_case("a");
            element.attrs.retain(|(name, _)| {
                let lowered = (*name.local).to_ascii_lowercase();
                !(lowered.starts_with("on") || REMOVED_ATTRS.contains(&lowered.as_str()))
            });
            let mut kept: Vec<(_, StrTendril)> = Vec::new();
            for (name, value) in element.attrs.drain(..) {
                let lowered = (*name.local).to_ascii_lowercase();
                if is_anchor {
                    if lowered == "href" {
                        // 링크는 쪽 안 이동(`#`)만 남긴다.
                        match Self::_build_html_resource_url(resource_base_url, &value) {
                            Some(safe) if safe.starts_with('#') => kept.push((name, StrTendril::from(safe.as_str()))),
                            _ => {}
                        }
                        continue;
                    }
                    if lowered == "rel" {
                        continue;
                    }
                } else if URL_ATTRS.contains(&lowered.as_str()) && !value.is_empty() {
                    if let Some(safe) = Self::_build_html_resource_url(resource_base_url, &value) {
                        kept.push((name, StrTendril::from(safe.as_str())));
                    }
                    continue;
                }
                kept.push((name, value));
            }
            if is_anchor && let Some(rel) = &rel_name {
                kept.push((rel.clone(), StrTendril::from("nofollow noopener noreferrer")));
            }
            element.attrs.extend(kept);
        }
        html.html()
    }

    /// `.html` 책 미리보기. 읽은 HTML을 걷어낸 뒤 보안 헤더와 함께 돌려준다.
    pub(super) fn preview_html(&self, book_id: i64, file_path: &Path, resource_base_url: &str) -> BookResponse {
        let read = || -> std::io::Result<Option<String>> {
            let file = self.fs.open(file_path)?;
            let mut bytes = Vec::new();
            file.take(MAX_HTML_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_HTML_BYTES {
                return Ok(None);
            }
            Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
        };
        match read() {
            Ok(Some(content)) => {
                let base = if resource_base_url.is_empty() { format!("/html-resource/{book_id}") } else { resource_base_url.to_string() };
                BookResponse { status_code: 200, media_type: Some("text/html".to_string()), headers: html_security_headers(), body: Self::_sanitize_html_for_viewer(&content, &base).into_bytes() }
            }
            Ok(None) => text_response(413, "HTML file too large"),
            Err(e) => {
                log::error!("HTML preview generation failed for book_id={book_id}: {e}");
                text_response(500, "HTML preview failed")
            }
        }
    }

    /// HTML 책이 참조하는 로컬 리소스(CSS, 이미지 등)를 돌려준다. HTML 파일이 있는 디렉터리 안의 허용 확장자만 내준다.
    pub async fn get_html_resource(&self, book_id: i64, resource_path: &str) -> BookResponse {
        log::debug!("# get_html_resource(book_id={book_id}, resource_path='{resource_path}')");
        let doc = match self.find_doc_async(book_id).await {
            Ok(Some(doc)) => doc,
            Ok(None) => return text_response(404, format!("Book not found: {book_id}")),
            Err(e) => return text_response(500, format!("Book lookup failed: {e}")),
        };
        let book = self.make_book(book_id, &doc, 0.0);
        if book.file_path.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("html".to_string()) {
            return text_response(400, "HTML resource preview is only supported for HTML files");
        }

        let normalized = posix_normpath(resource_path);
        // `normpath("")`은 "."이다. 비었거나 위로 올라가는 경로는 거부한다.
        if resource_path.is_empty() || normalized == "." || normalized == ".." || normalized.starts_with("../") || resource_path.contains('\0') {
            return text_response(400, "Invalid resource path");
        }
        let html_dir = match book.file_path.parent().map(|p| self.resolve_path(p)) {
            Some(Ok(dir)) => dir,
            _ => return text_response(400, "Invalid resource path"),
        };
        let target = match self.resolve_path(&html_dir.join(&normalized)) {
            Ok(target) => {
                // 경로 포함 검사가 실패하면(경로가 너무 길다 등) 거부한다.
                let inside = match self.seams().path_is_relative_to {
                    Some(check) => check(&target, &html_dir),
                    None => Ok(target.starts_with(&html_dir)),
                };
                if !matches!(inside, Ok(true)) {
                    return text_response(400, "Invalid resource path");
                }
                target
            }
            Err(_) => return text_response(400, "Invalid resource path"),
        };
        if !self.fs.is_file(&target).unwrap_or(false) {
            return text_response(404, format!("Resource not found: {resource_path}"));
        }
        let suffix = target.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
        if !HTML_VIEWER_RESOURCE_EXTENSIONS.contains(&suffix.as_str()) {
            return text_response(400, "Unsupported resource type");
        }
        let body = match self.fs.open(&target).and_then(|f| {
            let mut bytes = Vec::new();
            f.take(MAX_HTML_RESOURCE_BYTES + 1).read_to_end(&mut bytes).map(|_| bytes)
        }) {
            Ok(bytes) if bytes.len() as u64 <= MAX_HTML_RESOURCE_BYTES => bytes,
            Ok(_) => return text_response(413, "Resource too large"),
            Err(e) => return text_response(500, format!("Resource read failed: {e}")),
        };
        BookResponse { status_code: 200, media_type: Some(media_type_for_suffix(&suffix).to_string()), headers: html_security_headers(), body }
    }

    /// `get_book_content`가 돌려준 파일을 내려줄 때 쓸 미디어 타입과 헤더. HTML은 첨부로만 내려 브라우저가 직접 렌더링하지 못하게 한다.
    pub fn file_response_meta(path: &Path) -> (String, Vec<(String, String)>) {
        let suffix = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let media_type = media_type_for_suffix(&suffix).to_string();
        // Content-Encoding: identity는 압축 미들웨어를, no-transform은 외부 프록시의 응답 변환을 막는다.
        let mut headers = vec![("Content-Encoding".to_string(), "identity".to_string()), ("Cache-Control".to_string(), "no-transform".to_string())];
        if suffix.eq_ignore_ascii_case(".html") {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let disposition = if name.is_ascii() && !name.contains('"') && !name.contains('\\') { format!("attachment; filename=\"{name}\"") } else { format!("attachment; filename*=utf-8''{}", quote_all(&name)) };
            headers.push(("Content-Disposition".to_string(), disposition));
        }
        (media_type, headers)
    }

    /// 이 모듈이 본 적 있는 허용 확장자 집합(테스트용).
    pub fn html_viewer_resource_extensions() -> HashSet<&'static str> {
        HTML_VIEWER_RESOURCE_EXTENSIONS.iter().copied().collect()
    }
}
