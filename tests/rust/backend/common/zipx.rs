//! ZIP 쓰기·읽기·재작성 도구. book_classifier, epub_image_edit, book_manager_1·2·gaps, loader_1, epub_line_count, book_preview,
//! cover_fixtures에 흩어져 있던 `write_zip`/`read_zip_entry`/`write_epub`/`rewrite_epub_members`를 하나로 모았다.
//! 설계: docs/rust-test-seams.md 5장.
//!
//! 사용: `use common::zipx::{write_zip, read_zip_entry, ZipMode};`

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::Path;

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// 멤버 압축 방식. Python `zipfile.ZipFile(path, "w")`의 기본은 STORED, `writestr`에 ZIP_DEFLATED를 주면 압축한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZipMode {
    /// 모든 멤버를 저장만 한다.
    Stored,
    /// 모든 멤버를 압축한다.
    Deflated,
    /// `mimetype`만 저장하고 나머지는 압축한다. 실제 EPUB 규칙이다.
    EpubStyle,
}

impl ZipMode {
    fn method(self, name: &str) -> CompressionMethod {
        match self {
            ZipMode::Stored => CompressionMethod::Stored,
            ZipMode::Deflated => CompressionMethod::Deflated,
            ZipMode::EpubStyle if name == "mimetype" => CompressionMethod::Stored,
            ZipMode::EpubStyle => CompressionMethod::Deflated,
        }
    }
}

fn options(method: CompressionMethod) -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(method)
}

/// 멤버를 순서대로 담아 ZIP을 만든다. 부모 디렉터리가 없으면 만든다.
pub fn write_zip_with(path: &Path, entries: &[(&str, &[u8])], mode: ZipMode) {
    write_entries(path, entries, mode);
}

fn write_entries<N: AsRef<str>, D: AsRef<[u8]>>(path: &Path, entries: &[(N, D)], mode: ZipMode) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in entries {
        zip.start_file(name.as_ref(), options(mode.method(name.as_ref()))).unwrap();
        zip.write_all(data.as_ref()).unwrap();
    }
    zip.finish().unwrap();
}

/// `mimetype`만 저장하고 나머지를 압축한 ZIP. 가장 많이 쓰는 형태다.
pub fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    write_zip_with(path, entries, ZipMode::EpubStyle);
}

/// 텍스트 멤버만 있는 ZIP. `deflate`가 참이면 압축, 아니면 저장만 한다. book_classifier의 `write_zip(path, members, deflate)`.
pub fn write_zip_text(path: &Path, members: &[(&str, &str)], deflate: bool) {
    write_entries(path, members, if deflate { ZipMode::Deflated } else { ZipMode::Stored });
}

/// 멤버가 `(String, Vec<u8>)`로 이미 만들어져 있을 때. book_preview의 `write_epub`.
pub fn write_zip_owned(path: &Path, entries: &[(String, Vec<u8>)], mode: ZipMode) {
    write_entries(path, entries, mode);
}

/// ZIP 멤버 하나를 읽는다. Python `archive.read(name)`.
pub fn read_zip_entry(path: &Path, name: &str) -> Vec<u8> {
    let mut archive = open_zip(path);
    let mut file = archive.by_name(name).unwrap_or_else(|_| panic!("멤버 없음: {name}"));
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    data
}

pub fn read_zip_entry_string(path: &Path, name: &str) -> String {
    String::from_utf8(read_zip_entry(path, name)).expect("UTF-8 멤버")
}

pub fn open_zip(path: &Path) -> zip::ZipArchive<std::fs::File> {
    zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap()
}

/// ZIP 멤버 이름을 저장 순서대로 돌려준다. Python `archive.namelist()`.
pub fn member_names(path: &Path) -> Vec<String> {
    let mut archive = open_zip(path);
    (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().expect("zip 항목 이름").to_string()).collect()
}

pub fn has_member(path: &Path, name: &str) -> bool {
    open_zip(path).by_name(name).is_ok()
}

/// `ZipFile.getinfo(name).header_offset`.
pub fn zip_header_offset(path: &Path, name: &str) -> u64 {
    open_zip(path).by_name(name).unwrap().header_start()
}

/// Python `archive.testzip() is None`: 모든 멤버를 끝까지 읽어 CRC를 확인한다.
pub fn testzip_ok(path: &Path) -> bool {
    let mut archive = match std::fs::File::open(path).ok().and_then(|f| zip::ZipArchive::new(f).ok()) {
        Some(a) => a,
        None => return false,
    };
    for i in 0..archive.len() {
        let mut file = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut sink = Vec::new();
        if file.read_to_end(&mut sink).is_err() {
            return false;
        }
    }
    true
}

/// 바이트열 안의 `from`을 모두 `to`로 바꾼다. Python `bytes.replace`.
pub fn bytes_replace(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if !from.is_empty() && data[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// ZIP 멤버를 읽은 순서대로 담는 dict. Python `rewrite_epub_members`의 `members`.
/// 있는 이름은 제자리에서 바꾸고 새 이름은 뒤에 붙는다.
pub struct Members {
    items: Vec<(String, Vec<u8>)>,
}

impl Members {
    pub fn new(items: Vec<(String, Vec<u8>)>) -> Self {
        Self { items }
    }

    /// 디렉터리 항목은 건너뛰고 파일 멤버를 모두 읽는다.
    pub fn read(path: &Path) -> Self {
        let mut archive = open_zip(path);
        let mut items = Vec::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).unwrap();
            if file.is_dir() {
                continue;
            }
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            items.push((file.name().expect("zip 항목 이름").to_string(), data));
        }
        Self { items }
    }

    pub fn names(&self) -> Vec<String> {
        self.items.iter().map(|(n, _)| n.clone()).collect()
    }

    pub fn get(&self, name: &str) -> &[u8] {
        &self.items.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("멤버 없음: {name}")).1
    }

    pub fn contains(&self, name: &str) -> bool {
        self.items.iter().any(|(n, _)| n == name)
    }

    /// 있으면 덮어쓰고 없으면 뒤에 붙인다. Python `members[name] = data`.
    pub fn set(&mut self, name: &str, data: impl Into<Vec<u8>>) {
        let data = data.into();
        match self.items.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = data,
            None => self.items.push((name.to_string(), data)),
        }
    }

    /// Python `members.pop(name)` / `del members[name]`.
    pub fn remove(&mut self, name: &str) -> Vec<u8> {
        let at = self.items.iter().position(|(n, _)| n == name).unwrap_or_else(|| panic!("멤버 없음: {name}"));
        self.items.remove(at).1
    }

    /// 멤버 안의 바이트열을 바꿔 다시 넣는다. Python `members[name] = members[name].replace(a, b)`.
    pub fn replace_in(&mut self, name: &str, from: &[u8], to: &[u8]) {
        let replaced = bytes_replace(self.get(name), from, to);
        self.set(name, replaced);
    }

    /// 담은 멤버로 ZIP을 쓴다.
    pub fn write(&self, path: &Path, mode: ZipMode) {
        write_entries(path, &self.items, mode);
    }
}

/// 작은 fixture의 멤버를 중복 entry 없이 바꾼다. `mimetype`만 STORED, 나머지는 DEFLATED다.
pub fn rewrite_epub_members(path: &Path, transform: impl FnOnce(&mut Members)) {
    let mut members = Members::read(path);
    transform(&mut members);
    members.write(path, ZipMode::EpubStyle);
}
