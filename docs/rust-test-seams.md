# Rust 테스트 seam 통일 설계안

대상: `tests/rust/backend/*.rs`(49개), `tests/rust/backend/common/*.rs`(6개), `tests/rust/loader/*.rs`(4개).
작성 근거: 각 파일 맨 위 `//!` 가정 목록, `impl X for`/`trait`/`use backend::` grep, 테스트 본문 식별자 집계(스크립트는 scratchpad/tools).
기존 코드·테스트는 수정하지 않았고 cargo도 실행하지 않았다. 숫자는 정규식 집계라 ±25% 오차가 있다.

---

## 1. 요약

- 파일마다 테스트 대역을 이름만 같고 모양이 다른 trait로 가정했다. `EsManagerApi`는 최소 5가지 시그니처, `EsApi`는 2가지 뜻, `Session`은 2가지 뜻이다.
- seam 방식은 세 갈래다. (a) 생성자 주입(`Deps` 빌더), (b) 인스턴스 `set_*` 변경자, (c) 전역 `override_*`/`hooks::set_*`/`testing::*` 가드. (c)는 같은 test binary 안에서 병렬 테스트와 충돌한다. 저장소 규칙(전역 상태 금지)에도 어긋난다.
- 전체 테스트는 약 2,174개다. 그중 `#[ignore]`가 769개이고, 약 600개는 사유가 `monkeypatch`/`mock.patch`/seam 부재다. seam을 통일하면 이 600개가 열린다.
- 통일 원칙은 네 줄이다.
  1. 서비스 전체를 바꾸는 facade는 `XxxApi`(예: `BookManagerApi`, `EsManagerApi`). 좁은 능력은 역할 명사(예: `FsOps`, `Clock`, `CommandRunner`). 기존 crate(`isbn::CommandRunner`, `es_client::IndexStore`, `bookstore::Fetcher`, `doc_reader::StageRunner`)가 이미 이 규칙이다.
  2. 주입은 생성자 `XxxDeps` + 소비형 `with_*` 빌더 하나로 한다. `set_*`, 전역 가드, `backend::*::testing` 모듈은 쓰지 않는다.
  3. 말단 순수 함수만 `*_with(args, &XxxHooks)`를 쓴다(`doc_reader`, `epub_cover`, `cover_sources`가 이미 이 방식).
  4. 공용 능력(`Clock`, `FsOps`, `CommandRunner`, `HttpSession`)은 새 lib crate `libs/sysdeps`에 두고 backend, loader, doc_reader가 함께 쓴다.
- 재작성 규모: 테스트 본문 수정 약 410개(19%, 41개 파일). 그중 기계적 치환 약 190개, 수작업 약 220개. 대역(fake) 구현을 고치는 헬퍼 수정은 별도로 약 25개 파일이다. 본문·헬퍼 모두 안 고치는 파일은 약 11개다.

---

## 2. 관심사별 변형표

표의 "규모"는 그 변형을 쓰는 파일의 테스트 수 합이다(헬퍼 경유 포함). `#[ign]`은 그 변형을 못 구해서 ignore된 개수의 대략치다.

### 2.1 앱 진입점·인증

| 변형                                                                                                                                                                             | 사용 파일(테스트 수)                                                                             | 모양                  | 규모 |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------- | ---- |
| A1 `app_with(AppDeps)` + `AuthMode::{Cookies, Fixed, Split{auth,admin}}` + `.book_manager/.comics_manager/.category_mapping(Arc<dyn Trait>)`                                     | main(94), main_bulk_rename(16), main_gaps(9)                                                     | 빌더                  | 119  |
| A2 `app_with(AppDeps)` + `AuthMode::{Real, Overrides{auth,admin}, AuthOnly}` + 클로저 빌더 12개(`.verify_google_token(Fn)`, `.store_issued(Fn)` 등)                              | auth(96)                                                                                         | 클로저                | 96   |
| A3 `app_with(Overrides::new(bm,cm,cat))` + `Arc<Mutex<Value>>` 필드(`require_auth/admin`) + 선택 필드 8개(`list_dir`, `to_thread_spy` 등) + `DynManager::call(method, CallArgs)` | main_mock(129)                                                                                   | 필드 대입             | 129  |
| A4 `AppDeps::new(Arc<BookManager>)` + `.with_viewer_check(Fn)`                                                                                                                   | cover(21)                                                                                        | 첫 인자가 BookManager | 21   |
| A5 `AppDeps { view_history_store, book_access, auth_override, admin_override }` struct 리터럴 + `Default`                                                                        | view_history_api(19)                                                                             | 필드                  | 19   |
| A6 `app_with_overrides(Arc<dyn BookManagerApi>, Option<Value>)`                                                                                                                  | pdf_delete_pages(71), epub_image_edit(40), common/book_fixtures                                  | 위치 인자             | 111  |
| A7 `app_with_overrides(AppOverrides{book_manager: Arc<dyn EpubCoverApi>, admin})`                                                                                                | epub_cover_set(70)                                                                               | struct                | 70   |
| A8 `app_with_book_manager(Arc<BookManager>)`                                                                                                                                     | pdf_pages(19)                                                                                    | 한 인자               | 19   |
| A9 `create_cover_router(get_manager, AdminGate, CoverHooks)`                                                                                                                     | common/cover_fixtures → cover_management(65), cover_repair(5)                                    | 라우터 직접           | 70   |
| A10 `app()` + 실제 env/ES                                                                                                                                                        | client_error_log(8), main(통합 일부), common/mod.rs                                              | 인자 없음             | 8    |
| `create_jwt_token` 인자                                                                                                                                                          | 2개(client_error_log), 3개 `&str name`(cover, pdf_pages, book_preview), 4개 `Option`(auth, main) | 시그니처 3종          |      |

`BookManagerApi` fake 사이의 시그니처 불일치(통일 전 반드시 맞춰야 한다):

| 메서드                             | main.rs                   | main_bulk_rename                           | book_manager_2                                       | book_manager_1                          |
| ---------------------------------- | ------------------------- | ------------------------------------------ | ---------------------------------------------------- | --------------------------------------- |
| `get_books_in_category_paged` 반환 | -                         | `(Vec<Book>, i64, String, Option<String>)` | `(Vec<Book>, usize, Option<String>, Option<String>)` | -                                       |
| `get_book_content` 반환            | `String`                  | -                                          | `BookContent::{File,Text}`                           | `BookContent`                           |
| `get_book_preview`                 | `(id) -> Response`        | -                                          | `(id, chapters, base_url) -> Response`               | `(id, pages, chapters) -> BookResponse` |
| `get_pdf_pages` 인자               | `i64,i64,i64`             | -                                          | `usize`                                              | `u32`                                   |
| `get_category_mismatches`          | `Result<Value,String>`    | -                                          | `(force_refresh) -> Value`                           | `(force_refresh) -> Value`              |
| `update_book` 반환                 | `(Value, Option<String>)` | 같음                                       | `(String, Option<String>)`                           | `(String, Option<String>)`              |

### 2.2 ES

| 변형                                                                                  | 사용 파일(테스트 수)                                                                                                                                                                                                                                                                                      | 모양                        |
| ------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------- |
| E1 `EsApi::call(op, &Value) -> Result<Value, ElasticsearchError>` (저수준 클라이언트) | es_manager(91), es_manager_gaps(8), book_manager_2의 DummyEs(154), common/es_dummy                                                                                                                                                                                                                        | op 문자열 + kwargs JSON     |
| E2 `EsApi::{search_by_id, create_index}` (같은 이름, 다른 뜻)                         | cover(21)                                                                                                                                                                                                                                                                                                 | 고수준 2메서드              |
| E3 `EsManagerApi` 고수준, 반환형이 파일마다 다름                                      | book*fixtures(`search_by_id -> Option<Value>`, `delete -> bool`, `update(&Value) -> bool`), book_manager_1(`Result<*, String>`, `Docs`), book_preview(`-> Value`), epub_cover_set(`update(Value) -> bool`), main(`delete -> bool`, `search_categories_pdf_stats`), main_gaps, main_bulk_rename(`refresh`) | 부분 구현 fake              |
| E4 `EsError` 기반 `Result<_, EsError>` fake (이름 `EsApi`로 impl)                     | book_manager_2                                                                                                                                                                                                                                                                                            | `EsError::Other(msg)` 가정  |
| E5 `EsLookup::{get_by_id, search_by_path}`                                            | classifier(64)                                                                                                                                                                                                                                                                                            | `_source` 반환              |
| E6 `EsLike::{index_name, get(&Value), search(&Value)}`                                | classifier_corpus(13), classifier_service(10)                                                                                                                                                                                                                                                             | 저수준 kwargs               |
| E7 `BookIndex::{search_by_id, update}` → `Result<_, String>`                          | common/cover_fixtures, cover_management(65)                                                                                                                                                                                                                                                               | 2메서드                     |
| E8 `loader`의 실제 `es_client::IndexStore`                                            | loader_2, loader_gaps                                                                                                                                                                                                                                                                                     | 이미 존재                   |
| 에러 타입                                                                             | `ElasticsearchError{ConnectionError,NotFoundError,BadRequestError{message,status},RuntimeError}`(es_manager, es_dummy), `EsError`(실제 crate: MissingEnv/Transport/Status/Parse/Unsupported; bm2는 `Other` 가정), `String`(bm1, main)                                                                     |                             |
| mock seam                                                                             | `ESManager::set_search_override`, `set_search_paged_override`, `set_do_exist_index_override`, `set_ensure_category_nori_subfield_override`, `set_path_created_time_override`, `set_resolve_override` (15개 테스트)                                                                                        | 자기 비공개 메서드 바꿔치기 |
| 생성                                                                                  | `ESManager::with_client(Arc<dyn EsApi>, index)`, `new_with(index, env, connector, sleep)`                                                                                                                                                                                                                 |                             |

### 2.3 DB

| 변형                                                                                                                            | 사용 파일(테스트 수)                                                                                                                                                                                                        | 모양                                      |
| ------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| D1 `backend::db::{DbValue, Row, DbError, DbCursor, DbConnection, ConnectionProvider}`                                           | category_mapping(52), refresh_token_store(62)                                                                                                                                                                               | 두 파일 머리말이 똑같다. 이미 합의된 상태 |
| D2 `StoreConnection{begin, execute(&[Value]), fetch_all, commit, rollback}` + `with_connection_factory`, `set_max_history_rows` | view_history_store(15)                                                                                                                                                                                                      | D1과 다른 추상                            |
| D3 실제 MySQL: `CategoryMapping::new()`, `get_connection() -> Connection{execute(sql,&[Value]), fetch_all, fetch_one, commit}`  | classify_proposal_items_db(16), classify_proposal_status_db(11), main(1)                                                                                                                                                    | `serde_json::Value` 파라미터              |
| D4 실제 MySQL: `ViewHistoryStore::_get_connection() -> mysql::Conn`                                                             | view_history_store                                                                                                                                                                                                          | 구체 타입 노출                            |
| D5 `Conn::execute(&mut self, sql)` / `get_connection() -> Result<Conn,E>`                                                       | main(통합 1)                                                                                                                                                                                                                |                                           |
| 컨테이너                                                                                                                        | common/mysql_fixture(testcontainers) vs `TM_MYSQL_HOST` 외부 DB                                                                                                                                                             | 두 방식 혼재                              |
| 생성자                                                                                                                          | `CategoryMapping::with_connector(host,port,db,user,pw,provider)`, `set_connection_provider(&mut)`, `MySQLRefreshTokenStore::{with_connector (init_db 실행), with_connection_provider (init 생략), set_connection_provider}` | 3종                                       |

### 2.4 파일시스템

