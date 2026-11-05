//! 문서 묶음(`Docs`)을 돌려주는 `FileReader` 대역. 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use super::es_fake::Docs;
use book_manager::FileReader;
use std::path::Path;
use std::sync::Arc;

/// `Loader.read_file` 대역. 클로저 하나가 경로와 `path_prefix`를 받아 문서 묶음을 돌려준다.
pub struct FnFileReader {
    pub f: ReadFileFn,
}

impl FnFileReader {
    pub fn new(f: impl Fn(&Path, Option<&Path>) -> Docs + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { f: Box::new(f) })
    }
}

impl FileReader for FnFileReader {
    fn read_file(&self, path: &Path, path_prefix: Option<&Path>) -> Docs {
        (self.f)(path, path_prefix)
    }
}

pub type ReadFileFn = Box<dyn Fn(&Path, Option<&Path>) -> Docs + Send + Sync>;
