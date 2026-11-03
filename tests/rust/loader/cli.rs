//! `loader::cli` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

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

    use loader::cli::print_usage;

    mod test_print_usage {
        use super::*;

        #[test]
        fn test_print_usage() {
            // 원본의 print_usage는 SystemExit를 일으킨다. Rust의 print_usage는 쓰기만 하고 돌아온다(종료는 `run`의 반환 코드).
            let mut out = Vec::new();
            print_usage("test_program", &mut out);
            let text = String::from_utf8(out).unwrap();
            assert!(text.contains("test_program"), "{text}");
            assert!(text.contains("사용법"), "{text}");
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

    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fs::{self};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use doc_reader::pdf::{
        InProcess, PdftotextHooks, StageRunner,
        pdf_text_by_pdftotext_with,
    };

    use es_client::EsError;
    use es_client::indexing::IndexStore;
    use loader::cli::{RunHooks, World, parse_options, run, run_with_hooks};

    use serde_json::Value;

    use tempfile::TempDir;

    // ---- 공통 헬퍼 ----

    fn tmp() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn path_str(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    /// pypdf로 만든 72x72 빈 한 쪽짜리 PDF 대응.
    fn blank_pdf() -> Vec<u8> {
        let objs = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>"];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in &offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    fn write_blank_pdf(path: &Path) {
        fs::write(path, blank_pdf()).unwrap();
    }

    /// conftest.py의 환경 변수 중 loader가 읽는 것. 색인 이름은 `_setup_loader_env`가 정한 값이다.
    fn env_for(book: &Path, comics: &Path) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("TM_BOOK_DIR".to_owned(), path_str(book));
        env.insert("TM_COMICS_DIR".to_owned(), path_str(comics));
        env.insert("TM_ES_BOOK_INDEX".to_owned(), "test_books".to_owned());
        env.insert("TM_ES_COMICS_INDEX".to_owned(), "test_comics".to_owned());
        env
    }

    /// `_setup_loader_env`: 책 prefix는 root, 만화 prefix는 root/comics.
    fn setup_loader_env(root: &Path) -> HashMap<String, String> {
        env_for(root, &root.join("comics"))
    }

    // ---- ES 가짜 저장소 (MagicMock/ESManager 대체) ----

    #[derive(Default)]
    struct State {
        index_exists: bool,
        exists_error: Option<String>,
        deleted_index: bool,
        existing_paths: Option<Box<dyn Fn(&[i64]) -> HashMap<i64, String> + Send>>,
        created_meta: HashMap<i64, BTreeMap<String, String>>,
        delete_by_paths_result: i64,
        inserted: Vec<Vec<(i64, Value)>>,
        orphan_ids: HashSet<i64>,
        prefix_lookup_error: bool,
        deleted_ids: Option<Vec<i64>>,
    }

    #[derive(Clone)]
    struct FakeStore {
        state: Arc<Mutex<State>>,
        ping_ok: bool,
        name: String,
    }

    // 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
    #[allow(dead_code)]
    impl FakeStore {
        fn new() -> Self {
            FakeStore { state: Arc::new(Mutex::new(State::default())), ping_ok: true, name: "test_books".to_owned() }
        }

        fn update(&self, f: impl FnOnce(&mut State)) {
            f(&mut self.state.lock().unwrap());
        }

        fn set_existing_paths(&self, f: impl Fn(&[i64]) -> HashMap<i64, String> + Send + 'static) {
            self.update(|s| s.existing_paths = Some(Box::new(f)));
        }

        /// insert 호출 전체를 합친 `{inode: 문서}`. 같은 inode는 나중 것이 이긴다.
        fn inserted_docs(&self) -> HashMap<i64, Value> {
            self.state.lock().unwrap().inserted.iter().flatten().cloned().collect()
        }

        fn batch_sizes(&self) -> Vec<usize> {
            self.state.lock().unwrap().inserted.iter().map(Vec::len).collect()
        }
    }

    impl IndexStore for FakeStore {
        fn index_name(&self) -> &str {
            &self.name
        }
        fn ping(&self) -> Result<(), EsError> {
            if self.ping_ok { Ok(()) } else { Err(EsError::Transport("ping 실패".to_owned())) }
        }
        fn index_exists(&self) -> Result<bool, EsError> {
            let s = self.state.lock().unwrap();
            match &s.exists_error {
                Some(msg) => Err(EsError::Transport(msg.clone())),
                None => Ok(s.index_exists),
            }
        }
        fn create_index(&self) -> Result<(), EsError> {
            Ok(())
        }
        fn delete_index(&self) -> Result<(), EsError> {
            self.state.lock().unwrap().deleted_index = true;
            Ok(())
        }
        fn get_existing_paths(&self, doc_ids: &[i64]) -> Result<HashMap<i64, String>, EsError> {
            let s = self.state.lock().unwrap();
            Ok(s.existing_paths.as_ref().map(|f| f(doc_ids)).unwrap_or_default())
        }
        fn get_existing_created_time_metadata(&self, _doc_ids: &[i64]) -> Result<HashMap<i64, BTreeMap<String, String>>, EsError> {
            Ok(self.state.lock().unwrap().created_meta.clone())
        }
        fn delete_by_file_paths(&self, _file_paths: &[String], _exclude_ids: &[i64]) -> Result<i64, EsError> {
            Ok(self.state.lock().unwrap().delete_by_paths_result)
        }
        fn get_doc_ids_by_path_prefix(&self, _path_prefix: &str) -> Result<HashSet<i64>, EsError> {
            // 실제 적재된 inode + 디스크에 없는 orphan
            let s = self.state.lock().unwrap();
            if s.prefix_lookup_error {
                return Err(EsError::Transport("scroll failed".to_owned()));
            }
            Ok(s.inserted.iter().flatten().map(|(id, _)| *id).chain(s.orphan_ids.iter().copied()).collect())
        }
        fn delete_by_ids(&self, ids: &[i64]) -> Result<i64, EsError> {
            let mut sorted = ids.to_vec();
            sorted.sort_unstable();
            self.state.lock().unwrap().deleted_ids = Some(sorted);
            Ok(ids.len() as i64)
        }
        fn insert(&self, docs: &[(i64, Value)]) -> Result<(), EsError> {
            self.state.lock().unwrap().inserted.push(docs.to_vec());
            Ok(())
        }
        fn refresh(&self) -> Result<(), EsError> {
            Ok(())
        }
    }

    fn to_args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    /// `main()` 실행. (종료 코드, 표준 출력)
    fn run_main(args: &[&str], env: &HashMap<String, String>, store: &FakeStore) -> (i32, String) {
        run_main_with(args, env, store, &InProcess)
    }

    fn run_main_with(args: &[&str], env: &HashMap<String, String>, store: &FakeStore, runner: &(dyn StageRunner + Sync)) -> (i32, String) {
        let args = to_args(args);
        let get = |k: &str| env.get(k).cloned();
        let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { if store.ping_ok { Ok(Box::new(store.clone())) } else { Err("ES ping 실패".to_owned()) } };
        let world = World { env: &get, connect: &connect, runner };
        let mut out = Vec::new();
        let code = run("loader", &args, &world, &mut out);
        (code, String::from_utf8(out).unwrap())
    }

    fn run_main_with_hooks(args: &[&str], env: &HashMap<String, String>, store: &FakeStore, hooks: &RunHooks) -> (i32, String) {
        let args = to_args(args);
        let get = |k: &str| env.get(k).cloned();
        let connect = |_index: &str| -> Result<Box<dyn IndexStore>, String> { Ok(Box::new(store.clone())) };
        let world = World { env: &get, connect: &connect, runner: &InProcess };
        let mut out = Vec::new();
        let code = run_with_hooks("loader", &args, &world, &mut out, hooks);
        (code, String::from_utf8(out).unwrap())
    }

    mod test_loader_main {
        use super::*;

        #[test]
        fn test_main_no_args() {
            // Lines 886-888: no args. Python usage error와 같은 코드 1을 반환한다.
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let (code, out) = run_main(&[], &env, &FakeStore::new());
            assert_eq!(code, 1);
            assert!(!out.is_empty());
        }

        #[test]
        fn test_main_invalid_index() {
            // Lines 891-894: invalid index name. 원본은 1, Rust는 2.
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let (code, _) = run_main(&["invalid_index", &path_str(dir.path())], &env, &FakeStore::new());
            assert_eq!(code, 2);
        }

        #[test]
        fn test_main_no_file_args() {
            // Lines 898-900: no file args without --delete. 원본은 1, Rust는 2.
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let (code, _) = run_main(&["book"], &env, &FakeStore::new());
            assert_eq!(code, 2);
        }

        #[test]
        fn test_main_delete_mode() {
            // Lines 917-927: --delete mode
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let store = FakeStore::new();
            store.update(|s| s.index_exists = true);
            let (code, _) = run_main(&["--delete", "book"], &env, &store);
            assert_eq!(code, 0);
        }

        #[test]
        fn test_main_delete_index_not_exist() {
            // Lines 922-923: delete but index doesn't exist
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let store = FakeStore::new();
            store.update(|s| s.index_exists = false);
            let (code, _) = run_main(&["--delete", "book"], &env, &store);
            assert_eq!(code, 0);
        }

        #[test]
        fn test_main_delete_exception() {
            // Lines 924-926: delete raises exception. 원본은 -1, Rust는 255.
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let store = FakeStore::new();
            store.update(|s| s.exists_error = Some("boom".to_owned()));
            let (code, _) = run_main(&["--delete", "book"], &env, &store);
            assert_eq!(code, 255);
        }

        #[test]
        fn test_main_ping_failure() {
            // Lines 907-913: ES ping fails. 원본은 SystemExit, Rust는 접속 실패 종료 코드 255.
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let mut store = FakeStore::new();
            store.ping_ok = false;
            let (code, _) = run_main(&["book", &path_str(dir.path())], &env, &store);
            assert_eq!(code, 255);
        }

        #[test]
        fn test_main_file_not_found() {
            // Lines 1007-1009: file not found
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let (code, _) = run_main(&["book", &path_str(&dir.path().join("nonexistent"))], &env, &FakeStore::new());
            assert_eq!(code, 0);
        }

        #[test]
        fn test_main_single_file() {
            // Lines 1019-1025: single file force reload
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let cat_dir = dir.path().join("cat");
            fs::create_dir(&cat_dir).unwrap();
            let test_file = cat_dir.join("[author] title.txt");
            fs::write(&test_file, "hello world").unwrap();
            let (code, _) = run_main(&["book", &path_str(&test_file)], &env, &FakeStore::new());
            assert_eq!(code, 0);
        }

        #[test]
        fn test_main_batches_file_arguments_by_hundred() {
            // 100개 파일씩 process_file_iter에 전달한다.
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let mut files = Vec::new();
            for number in 0..101 {
                let path = dir.path().join(format!("book-{number}.txt"));
                fs::write(&path, format!("content {number}")).unwrap();
                files.push(path_str(&path));
            }
            let mut args = vec!["book"];
            args.extend(files.iter().map(String::as_str));
            let store = FakeStore::new();
            let (code, _) = run_main(&args, &env, &store);
            assert_eq!(code, 0);
            assert_eq!(store.batch_sizes(), [100, 1]);
        }

        #[test]
        fn test_main_recursive() {
            // Lines 1026-1038: recursive directory processing
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let sub = dir.path().join("sub");
            fs::create_dir(&sub).unwrap();
            fs::write(sub.join("[auth] book.txt"), "content").unwrap();
            let (code, _) = run_main(&["--recursive", "book", &path_str(dir.path())], &env, &FakeStore::new());
            assert_eq!(code, 0);
        }

        #[test]
        fn test_main_directory_non_recursive() {
            // Lines 1039-1074: directory without --recursive (2-stage)
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let sub1 = dir.path().join("sub1");
            fs::create_dir(&sub1).unwrap();
            fs::write(sub1.join("[auth] book1.txt"), "content1").unwrap();
            let sub2 = dir.path().join("sub2");
            fs::create_dir(&sub2).unwrap();
            fs::write(sub2.join("[auth] book2.txt"), "content2").unwrap();
            fs::write(dir.path().join("[auth] root.txt"), "root content").unwrap();
            let (code, _) = run_main(&["book", &path_str(dir.path())], &env, &FakeStore::new());
            assert_eq!(code, 0);
        }

        #[test]
        fn test_main_recursive_reload() {
            // Lines 1032-1033: --recursive --reload (skip_check=True)
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            fs::write(dir.path().join("[auth] book.txt"), "content").unwrap();
            let (code, _) = run_main(&["--recursive", "--reload", "book", &path_str(dir.path())], &env, &FakeStore::new());
            assert_eq!(code, 0);
        }

        #[test]
        fn test_main_path_not_in_prefix() {
            // Lines 1010-1012: path not in book/comics prefix
            let dir = tmp();
            let outside = dir.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("test.txt"), "x").unwrap();
            // Set prefix to a subdirectory so outside is not relative
            let books_only = dir.path().join("books_only");
            fs::create_dir(&books_only).unwrap();
            let env = env_for(&books_only, &dir.path().join("comics_only"));
            let (code, _) = run_main(&["book", &path_str(&outside)], &env, &FakeStore::new());
            assert_eq!(code, 0);
        }

        #[test]
        fn test_main_getopt_error() {
            // Python getopt 오류는 print_usage()의 SystemExit(0)으로 끝난다.
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let (code, out) = run_main(&["--invalid-option", "book"], &env, &FakeStore::new());
            assert_eq!(code, 0);
            assert!(out.contains("사용법"));
        }
    }

    #[test]
    fn test_loader_module_executed_as_main() {
        // if __name__ == '__main__' 가드 실행 (loader.py:1055).
        // 인자 없이 실행 -> Python main()처럼 사용법을 보이고 오류 코드로 끝난다.
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        let (code, out) = run_main(&[], &env, &FakeStore::new());
        assert_eq!(code, 1);
        assert!(!out.is_empty());
    }

    #[test]
    fn test_main_recursive_reload_deletes_orphans() {
        // --recursive --reload: 대상 경로 하위 orphan(디스크에 없는) 레코드를 삭제한다.
        let dir = tmp();
        // 대상 디렉토리를 book prefix 로 지정하고 파일 1개 생성
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("a.txt"), "hello world\n").unwrap();
        let mut env = setup_loader_env(dir.path());
        env.insert("TM_ES_BOOK_INDEX".to_owned(), "book_idx".to_owned());
        env.insert("TM_ES_COMICS_INDEX".to_owned(), "comics_idx".to_owned());

        let store = FakeStore::new();
        // 실제 적재된 inode + 디스크에 없는 orphan 하나
        store.update(|s| {
            s.orphan_ids.insert(999999);
        });
        let (rc, _) = run_main(&["--recursive", "--reload", "book", &path_str(dir.path())], &env, &store);

        let inserted = store.inserted_docs();
        assert_eq!(rc, 0);
        assert!(!inserted.is_empty(), "파일이 적재되어야 한다");
        assert_eq!(store.state.lock().unwrap().deleted_ids, Some(vec![999999]), "orphan 만 삭제되어야 한다 (live inode 는 보존)");
        assert!(!inserted.contains_key(&999999));
    }

    #[test]
    fn test_main_recursive_reload_skips_orphan_delete_when_directory_unreadable() {
        // 읽지 못한 디렉토리의 책은 seen에 없다. orphan으로 지우면 실존 책 레코드가 사라진다.
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("a.txt"), "hello world\n").unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("b.txt"), "hidden\n").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&locked).is_ok() {
            // root로 돌면 권한으로 막을 수 없다.
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let mut env = setup_loader_env(dir.path());
        env.insert("TM_ES_BOOK_INDEX".to_owned(), "book_idx".to_owned());
        env.insert("TM_ES_COMICS_INDEX".to_owned(), "comics_idx".to_owned());

        let store = FakeStore::new();
        store.update(|s| {
            s.orphan_ids.insert(999999);
        });
        let (rc, out) = run_main(&["--recursive", "--reload", "book", &path_str(dir.path())], &env, &store);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(rc, 0);
        assert!(!store.inserted_docs().is_empty(), "읽을 수 있는 파일은 적재되어야 한다");
        assert_eq!(store.state.lock().unwrap().deleted_ids, None, "읽지 못한 디렉토리가 있으면 orphan을 지우지 않는다");
        assert!(out.contains("orphan 삭제 건너뜀"), "{out}");
        assert!(out.contains("locked"), "{out}");
    }

    #[test]
    fn test_main_recursive_reload_skips_orphan_delete_when_lookup_fails() {
        // 기존 레코드 조회가 실패하면 "orphan 없음"으로 보고하지 않고 삭제도 하지 않는다.
        let dir = tmp();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("a.txt"), "hello world\n").unwrap();
        let mut env = setup_loader_env(dir.path());
        env.insert("TM_ES_BOOK_INDEX".to_owned(), "book_idx".to_owned());
        env.insert("TM_ES_COMICS_INDEX".to_owned(), "comics_idx".to_owned());

        let store = FakeStore::new();
        store.update(|s| s.prefix_lookup_error = true);
        let (rc, out) = run_main(&["--recursive", "--reload", "book", &path_str(dir.path())], &env, &store);

        assert_eq!(rc, 0);
        assert_eq!(store.state.lock().unwrap().deleted_ids, None);
        assert!(out.contains("orphan 삭제 건너뜀"), "{out}");
        assert!(!out.contains("orphan 없음"), "{out}");
    }

    mod test_pdftotext_failure_modes {
        use super::*;

        #[test]
        fn test_missing_binary_returns_empty() {
            // pdftotext가 설치되어 있지 않으면 빈 결과로 넘어간다 (658).
            let dir = tmp();
            let pdf = dir.path().join("nobinary.pdf");
            fs::write(&pdf, b"%PDF-1.4 mock").unwrap();

            let which = || -> Option<PathBuf> { None };
            let run_unused = |_: &[String]| -> Option<(i32, Vec<u8>)> { None };
            let path_cache = std::sync::OnceLock::new();
            let hooks = PdftotextHooks { which: Some(&which), run: Some(&run_unused), path_cache: &path_cache };
            assert_eq!(pdf_text_by_pdftotext_with(&pdf, &hooks), (String::new(), 0));
        }

        #[test]
        fn test_nonzero_returncode_returns_empty() {
            // pdftotext가 실패 코드로 끝나면 stdout을 신뢰하지 않는다 (663).
            let dir = tmp();
            let pdf = dir.path().join("badexit.pdf");
            fs::write(&pdf, b"%PDF-1.4 mock").unwrap();

            let which = || -> Option<PathBuf> { Some(PathBuf::from("/usr/bin/pdftotext")) };
            let run_fails = |_: &[String]| -> Option<(i32, Vec<u8>)> { Some((1, b"partial garbage".to_vec())) };
            let path_cache = std::sync::OnceLock::new();
            let hooks = PdftotextHooks { which: Some(&which), run: Some(&run_fails), path_cache: &path_cache };
            assert_eq!(pdf_text_by_pdftotext_with(&pdf, &hooks), (String::new(), 0));
        }
    }

    mod test_loader_main_reencode_options {
        use super::*;

        #[test]
        fn test_reencode_options_are_parsed() {
            // --reencode / --reencode-dry-run / --reencode-backup-dir (1445-1451).
            // 원본은 Loader 클래스 속성을 확인한다. Rust는 파싱된 Options와 종료 코드를 확인한다.
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let backup_dir = dir.path().join("backup");
            fs::write(dir.path().join("[작가] 책.txt"), "본문입니다\n").unwrap();
            let backup = path_str(&backup_dir);
            let root = path_str(dir.path());
            let argv = ["--reencode", "--reencode-dry-run", "--reencode-backup-dir", backup.as_str(), "book", root.as_str()];

            let (code, _) = run_main(&argv, &env, &FakeStore::new());
            assert_eq!(code, 0);

            let options = parse_options(&to_args(&argv)).unwrap();
            assert!(options.reencode);
            assert!(options.reencode_dry_run);
            assert_eq!(options.reencode_backup_dir, Some(backup_dir));
        }

        #[test]
        fn test_pdf_only_batch_runs_without_worker_pool() {
            // PDF는 워커 스레드에 올리지 않으므로 스레드풀 없이 처리된다 (1591-1593).
            let dir = tmp();
            let env = setup_loader_env(dir.path());
            let pdf = dir.path().join("[작가] 만화.pdf");
            write_blank_pdf(&pdf);

            let used_pool = AtomicUsize::new(0);
            let on_spawn = || {
                used_pool.fetch_add(1, Ordering::SeqCst);
            };
            let hooks = RunHooks { on_worker_spawn: Some(&on_spawn), ..Default::default() };
            let store = FakeStore::new();
            let (code, _) = run_main_with_hooks(&["book", &path_str(&pdf)], &env, &store, &hooks);

            assert_eq!(code, 0);
            assert_eq!(used_pool.load(Ordering::SeqCst), 0, "PDF만 있는 배치에서 스레드풀을 만들었다");
            let inserted = store.inserted_docs();
            assert_eq!(inserted.len(), 1);
            assert_eq!(inserted.values().next().unwrap()["page_count"], 1);
        }
    }
}