| 변형                                                                                                                                          | 사용 파일                                       | 모양            |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- | --------------- |
| F1 `FsOps` 9메서드(scandir, read_dir, stat, rename, unlink, resolve, samefile, remove_dir_all, open) + `FaultyFs` + `manager.set_fs_ops(Arc)` | book_manager_1(40회)                            | 인스턴스 setter |
| F2 전역 가드 `fs_{rename,move,resolve,iterdir,unlink,samefile,is_file}_override`, `path_is_relative_to_override`, `override_normpath`         | book_manager_2(24회)                            | 전역            |
| F3 `StoreFs{disk_free, write}` + `Yes24Bookstore::with_fs`                                                                                    | bookstore(4회)                                  | 빌더            |
| F4 `BookManagerDeps::with_is_file(Fn)`                                                                                                        | pdf_pages                                       | 클로저          |
| F5 `hooks::set_unlink_hook`, `set_temp_file_observer`                                                                                         | book_preview                                    | 전역            |
| F6 `testing::fail_next_iterdir`                                                                                                               | main_bulk_rename                                | 전역            |
| F7 `Hooks.fsync(Fn(&File))`, `Hooks.flock(Fn(&File))`, `epub_cover::hooks::set_fsync`, `store_lock_with(dir, store, flock, monotonic)`        | cover_management, epub_cover_set, cover_sources | 클로저+전역     |
| F8 `ReencodeIo` trait                                                                                                                         | loader_1                                        | 파일 전용 trait |
| F9 `CoverHooks.with_delete_book_cache`, `with_resolve`, `with_directories`                                                                    | cover_management                                | 클로저          |

### 2.5 외부 명령·서브프로세스

| 변형                                                                                                                                                            | 사용 파일                                                                      | 모양                 |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | -------------------- |
| C1 `CommandRunner::run(&[String], Duration) -> Result<CommandOutput{returncode, stdout, stderr}, CommandError{Timeout,NotFound,Failed}>` + `set_command_runner` | book_manager_1(12회), `_convert_with_libreoffice_with(&dyn CommandRunner, ..)` | 인스턴스             |
| C2 실제 crate `isbn::CommandRunner::run(program, args) -> io::Result<CmdOutput{code, stdout: String}>`                                                          | libs/isbn                                                                      | 시그니처가 C1과 다름 |
| C3 `EpubcheckSpawner::spawn -> Box<dyn EpubcheckProcess{async communicate, returncode, kill, async wait}>` + `hooks::set_epubcheck_spawner`                     | book_preview                                                                   | 전역, async          |
| C4 `override_convert_with_libreoffice`, `set_convert_with_libreoffice_hook`, `set_convert_override`                                                             | book_manager_2(8), hwp_preview, book_manager_1(2)                              | 3종 이름, 같은 목적  |
| C5 `_find_libreoffice_with(which, exists)` ≡ `doc_reader::office::find_libreoffice_with(which, exists)`                                                         | book_manager_1, loader_1                                                       | 이미 일치            |
| C6 `Pdftotext::new(&dyn PdftotextEnv).text(path)`                                                                                                               | loader_1                                                                       | trait                |
| C7 `pdf_text_by_pdftotext_with(&Path, which, run)`                                                                                                              | loader_2                                                                       | 클로저 2개           |
| C8 실제 `doc_reader::pdf::StageRunner`                                                                                                                          | loader_1, loader_2                                                             | 이미 존재, 변경 없음 |

### 2.6 HTTP·세션·이미지 fetch

| 변형                                                                                                    | 사용 파일                                                                                      | 모양                        |
| ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | --------------------------- |
| H1 `bookstore::Session::get(&self, url, params) -> Result<HttpResponse{status_code,text,json}, String>` | bookstore(97), bookstore_cache                                                                 | `&self`, 완결 본문          |
| H2 `cover_sources::Session::get(&mut self, url) -> Response{is_redirect, location, chunks, closed}`     | cover_sources(23)                                                                              | `&mut self`, 청크 스트림    |
| H3 `Net{resolve, connect} / Pool{urlopen} / PoolResponse{status, header, stream, close}`                | direct_cover_url(8)                                                                            | SSRF 방어용 저수준          |
| H4 `FetchUrlImage::fetch_url_image -> Result<Vec<u8>, FetchError{Value,Os,Other}>`                      | common/book_fixtures → pdf_delete_pages(71), pdf_delete_pages_gaps, pdf_pages, epub_image_edit | trait                       |
| H5 `ImageFetcher::fetch_url_image -> Result<Vec<u8>, EpubCoverError>`                                   | epub_cover_set(70)                                                                             | 같은 이름 메서드, 다른 에러 |
| H6 `CoverHooks.with_fetch_url_image(Fn -> Result<_, CoverError>)`, `with_fetch_bytes`, `with_search`    | cover_management(65), cover_repair, cover_fixtures                                             | 클로저                      |
| H7 `SearchHooks::with_fetch / with_client_factory`, `StoreClient`                                       | cover_sources                                                                                  |                             |
| H8 `HttpGet` (파일 안 로컬 정의)                                                                        | api_exploration(9)                                                                             | 영향 없음                   |
| H9 `libs/bookstore::Fetcher::get(store, url) -> Result<String,String>`                                  | 실제 crate                                                                                     |                             |
| `Bookstore` seam                                                                                        | `set_fetch_search_results`, `set_search_by_keyword`, `new_detail_session`, `tmp_dir`           | 인스턴스 setter 4종         |

### 2.7 시계·sleep

| 변형                                                                                                                               | 사용 파일                                                      |
| ---------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| T1 `Clock{now()->f64, sleep(f64)}` + `FakeClock` + `StoreGate::with_clock`                                                         | cover_sources(23), cover_management(65), common/cover_fixtures |
| T2 sleep 주입 `&dyn Fn(u64)` in `ESManager::new_with(.., sleep)`; `connect_with_retry(max, sleep)` (실제 es_client)                | es_manager                                                     |
| T3 `store_lock_with(.., monotonic: &dyn Fn()->f64)`                                                                                | cover_sources                                                  |
| T4 `classify_cli::testing::override_time(Fn)`                                                                                      | classify_cli_gaps                                              |
| T5 `BookClassifierService::with_delay(0.0)`                                                                                        | book_classifier, bookstore_cache (값 설정이므로 seam이 아님)   |
| T6 `set_mismatch_cache(Option<Value>, Instant)`, `stale_running_status(.., now: Option<f64>)`, `set_uncovered_scan_budget_seconds` | book_manager_1, main, cover_status (값 설정)                   |
| T7 `testing::set_reload_progress_flush_interval_seconds`                                                                           | main                                                           |

### 2.8 환경 변수

| 변형                                                                                                                                                        | 사용 파일                                                                                         |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| V1 `common::setup_env()` (Once, 프로세스 전역 `set_var`)                                                                                                    | 25개 파일                                                                                         |
| V2 `EnvGuard`(락+복원) 파일마다 자체 구현                                                                                                                   | auth, refresh_token_store(9회), main(14회), main_mock(11회), book_manager_2(9회), classifier(3회) |
| V3 `get: &dyn Fn(&str) -> Option<String>` 주입 (`path_prefix_from_env`, `from_env`, `new_from_env`, `ESManager::new_with`, 실제 `loader::Config::from_env`) | book, comics, book_manager_1, es_manager, loader_1                                                |
| V4 모듈 로딩 시점 `SystemExit` (`load_jwt_secret() -> Result<_, SystemExit>`)                                                                               | auth                                                                                              |

test binary가 파일마다 하나라서 V1은 파일 간 충돌이 없다. 한 binary 안에서는 V2가 필요하다.

### 2.9 로깅 캡처

| 변형                                                                                                        | 사용 파일                                              |
| ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| L1 `common/log_capture.rs`: 자유 함수 `capture_at_level(Level)`, `records()`, `text()` (스레드 로컬)        | client_error_log, view_history_api, view_history_store |
| L2 `backend::log_capture::LogCapture::start(&str)`, `.text()`, `.messages()` (**backend 안의 모듈로 가정**) | auth, cover_management, refresh_token_store            |
| L3 파일 안 `CaptureLogger` + `capture_logs()`/`captured()`                                                  | bookstore_cache                                        |
| L4 `utils::classify_cli::{capture_stdout, capture_logs}` (**프로덕션 함수로 가정**)                         | classify_cli, classify_cli_gaps                        |
| L5 `capture_warnings` (book_manager_2 내부 import)                                                          | book_manager_2                                         |

### 2.10 PDF·이미지 라이브러리

| 변형                                                                                                                                               | 사용 파일                                                         | Python 대응                   |
| -------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | ----------------------------- |
| P1 `PdfBackend{page_count, extract_pages}` + `PdfError{kind,message}`                                                                              | book_manager_1(19회)                                              | pypdf                         |
| P2 `PdfInspector::open -> Result<PdfInfo{docinfo,page_count,pdf_version,syntax_problems}, String>`                                                 | book_manager_1                                                    | pikepdf(데이터형)             |
| P3 `PdfOpener::open -> Box<dyn PdfDoc{check_pdf_syntax, docinfo, page_count, pdf_version, close}>` + `hooks::set_pdf_opener`                       | book_preview(6회)                                                 | pikepdf(close 호출 검증 포함) |
| P4 `PdfDocument{len, close}` + `*_with(.., open: &dyn Fn)`                                                                                         | cover_status(14)                                                  | PDF document interface        |
| P5 `PlumberDocument` + `pdf_text_by_plumber_with`                                                                                                  | loader_2                                                          | pdfplumber                    |
| P6 클래스 수준 캐시: `BookManager::{pdf_reader_cache_clear, ..., set_pdf_reader_cache_limits -> Guard}` + `with_open_pdf_observer` + `serial()` 락 | pdf_pages(19)                                                     | 전역 캐시                     |
| P7 이미지: `image::DynamicImage`, `crop_pil`, `set_max_image_pixels -> Guard`, `Limits{max_image_pixels..}`                                        | image_edit, pdf_delete_pages_gaps, cover_repair, cover_management | PIL                           |
| P8 `set_pixel_decode_hook(Box<dyn Fn>) -> Guard`                                                                                                   | cover_repair                                                      | `Image.load` patch            |

### 2.11 매니저 생성자

| 변형                                                                                                                          | 사용 파일(테스트 수)                                                                                    |
| ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| M1 `BookManager::for_test(path, Arc<dyn EsManagerApi>)`                                                                       | book_manager_1(155)                                                                                     |
| M2 `BookManager::new_for_test(path, Arc<dyn EsApi>)`                                                                          | book_manager_2(154)                                                                                     |
| M3 `BookManager::with_es(path, Arc<dyn EsApi>)`, `with_es_and_extractor(path, es, ThumbnailExtractor)`, `CoverManaging` trait | cover(21)                                                                                               |
| M4 `BookManager::without_es(path)`                                                                                            | cover_status(14)                                                                                        |
| M5 `BookManager::for_test(path, es, Arc<dyn BookLookup>)`, `BookManager::new()`(env)                                          | book_preview(99)                                                                                        |
| M6 `BookManagerDeps::new(es, path).with_*` 빌더 + `BookManager::with_deps`                                                    | common/book_fixtures → pdf_delete_pages, pdf_delete_pages_gaps, pdf_pages, epub_image_edit              |
| M7 `BookManagerDeps { path_prefix, trash_dirname, es_manager, book_lookup, image_fetcher }` struct 리터럴                     | epub_cover_set(70)                                                                                      |
| M8 `backend::book_manager::testing::{DummyEs, make_manager, make_doc}` (프로덕션 crate 안의 테스트 모듈로 가정)               | book_manager_gaps(47), book_manager_hwp_preview, main(11회), classify_cli(16회), classify_cli_gaps(9회) |
| M9 `new_from_env(get)`, `new_with(get, es_factory: Box<dyn FnOnce>)`, `with_es_url(url)`                                      | book_manager_1                                                                                          |
| M10 `ComicsManager::from_env(get)`, `from_env_with_es(get, es_factory: FnOnce(&str) -> Box<dyn IndexManager>)`                | comics                                                                                                  |
| 인자 순서                                                                                                                     | `(path, es)` 4곳, `(es, path)` 1곳(book_fixtures)                                                       |

### 2.12 그 외

