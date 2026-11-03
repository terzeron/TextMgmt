//! `loader::flow` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

// `tests/rust/loader/loader_1.rs`에서 옮긴 테스트.
mod loader_1 {
    //! 원본: tests/test_loader.py (전반부)
    //!
    //! 이 파일이 가정하는 API (실제 crate에 없는 것만, 이름은 Python 원본을 따른다):
    //! - dev-dependency: `encoding_rs` (cp949/cp1252 바이트 만들기)
    //!
    //! 설계: docs/rust-test-seams.md 3.12, 3.13, 4.2. loader/doc_reader의 `_with`는 `*_with(args, &XxxHooks)` 구조체 하나로 통일했다(loader_2.rs와 같은 이름·필드).
    //!   훅은 모두 `Option<&dyn Fn(..)>` 필드이고 `Default`로 안 쓰는 훅을 생략한다.
    //! - `doc_reader::text::{TextHooks, read_from_text_with(&Path, &TextHooks) -> Result<TextRead, String>,
    //!   reencode_text_file_to_utf8_with(&Path, dry_run, backup_dir, relative, &TextHooks) -> Reencode}`
    //!   `TextHooks { detect (chardet 대체), write_backup (백업 쓰기 대체), read_bytes (원본 읽기와 저장 후 재읽기 대체), .. }`.
    //!   `write_backup: Fn(&Path, &[u8]) -> io::Result<()>`, `read_bytes: Fn(&Path) -> io::Result<Vec<u8>>`. 구 `ReencodeIo`를 흡수한다.
    //! - `doc_reader::pdf::{collect_pages, find_xref_offset, apply_png_predictor, xref_stream_find_entry}`를 `pub`으로 연 것
    //! - `doc_reader::pdf::{FastCountHooks, fast_page_count_with(&Path, &FastCountHooks) -> Option<usize>}` xref 해석 바꿔치기 자리.
    //!   `FastCountHooks { find_xref_offset, parse_one_xref_stream, xref_stream_find_entry, read_from_obj_stream }`. 구 `XrefOps`를 흡수한다
    //! - `doc_reader::pdf::{PdftotextHooks, pdf_text_by_pdftotext_with(&Path, &PdftotextHooks) -> (String, usize)}`
    //!   `PdftotextHooks { which: Fn() -> Option<PathBuf>, run: Fn(&[String]) -> Option<(i32, Vec<u8>)>, path_cache: OnceLock }`. 구 `Pdftotext`/`PdftotextEnv`를 대체한다.
    //! - `doc_reader::epub::{read_from_epub_with_extracting_zip, read_from_epub_with_extracting_zip_using}` zip 직접 추출 경로
    //! - `doc_reader::office::find_libreoffice_with(which, exists)` PATH 조회와 파일 존재 확인 바꿔치기 자리
    //! - `loader::flow::get_file_list(path, recursive, num_files: Option<usize>) -> Vec<PathBuf>`, `loader::flow::read_files(cfg, path, runner, stat) -> HashMap<i64, Value>` 원본의 `Loader.get_file_list` / `Loader.read_files`
    //! - `file_record::record::{ReadHooks, read_file_with_hooks(cfg, path, .., &ReadHooks)}`
    //!   `ReadHooks { fast_page_count (_fast_pdf_page_count 대체), fallback_page_count (pypdf 대체), inspect (_zip_container_type 대체, 구 detect_file_type_with) }`
    //! - `stat::Stat::{pdf_stage_count, pdf_stage_total_time, pdf_stage_outcome_count}` 단계별 통계 조회
    #![allow(clippy::type_complexity)]

    use std::collections::HashMap;
    use std::fs;

    use std::path::{Path, PathBuf};

    use doc_reader::pdf::InProcess;

    use file_record::config::Config;
    use loader::flow::{get_file_list, read_files};
    use file_record::record::read_file;
    use serde_json::Value;
    use stat::Stat;

    // ---- 공통 헬퍼 ------------------------------------------------------------------------------

