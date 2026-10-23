//! 커버 목록과 표지 썸네일 응답. Python `get_cover_books_paged`, `get_cover`, `_extract_cover_thumbnail`, `normalize_cover_width`에 대응한다.
//!
//! TODO(cover): `get_uncovered_books_paged`(정식 표지가 없는 책만)는 `CoverStatusCache`가 없어 아직 없다.

use std::collections::HashMap;
use std::io::Cursor;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use image::{DynamicImage, RgbImage};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::preview::text_response;
use super::{BookManager, BookResponse, MAX_CATEGORY_PAGE_SIZE};
use crate::book::Book;
use crate::cover_browse::{CoverEntry, CoverGroup, CoverStats, MISSING_TIME, latest_rank, merge_cover_page, pick_representative, plan_cover_scope};
use crate::es_manager::{decode_category_cursor, encode_category_cursor};

/// `get_cover_books_paged` 입력.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoverPageOpts {
    /// 비어 있거나 `None`이면 라이브러리 루트다.
    pub category: Option<String>,
    pub size: usize,
    pub cursor: Option<String>,
    pub exclude_categories: Vec<String>,
}

/// 커서 한 칸(`[수정, 생성, 경로]`)이 맞는 모양인가. `null`이거나 세 칸 배열이어야 한다.
fn cursor_member(value: &Value) -> Option<Option<Vec<Value>>> {
    match value {
        Value::Null => Some(None),
        Value::Array(list) if list.len() == 3 => Some(Some(list.clone())),
        _ => None,
    }
}

/// 정렬값(JSON)을 ES 검색 인자 `(수정, 생성, 경로)`로 바꾼다. 비어 있으면(null) ES가 값 없는 문서에 주는 `i64::MIN`이다.
fn search_after_triple(values: &[Value]) -> (i64, i64, String) {
    let time = |v: &Value| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(i64::MIN);
    (time(&values[0]), time(&values[1]), values[2].as_str().map_or_else(|| values[2].to_string(), str::to_string))
}

impl BookManager {
    pub const COVER_THUMBNAIL_WIDTHS: [u32; 2] = [150, 300];
    pub const COVER_THUMBNAIL_WIDTH: u32 = 300;
    pub const COVER_JPEG_QUALITY: u8 = 85;
    pub const COVER_CACHE_VERSION: u32 = 8;
    pub const COVER_ASPECT_RATIO: f64 = 2.0 / 3.0;
    pub const COVER_TALL_RATIO: f64 = 2.0;
    pub const COVER_BACKGROUND_RATIO_THRESHOLD: f64 = 0.60;
    pub const COVER_DETAIL_RATIO_THRESHOLD: f64 = 0.15;

    /// 허용한 썸네일 폭만 통과시킨다. 임의 값이 캐시 파일을 무한히 늘리지 못하게 한다.
    pub fn normalize_cover_width(width: Option<i64>) -> u32 {
        match width {
            Some(w) if Self::COVER_THUMBNAIL_WIDTHS.iter().any(|&allowed| i64::from(allowed) == w) => w as u32,
            _ => Self::COVER_THUMBNAIL_WIDTH,
        }
    }

    /// EPUB 표지 멤버 찾기(`_find_epub_cover_member`).
    pub fn _find_epub_cover_member<R: std::io::Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> Option<String> {
        Self::find_epub_cover_member(archive)
    }

    /// EPUB 표지 이미지나 PDF 앞쪽 표지 쪽을 JPEG 썸네일로 만든다. 만들 수 없으면 `None`이다.
    pub fn _extract_cover_thumbnail(file_path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        let width = Self::normalize_cover_width(width.map(i64::from));
        match file_type {
            "epub" => crate::epub_cover::extract_epub_thumbnail(file_path, Some(width)),
            "pdf" => Self::render_pdf_cover(file_path, width).and_then(|img| Self::encode_cover_jpeg(&img, width)),
            _ => None,
        }
    }