| 항목                | 변형                                                                                                                                                                                                                                                                                                       |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 분류기 모델         | `Model`(classifier_service), `ClassifierModel`(classifier_gaps), `CategoryModel` 구체 타입(classifier)                                                                                                                                                                                                     |
| 분류기 facade       | `Classify{is_available, classify_path -> Result<Prediction,String>}`(book_classifier, bookstore_policy), `Classifier{is_loaded, classify_path -> Prediction}`(book_manager_1), `FileClassifier`(book_manager_2), `ClassifierService`(book_manager_gaps)                                                    |
| 분류기 생성         | `BookCategoryClassifier::new(Option<CategoryModel>, ClassifierOptions)` / `new(Option<Box<dyn ClassifierModel>>, ClassifierOptions)` / `new(model, es, root).with_min_confidence()` 3종. `Prediction::new(Option<&str>, ..)` vs `(&str, ..)`                                                               |
| 분류 서비스         | `BookClassifierService::new(root, Option<&Path>, Option<Box<dyn Classify>>)` vs `(root, &Path, Option<BookCategoryClassifier>)`, `wrap_decide_by_bookstore(wrapper)`                                                                                                                                       |
| classify CLI        | `utils::classify_cli::testing::override_*` 15종 전역 가드, `capture_stdout/logs`, `main(args) -> Result<i32, SystemExit>`                                                                                                                                                                                  |
| 백그라운드 작업     | `override_thread_start(_run_inline)`, `BackfillThread` trait, `join_created_time_backfill_thread`, Starlette `BackgroundTasks`처럼 응답 전 종료 가정                                                                                                                                                       |
| epub_cover 훅       | cover_management `Hooks{limits, find_opf_path, fsync, flock, revision, testzip, inspect_archive}` ≈ epub_cover_set `hooks::{set_fsync, set_max_xml_bytes, set_zip_test, set_inspect_archive, set_uuid_hex}` ≈ pdf_delete_pages_gaps `set_max_image_pixels` ≈ cover_repair `Limits`+`set_pixel_decode_hook` |
| EPUB 에러           | `CoverError`, `EpubCoverError{Cover,Io}`, `FetchError{Value,Os,Other}`, `ValueError`, `SearchError`                                                                                                                                                                                                        |
| loader `doc_reader` | `read_from_text_with(path, detect)`(loader*1) vs `read_from_text_with(path, &TextHooks)`(loader_2). `fast_page_count_with(.., XrefOps)` vs `(.., &FastCountHooks)`. `reencode*\*\_with_detect / \_with_io / \_with(&TextHooks)`3종.`read_file_with_fast_count`vs`read_file_with_hooks(ReadHooks)`          |
| 서점 대역           | `SoupSource`(예외 던지는 soup), `BookstoreSearch`, `set_fetch_search_results`                                                                                                                                                                                                                              |

---

## 3. 통일안

### 3.0 규칙

1. 이름: facade = `XxxApi`, 좁은 능력 = 역할 명사, 번들 = `XxxDeps`(생성자 주입)/`XxxHooks`(말단 함수 주입).
2. 빌더: 소비형 `fn with_xxx(mut self, v) -> Self`. `set_*`, `override_*`, `hooks::set_*`는 금지. 값(설정)은 `with_xxx(value)`, 대역은 `with_xxx(Arc<dyn Trait>)`.
3. 모든 trait는 `Send + Sync`. 서비스 facade의 async 메서드는 `#[async_trait]`(dyn 필요).
4. 모든 메서드에 기본 구현을 둔다. 기본값은 `Err(..Unimplemented)`/빈 값이다. 테스트 fake는 쓰는 메서드만 구현한다(main.rs, main_gaps가 이미 이 방식).
5. 실제 구현체는 `SystemClock`, `RealFs`, `SystemRunner` 같은 이름으로 같은 crate에 둔다. 프로덕션 crate에는 `testing` 모듈이 없다. fake는 `tests/rust/backend/common/`에 둔다.
6. 정수 파라미터(ID, 쪽수, 크기, 오프셋)는 `i64`다. 음수 입력 검증이 테스트 대상이라서 `usize`로 좁히지 않는다.
7. 반환 `(T, Option<String>)` 튜플은 Python과 같게 유지한다.

### 3.1 `libs/sysdeps` (새 crate, 공용 능력)

```rust
// libs/sysdeps/src/lib.rs
pub type EnvFn<'a> = &'a dyn Fn(&str) -> Option<String>;   // loader::Config::from_env와 같은 모양

pub trait Clock: Send + Sync {
    fn now(&self) -> f64;                    // time.time()
    fn monotonic(&self) -> f64;              // time.monotonic()
    fn sleep(&self, seconds: f64);
}
pub struct SystemClock;

pub trait FsOps: Send + Sync {
    fn read_dir(&self, p: &Path) -> io::Result<Vec<PathBuf>>;      // scandir, iterdir 통합
    fn metadata(&self, p: &Path) -> io::Result<fs::Metadata>;      // stat
    fn is_file(&self, p: &Path) -> io::Result<bool>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&self, p: &Path) -> io::Result<()>;             // unlink
    fn remove_dir_all(&self, p: &Path) -> io::Result<()>;
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf>;       // resolve
    fn same_file(&self, a: &Path, b: &Path) -> io::Result<bool>;
    fn open(&self, p: &Path) -> io::Result<fs::File>;
    fn write(&self, p: &Path, data: &[u8]) -> io::Result<()>;
    fn disk_free(&self, dir: &Path) -> io::Result<u64>;            // StoreFs
    fn sync_file(&self, f: &fs::File) -> io::Result<()>;           // fsync
    fn lock_exclusive(&self, f: &fs::File, nonblocking: bool) -> io::Result<()>; // flock (WouldBlock = BlockingIOError)
}
pub struct RealFs;

#[derive(Debug)] pub enum CommandError { Timeout, NotFound, Failed(String) }
pub struct CommandOutput { pub returncode: i32, pub stdout: Vec<u8>, pub stderr: Vec<u8> }
pub trait CommandRunner: Send + Sync {
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> Result<CommandOutput, CommandError>;
    fn which(&self, program: &str) -> Option<PathBuf>;
}
pub struct SystemRunner;

// 타임아웃 시 kill/wait 호출을 검증해야 하는 경우(epubcheck)
pub trait ProcessSpawner: Send + Sync {
    fn spawn(&self, argv: &[String]) -> io::Result<Box<dyn ChildProcess>>;
}
#[async_trait]
pub trait ChildProcess: Send {
    async fn communicate(&mut self, timeout: Duration) -> Result<(Vec<u8>, Vec<u8>), TimedOut>;
    fn returncode(&self) -> Option<i32>;
    fn kill(&mut self);
    async fn wait(&mut self);
}

pub trait HttpSession: Send + Sync {
    fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError>;
}
pub struct HttpResponse { pub status_code: u16, pub headers: Vec<(String, String)>, body: Box<dyn BodyStream> }
pub trait BodyStream: Send { fn next_chunk(&mut self) -> Option<Result<Vec<u8>, HttpError>>; fn close(&mut self); }
impl HttpResponse {
    pub fn text(self) -> String;            // 본문 전체를 UTF-8로
    pub fn json(self) -> Value;
    pub fn from_text(status: u16, text: &str) -> Self;           // fake 생성용
    pub fn from_chunks(status: u16, headers: Vec<(String, String)>, chunks: Vec<Vec<u8>>, closed: Arc<AtomicBool>) -> Self;
    pub fn is_redirect(&self) -> bool; pub fn location(&self) -> Option<&str>;
}
```

- `CommandRunner`는 `book_manager_1`의 `run(argv, timeout)`에 맞췄다(12회 사용). `libs/isbn::CommandRunner`는 모양이 다르다. 나중에 `sysdeps`로 옮기고 지금은 그대로 둔다.
- `find_libreoffice_with(which, exists)`는 `doc_reader::office`에 이미 있고 book_manager_1과 loader_1이 같은 모양이다. 그대로 쓰고 `which`/`exists`에는 `CommandRunner::which`, `FsOps::is_file`을 넘긴다.
- `HttpSession`은 bookstore H1(완결 본문)과 cover_sources H2(청크 스트림, `closed` 플래그 검증)를 함께 덮는다. `direct_cover_url`의 `Net/Pool`은 DNS 고정·TLS 호스트명 검증용 저수준이라 따로 둔다.

### 3.2 앱 진입점·인증 (`backend::main`)

```rust
pub fn app() -> Router { app_with(AppDeps::from_env()) }
pub fn app_with(deps: AppDeps) -> Router;

pub enum AuthMode {
    Cookies,                                   // 실제 쿠키 인증 (구 Real)
    Fixed(Value),                              // require_auth, require_admin 둘 다 고정
    Split { auth: Value, admin: Value },       // 구 Overrides{auth,admin}
    AuthOnly(Value),                           // require_auth만 고정, admin은 실제
    Shared(Arc<AuthSlots>),                    // 테스트 중간에 바꾸는 값 (main_mock). AuthSlots{auth: Mutex<Value>, admin: Mutex<Value>}
}

pub struct AppDeps { /* 비공개 필드 */ }
impl AppDeps {
    pub fn new(auth: AuthMode) -> Self;        // 나머지는 실제 구현 또는 env 기본값
    pub fn from_env() -> Self;
    pub fn with_book_manager(self, m: Arc<dyn BookManagerApi>) -> Self;
    pub fn with_comics_manager(self, m: Arc<dyn BookManagerApi>) -> Self;
    pub fn with_category_mapping(self, m: Arc<dyn CategoryMappingApi>) -> Self;
    pub fn with_view_history_store(self, s: Arc<dyn ViewHistoryApi>) -> Self;
    pub fn with_book_access(self, a: Arc<dyn BookAccess>) -> Self;           // 구 with_viewer_check
    pub fn with_refresh_token_store(self, s: Arc<dyn RefreshTokenStoreApi>) -> Self;
    pub fn with_google_verifier(self, v: Arc<dyn GoogleVerifier>) -> Self;   // client_id 포함
    pub fn with_role_resolver(self, r: Arc<dyn RoleResolver>) -> Self;
    pub fn with_fs(self, fs: Arc<dyn FsOps>) -> Self;                        // list_dir, fail_next_iterdir
    pub fn with_clock(self, c: Arc<dyn Clock>) -> Self;
    pub fn with_task_runner(self, r: Arc<dyn TaskRunner>) -> Self;           // 백그라운드 작업 인라인 실행, to_thread_spy
    pub fn with_bookstore_factory(self, f: Arc<dyn Fn() -> Box<dyn BookstoreSearch> + Send + Sync>) -> Self;
    pub fn with_reload_progress_flush_interval(self, d: Duration) -> Self;   // 구 testing::set_reload_..
}

pub trait GoogleVerifier: Send + Sync {
    fn client_id(&self) -> Option<String>;
    fn verify(&self, credential: &str, audience: &str) -> Result<Value, String>;
}
pub trait RoleResolver: Send + Sync { fn determine_role(&self, email: &str) -> Option<String>; }

pub trait TaskRunner: Send + Sync {
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle>;
    fn spawn_blocking_observed(&self, name: &str);   // to_thread_spy 대응: 이름만 기록하는 기본 구현
}
pub struct ThreadRunner;    // 실제
pub struct InlineRunner;    // 응답 전에 끝난다 (Starlette TestClient 가정)

// auth 모듈: 요청 추출을 trait로 두는 현재 가정을 유지한다.
pub fn create_jwt_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String;
pub fn create_refresh_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String;
```

- 가장 많은 테스트가 쓰는 A1(main 계열 119개)의 이름을 채택했다. 변경점은 두 가지다. 빌더 이름에 `with_` 접두사를 붙이고, 5번째 변형 `Shared`를 추가했다.
- `create_jwt_token`은 4인자(auth, main이 쓴다) 하나로 고정한다. 2·3인자를 쓰는 파일은 `common::fx::jwt(email, role)` 헬퍼로 감싼다(호출 8곳).
- A3 `DynManager`는 폐기하지 않는다. main_mock의 `Mock`(호출 기록·반환값 스크립트)은 테스트 로컬 어댑터가 된다. `impl BookManagerApi for Mock`은 모든 메서드를 `self.call("name", CallArgs)`로 넘기는 매크로로 만든다. 129개 테스트 본문은 안 바뀐다.
- A7 `EpubCoverApi`는 `BookManagerApi`에 합친다(`set_epub_cover`, `remove_epub_cover`, `insert_epub_illustration`을 메서드로 추가).
- A9 `create_cover_router`는 공개 API가 아니라 `cover_management` 모듈의 내부 조립이다. `AdminGate`는 `AuthMode::Fixed`/`Split`으로 대체하고 `AdminGate::deny(status, detail)`만 `common`에 남긴다(401/403을 내는 `RoleResolver`가 아니라 `AuthMode`에 `Denied{status, detail}` 한 변형이 필요한지는 미결정 2번).

