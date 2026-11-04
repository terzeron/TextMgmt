//! `backend::epub_cover::inplace_replacer` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;

#[path = "../common/cover_fixtures.rs"]
mod cover_fixtures;
use std::fs::File;
use std::path::{Path, PathBuf};
use backend::book_manager::BookManager;
use backend::epub_cover;
use crate::common::{epub::BODY, epub::count_elements, epub::EpubSpec, epub::make_epub, fsx::ino_mtime, image::BLUE, image::encode_image, image::red_jpeg, zipx::member_names, zipx::read_zip_entry};
use image::ImageFormat;
use serde_json::Value;

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn backups(root: &Path) -> PathBuf {
    root.join(".cover_backups")
}

/// 현재 `revision`. Python `inspect_epub(path)["revision"]`.
fn rev(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap()["revision"].as_str().unwrap().to_string()
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap()
}

fn opf_of(path: &Path) -> Vec<u8> {
    read_zip_entry(path, "OPS/book.opf")
}

#[test]
fn test_cover_replacement_preserves_content_and_has_one_generated_cover() {
    for version in ["2.0", "3.0"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = make_epub(&root.join("book.epub"), &EpubSpec::cover(version, false));
        let stat = ino_mtime(&path);
        let first = epub_cover::add_cover(&path, &red_jpeg(), &rev(&path), &backups(root), false).unwrap();
        let mut before = read(&path);
        // 600px 축소 경로만 지나면 된다. 크게 만들면 디버그 빌드에서 Lanczos 축소가 수 초씩 걸린다.
        let png = encode_image(900, 1350, BLUE, ImageFormat::Png);
        let mut plates: Vec<usize> = Vec::new();
        let mut result = Value::Null;
        for data in [png.clone(), png.clone(), red_jpeg()] {
            result = epub_cover::add_cover(&path, &data, &rev(&path), &backups(root), true).unwrap();
            assert_eq!(read(Path::new(str_of(&result, "backup_path"))), before, "version={version}");
            assert_eq!(ino_mtime(&path), stat, "version={version}");
            let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
            let member = BookManager::find_epub_cover_member(&mut archive).unwrap();
            drop(archive);
            assert_eq!(read_zip_entry(&path, &member), epub_cover::normalize_image(&data, Some(600)).unwrap(), "version={version}");
            let names = member_names(&path);
            assert_eq!(names.iter().filter(|name| name.contains("tm-cover-")).count(), 2, "version={version}");
            // 표지 페이지와 본문 1쪽에, 교체로 밀려난 이전 표지가 도판 페이지로 남는다.
            let plate_pages = names.iter().filter(|name| name.contains("tm-plate-page-")).count();
            assert_eq!(count_elements(&opf_of(&path), "itemref", None), 2 + plate_pages, "version={version}");
            plates.push(plate_pages);
            assert_eq!(read_zip_entry(&path, "OPS/chapter.xhtml"), BODY.as_bytes(), "version={version}");
            assert_eq!(read_zip_entry(&path, "OPS/style.css"), b"body { color: red; }", "version={version}");
            assert_eq!(names.len(), names.iter().collect::<std::collections::BTreeSet<_>>().len(), "version={version}");
            before = read(&path);
        }
        // 같은 이미지로 다시 교체하면 도판이 늘지 않고, 다른 이미지로 바꿀 때만 늘어난다.
        assert_eq!(plates, [1, 1, 2], "version={version}");
        assert_ne!(first["revision"], result["revision"], "version={version}");
    }
}