    /// `thumbnail((width, width * 10))`: 상자 안에 비율을 지켜 줄이고(키우지 않는다) JPEG로 쓴다.
    fn encode_cover_jpeg(img: &RgbImage, width: u32) -> Option<Vec<u8>> {
        let (w, h) = (img.width(), img.height());
        let (bound_w, bound_h) = (width, width * 10);
        let resized = if w <= bound_w && h <= bound_h {
            DynamicImage::ImageRgb8(img.clone())
        } else {
            let scale = (f64::from(bound_w) / f64::from(w)).min(f64::from(bound_h) / f64::from(h));
            let (nw, nh) = (((f64::from(w) * scale).round() as u32).max(1), ((f64::from(h) * scale).round() as u32).max(1));
            DynamicImage::ImageRgb8(image::imageops::resize(img, nw, nh, image::imageops::FilterType::Lanczos3))
        };
        let mut out = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut out), Self::COVER_JPEG_QUALITY);
        resized.write_with_encoder(encoder).ok()?;
        Some(out)
    }

    /// EPUB 커버나 PDF 첫 쪽을 JPEG 썸네일로 돌려준다. 표지가 없거나 만들 수 없으면 404다.
    pub async fn get_cover(&self, book_id: i64, width: Option<i64>) -> BookResponse {
        let width = Self::normalize_cover_width(width);
        log::debug!("# get_cover(book_id={book_id}, width={width})");
        let doc = match self.find_doc_async(book_id).await {
            Ok(Some(doc)) => doc,
            Ok(None) => return text_response(404, format!("Book not found: {book_id}")),
            Err(e) => return text_response(500, format!("Book lookup failed: {e}")),
        };
        let book = self.make_book(book_id, &doc, 0.0);
        if book.file_type != "epub" && book.file_type != "pdf" {
            return text_response(404, format!("Cover not supported for type: {}", book.file_type));
        }
        let source = match self.fs.is_file(&book.file_path).and_then(|is_file| if is_file { self.fs.metadata(&book.file_path).map(Some) } else { Ok(None) }) {
            Ok(Some(meta)) => meta,
            Ok(None) => return text_response(404, format!("File not found: {}", book.file_path.display())),
            Err(e) => {
                log::error!("Cover file access failed for book_id={book_id} ({}): {e}", book.file_path.display());
                return text_response(503, format!("Storage access error ({}): {}", super::util::strerror(&e), book.file_path.display()));
            }
        };

        let headers = vec![("Cache-Control".to_string(), "private, max-age=86400".to_string())];
        let jpeg = |body: Vec<u8>| BookResponse { status_code: 200, media_type: Some("image/jpeg".to_string()), headers: headers.clone(), body };
        // book_id는 재사용 가능한 inode다. 경로와 원본 revision까지 구분해 다른 책의 표지를 피한다. ctime은 mtime을 보존한 덮어쓰기도 잡는다.
        let source_key = format!(
            "[{}, {}, {}, {}, {}, {}, {}]",
            serde_json::to_string(&book.file_path.to_string_lossy()).unwrap_or_default(),
            serde_json::to_string(&book.file_type).unwrap_or_default(),
            source.dev(),
            source.ino(),
            source.size(),
            source.mtime() * 1_000_000_000 + source.mtime_nsec(),
            source.ctime() * 1_000_000_000 + source.ctime_nsec()
        );
        let digest = Sha256::digest(source_key.as_bytes());
        let revision: String = digest.iter().map(|b| format!("{b:02x}")).collect::<String>().chars().take(32).collect();
        let cache_dir = self.path_prefix.join(".cover_cache");
        let cache_file = cache_dir.join(format!("{book_id}.v{}.w{width}.{revision}.jpg", Self::COVER_CACHE_VERSION));
        if let Ok(bytes) = std::fs::read(&cache_file) {
            return jpeg(bytes);
        }

        let (path, file_type) = (book.file_path.clone(), book.file_type.clone());
        let data = tokio::task::spawn_blocking(move || Self::_extract_cover_thumbnail(&path, &file_type, Some(width))).await.unwrap_or_else(|e| {
            // 손상 파일 등은 여기로 온다. 화면은 포맷명 상자로 대체한다.
            log::warn!("Cover extraction failed for book_id={book_id} ({}): {e}", book.file_path.display());
            None
        });
        let Some(data) = data else { return text_response(404, format!("Cover not found: {book_id}")) };
        if std::fs::create_dir_all(&cache_dir).and_then(|()| std::fs::write(&cache_file, &data)).is_err() {
            log::warn!("Cover cache write failed: {}", cache_file.display());
        }
        jpeg(data)
    }

    /// 커버 목록용 책을 최신순으로 한 페이지 조회한다. epub, pdf만 대상이다. 작품 디렉터리는 대표 한 권으로 묶는다.
    /// 반환: (항목 [(대표 책, 작품 디렉터리 이름 또는 ""))], 전체 항목 수, 다음 커서, 오류).
    pub async fn get_cover_books_paged(&self, opts: CoverPageOpts) -> (Vec<(Book, String)>, usize, Option<String>, Option<String>) {
        let invalid = || (Vec::new(), 0, None, Some("invalid cursor".to_string()));
        let category = opts.category.clone().unwrap_or_default();
        let cursor = opts.cursor.clone().unwrap_or_default();
        log::debug!("# get_cover_books_paged(category='{category}', size={}, cursor='{cursor}', exclude_categories={:?})", opts.size, opts.exclude_categories);
        let size = opts.size.clamp(1, MAX_CATEGORY_PAGE_SIZE as usize);
        let (mut group_after, mut file_after): (Option<Vec<Value>>, Option<Vec<Value>>) = (None, None);
        if !cursor.is_empty() {
            let Some(Value::Array(decoded)) = decode_category_cursor(&cursor).map(Value::Array) else { return invalid() };
            let [group, file] = &decoded[..] else { return invalid() };
            let (Some(g), Some(f)) = (cursor_member(group), cursor_member(file)) else { return invalid() };
            (group_after, file_after) = (g, f);
        }
        let excluded = opts.exclude_categories.clone();

        let stats_result = self.blocking(|es| es.aggregate_cover_categories()).await;
        let stats: CoverStats = match stats_result {
            Ok(stats) => stats
                .into_iter()
                .map(|(name, (count, updated, created))| {
                    let optional = |v: f64| (v > MISSING_TIME).then_some(v);
                    (name, (count as u64, optional(updated), optional(created)))
                })
                .collect(),
            Err(e) => return (Vec::new(), 0, None, Some(e.to_string())),
        };
        let (file_categories, groups) = plan_cover_scope(&stats, &category, &excluded);
        let remaining: Vec<CoverGroup> = match &group_after {
            Some(after) => {
                let rank_after = latest_rank(after[0].as_f64(), after[1].as_f64(), after[2].as_str().unwrap_or(""));
                groups.iter().filter(|g| g.rank() > rank_after).cloned().collect()
            }
            None => groups.clone(),
        };
        let (cats, excl, after, take) = (file_categories, excluded.clone(), file_after.as_deref().map(search_after_triple), size as i64);
        let (files, files_total) = match self.blocking(move |es| es.search_cover_files_paged(&cats, take, after, &excl)).await {
            Ok(result) => result,
            Err(e) => return (Vec::new(), 0, None, Some(e.to_string())),
        };
        let page_groups: Vec<CoverGroup> = remaining.iter().take(size).cloned().collect();
        let (entries, groups_used, files_used) = merge_cover_page(&page_groups, &files, size);
        let used_groups: Vec<&CoverGroup> = entries.iter().filter_map(|e| if let CoverEntry::Group(g) = e { Some(g) } else { None }).collect();
        let (keys, excl) = (used_groups.iter().map(|g| g.key.clone()).collect::<Vec<_>>(), excluded.clone());
        let representatives: HashMap<String, Vec<(i64, Value)>> = match self.blocking(move |es| es.search_group_representatives(&keys, &excl)).await {
            Ok(r) => r,
            Err(e) => return (Vec::new(), 0, None, Some(e.to_string())),
        };

        let mut items: Vec<(Book, String)> = Vec::new();
        for entry in &entries {
            match entry {
                CoverEntry::File((book_id, doc, _sort)) => items.push((self.make_book(*book_id, doc, 0.0), String::new())),
                CoverEntry::Group(group) => {
                    if let Some((book_id, doc)) = pick_representative(representatives.get(&group.key).map_or(&[][..], Vec::as_slice)) {
                        items.push((self.make_book(book_id, &doc, 0.0), group.name.clone()));
                    }
                }
            }
        }
        let next_group_after = used_groups.last().map(|g| Value::Array(g.cursor_values())).or_else(|| group_after.clone().map(Value::Array)).unwrap_or(Value::Null);
        let next_file_after = if files_used > 0 { Value::Array(files[files_used - 1].2.clone()) } else { file_after.clone().map(Value::Array).unwrap_or(Value::Null) };
        let has_more = remaining.len() > groups_used || files_used < files.len() || files.len() == size;
        let next_cursor = has_more.then(|| encode_category_cursor(&[next_group_after, next_file_after]));
        (items, groups.len() + files_total.max(0) as usize, next_cursor, None)
    }
}