`BookManagerApi` 통일 시그니처(충돌 메서드만):

```rust
#[async_trait]
pub trait BookManagerApi: Send + Sync {
    fn path_prefix(&self) -> PathBuf;
    fn es_manager(&self) -> Arc<dyn EsManagerApi>;
    async fn get_book(&self, id: i64) -> (Option<Book>, Option<String>);
    async fn get_books_in_category_paged(&self, category: &str, size: Option<i64>, cursor: Option<&str>)
        -> (Vec<Book>, i64, Option<String>, Option<String>);      // (목록, 전체 수, 다음 커서, 오류)
    async fn get_book_content(&self, id: i64) -> BookContent;      // File(PathBuf) | Text(String)
    async fn get_book_preview(&self, id: i64, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse;
    async fn get_pdf_pages(&self, id: i64, start: i64, end: i64) -> BookResponse;
    fn get_category_mismatches(&self, force_refresh: bool) -> Result<Value, String>;
    async fn update_book(&self, id: i64, category: &str, title: &str, author: &str, path: &Path, file_type: &str, force: bool)
        -> (Value, Option<String>);
    // ... 나머지는 각 파일 머리말 목록. 모든 메서드에 기본 구현.
}
pub struct BookResponse { pub status_code: u16, pub media_type: Option<String>, pub headers: Vec<(String, String)>, pub body: Vec<u8> }
// 구 PreviewResponse, PdfPagesResponse, CoverResponse, Response 를 모두 BookResponse 하나로 쓴다.
```

### 3.3 ES

```rust
// backend::es_manager
pub enum EsError {              // 실제 es_client::EsError에 변형 3개를 추가한다
    MissingEnv(String), Transport(String), Status { status: u16, body: String }, Parse(String), Unsupported(String),
    Other(String),              // 구 ElasticsearchError::RuntimeError, book_manager_2의 EsError::Other
}
impl EsError { pub fn is_not_found(&self) -> bool; pub fn is_connection(&self) -> bool; }
// 구 ElasticsearchError 매핑: ConnectionError -> Transport, NotFoundError -> Status{404}, BadRequestError{message,status} -> Status{status, body:message}, RuntimeError -> Other

// 저수준: ESManager 단위 테스트와 분류기 corpus가 쓴다. 구 EsApi(call), EsLike
pub trait EsTransport: Send + Sync {
    fn index_name(&self) -> &str;
    fn call(&self, op: &str, kwargs: &Value) -> Result<Value, EsError>;          // "indices.exists" 등
    fn get(&self, kwargs: &Value) -> Result<Value, EsError> { self.call("get", kwargs) }
    fn search(&self, kwargs: &Value) -> Result<Value, EsError> { self.call("search", kwargs) }
}

// 고수준: BookManager, 라우트, 분류기 조회가 쓴다. 모든 메서드 기본 구현 = Err(EsError::Unsupported)
pub trait EsManagerApi: Send + Sync {
    fn create_index(&self) -> Result<Value, EsError>;
    fn search_by_id(&self, doc_id: i64) -> Result<Option<Value>, EsError>;
    fn update(&self, doc_id: i64, fields: &Value) -> Result<bool, EsError>;
    fn delete(&self, doc_id: i64) -> Result<bool, EsError>;
    fn insert(&self, docs: &Docs) -> Result<Vec<i64>, EsError>;                       // Docs = BTreeMap<i64, Value>
    fn delete_by_file_paths(&self, paths: &[String], exclude_ids: Option<&[i64]>) -> Result<i64, EsError>;
    fn delete_by_ids(&self, ids: &[i64], chunk_size: usize) -> Result<i64, EsError>;
    fn refresh(&self) -> Result<(), EsError>;
    fn search_and_aggregate_by_category(&self) -> Result<BTreeMap<String, i64>, EsError>;
    fn iter_all_category_file_paths(&self, batch: usize) -> Result<Vec<(String, String, i64)>, EsError>;
    fn search_categories_pdf_stats(&self, categories: &[String]) -> Result<Value, EsError>;
    fn search_category_pdf_stats(&self, category: &str) -> Result<Value, EsError>;
    fn get_by_id_source(&self, id: &str) -> Result<Option<Value>, EsError>;            // 구 EsLookup::get_by_id
    fn search_by_path_source(&self, path: &str) -> Result<Option<Value>, EsError>;     // 구 EsLookup::search_by_path
    // 나머지: es_manager.rs 머리말 목록 (search_*_paged, count_*, aggregate_cover_categories 등)
}

impl ESManager {
    pub fn with_transport(es: Arc<dyn EsTransport>) -> Self;                    // 구 with_client(es, index)
    pub fn connect(env: EnvFn, connector: &dyn EsConnector, clock: &dyn Clock) -> Result<Self, ESManagerError>;  // 구 new_with(.., sleep)
}
// ESManager는 EsManagerApi를 구현한다. BookManager는 Arc<dyn EsManagerApi>만 안다.
```

- `ESManager` 자기 비공개 메서드 바꿔치기(`set_search_override` 등 6종, 15개 테스트)는 trait로 올리지 않는다. 대신 `EsTransport`에 `call("search")` 응답을 스크립트하는 `DummyEs::on(op, handler)`(common/es_dummy에 이미 있음)로 같은 분기를 만든다. `_search` 반환을 직접 주입해야 하는 3~4개 테스트만 `ESManager::with_search_stub(Arc<dyn Fn>)` 한 개를 남긴다(미결정 아님, 구현 시 판단).
- 분류기: `EsLookup`/`EsLike`는 없앤다. `ClassifierOptions.es: Option<Arc<dyn EsTransport>>`로 `get`/`search`를 직접 쓰고, 상위 `_source` 조회가 필요하면 `EsManagerApi::get_by_id_source`를 쓴다(미결정 아님).
- `BookIndex`(cover)는 `EsManagerApi`의 부분집합이라 없앤다. 기본 구현 덕분에 `search_by_id`와 `update`만 구현한 fake가 그대로 `EsManagerApi`가 된다.
- `loader`의 `IndexStore`, `es_client::BookStore/BookSearch`는 그대로 둔다.

### 3.4 DB (`backend::db`)

D1이 이미 두 파일의 공통 가정이다. 그대로 채택하고 세 가지만 더한다.

```rust
pub enum DbValue { Null, Int(i64), Text(String), DateTime(NaiveDateTime) }   // From<&str>, From<i64>, From<Value>
pub type Row = BTreeMap<String, DbValue>;
pub enum DbError { IntegrityError(String), Other(String) }
pub trait DbCursor { fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError>;
                     fn executemany(&mut self, sql: &str, seq: &[Vec<DbValue>]) -> Result<(), DbError>;
                     fn fetchall(&mut self) -> Option<Vec<Row>>; fn fetchone(&mut self) -> Option<Row>; fn rowcount(&self) -> u64; }
pub trait DbConnection: Send {
    fn cursor(&mut self) -> Box<dyn DbCursor>;
    fn commit(&mut self) -> Result<(), DbError>; fn rollback(&mut self) -> Result<(), DbError>; fn close(&mut self);
    // 추가: 실제 MySQL 테스트용 편의 메서드 (기본 구현은 cursor()를 감싼다)
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<u64, DbError> { /* ... */ }
    fn fetch_all(&mut self, sql: &str, params: &[DbValue]) -> Result<Vec<Row>, DbError> { /* ... */ }
    fn fetch_one(&mut self, sql: &str, params: &[DbValue]) -> Result<Option<Row>, DbError> { /* ... */ }
}
pub type ConnectionProvider = Arc<dyn Fn() -> Result<Box<dyn DbConnection>, DbError> + Send + Sync>;
pub struct DbConfig { pub host: String, pub port: u16, pub database: String, pub user: String, pub password: String }
impl DbConfig { pub fn from_env(env: EnvFn) -> Result<Self, String>; }   // TM_MYSQL_*
```

생성자 규칙(세 저장소 공통: `CategoryMapping`, `MySQLRefreshTokenStore`, `ViewHistoryStore`):

```rust
X::new() -> Result<Self, XError>                                   // env + 실제 MySQL, init_db 실행
X::with_connector(provider: ConnectionProvider) -> Result<Self, XError>   // init_db 실행 (마이그레이션 예외는 삼킴)
X::with_connection_provider(provider: ConnectionProvider) -> Self        // init_db 생략 (구 __new__ + 대입)
```

- `set_connection_provider(&mut self)`는 없앤다. category_mapping 37개 테스트가 `with_connection_provider`로 바뀐다(기계적).
- `StoreConnection`(D2)은 `DbConnection`으로 대체한다. `with_connection_factory(Fn)`는 `with_connection_provider`, `set_max_history_rows(n)`는 `with_max_history_rows(n)`(설정 값)이 된다.
- `get_connection()`은 모두 `Result<Box<dyn DbConnection>, DbError>`를 돌려준다. `mysql::Conn`을 노출하지 않는다. 실제 MySQL 테스트(D3)는 편의 메서드를 쓰고 파라미터를 `DbValue`로 쓴다. `serde_json::Value` 파라미터는 `DbValue::from(Value)`로 감싼다.
- 컨테이너는 `common/mysql_fixture`(testcontainers)로 하나만 둔다. `TM_MYSQL_HOST`가 있으면 외부 DB를 쓴다. 둘 다 `#[ignore]`다.

### 3.5 파일시스템 (`sysdeps::FsOps`)

`FsOps`(3.1)가 F1~F9를 모두 덮는다. 주입 지점은 세 곳이다.

```rust
BookManagerDeps::with_fs(Arc<dyn FsOps>)          // F1, F2, F4, F5, F6
BookstoreBase::with_fs(Arc<dyn FsOps>)            // F3 (disk_free, write)
EpubCoverHooks::with_fs(Arc<dyn FsOps>)           // F7 (sync_file, lock_exclusive)
store_lock_with(dir, store, &dyn FsOps, &dyn Clock)   // cover_sources
doc_reader::text::TextHooks.fs                    // F8 (ReencodeIo 대체: write 실패, 재읽기 불일치)
```

- `FaultyFs`(book_manager_1의 9개 훅 구현)를 `common/faulty_fs.rs`로 옮겨 모든 파일이 공유한다. 호출 N번째만 실패시키는 `fail_on_nth_call`도 함께 옮긴다.
- 전역 가드 24곳(F2)은 `let manager = manager.with_fs(Arc::new(faulty))` 형태로 바뀐다.
- `scandir`과 `read_dir`은 `read_dir` 하나로 합친다. Rust 구현에서 둘 다 `fs::read_dir`이고 구분해서 실패시키는 테스트는 `FaultyFs::set_read_dir(path_filter)`로 경로로 구분한다.

### 3.6 외부 명령 (`sysdeps::CommandRunner`, `ProcessSpawner`)

```rust
BookManagerDeps::with_runner(Arc<dyn CommandRunner>)        // C1, C4 (libreoffice 변환은 runner 위에서 동작)
BookManagerDeps::with_process_spawner(Arc<dyn ProcessSpawner>)   // C3 (epubcheck)
doc_reader::pdf::PdftotextHooks { which, run }             // C6, C7 통합. 이름: Pdftotext/PdftotextEnv 제거
```

- C4의 세 이름(`override_convert_with_libreoffice`, `set_convert_with_libreoffice_hook`, `set_convert_override`)은 모두 `with_runner(FakeRunner)` + `convert_with_libreoffice_with(&dyn CommandRunner, ..)`로 합친다. "변환기가 문자열을 돌려준다"는 단순 대역이 필요한 테스트는 `common::FakeRunner::office_output(text)` 헬퍼를 쓴다.
- `StageRunner`(실제 crate)는 건드리지 않는다.

### 3.7 HTTP·이미지 fetch

