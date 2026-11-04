//! 실행 설정: 환경변수에서 읽는 경로와 명령행 옵션.

use std::path::{Path, PathBuf};

use doc_reader::office::find_libreoffice;

/// loader 실행 설정. 원본의 `Loader` 클래스 속성에 해당한다.
#[derive(Debug, Clone)]
pub struct Config {
    /// `TM_BOOK_DIR`
    pub book_prefix: PathBuf,
    /// `TM_COMICS_DIR`
    pub comics_prefix: PathBuf,
    /// `--reencode` 또는 `--reencode-dry-run`
    pub reencode: bool,
    pub reencode_dry_run: bool,
    pub reencode_backup_dir: Option<PathBuf>,
    pub libreoffice: PathBuf,
}

impl Config {
    pub fn new(book_prefix: impl Into<PathBuf>, comics_prefix: impl Into<PathBuf>) -> Self {
        Config { book_prefix: book_prefix.into(), comics_prefix: comics_prefix.into(), reencode: false, reencode_dry_run: false, reencode_backup_dir: None, libreoffice: find_libreoffice() }
    }

    /// 환경변수에서 만든다. 빠진 변수가 있으면 그 이름을 담은 오류.
    pub fn from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let book = get("TM_BOOK_DIR").ok_or("The environment variable TM_BOOK_DIR is not set.")?;
        let comics = get("TM_COMICS_DIR").ok_or("The environment variable TM_COMICS_DIR is not set.")?;
        Ok(Config::new(book, comics))
    }

    /// 파일 경로에 해당하는 prefix. 만화 디렉토리 아래면 만화 prefix, 아니면 책 prefix.
    pub fn path_prefix(&self, file_path: &Path) -> &Path {
        if file_path.starts_with(&self.comics_prefix) { &self.comics_prefix } else { &self.book_prefix }
    }

    pub fn is_comics(&self, file_path: &Path) -> bool {
        file_path.starts_with(&self.comics_prefix)
    }
}