    /// tests/conftest.py가 정하는 TM_BOOK_DIR / TM_COMICS_DIR 값.
    fn tests_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests")
    }

    /// conftest.py의 환경변수. 이 파일의 테스트는 프로세스 환경을 건드리지 않고 이 함수를 `Config::from_env`에 넘긴다.
    fn conftest_env(name: &str) -> Option<String> {
        match name {
            "TM_BOOK_DIR" => Some(tests_root().join("books").to_string_lossy().into_owned()),
            "TM_COMICS_DIR" => Some(tests_root().join("comics").to_string_lossy().into_owned()),
            _ => None,
        }
    }

    /// `Loader.path_prefix = book` 과 같다. 만화 prefix는 conftest 기본값이다.
    fn config_with_book(book: &Path) -> Config {
        Config::new(book, tests_root().join("comics"))
    }

    fn write(path: &Path, data: impl AsRef<[u8]>) {
        fs::write(path, data).unwrap();
    }

    // ---- TestLoaderInit -------------------------------------------------------------------------

    mod test_loader_init {
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

    mod test_get_file_list {
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

    mod test_read_files {
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
}

// `tests/rust/loader/loader_2.rs`에서 옮긴 테스트.
mod loader_2 {
    //! 원본: tests/test_loader.py (후반부)
    //!
    //! loader 후반부: PDF 경량 쪽수 헬퍼, `main()` 분기, HWP, 재인코딩, PDF 단계 테스트. 외부 서비스 없이 실행한다.
    //! ES는 `IndexStore`를 구현한 가짜 저장소로, LibreOffice는 셸 스크립트로 바꿔 끼운다.
    //! 실패 코드는 Rust crate의 실제 값을 쓴다: 잘못된 사용은 2, 접속·환경 오류는 255, 인자 없음은 도움말과 0이다(원본은 1, -1).
    //! utils.hwp3_parser 단위 테스트(`TestHwp3ParserDirect`, `TestHwp3ParserEdgeCases`, `TestHwp3StreamMethods`, `TestHncToUnicodeExtended`,
    //! `TestHandleControlCharCoverage`, `TestParseEdgeCases`, `TestHandleControlCharRecursive`, `TestSafeChr`, `TestControlCharTab`,
    //! `TestControlCharDateLine`, `TestV200BruteForce`)는 tests/rust/loader/loader_hwp3.rs로 옮겼다.
    //! 69개 중 8개만 tests/rust/hwp3_parser/에 같은 입력·기대값의 테스트가 있어 loader_hwp3.rs에서 뺐다.
    //! `TestBookManagerPreviewEmptyContent`는 backend crate 의존이라 tests/rust/backend/book_manager_hwp_preview.rs로 옮겼다.
    //!
    //! 가정한 API (실제 crate에 없는 것만):
    //! - `doc_reader::pdf::{read_from_obj_stream(&mut File, u64, i64) -> Option<Vec<u8>>,
    //!   parse_one_xref_stream(&mut File, u64) -> Option<(Vec<u8>, [usize; 3], Vec<i64>, Option<i64>)>,
    //!   find_xref_offset(&[u8], i64) -> Option<i64>}` (현재 비공개 함수를 공개)
    //!
    //! 설계: docs/rust-test-seams.md 3.12, 3.13, 4.2. loader_1.rs와 같은 Hooks 이름·필드를 쓴다. 훅은 모두 `Option<&dyn Fn(..)>` 필드이고 `Default` 구현이 있다.
    //! - `doc_reader::pdf::{FastCountHooks, fast_page_count_with(&Path, &FastCountHooks) -> Option<usize>}`:
    //!   `find_xref_offset`, `parse_one_xref_stream`, `xref_stream_find_entry`, `read_from_obj_stream` 대체 훅(`Option<&dyn Fn>`), `Default` 구현
    //! - `doc_reader::pdf::{PlumberDocument, pdf_text_by_plumber_with(&Path, &dyn PlumberDocument) -> Result<(String, usize), String>,
    //!   PdftotextHooks, pdf_text_by_pdftotext_with(&Path, &PdftotextHooks) -> (String, usize)}`
    //!   `PdftotextHooks { which: Fn() -> Option<PathBuf>, run: Fn(&[String]) -> Option<(i32, Vec<u8>)>, path_cache: OnceLock }`
    //! - `doc_reader::text::{TextHooks, Candidate, read_from_text_with(&Path, &TextHooks) -> Result<TextRead, String>,
    //!   reencode_text_file_to_utf8_with(&Path, bool, Option<&Path>, &Path, &TextHooks) -> Reencode}`
    //!   `TextHooks { detect, encodings, read_bytes, read_replace, write_backup }` (loader_1.rs는 detect, read_bytes, write_backup을 쓴다)
    //! - `doc_reader::mojibake::count_mojibake_runs_with(&str, &str) -> usize` (런 정규식 패턴을 받는다)
    //! - `doc_reader::office::read_from_hwp_with(&Path, &Path, &dyn Fn(&Path) -> Result<String, String>) -> (String, usize, Vec<String>)`
    //! - `file_record::record::{ReadHooks, read_file_with_hooks(...)}`: `fast_page_count`, `fallback_page_count`, `inspect` 대체 훅 (`Default` 구현)
    //! - `loader::cli::{RunHooks, run_with_hooks(...)}`: `read_file`, `stat`, `on_worker_spawn` 대체 훅
    #![allow(clippy::type_complexity)]

    use std::fs::{self};

    use std::path::Path;

    use loader::flow::report_problems;

    use tempfile::TempDir;

    // ---- 공통 헬퍼 ----

    fn tmp() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn path_str(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    mod test_report_problems {
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

    mod test_loader_module_import_guards {
        #[test]
        fn test_pypdf_read_warning_filter_failure_is_ignored() {
            // pypdf가 PdfReadWarning을 더 이상 노출하지 않아도 import는 살아 있어야 한다 (55-56).
            // Rust에는 import 시점 경고 필터가 없으므로 crate 공개 상수에 접근되는지만 확인한다.
            assert_eq!(loader::flow::BATCH_SIZE, 100);
        }
    }
}