```rust
// backend::direct_cover_url
pub trait ImageFetcher: Send + Sync { fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError>; }   // 구 FetchUrlImage, ImageFetcher
pub enum FetchError { Value(String), Os(String), Other(String) }
impl From<FetchError> for CoverError; impl From<FetchError> for EpubCoverError;
pub struct RealImageFetcher { net: Arc<dyn Net>, max_image_bytes: usize }
pub fn fetch_url_image_with(url: &str, net: &dyn Net, max_image_bytes: usize) -> Result<Vec<u8>, FetchError>; // 변경 없음, Net/Pool은 그대로

// 주입 지점
BookManagerDeps::with_image_fetcher(Arc<dyn ImageFetcher>)
CoverHooks::with_image_fetcher(Arc<dyn ImageFetcher>)        // 구 with_fetch_url_image(Fn)
SearchHooks::with_session_factory(Arc<dyn Fn(&str) -> Arc<dyn HttpSession> + Send + Sync>)   // 구 with_fetch/with_client_factory의 fetch 부분
Bookstore::with_session(Arc<dyn HttpSession>)                // 구 pub 필드 session: Box<dyn Session>, new_detail_session
```

- `common::fx::fetcher(|url| ...)`가 클로저를 `ImageFetcher`로 감싼다. cover_management의 39개 `with_fetch_url_image(closure)` 호출은 `with_image_fetcher(fx::fetcher(closure))`로 기계 치환된다.
- `Net`/`Pool`/`PoolResponse`(H3)는 유일한 사용처가 `direct_cover_url`이라 그대로 둔다. `HttpSession`과 합치지 않는다. 이유는 DNS 결과 고정과 TLS 호스트명 단언 같은 연결 수준 계약이 `HttpSession`에 없어서다.
- `api_exploration`의 `Okt`, `HttpGet`은 파일 안 로컬이라 건드리지 않는다.

### 3.8 시계·sleep

```rust
// sysdeps::Clock (3.1). 주입은 항상 with_clock(Arc<dyn Clock>)
AppDeps::with_clock, BookManagerDeps::with_clock, CoverHooks::with_clock, StoreGate::with_clock,
ESManager::connect(env, connector, clock), ClassifyCliDeps::with_clock
// common
pub struct FakeClock { /* now, monotonic 동일 시간선 */ }
impl FakeClock { pub fn new(start: f64) -> Arc<Self>; pub fn advance(&self, s: f64); pub fn slept(&self) -> Vec<f64>; pub fn clear_slept(&self); }
```

- `ESManager::new_with(.., sleep: &dyn Fn(u64))`의 `sleeps`는 `FakeClock::slept()`로 대체한다(재시도 대기 2, 4초 검증은 그대로 가능).
- 설정값(T5, T6, T7)은 시계 seam이 아니다. `with_delay`, `set_mismatch_cache`, `with_reload_progress_flush_interval`은 값 설정자로 둔다.

### 3.9 환경 변수

- 프로덕션: 모든 `from_env`는 `EnvFn`(`&dyn Fn(&str) -> Option<String>`)을 받는 `from_env(env)`와 실제 환경을 읽는 `from_process_env()` 한 쌍이다. `loader::Config::from_env`, `path_prefix_from_env`, `ComicsManager::from_env`, `BookManager::from_env`가 이 모양이다.
- 테스트: `common::setup_env()`는 test binary당 한 번만 실행하는 고정 값(경로, 비밀키, 관리자)에 쓴다. 테스트마다 값이 바뀌어야 하면 `common::env::EnvGuard`(락 + 복원)를 쓴다. auth.rs, main.rs, main_mock.rs, refresh_token_store.rs, book_manager_2.rs, classifier.rs가 따로 만든 6개 구현을 하나로 합친다.
- `auth`는 환경 변수를 호출 시점에 읽는다(Python `importlib.reload` 불필요라는 현재 가정 유지). 시크릿 로딩은 `load_jwt_secret(env: EnvFn) -> Result<String, SystemExit>`.

### 3.10 로깅 캡처

```rust
// tests/rust/backend/common/log_capture.rs  (프로덕션 crate에는 log_capture 모듈이 없다)
pub struct LogCapture { /* 스레드 로컬 버퍼 소유, drop 시 비움 */ }
impl LogCapture {
    pub fn start(level: log::Level) -> Self;       // caplog.at_level
    pub fn records(&self) -> Vec<CapturedRecord>;  // caplog.records
    pub fn messages(&self) -> Vec<String>;
    pub fn text(&self) -> String;                  // caplog.text
}
```

- L2(`backend::log_capture`), L4(`utils::classify_cli::capture_*`), L3(로컬 `CaptureLogger`)는 모두 이 하나로 바꾼다. 프로덕션 코드는 `log` crate만 쓴다.
- classify CLI 표준 출력은 `main_with(args, &mut CliIo, &deps)`의 `CliIo{out, err}: &mut dyn Write`로 캡처한다(3.12).
- 한계: 스레드 로컬이라 `spawn_blocking`·워커 스레드에서 남긴 로그는 안 잡힌다. 그런 테스트는 `InlineRunner`(3.2)로 같은 스레드에서 돌린다.

### 3.11 PDF·이미지

```rust
// backend::pdf
pub trait PdfOpener: Send + Sync { fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>, PdfError>; }       // 구 PdfOpener, PdfInspector
pub trait PdfDoc: Send {
    fn page_count(&self) -> usize;                                   // cover_status의 len()도 이 이름
    fn pdf_version(&self) -> String;
    fn docinfo(&self) -> HashMap<String, String>;
    fn check_pdf_syntax(&self) -> Result<Vec<String>, PdfError>;
    fn close(&self);                                                 // 호출 여부를 검증하는 테스트가 있다
}
pub trait PdfPageOps: Send + Sync {                                  // 구 PdfBackend
    fn page_count(&self, path: &Path) -> Result<usize, PdfError>;
    fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, PdfError>;
}
pub struct PdfError { pub kind: String, pub message: String }
pub struct PdfCacheLimits { pub max_file_bytes: Option<u64>, pub max_bytes: Option<u64>, pub max_entries: Option<usize> }

BookManagerDeps::with_pdf_opener(Arc<dyn PdfOpener>)
BookManagerDeps::with_pdf_pages(Arc<dyn PdfPageOps>)
BookManagerDeps::with_pdf_cache_limits(PdfCacheLimits)
BookManager::pdf_cache(&self) -> &PdfReaderCache                    // clear(), contains(&Path), len(), entry_sizes()  (구 클래스 수준 정적 캐시)
```

- `PdfInfo` 데이터형(`PdfInspector`)은 `common::StaticPdfDoc(PdfInfo)`가 `PdfDoc`을 구현하는 헬퍼로 흡수한다. 정적 캐시는 인스턴스 소유로 바꾸므로 `serial()` 락과 `set_pdf_reader_cache_limits` 가드가 필요 없다.
- `with_open_pdf_observer`는 `PdfOpener` fake가 호출 횟수를 세는 것으로 대체한다.
- `pdf_text_by_plumber_with(&Path, &dyn PlumberDocument)`(loader_2)는 PdfDoc이 아니라 텍스트 추출 모델이라 그대로 둔다.
- 이미지 한도(`MAX_IMAGE_PIXELS` 등)는 값 설정이다. `EpubCoverHooks::with_limits(Limits)` 하나로 받는다(3.12).

### 3.12 매니저 생성자와 그 외

```rust
pub struct BookManagerDeps { /* 비공개 */ }
impl BookManagerDeps {
    pub fn new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>) -> Self;   // 인자 순서 (path, es). book_fixtures의 (es, path)를 뒤집는다
    pub fn with_book_lookup(self, l: Arc<dyn BookLookup>) -> Self;           // 구 .with_book(path, type)은 common의 StaticBookLookup
    pub fn with_file_reader(self, r: Arc<dyn FileReader>) -> Self;           // 구 ReadFile, set_read_file, loader_read_file_override
    pub fn with_book_adder(self, a: Arc<dyn BookAdder>) -> Self;             // 구 AddBook, override_add_book
    pub fn with_fs(self, fs: Arc<dyn FsOps>) -> Self;
    pub fn with_runner(self, r: Arc<dyn CommandRunner>) -> Self;
    pub fn with_process_spawner(self, s: Arc<dyn ProcessSpawner>) -> Self;
    pub fn with_pdf_opener(self, o: Arc<dyn PdfOpener>) -> Self;
    pub fn with_pdf_pages(self, p: Arc<dyn PdfPageOps>) -> Self;
    pub fn with_pdf_cache_limits(self, l: PdfCacheLimits) -> Self;
    pub fn with_image_fetcher(self, f: Arc<dyn ImageFetcher>) -> Self;
    pub fn with_thumbnailer(self, t: Arc<dyn Thumbnailer>) -> Self;          // 구 ThumbnailExtractor, with_es_and_extractor
    pub fn with_classifier_factory(self, f: Arc<dyn Fn() -> Arc<dyn Classify> + Send + Sync>) -> Self;
    pub fn with_clock(self, c: Arc<dyn Clock>) -> Self;
    pub fn with_task_runner(self, r: Arc<dyn TaskRunner>) -> Self;           // 구 override_thread_start(_run_inline), BackfillThread
    pub fn with_epub_hooks(self, h: EpubCoverHooks) -> Self;
    pub fn with_trash_dirname(self, n: &str) -> Self;
    pub fn with_bulk_reindex_batch_size(self, n: usize) -> Self;
    pub fn with_scan_observer(self, f: Arc<dyn Fn() + Send + Sync>) -> Self; // 구 set_scan_hook
}
impl BookManager {
    pub fn with_deps(deps: BookManagerDeps) -> Self;
    pub fn from_env(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>) -> Result<Self, RuntimeError>;  // 구 new_from_env/new_with/with_es_url
}
// ComicsManager도 같은 모양: ComicsManager::from_env(env, es_factory). 구 IndexManager trait은 EsManagerApi로 대체.
// 모든 BookManager 메서드는 &self. 소비형 with_* 빌더 한 번에 구성하고 이후 변경하지 않는다.

// 분류기 3계층
pub trait CategoryModelApi: Send + Sync {                  // 구 Model, ClassifierModel
    fn min_confidence(&self) -> f64;
    fn predict_one(&self, doc: &Value, min_confidence: Option<f64>) -> Prediction;
    fn predict(&self, docs: &[Value], min_confidence: Option<f64>) -> Vec<Prediction>;
    fn expected_accuracy(&self, confidence: f64) -> Option<f64>;
}
pub trait Classify: Send + Sync {                          // 구 Classify, Classifier. is_loaded -> is_available
    fn is_available(&self) -> bool;
    fn classify_path(&self, path: &Path, min_confidence: Option<f64>) -> Result<Prediction, String>;
    fn classify_document(&self, doc: &Value, min_confidence: Option<f64>) -> Result<Prediction, String>;
}
pub trait BookClassifierApi: Send + Sync {                 // 구 FileClassifier, ClassifierService (BookManager가 쓰는 서비스 facade)
    fn classify_file(&self, path: &Path) -> (Option<String>, String, String, Value);
    fn override_below(&self) -> f64; fn prefers_bookstore(&self, confidence: f64) -> bool;
    fn min_confidence(&self) -> Option<f64>; fn expected_accuracy(&self, confidence: Option<f64>) -> Option<Option<f64>>;
}
BookCategoryClassifier::new(model: Option<Arc<dyn CategoryModelApi>>, ClassifierOptions { es: Option<Arc<dyn EsTransport>>, library_root: Option<PathBuf>, min_confidence: Option<f64> })
Prediction::new(category: Option<&str>, confidence: f64, top: Vec<(String, f64)>, reason: &str)   // 필드: category, confidence, ranked, reason
BookClassifierService::new(root: &Path, cache_file: Option<&Path>, classifier: Option<Arc<dyn Classify>>)
    .with_delay(f64).with_bookstore_cache_file(&Path).with_bookstore_policy(BookstorePolicy).with_bookstore_decider(Arc<dyn BookstoreDecider>)   // 구 wrap_decide_by_bookstore
```

