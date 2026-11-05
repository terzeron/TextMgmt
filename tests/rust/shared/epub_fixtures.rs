//! EPUB·표지 테스트 준비물: 표지 EPUB 만들기, 파일·OPF 읽기, revision, 표지 훅·이미지 받기 대역, 오류 단언.
//! `cover_manager`와 backend 테스트가 함께 쓴다. 포함하는 쪽 `common`에 `epub`, `image`, `zipx`, `es_dummy`가 있어야 한다.

#![allow(dead_code)]

use super::epub::{EpubSpec, make_epub};
use super::es_dummy;
use super::image::{BLUE, color, encode, encode_image, image_size, jpeg_bytes};
use super::zipx::{read_zip_entry, rewrite_epub_members};
use cover_manager::cover_management::{CoverManagement, Thumbnailer};
use cover_manager::direct_cover_url::{FetchError, ImageFetcher};
use cover_manager::epub_cover;
use cover_manager::epub_cover::{CoverError, EditImages, EpubCoverHooks, EpubRewrite, Limits, edit_images};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, RgbaImage};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn assert_refused<T: std::fmt::Debug>(result: Result<T, CoverError>, reason: &str, label: &str) {
    let text = err_of(result);
    assert!(text.contains(reason), "{label}: {text:?} 에 {reason:?}가 없다");
}

pub fn backups(root: &Path) -> PathBuf {
    root.join(".cover_backups")
}

pub fn chapter(index: usize) -> String {
    make_chapter_xhtml(index, false)
}

pub fn edit(path: &Path, trash: &Path, args: EditImages) -> Result<EpubRewrite, CoverError> {
    edit_images(path, &revision(path), trash, EditImages { backup_name: Some("book.epub".to_string()), ..args })
}

pub fn epub_with_cover(root: &Path, width: u32, height: u32) -> PathBuf {
    let path = make_epub(&root.join("book.epub"), &EpubSpec::cover("3.0", true));
    let data = encode_image(width, height, BLUE, ImageFormat::Jpeg);
    rewrite_epub_members(&path, |members| members.set("OPS/cover.jpg", data.clone()));
    path
}

pub fn err_of<T: std::fmt::Debug>(result: Result<T, CoverError>) -> String {
    result.expect_err("CoverError가 나야 한다").to_string()
}

pub fn error_text(error: &Option<String>) -> &str {
    error.as_deref().unwrap_or("")
}

pub fn full() -> Value {
    json!([0, 0, 1, 1, 0])
}

pub fn gif() -> Vec<u8> {
    encode(DynamicImage::ImageRgba8(RgbaImage::new(20, 20)), ImageFormat::Gif)
}

pub fn hooks_with(limits: Limits) -> EpubCoverHooks {
    EpubCoverHooks::default().with_limits(limits)
}

pub fn jpeg(c: &str) -> Vec<u8> {
    jpeg_sized(c, (200, 100))
}

pub fn jpeg_sized(c: &str, size: (u32, u32)) -> Vec<u8> {
    jpeg_bytes(&RgbImage::from_pixel(size.0, size.1, Rgb(color(c))), 75)
}

pub fn make_chapter_xhtml(index: usize, link_css: bool) -> String {
    let css_link = if link_css { r#"<link rel="stylesheet" href="style.css"/>"# } else { "" };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter {index}</title>{css_link}</head>
<body><h1>Chapter {index}</h1><p>Content of chapter {index}.</p></body>
</html>"#
    )
}

pub fn opf_of(path: &Path) -> Vec<u8> {
    read_zip_entry(path, "OPS/book.opf")
}

pub fn png(c: &str) -> Vec<u8> {
    encode(DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 100, Rgb(color(c)))), ImageFormat::Png)
}

pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// 현재 `revision`.
pub fn rev(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap().revision
}

pub fn revision(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap().revision
}

pub fn size(data: &[u8]) -> (u32, u32) {
    image_size(data)
}

/// `Result`를 `(결과, 오류)` 모양으로 펼쳐 단언을 짧게 쓴다.
pub fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

pub fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap()
}

pub fn truthy(value: &Value) -> bool {
    es_dummy::truthy(Some(value))
}

/// 클로저를 `ImageFetcher`로 감싼다.
pub struct FnFetcher<F>(pub F);

impl<F: Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync> ImageFetcher for FnFetcher<F> {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        (self.0)(url)
    }
}

/// 클로저를 `Thumbnailer`로 감싼다.
pub struct FnThumbnailer<F>(pub F);

impl<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync> Thumbnailer for FnThumbnailer<F> {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        (self.0)(path, file_type, width)
    }
}

pub const MULTI_PAGE: &str = concat!(
    "<html>\n  <head>\n    <title>엄마의 말뚝</title>\n  </head>\n    <body><li class=\"chapter\">\n",
    "<p><img alt=\"image\" height=\"597\" src=\"../Images/m1.jpg\" width=\"412\"/></p>\n",
    "<p><img alt=\"image\" height=\"597\" src=\"../Images/m2.jpg\" width=\"412\"/></p>\n",
    "</li></body></html>"
);

pub fn fetcher<F: Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync + 'static>(f: F) -> Arc<dyn ImageFetcher> {
    Arc::new(FnFetcher(f))
}

pub fn thumbnailer<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync + 'static>(f: F) -> Arc<dyn Thumbnailer> {
    Arc::new(FnThumbnailer(f))
}

pub fn broken_cover(tmp: &Path) -> PathBuf {
    let path = make_epub(&tmp.join("book.epub"), &EpubSpec::cover("3.0", true));
    rewrite_epub_members(&path, |members| members.set("OPS/cover.jpg", &b"not a JPEG"[..]));
    path
}

/// 표지 목록 첫 항목의 JSON 모양.
pub fn first_item(service: &CoverManagement, covered: bool) -> Value {
    serde_json::to_value(service.missing("", "", 60, covered).unwrap()).unwrap()["items"][0].clone()
}
