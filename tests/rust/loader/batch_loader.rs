//! `loader::batch_loader` 단위 테스트.
#![allow(clippy::type_complexity)]

mod common;
use crate::common::shared_fixtures::{config_with_book, conftest_env, path_str, tmp, write};
use doc_reader::pdf::InProcess;
use file_record::env_config::Config;
use file_record::record_builder::read_file;
use loader::batch_loader::{get_file_list, read_files, report_problems};
use serde_json::Value;
use stat::Stat;
use std::collections::HashMap;
use std::fs;

// ---- TestLoaderInit -------------------------------------------------------------------------

mod loader_init {
    use super::*;

    #[test]
    fn test_loader_class_exists() {
        // Python의 hasattr(Loader, "read_file"/"read_files") 대신, 두 함수가 같은 이름으로 있음을 컴파일 시점에 확인한다.
        let _read_file = read_file;
        let _read_files = read_files;
    }

    #[test]
    fn test_loader_requires_book_dir() {
        // 원본은 import 시점 SystemExit. Rust는 Config::from_env가 빠진 변수 이름을 담은 오류를 돌려준다.
        let get = |name: &str| {
            if name == "TM_COMICS_DIR" { Some("/tmp/comics".to_owned()) } else { None }
        };
        let err = Config::from_env(&get).unwrap_err();
        assert!(err.contains("TM_BOOK_DIR"), "{err}");
    }

    #[test]
    fn test_loader_requires_comics_dir() {
        let get = |name: &str| {
            if name == "TM_BOOK_DIR" { conftest_env(name) } else { None }
        };
        let err = Config::from_env(&get).unwrap_err();
        assert!(err.contains("TM_COMICS_DIR"), "{err}");
    }
}

// ---- TestGetFileList ------------------------------------------------------------------------

mod get_file_list {
    use super::*;

    #[test]
    fn test_single_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("test.txt");
        write(&f, "hello");
        let result = get_file_list(&f, false, None);
        assert_eq!(result, vec![f]);
    }

    #[test]
    fn test_directory() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("a.txt"), "a");
        write(&d.path().join("b.txt"), "b");
        let result = get_file_list(d.path(), false, None);
        assert!(result.len() >= 2, "{result:?}");
    }

    #[test]
    fn test_directory_with_subdirs() {
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("sub");
        fs::create_dir(&sub).unwrap();
        write(&sub.join("c.txt"), "c");
        write(&d.path().join("a.txt"), "a");
        let result = get_file_list(d.path(), false, None);
        // Should include one file from subdir + direct files
        assert!(result.len() >= 2, "{result:?}");
    }

    #[test]
    fn test_directory_recursive() {
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("sub");
        fs::create_dir(&sub).unwrap();
        write(&sub.join("c.txt"), "c");
        write(&d.path().join("a.txt"), "a");
        let result = get_file_list(d.path(), true, None);
        assert!(result.len() >= 2, "{result:?}");
    }

    #[test]
    fn test_directory_num_files_limit() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..5 {
            write(&d.path().join(format!("{i}.txt")), i.to_string());
        }
        let result = get_file_list(d.path(), false, Some(2));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_directory_skips_hidden() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join(".hidden"), "x");
        write(&d.path().join("visible.txt"), "y");
        let hidden_dir = d.path().join(".hidden_dir");
        fs::create_dir(&hidden_dir).unwrap();
        write(&hidden_dir.join("z.txt"), "z");
        let result = get_file_list(d.path(), true, None);
        let names: Vec<String> = result.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(!names.contains(&".hidden".to_owned()), "{names:?}");
        assert!(!names.contains(&"z.txt".to_owned()), "{names:?}");
    }
}

// ---- TestReadFiles / TestGetStat / TestPrintUsage -------------------------------------------

mod read_files {
    use super::*;

    #[test]
    fn test_read_files() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        write(&d.path().join("a.txt"), "hello");
        write(&d.path().join("b.txt"), "world");
        let result: HashMap<i64, Value> = read_files(&cfg, d.path(), &InProcess, &Stat::new());
        assert!(result.len() >= 2, "{}", result.len());
    }
}

mod report_problems {
    use super::*;

    // indexed=False일 때 사유를 만들어 붙이는 분기.

    #[test]
    fn test_unsupported_suffix_is_reported() {
        let dir = tmp();
        let f = dir.path().join("book.xyz");
        fs::write(&f, "x").unwrap();

        let mut out = Vec::new();
        assert!(report_problems(&mut out, &f, &[], false));
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("지원하지 않는 파일 형식 '.xyz'"));
    }

    #[test]
    fn test_vanished_file_is_reported() {
        let dir = tmp();
        let f = dir.path().join("gone.txt"); // 만들지 않는다

        let mut out = Vec::new();
        assert!(report_problems(&mut out, &f, &[], false));
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("파일이 사라짐"));
    }

    #[test]
    fn test_duplicate_path_line_is_dropped() {
        // read_from_pdf 등이 경로를 따로 로깅하므로 경로 중복 줄은 제거한다.
        let dir = tmp();
        let f = dir.path().join("ok.txt");
        fs::write(&f, "x").unwrap();

        let mut out = Vec::new();
        assert!(!report_problems(&mut out, &f, &[path_str(&f)], true));
        assert_eq!(String::from_utf8(out).unwrap(), "");
    }
}

mod loader_module_import_guards {
    #[test]
    fn test_pypdf_read_warning_filter_failure_is_ignored() {
        // pypdf가 PdfReadWarning을 더 이상 노출하지 않아도 import는 살아 있어야 한다 (55-56).
        // Rust에는 import 시점 경고 필터가 없으므로 crate 공개 상수에 접근되는지만 확인한다.
        assert_eq!(loader::batch_loader::BATCH_SIZE, 100);
    }
}