```rust
// epub_cover: 훅 구조체 하나로 통일 (구 cover_management::Hooks ∪ epub_cover::hooks::set_* ∪ set_max_image_pixels ∪ set_pixel_decode_hook)
#[derive(Clone, Default)]
pub struct EpubCoverHooks { /* Option 필드 */ }
impl EpubCoverHooks {
    pub fn with_limits(self, l: Limits) -> Self;                 // max_epub_bytes, max_xml_bytes, max_image_pixels, max_image_bytes
    pub fn with_fs(self, fs: Arc<dyn FsOps>) -> Self;            // fsync, flock
    pub fn with_find_opf_path(self, f: Arc<dyn Fn() -> String + Send + Sync>) -> Self;
    pub fn with_revision(self, f: Arc<dyn Fn(&Path) -> String + Send + Sync>) -> Self;
    pub fn with_testzip(self, f: Arc<dyn Fn(&Path) -> Option<String> + Send + Sync>) -> Self;
    pub fn with_inspect_archive(self, f: Arc<dyn Fn(Value) -> Value + Send + Sync>) -> Self;
    pub fn with_uuid_hex(self, f: Arc<dyn Fn() -> String + Send + Sync>) -> Self;
    pub fn with_pixel_decode_observer(self, f: Arc<dyn Fn() + Send + Sync>) -> Self;
}
// 말단 함수: inspect_epub_with(path, &hooks), add_cover_with(.., &hooks), normalize_image_with(.., &hooks), edit_images_with(.., &hooks)

// CoverHooks (cover_management): 이미 with_* 빌더. 이름만 정리
CoverHooks::{with_search, with_fetch_bytes, with_image_fetcher, with_clock, with_epub_hooks(EpubCoverHooks), with_cached_search, with_extract_thumbnail → with_thumbnailer, ...}

// classify CLI: 전역 가드 15종을 Deps 하나로
pub struct ClassifyCliDeps { /* config: Arc<dyn ConfigStore>, es_factory, corpus: Arc<dyn CorpusOps>,
                                service_factory, fs: Arc<dyn FsOps>, clock: Arc<dyn Clock>, boundary_factor ... */ }
impl ClassifyCliDeps { pub fn system() -> Self; pub fn with_config_store(..) -> Self; pub fn with_corpus(..) -> Self; /* ... */ }
pub struct CliIo<'a> { pub out: &'a mut dyn Write, pub err: &'a mut dyn Write }
pub fn main_with(args: &[&str], io: &mut CliIo, deps: &ClassifyCliDeps) -> Result<i32, SystemExit>;
pub fn main(args: &[&str]) -> Result<i32, SystemExit> { main_with(args, &mut CliIo::stdio(), &ClassifyCliDeps::system()) }

// loader / doc_reader: 말단 *_with(args, &Hooks) 하나로 통일 (loader_2의 방식을 채택)
doc_reader::text::TextHooks { detect, fs, ... }       // loader_1의 detect 클로저, ReencodeIo를 흡수
doc_reader::pdf::FastCountHooks { find_xref_offset, parse_one_xref_stream, xref_stream_find_entry, read_from_obj_stream }   // XrefOps를 흡수
doc_reader::pdf::PdftotextHooks { which, run }
doc_reader::epub::EpubHooks { read_epub (ebooklib 경로) }      // 구 read_from_epub_using
loader::record::ReadHooks { fast_page_count, fallback_page_count, inspect (detect_file_type_with), ... }
loader::cli::RunHooks { read_file, stat, on_worker_spawn }
```

- `loader`는 이미 실제 crate와 실제 API를 쓰는 파일이 많다(loader_gaps, loader_hwp3는 변경 없음). loader_1과 loader_2 사이에서만 `_with` 시그니처가 충돌하고, 해법은 Hooks 구조체 쪽(loader_2)이다. 이유는 (a) 한 함수에 훅이 2개 이상일 때 위치 인자가 늘지 않고 (b) `Default`로 안 쓰는 훅을 생략할 수 있어서다.

### 3.13 이름 매핑표(구 → 신)

| 구                                                                           | 신                                                                                                 |
| ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `EsApi`(call) / `EsLike` / `EsLookup` / `BookIndex` / `IndexManager`         | `EsTransport` / `EsTransport` / `EsManagerApi::get_by_id_source` / `EsManagerApi` / `EsManagerApi` |
| `ElasticsearchError`                                                         | `EsError`(`Status{404}`, `Transport`, `Other`)                                                     |
| `StoreConnection`, `Connection`, `mysql::Conn`                               | `DbConnection`                                                                                     |
| `StoreFs`, `FaultyFs`(trait impl)                                            | `FsOps` / `common::FaultyFs`                                                                       |
| `FetchUrlImage`, `ImageFetcher`                                              | `ImageFetcher`                                                                                     |
| `bookstore::Session`, `cover_sources::Session`                               | `HttpSession`                                                                                      |
| `EpubcheckSpawner`/`EpubcheckProcess`                                        | `ProcessSpawner`/`ChildProcess`                                                                    |
| `PdfInspector`, `PdfOpener`+`PdfDoc`, `PdfDocument`                          | `PdfOpener`+`PdfDoc`                                                                               |
| `PdfBackend`                                                                 | `PdfPageOps`                                                                                       |
| `ReadFile`, `AddBook`, `ThumbnailExtractor`                                  | `FileReader`, `BookAdder`, `Thumbnailer`                                                           |
| `Model`, `ClassifierModel`                                                   | `CategoryModelApi`                                                                                 |
| `Classifier`, `Classify`                                                     | `Classify`                                                                                         |
| `FileClassifier`, `ClassifierService`                                        | `BookClassifierApi`                                                                                |
| `Overrides`, `AppOverrides`, `AuthMode::{Real, Overrides}`                   | `AppDeps`, `AppDeps`, `AuthMode::{Cookies, Split}`                                                 |
| `EpubCoverApi`, `PreviewResponse`, `PdfPagesResponse`, `CoverResponse`       | `BookManagerApi`, `BookResponse`                                                                   |
| `for_test`, `new_for_test`, `with_es`, `without_es`, `testing::make_manager` | `BookManager::with_deps(BookManagerDeps::new(path, es)...)`                                        |
| `Hooks`(cover*management), `epub_cover::hooks::set*\*`                       | `EpubCoverHooks`                                                                                   |
| `override_*`, `fs_*_override`, `testing::*`, `set_*_override`                | 삭제. 위 Deps/Hooks로 대체                                                                         |

---

## 4. 파일별 영향표

기호: **본문**은 테스트 함수 본문을 고쳐야 하는 개수 추정(기계적 M / 수작업 R). **헬퍼**는 파일 안 fake·helper만 고치는 경우(○), `-`는 변경 없음. `T`는 테스트 수, `ign`은 `#[ignore]` 개수.

### 4.1 backend

| 파일                        | T   | ign | 쓰는 변형                                | 통일 후 변경                                                                                                                                                                                                                                              | 본문           | 헬퍼 |
| --------------------------- | --- | --- | ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------- | ---- |
| api_exploration             | 9   | 0   | H8(로컬)                                 | 없음                                                                                                                                                                                                                                                      | 0              | -    |
| auth                        | 96  | 34  | A2, V1, V2, V4, L2                       | 클로저 빌더 12종 → `GoogleVerifier`/`RoleResolver`/`RefreshTokenStoreApi` fake, `AuthMode::Real/Overrides` → `Cookies/Split`, `backend::log_capture` → `common::LogCapture`, `EnvGuard` → `common::env`                                                   | 35 R           | ○    |
| auth_bookstore_gaps         | 2   | 0   | -                                        | 없음                                                                                                                                                                                                                                                      | 0              | -    |
| book                        | 6   | 0   | V3                                       | 없음(`EnvFn` 그대로)                                                                                                                                                                                                                                      | 0              | -    |
| book_classifier             | 40  | 2   | `Classify`, `BookClassifierService::new` | `Classify`는 현행 유지, `Option<Box<dyn Classify>>` → `Option<Arc<..>>`                                                                                                                                                                                   | 2 M            | ○    |
| book_manager_1              | 155 | 62  | M1, M9, F1, C1, C5, P1, P2, E3           | `for_test` → `Deps`(헬퍼), `manager.set_*` 37곳 → `with_*`, `set_fs_ops` 등 trait를 `sysdeps`로 이동, `DummyEs` 반환형 정리, `new_from_env/new_with/with_es_url` → `from_env`                                                                             | 45 (M 40, R 5) | ○    |
| book_manager_2              | 154 | 34  | M2, E1, E4, F2, C4, L5                   | 전역 가드 `override_*`/`fs_*_override` 39곳 → `Deps.with_*`, `DummyEs`를 `EsApi`에서 `EsManagerApi`로 재작성(~150줄), `capture_warnings` → `LogCapture`                                                                                                   | 40 R           | ○    |
| book_manager_gaps           | 47  | 13  | M8                                       | `testing::{DummyEs, make_manager, make_doc}` → `common::es_fake`(import만), `_model_decision_floor(&dyn ClassifierService)` → `BookClassifierApi`                                                                                                         | 3 M            | ○    |
| book_manager_hwp_preview    | 1   | 1   | M8, C4                                   | `set_convert_with_libreoffice_hook` → `with_runner(FakeRunner)`                                                                                                                                                                                           | 1 R            | -    |
| book_preview                | 99  | 52  | M5, C3, P3, F5, E3, A10                  | `hooks::set_*` 4종 → `with_process_spawner`/`with_pdf_opener`/`with_fs`, `BookManager::new()` → `from_env`                                                                                                                                                | 11 R           | ○    |
| bookstore                   | 97  | 17  | H1, F3                                   | `Session` → `HttpSession`(fake `FakeSession` 수정), `StoreFs` → `FsOps`, `set_fetch_search_results` → `with_*`                                                                                                                                            | 5 M            | ○    |
| bookstore_cache             | 19  | 0   | L3, T5                                   | `CaptureLogger` → `common::LogCapture`                                                                                                                                                                                                                    | 3 M            | ○    |
| bookstore_policy            | 26  | 3   | `wrap_decide_by_bookstore`               | → `with_bookstore_decider(Arc<dyn BookstoreDecider>)`                                                                                                                                                                                                     | 8 R            | ○    |
| category_mapping            | 52  | 52  | D1                                       | `set_connection_provider` → `with_connection_provider`, `with_connector(host.., provider)` → `with_connector(provider)`. 통일 후 ignore 해제 대상                                                                                                         | 37 M           | ○    |
| classifier                  | 64  | 23  | E5, V2                                   | `EsLookup` → `EsTransport`/`EsManagerApi`, `ClassifierOptions.es_manager` → `es`                                                                                                                                                                          | 4 M            | ○    |
| classifier_calibration      | 18  | 0   | -                                        | 없음                                                                                                                                                                                                                                                      | 0              | -    |
| classifier_corpus           | 13  | 9   | E6                                       | `EsLike` → `EsTransport`(FakeEs). `*_with` 클로저는 현행 유지                                                                                                                                                                                             | 2 M            | ○    |
| classifier_gaps             | 8   | 2   | `ClassifierModel`                        | → `CategoryModelApi`, `BookClassifierService::new(.., &Path, Option<BookCategoryClassifier>)` → `Option<Arc<dyn Classify>>`                                                                                                                               | 4 M            | ○    |
| classifier_reader           | 4   | 0   | -                                        | 없음                                                                                                                                                                                                                                                      | 0              | -    |
| classifier_service          | 10  | 2   | E6, `Model`                              | `BookCategoryClassifier::new(model, es, root).with_min_confidence()` → `new(model, ClassifierOptions{..})`                                                                                                                                                | 10 M           | ○    |
| classify_cli                | 36  | 26  | L4, 전역 가드 15종                       | `testing::override_*`, `capture_stdout/logs` → `ClassifyCliDeps` + `main_with(args, &mut CliIo, &deps)`                                                                                                                                                   | 30 R           | ○    |
| classify_cli_gaps           | 9   | 5   | L4, T4                                   | 같은 변경                                                                                                                                                                                                                                                 | 9 R            | ○    |
| classify_proposal_items_db  | 16  | 16  | D3                                       | `Connection{execute(&[Value])}` → `DbConnection::execute(&[DbValue])`                                                                                                                                                                                     | 5 M            | -    |
| classify_proposal_status_db | 11  | 11  | D3                                       | 같은 변경                                                                                                                                                                                                                                                 | 1 M            | -    |
| client_error_log            | 8   | 0   | A10, L1                                  | `create_jwt_token(email, role)` 2인자 → `fx::jwt`                                                                                                                                                                                                         | 1 M            | ○    |
| comics                      | 6   | 1   | V3, M10                                  | `IndexManager` → `EsManagerApi`, `from_env_with_es` → `from_env(env, es_factory)`                                                                                                                                                                         | 3 M            | ○    |
| cover                       | 21  | 4   | A4, M3, E2, T1                           | `AppDeps::new(Arc<BookManager>)` → `AppDeps::new(AuthMode).with_book_manager`, `with_es(path, EsApi)` → `Deps`, `CoverManaging`/`ThumbnailExtractor` → `with_thumbnailer`                                                                                 | 8 R            | ○    |
| cover_browse                | 28  | 0   | -                                        | 없음                                                                                                                                                                                                                                                      | 0              | -    |
| cover_management            | 65  | 34  | A9, E7, H6, T1, F7, L2                   | `with_fetch_url_image(Fn)` → `with_image_fetcher(fx::fetcher(..))` 39곳(M), `fsync/flock` 클로저 → `FsOps`, `set_extract_payload_override` → `AuthMode`, `Hooks` → `EpubCoverHooks`, `LogCapture` import                                                  | 12 (M 8, R 4)  | ○    |
| cover_repair                | 5   | 3   | A9, P7, P8                               | `set_pixel_decode_hook` → `EpubCoverHooks::with_pixel_decode_observer`, `Limits` → `with_limits`                                                                                                                                                          | 2 R            | -    |
| cover_sources               | 23  | 15  | H2, T1, T3, F7                           | `Session` → `HttpSession`, `store_lock_with(dir, store, flock, monotonic)` → `(dir, store, &FsOps, &Clock)`, `set_request_cancelled` 가드 → `SearchHooks::with_cancel(Fn)`                                                                                | 10 R           | ○    |
| cover_status                | 14  | 7   | M4, P4, `set_cover_books_pager`          | `without_es(path)` → `Deps`, `PdfDocument::len` → `PdfDoc::page_count`, `set_cover_books_pager` → `with_cover_books_pager`                                                                                                                                | 7 M            | ○    |
| direct_cover_url            | 8   | 8   | H3                                       | `CoverError` → `FetchError`만                                                                                                                                                                                                                             | 0              | ○    |
| epub_cover_set              | 70  | 54  | A7, M7, H5, F7, 전역 `hooks::set_*`      | `AppOverrides`/`EpubCoverApi` → `AppDeps`+`BookManagerApi`, `BookManagerDeps{..}` 리터럴 → 빌더, `ImageFetcher` 에러 타입, `hooks::set_*` 5종 → `EpubCoverHooks`                                                                                          | 7 R            | ○    |
| epub_image_edit             | 40  | 3   | A6, M6                                   | `app_with_overrides(Arc, Option<Value>)` → `AppDeps`                                                                                                                                                                                                      | 2 M            | ○    |
| epub_line_count             | 4   | 1   | `read_from_epub_using`                   | → `read_from_epub_with(path, &EpubHooks)`                                                                                                                                                                                                                 | 2 M            | -    |
| es_manager                  | 91  | 16  | E1, `set_*_override`, T2                 | `EsApi` → `EsTransport`, `ElasticsearchError` → `EsError`, `new_with(.., sleep)` → `connect(.., &FakeClock)`, `set_*_override` 6종 → `DummyEs::on`                                                                                                        | 17 R           | ○    |
| es_manager_gaps             | 8   | 2   | E1, `set_search_override`                | 같은 변경                                                                                                                                                                                                                                                 | 2 R            | ○    |
| image_edit                  | 4   | 0   | -                                        | 없음                                                                                                                                                                                                                                                      | 0              | -    |
| main                        | 94  | 35  | A1, E3, D5, V2                           | 빌더 `.book_manager` → `.with_book_manager`(헬퍼 3곳), `testing::set_reload_progress_flush_interval_seconds` 10곳 → `with_reload_progress_flush_interval`, `EsManagerApi`/`BookManagerApi` fake 반환형 정리, `DummyEs` 이름 충돌 정리                     | 12 M           | ○    |
| main_bulk_rename            | 16  | 0   | A1, F6                                   | 빌더 이름, `testing::fail_next_iterdir` → `with_fs(FaultyFs)`                                                                                                                                                                                             | 1 M            | ○    |
| main_gaps                   | 9   | 0   | A1                                       | 빌더 이름만                                                                                                                                                                                                                                               | 0              | ○    |
| main_mock                   | 129 | 31  | A3                                       | `Overrides` → `AppDeps`(`build()` 헬퍼 한 곳), `Mock` → `impl BookManagerApi`(매크로), `tweaked` 17개: `list_dir`→`with_fs`, `google_verify`→`with_google_verifier`, `refresh_token_store`→`with_refresh_token_store`, `to_thread_spy`→`with_task_runner` | 17 R           | ○    |
| pdf_delete_pages            | 71  | 10  | A6, M6, H4                               | `app_with_overrides` → `AppDeps`, `FetchUrlImage` → `ImageFetcher`, `BookManagerDeps::new(path, es)` 순서(헬퍼)                                                                                                                                           | 2 M            | ○    |
| pdf_delete_pages_gaps       | 12  | 5   | M6, P7                                   | `with_remove_pdf_pages_with_backup_failure`/`with_normalize_uploaded_image_failure` → `FaultyFs`/`EpubCoverHooks::with_limits`, `set_max_image_pixels` 가드 → `with_limits`                                                                               | 5 R            | ○    |
| pdf_pages                   | 19  | 10  | A8, M6, F4, P6                           | `app_with_book_manager` → `AppDeps`, `with_is_file` → `with_fs`, 정적 캐시 API → `bm.pdf_cache()`, `serial()` 삭제                                                                                                                                        | 10 R           | ○    |
| refresh_token_store         | 62  | 26  | D1, L2, V2                               | `backend::log_capture` → `common::LogCapture`, `with_connection_provider` 확정                                                                                                                                                                            | 4 M            | ○    |
| view_history_api            | 19  | 17  | A5, L1                                   | `AppDeps{..}` 리터럴 → 빌더, `auth_override/admin_override` → `AuthMode::Split`                                                                                                                                                                           | 4 M            | ○    |
| view_history_store          | 15  | 15  | D2, D4, L1                               | `StoreConnection`/`with_connection_factory` → `DbConnection`/`with_connection_provider`, `set_max_history_rows` → `with_max_history_rows`                                                                                                                 | 5 M            | ○    |

### 4.2 loader

| 파일        | T   | ign | 쓰는 변형                                                                                                                                       | 통일 후 변경                                                              | 본문 | 헬퍼 |
| ----------- | --- | --- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | ---- | ---- |
| loader_1    | 175 | 24  | `_with(path, detect)`, `ReencodeIo`, `XrefOps`, `Pdftotext(Env)`, `detect_file_type_with`, `read_file_with_fast_count`, `find_libreoffice_with` | `TextHooks`/`FastCountHooks`/`PdftotextHooks`/`ReadHooks` 구조체 방식으로 | 20 R | ○    |
| loader_2    | 104 | 40  | `*Hooks`, `fast_page_count_with(&FastCountHooks)`, `pdf_text_by_pdftotext_with(which, run)`                                                     | `Pdftotext` 쪽만 `PdftotextHooks`로                                       | 4 M  | -    |
| loader_gaps | 1   | 0   | 실제 `IndexStore`                                                                                                                               | 없음                                                                      | 0    | -    |
| loader_hwp3 | 61  | 12  | 실제 `hwp3_parser`                                                                                                                              | 없음                                                                      | 0    | -    |

### 4.3 합계(추정)

| 구분                    | 파일                                                                                                                                                                                                                                                                    | 테스트                                          |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| 전체                    | 53 (+ common 6)                                                                                                                                                                                                                                                         | 약 2,174 (ignore 769)                           |
| 본문 수정 필요          | 41                                                                                                                                                                                                                                                                      | 약 410 (19%). 기계적 치환 약 190, 수작업 약 220 |
| 헬퍼·fake만 수정        | 약 25                                                                                                                                                                                                                                                                   | 본문 0 (약 1,400개 테스트가 헬퍼 덕에 그대로)   |
| 변경 없음               | 약 11 (api_exploration, auth_bookstore_gaps, book, classifier_calibration, classifier_reader, cover_browse, image_edit, loader_gaps, loader_hwp3, main_gaps 일부, direct_cover_url)                                                                                     |                                                 |
| 해제 가능한 `#[ignore]` | seam 사유 약 600 (pymysql 67, `fetch_url_image` 39, `search_candidates` 16, `book_manager` 프록시 13, `create_subprocess_exec` 13, `_convert_with_libreoffice` 10, `dependency_overrides`/`view_history` 11 등). 실제 ES/MySQL/Docker/모델이 필요한 약 170은 해제 안 됨 |                                                 |

수작업 비중이 큰 곳: book_manager_2(40), classify_cli+gaps(39), auth(35), main_mock(17), es_manager(+gaps 19), loader_1(20), pdf_pages(10).

---

## 5. common 중복 구현과 통합 위치

현재 `common/`은 파일 6개이고 test binary가 `#[path = "common/xxx.rs"] mod ..;`로 가져온다. 아래는 통합 후보다.

| 중복                                                                                                                                                                                                | 위치(현재)                                                                                                                   | 문제                                              | 통합 위치 제안                                                                                                                                                               |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `b64(&[u8]) -> String`                                                                                                                                                                              | cover_fixtures, book_fixtures, epub_cover_set(인라인 `STANDARD.encode`)                                                      | 동일                                              | `common/image.rs` 의 `pub fn b64` 하나                                                                                                                                       |
| `image_size(&[u8]) -> (u32,u32)`                                                                                                                                                                    | cover_fixtures, book_fixtures                                                                                                | 동일 (`unwrap` vs `expect`)                       | `common/image.rs`                                                                                                                                                            |
| `encode_image(w,h,rgb,fmt)` / `encode(DynamicImage, fmt)` / `png_solid` / `solid` / `jpeg_bytes` / cover.rs `png_bytes` / bm1·book_manager_gaps·epub_cover_set 인라인 `write_to`                    | cover_fixtures, book_fixtures, cover, book_manager_1, book_manager_gaps, epub_cover_set                                      | 같은 `write_to(Cursor)` 코드 7곳                  | `common/image.rs`: `encode(img, fmt)`, `solid(w,h,rgb) -> RgbImage`, `png_solid`, `jpeg_solid`, `jpeg_bytes(img, quality)`. `encode_image`는 `encode(solid(..), fmt)`로 정의 |
| `image_bytes()`(빨간 120x180 JPEG)                                                                                                                                                                  | cover_fixtures, epub_cover_set, book_manager_gaps                                                                            | 각자 정의                                         | `common/image.rs::red_jpeg()`                                                                                                                                                |
| `part_files(dir)` vs `count_matching(dir, prefix, suffix)` vs `dir_names`                                                                                                                           | book_fixtures(자유 함수+메서드 중복), cover_fixtures                                                                         | "임시 파일이 남지 않았는가"를 두 방식으로 구현    | `common/fsx.rs`: `leftover(dir, prefix, suffix) -> Vec<String>` 하나. `part_files(dir) = leftover(dir, "", ".part")`                                                         |
| `crop(page,x0,..)->Value`, `crop_at`, `split`, `insert`, `upload` (JSON 편집 인자) vs `Raster::crop`(픽셀)                                                                                          | book_fixtures                                                                                                                | 이름 충돌                                         | 편집 인자 빌더는 `common/edits.rs`(`edits::crop`, `edits::split`), 픽셀 `Raster::crop`은 `common/raster.rs`                                                                  |
| `make_epub`                                                                                                                                                                                         | cover_fixtures, epub_cover_set, classifier, classifier_reader                                                                | 4곳, 서명 다름                                    | `common/epub.rs::make_epub(path, spec: &EpubSpec)`. 현재 4개 서명을 `EpubSpec::{cover, publisher, minimal}` 생성자로 흡수                                                    |
| `write_zip(path, entries)` / `read_zip_entry` / `write_epub`                                                                                                                                        | book_classifier, epub_image_edit, book_manager_1, book_manager_2, book_manager_gaps, loader_1, epub_line_count, book_preview | 6~8곳                                             | `common/zipx.rs`                                                                                                                                                             |
| PDF 생성: `PdfBuilder`/`make_pdf`/`make_blank_pdf`/`make_image_pdf`(book_fixtures) vs `write_image_pdf`/`write_blank_pdf`(cover_fixtures, `Document::with_version` 직접) vs cover_status `make_pdf` | 3곳                                                                                                                          | lopdf 조립이 두 구현                              | `common/pdf.rs`: `PdfBuilder` 하나, `write_blank_pdf = PdfBuilder 래퍼`, `write_image_pdf(pages: &[RgbImage])` 포함                                                          |
| `sha256_hex`, `truthy`, `touch`, `set_mtime`                                                                                                                                                        | cover_fixtures + epub_cover_set, cover_fixtures + es_dummy, cover_fixtures + main_bulk_rename, bm1+bm2+loader_2              | 각 2~3곳                                          | `common/fsx.rs`(`touch`, `set_mtime`, `sha256_hex`), `truthy`는 es_dummy 전용이라 cover_fixtures 쪽 삭제                                                                     |
| `FakeClock`, `Event`                                                                                                                                                                                | cover_fixtures                                                                                                               | 다른 파일도 필요(es_manager sleep, cover_sources) | `common/clock.rs` (3.8)                                                                                                                                                      |
| `DummyEs`(4종: es_dummy, bm1, bm2, main), `FakeEs`(book_fixtures, book_preview, epub_cover_set, classifier, classifier_corpus, classifier_service), `MockEs`(cover, main_bulk_rename, main_gaps)    | 13곳                                                                                                                         | fake ES 구현 13개                                 | `common/es_fake.rs`: `FakeEsManager`(`EsManagerApi`, 문서 맵 + 호출 기록 + 메서드별 오류 주입)와 `common/es_dummy.rs`(`EsTransport`) 두 개로 줄인다                          |
| `make_manager`, `make_doc`                                                                                                                                                                          | bm1, bm2, cover_status, es_manager, cover, pdf_pages (모두 다른 시그니처)                                                    | 6곳                                               | `common/manager.rs::make_manager(tmp, es) -> BookManager`(= `with_deps(Deps::new(..))`), `make_doc(rel, file_type) -> Value`                                                 |
| `admin_payload`, `user_payload`, `post_empty`, `client`                                                                                                                                             | view_history_api, main_bulk_rename, main_mock, main_gaps, main, auth, ...                                                    | 4~9곳                                             | `common/mod.rs`의 `TestClient`와 `common/payload.rs`                                                                                                                         |
| `make_book`, `make_docs`, `hwp_dir`                                                                                                                                                                 | 3곳씩                                                                                                                        |                                                   | `common/` 해당 모듈로                                                                                                                                                        |
| `FaultyFs`, `FakeRunner`, `FakePdfOpener`                                                                                                                                                           | book_manager_1                                                                                                               | 다른 파일도 필요                                  | `common/faulty_fs.rs`, `common/fake_runner.rs`, `common/fake_pdf.rs`                                                                                                         |
| `EnvGuard`                                                                                                                                                                                          | 6개 파일                                                                                                                     |                                                   | `common/env.rs`                                                                                                                                                              |
| `LogCapture`                                                                                                                                                                                        | log_capture(자유 함수), bookstore_cache(로컬), auth·cover_management·refresh_token_store(backend 안 모듈로 가정)             |                                                   | `common/log_capture.rs`를 RAII 가드로 (3.10)                                                                                                                                 |

`common/mod.rs`는 아직 `#[path]` 방식이라, 위 모듈을 쓰는 binary가 늘면 `common/mod.rs`에 `pub mod` 선언을 모으는 게 낫다(현재 "mod.rs는 수정하지 않는다"는 제약은 이 설계 이후 풀어야 한다).

---

## 6. 결정 사항

2026-10-23에 아래 세 항목을 모두 권장안으로 확정했다. 이 문서의 3장(통일안)이 기준이다.

| 항목            | 결정                                                                    |
| --------------- | ----------------------------------------------------------------------- |
| 공용 능력 위치  | 새 crate `libs/sysdeps` (아직 만들지 않음. 테스트가 이 경로를 가정한다) |
| 인증 대역       | `AuthMode` enum 유지 (필요하면 `Denied` 변형 추가)                      |
| trait 이름 규칙 | facade에만 `Api` 접미사, 좁은 능력은 역할 명사                          |

아래는 결정 전의 논의 기록이다.

### 6.1 논의 기록 (사용자 결정 필요였던 항목)

1. **공용 능력을 새 crate `libs/sysdeps`에 둘지, `backend` 안에 둘지.**
   - `sysdeps`(권장): `loader`, `doc_reader`, `isbn`이 이미 같은 목적의 trait(`CommandRunner`, `StageRunner`, `find_libreoffice_with`)를 따로 갖고 있다. 저장소 규칙("공유할 코드는 libs/")과도 맞는다. 대신 workspace 멤버와 문서(`docs/rust-workspace.md`)가 늘고, `isbn::CommandRunner`를 나중에 옮겨야 한다.
   - backend 안: crate 하나 덜 만든다. 대신 loader/doc_reader가 backend에 의존하거나 같은 trait를 다시 만들게 된다.
2. **인증 대역을 `AuthMode`에 모을지, 별도 `AuthProvider` trait로 뺄지.** 지금 안은 enum 5변형(`Cookies`, `Fixed`, `Split`, `AuthOnly`, `Shared`)이다. 쿠키 만료·거부(401/403) 같은 동작 테스트가 더 필요하면 `AuthMode::Denied{status, detail}` 추가나 `trait AuthProvider`로 가야 한다. 변형 추가는 쉽지만 trait로 가면 main.rs의 `Fixed/Split` 119개 호출을 다시 고쳐야 한다. 권장은 enum 유지.
3. **trait 이름 규칙 `XxxApi`를 어디까지 쓸지.** 요청안은 `XxxApi`를 일괄 규칙으로 보지만, 기존 crate는 `CommandRunner`, `IndexStore`, `Fetcher`, `StageRunner`처럼 역할 명사다. 이 문서는 "facade만 `Api`, 좁은 능력은 명사"로 정했다. 일괄 `Api` 접미사로 가면 `ClockApi`, `FsOpsApi`처럼 어색한 이름이 생기고 기존 crate 4곳과 어긋난다. 권장은 현재 안.

그 밖에 구현 때 판단해도 되는 사항은 다음과 같다. ES `_search` 같은 비공개 메서드 스텁 3~4개의 처리, `scandir`/`read_dir` 통합 시 경로 필터 방식, `LogCapture`의 스레드 로컬 한계(워커 스레드 로그 미수집), `i64` 통일로 인한 `usize` 파라미터 테스트 수정.

---

## 7. 재작성 결과와 정정 (2026-10-23)

설계 확정 뒤 `tests/rust/backend`와 `tests/rust/loader`의 테스트 파일을 이 문서에 맞춰 다시 썼다. 컴파일은 하지 않았다(구현이 없다). 테스트 개수는 `grep`으로 센 값이다.

| 항목 | 재작성 전 | 재작성 후 |
| --- | --- | --- |
| 테스트 수 (backend 1,833 + loader 341) | 2,174 | 2,174 (이름·개수 동일) |
| `#[ignore]` | 778 | 396 |

### 7.1 이 문서의 사실 오류

- **`with_fetch_url_image(Fn)` 39곳은 틀렸다.** 실제 호출은 5곳이다. 나머지는 `#[ignore]` 사유 문자열에 쓰인 횟수였다. 1장의 "seam을 통일하면 약 600개가 열린다"도 같은 방식으로 부풀려졌다. 실제로는 약 380개가 풀렸다.
- **`find_libreoffice_with(which, exists)`는 `doc_reader::office`에 아직 없다.** 현재는 `find_libreoffice()`만 있다(2.5 C5, 3.1). 구현 때 추가해야 한다.
- **`common::fx::jwt`, `fx::fetcher`, `StaticBookLookup`은 `common`에 없다.** 3.7, 3.12가 있다고 했지만 만들지 않았다. 아래 7.3의 승격 대상이다.

### 7.2 문서에 없어서 테스트가 가정한 것 (구현 때 확정)

| 영역 | 가정 | 비고 |
| --- | --- | --- |
| `BookManagerApi` | `get_book_preview` 시그니처가 세 가지다: `(id, pages, chapters)`(book_manager_1), `(id, chapters)`(gaps·hwp_preview), `(id, chapters, resource_base_url)`(3.2) | `main_mock`의 `test_default_params`, `test_custom_params`가 `pages`를 단언해 `#[ignore]` |
| `BookManagerApi` | 콜백 인자: `apply_category_changes`·`propose_category_changes`는 main_mock이 콜백 포함, main_gaps는 3인자 | |
| `BookManagerApi` | `search_by_keyword_paged` 인자 수: main.rs 4개, main_mock 5개 | |
| `BookContent::File` | 문서 `File(PathBuf)`, 테스트 `File(BookResponse)` | 헤더를 읽는 테스트가 있다 |
| `DbConnection` | `begin()`이 없다 | `view_history_store`의 롤백 테스트 1개가 `#[ignore]` |
| `FsOps` | 오류 타입이 `io::Error`뿐이다 | `RuntimeError` 주입 테스트가 `#[ignore]` |
| `FsOps` | `rmdir`, 읽기 결과 치환(`ListFs`)이 없다 | `FaultyFs`는 실패 주입만 한다 |
| `AuthMode` | viewer 전용 라우트용 변형이 없다 | `AuthOnly` 또는 viewer 쿠키 JWT로 대신했다 |
| `EsConnector` | trait 모양이 없다 | `FnConnector`로 감쌌다 |
| `CoverHooks`, `StoreGate` | `with_inspect_epub`, `with_store_lock`, `with_delete_book_cache`, `with_directories`, `with_candidates`, `with_is_disconnected`, `with_resolve`, `with_lock_wait_observer`, `with_max_image_bytes`, `StoreGate::with_cancel/with_fs` | cover_management 10개, cover_sources 5개가 `#[ignore]` |
| `BookManagerDeps` | `with_cover_books_pager`, `with_uncovered_*`, `with_max_category_result_count`, `with_page_count_cache_max`, `with_preview_epub_validator`, `with_normpath`, `with_path_is_relative_to`, `with_category_mismatches` 외 | 모두 `//!` 헤더에 적혀 있다 |
| `ClassifyCliDeps` | 주입 지점 7개: `with_config_store`, `with_es_factory`, `with_corpus`, `with_service_factory`, `with_fs`, `with_clock`, `with_boundary_factor` | 3.12 코드 블록은 "..."뿐이다 |
| `FakeEsManager` | `search_cover_files_paged`, `search_group_representatives`, `aggregate_cover_categories`가 없다 | gaps가 로컬 `CoverEs`를 쓴다 |

### 7.3 남은 정리

- **`TODO(common)` 승격:** `common/auth_fakes.rs`(`FixedRole`, `FakeGoogle`, `FakeStore`, `ScriptedRole`/`Verifier`), `common/db_fake.rs`(`FakeCursor`, `FakeConn`), `common/fake_pdf.rs`(`StaticPdfDoc`, `FakeOpener`, `CountingOpener`), `fx::fetcher`/`thumbnailer`, `HttpSession` 대역(`BodySession`), `ListFs`, `TickingClock`, `jwt` 헬퍼.
- **옛 fixture 정리:** `book_fixtures.rs`, `cover_fixtures.rs`, `mysql_fixture.rs`를 아직 `#[path]`로 쓰는 파일이 7개 남았다.
- **원본 단언이 달라진 곳:** `auth`의 `test_never_logs_raw_identifiers`(원문 쿠키 대신 보낸 토큰을 검사), `main`의 `insert(&data)` 두 번째 인자 제거, 1비트 이미지 검증이 `L8` 색상 타입 확인으로 약해진 것.
- **`rustfmt` 주의:** `mod common;`이 있는 파일에 `rustfmt`를 `--check` 없이 돌리면 `common/*.rs`까지 다시 쓴다. 임시 복사본에서 돌리거나 `--check`만 쓴다.
