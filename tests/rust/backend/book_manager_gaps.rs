//! 원본: tests/test_book_manager_gaps.py
//!
//! BookManager의 드문 경로(오류 분기, 비동기 콜백, 경계 입력) 테스트. ES/Docker 없이 실행한다.
//! 설계: docs/rust-test-seams.md 3장(통일안). Python의 `DummyES`는 `common::es_fake::FakeEsManager`, `make_manager`는 아래 로컬 헬퍼
//! (`BookManager::with_deps(BookManagerDeps::new(path, es)...)`), 파일 시스템 실패 주입은 `common::faulty_fs::FaultyFs`다.
//!
//! 가정한 API:
//! - `backend::book_manager::{BookManager, BookManagerDeps}`: `BookManager::with_deps(BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>).with_*(..))`
//!   - `with_fs(Arc<dyn FsOps>)`, `with_file_reader(Arc<dyn FileReader>)` (3.12)
//!   - `FileReader { fn read_file(&self, path: &Path, path_prefix: Option<&Path>) -> Docs }` (구 `Loader.read_file` 바꿔치기. 모양은 book_manager_1과 같다고 가정)
//! - `backend::es_manager::EsManagerApi`의 커버 목록 메서드(문서 3.3 "나머지"): `search_cover_files_paged(&[String], size: i64, search_after: Option<(i64, i64, String)>, exclude: &[String]) -> Result<(Vec<(i64, Value, Vec<Value>)>, i64), EsError>`,
//!   `search_group_representatives(&[String], exclude: &[String]) -> Result<HashMap<String, Vec<(i64, Value)>>, EsError>`, `aggregate_cover_categories() -> Result<HashMap<String, (usize, f64, f64)>, EsError>`
//! - `backend::book_classifier::BookClassifierApi` (구 `ClassifierService`. 3.12): `min_confidence() -> Option<f64>`, `expected_accuracy(Option<f64>) -> Option<Option<f64>>`. 모든 메서드에 기본 구현이 있다
//! - `BookManager::_downscale_epub_image(&[u8]) -> Vec<u8>` (정적), `_model_decision_floor(&dyn BookClassifierApi) -> f64`,
//!   `_expected_accuracy(&dyn BookClassifierApi, f64) -> Option<f64>`, `_vote_count_for(&str, &[(String, i64)]) -> i64`
//! - `manager._fill_second_with_model(Vec<Value>, &[(String, f64)]) -> Vec<Value>`,
//!   `_fill_second_with_bookstore(Vec<Value>, &[(String, i64)]) -> Vec<Value>`,
//!   `_build_candidates(Option<..>, &str, Option<..>, &[(String, i64)]) -> Vec<Value>`,
//!   `_normalize_stored_file_path(&str) -> String`
//! - `async manager.propose_category_changes(&str, ProposeOpts) -> (Value, Option<String>)`
//! - `async manager.apply_category_changes(&[Value], &HashSet<String>, ApplyOpts) -> (Value, Option<String>)`
//! - `async manager._move_classified_file(&Path, &str, &str, MoveOpts) -> (Option<Value>, Option<String>)`
//! - `manager._iter_category_indexable_files(&str) -> Vec<PathBuf>`, `BookManager::_is_duplicate_content(&Path, &Path) -> bool`,
//!   `BookManager::_next_numbered_path(&Path) -> Option<PathBuf>`
//! - `manager.get_category_mismatches(force_refresh: bool) -> Value`, `async manager._reload_root_files() -> (Value, Option<String>)`
//! - `BookManager::_find_epub_cover_member(&mut ZipArchive<File>) -> Option<String>`, `_is_mostly_blank_cover(&DynamicImage) -> bool`,
//!   `_extract_cover_thumbnail(&Path, &str, Option<u32>) -> Option<Vec<u8>>`, `BookManager::COVER_ASPECT_RATIO: f64`
//! - `async manager.get_cover(i64, None) -> BookResponse`, `async BookManagerApi::get_book_preview(&manager, i64, Some(chapters), None) -> BookResponse`
//!   (`BookResponse { status_code: u16, body: Vec<u8> }`. 구 `PreviewResponse { status_code, path }`의 파일 내용은 `body`에 담긴다고 가정한다)
//! - `async manager.get_cover_books_paged(CoverPageOpts) -> (Vec<(Book, String)>, usize, Option<String>, Option<String>)`
//! - `backend::book_manager::{encode_category_cursor(&Value) -> String, MAX_CATEGORY_PAGE_SIZE: usize}`
//!
//! TODO(common): 로컬 정의 `FnFileReader`(FileReader 대역), `CoverEs`(`FakeEsManager`에 커버 목록 메서드가 없다), `StubService`(`BookClassifierApi` 대역).

mod common;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use backend::book::Book;
use backend::book_manager::{ApplyOptions, BookManager, BookManagerDeps, CoverPage, CoverPageOpts, FileReader, MAX_CATEGORY_PAGE_SIZE, MoveOpts, ProposeOptions, encode_category_cursor};
use backend::classifier::BookClassifierApi;
use backend::es_manager::{EsError, EsManagerApi};
use backend::main::BookManagerApi;
use common::es_fake::{Docs, FakeEsManager};
use common::faulty_fs::FaultyFs;
use common::image::encode_image;
use common::zipx::{open_zip, write_zip};
use serde_json::{Value, json};
use tempfile::TempDir;
use zip::ZipArchive;

fn category_with_files(root: &Path, category: &str, names: &[&str]) {
    fs::create_dir_all(root.join(category)).unwrap();
    for name in names {
        fs::write(root.join(category).join(name), "본문").unwrap();
    }
}

const CONTAINER: &str = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;

/// ES 대역을 쓰는 `BookManagerDeps`. 대역(`with_*`)을 꽂을 테스트는 이걸로 시작한다.
fn make_deps(tmp_path: &Path, es: Arc<dyn EsManagerApi>) -> BookManagerDeps {
    BookManagerDeps::new(tmp_path.to_path_buf(), es)
}

/// Python `make_manager(tmp_path, es)`.
fn make_manager(tmp_path: &Path, es: Arc<dyn EsManagerApi>) -> BookManager {
    BookManager::with_deps(make_deps(tmp_path, es))
}

/// Python `make_doc(path, file_type)`.
fn make_doc(rel_path: &str, file_type: &str) -> Value {
    json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
}

/// 이미지 한 장의 바이트. 색은 고정이다.
fn image_bytes(fmt: image::ImageFormat, size: (u32, u32)) -> Vec<u8> {
    encode_image(size.0, size.1, [10, 120, 200], fmt)
}

type ReadFileFn = Box<dyn Fn(&Path, Option<&Path>) -> Docs + Send + Sync>;

/// `Loader.read_file` 대역. 클로저 하나가 경로와 `path_prefix`를 받아 문서 묶음을 돌려준다.
struct FnFileReader {
    f: ReadFileFn,
}

impl FnFileReader {
    fn new(f: impl Fn(&Path, Option<&Path>) -> Docs + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { f: Box::new(f) })
    }
}

impl FileReader for FnFileReader {
    fn read_file(&self, path: &Path, path_prefix: Option<&Path>) -> Docs {
        (self.f)(path, path_prefix)
    }
}

/// `BookClassifierApi` 대역. 모델 판정 하한과 보정 정확도만 정할 수 있다.
/// 타입이 있는 API라 Python의 "파싱할 수 없는 값"과 "보정 중 예외"는 둘 다 `None`으로 나타낸다.
struct StubService {
    min_confidence: Option<f64>,
    accuracy: Option<Option<f64>>,
}

impl BookClassifierApi for StubService {
    fn min_confidence(&self) -> Option<f64> {
        self.min_confidence
    }

    fn expected_accuracy(&self, _confidence: Option<f64>) -> Option<Option<f64>> {
        self.accuracy
    }
}

// ---- 이미지/EPUB 미리보기 ----

#[test]
fn test_downscale_epub_image_keeps_non_jpeg_png_format() {
    // GIF 같은 형식은 확장자와 실제 형식이 어긋나지 않게 그대로 돌려준다.
    let data = image_bytes(image::ImageFormat::Gif, (3000, 3000));
    assert_eq!(BookManager::_downscale_epub_image(&data), data);
}

#[tokio::test]
async fn test_preview_downscales_large_epub_images() {
    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 최소 바이트 값을 0으로 두고 축소 함수 호출 길이를 기록하는 훅
    let calls: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
    let spy = calls.clone();

    let tmp = TempDir::new().unwrap();
    let opf = r#"<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf">
              <manifest>
                <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                <item id="img" href="images/a.png" media-type="image/png"/>
              </manifest>
              <spine><itemref idref="c1"/></spine>
            </package>"#;
    write_zip(
        &tmp.path().join("pic.epub"),
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/ch1.xhtml", br#"<html><body><img src="images/a.png"/></body></html>"#),
            ("OEBPS/images/a.png", &image_bytes(image::ImageFormat::Png, (64, 64))),
            ("OEBPS/content.opf", opf.as_bytes()),
        ],
    );
    let deps = make_deps(tmp.path(), FakeEsManager::with_doc(make_doc("pic.epub", "epub"))).with_epub_image_min_bytes(0).with_downscale_epub_image_hook(Arc::new(move |data: &[u8]| {
        spy.lock().unwrap().push(data.len());
        BookManager::_downscale_epub_image(data)
    }));
    let manager = BookManager::with_deps(deps);

    let resp = BookManagerApi::get_book_preview(&manager, 1, Some(1), None).await;

    assert_eq!(resp.status_code, 200);
    assert!(!calls.lock().unwrap().is_empty(), "이미지 멤버가 축소 경로를 지나야 한다");
}

#[tokio::test]
async fn test_preview_keeps_cover_image_not_referenced_by_chapters() {
    // parametrize: meta-cover, cover-image-property
    let cases = [("meta-cover", r#"<meta name="cover" content="cover-img"/>"#), ("cover-image-property", "")];
    for (case_id, cover_decl) in cases {
        let properties = if cover_decl.is_empty() { r#" properties="cover-image""# } else { "" };
        let tmp = TempDir::new().unwrap();
        let opf = format!(
            r#"<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf">
              <metadata>{cover_decl}</metadata>
              <manifest>
                <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                <item id="cover-img" href="images/front.jpg" media-type="image/jpeg"{properties}/>
              </manifest>
              <spine><itemref idref="c1"/></spine>
            </package>"#
        );
        write_zip(
            &tmp.path().join("cov.epub"),
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER.as_bytes()),
                ("OEBPS/ch1.xhtml", "<html><body><p>본문</p></body></html>".as_bytes()),
                ("OEBPS/images/front.jpg", &image_bytes(image::ImageFormat::Jpeg, (64, 64))),
                ("OEBPS/content.opf", opf.as_bytes()),
            ],
        );
        let manager = make_manager(tmp.path(), FakeEsManager::with_doc(make_doc("cov.epub", "epub")));

        let resp = BookManagerApi::get_book_preview(&manager, 1, Some(1), None).await;

        assert_eq!(resp.status_code, 200, "case {case_id}");
        let mut zf = ZipArchive::new(Cursor::new(resp.body)).unwrap();
        assert!(zf.file_names().any(|n| n.is_ok_and(|n| n == "OEBPS/images/front.jpg")), "case {case_id}");
        let opf_text = std::io::read_to_string(zf.by_name("OEBPS/content.opf").unwrap()).unwrap();
        assert!(opf_text.contains(r#"id="cover-img""#), "case {case_id}");
    }
}

// ---- 분류 후보 조립 ----

#[test]
fn test_model_decision_floor_ignores_unparsable_value() {
    // 파싱할 수 없는 값(Python의 "높음")은 타입이 있는 API에서 `None`이다
    let service = StubService { min_confidence: None, accuracy: None };
    assert_eq!(BookManager::_model_decision_floor(&service), 0.0);
}

#[test]
fn test_expected_accuracy_swallows_calibration_errors() {
    // 보정 중 예외(Python의 "보정 실패")는 타입이 있는 API에서 `None`이다
    let service = StubService { min_confidence: None, accuracy: None };
    assert_eq!(BookManager::_expected_accuracy(&service, 0.9), None);
}

#[test]
fn test_fill_second_with_model_keeps_two_candidates_untouched() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), FakeEsManager::new());
    let candidates = vec![json!({"category": "A"}), json!({"category": "B"})];
    assert_eq!(manager._fill_second_with_model(candidates, &[("C".to_string(), 0.9)]), vec![json!({"category": "A"}), json!({"category": "B"})]);
}

#[test]
fn test_build_candidates_without_model_uses_top_two_bookstore_votes() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), FakeEsManager::new());
    let votes = [("A".to_string(), 3), ("B".to_string(), 2), ("C".to_string(), 1)];
    let result = manager._build_candidates(None, "not_found", None, &votes);
    let categories: Vec<&str> = result.iter().map(|c| c["category"].as_str().unwrap()).collect();
    assert_eq!(categories, vec!["A", "B"]);
}

#[test]
fn test_fill_second_with_bookstore_keeps_two_candidates_untouched() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), FakeEsManager::new());
    let candidates = vec![json!({"category": "A"}), json!({"category": "B"})];
    assert_eq!(manager._fill_second_with_bookstore(candidates, &[("C".to_string(), 3)]), vec![json!({"category": "A"}), json!({"category": "B"})]);
}

#[test]
fn test_vote_count_returns_recorded_votes_for_known_category() {
    assert_eq!(BookManager::_vote_count_for("A", &[("B".to_string(), 5), ("A".to_string(), 3)]), 3);
}

#[test]
fn test_normalize_stored_file_path_strips_leading_dot_slash() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), FakeEsManager::new());
    assert_eq!(manager._normalize_stored_file_path("./A/x.txt"), "A/x.txt");
}

#[test]
fn test_vote_count_defaults_to_one_when_category_has_no_votes() {
    assert_eq!(BookManager::_vote_count_for("Z", &[("A".to_string(), 3)]), 1);
}

// ---- 분류 제안 ----

#[tokio::test]
async fn test_propose_rejects_unsafe_category_name() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), FakeEsManager::new());
    let (result, err) = manager.propose_category_changes("../밖", None, Default::default()).await;
    assert_eq!(result, json!({}));
    assert_eq!(err.as_deref(), Some("잘못된 카테고리 경로입니다"));
}

#[tokio::test]
async fn test_propose_awaits_async_progress_callback_and_records_failures() {
    let tmp = TempDir::new().unwrap();
    category_with_files(tmp.path(), "A", &["ok.txt", "bad.txt"]);

    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 파일별 제안 함수를 교체하는 훅
    let deps = make_deps(tmp.path(), FakeEsManager::new()).with_propose_category_for_file_hook(Arc::new(|file_path: &Path| {
        if file_path.file_name().unwrap() == "bad.txt" {
            return Err("분류기 오류".to_string());
        }
        Ok(json!({"target_category": "B", "grade": "high"}))
    }));
    let manager = BookManager::with_deps(deps);
    let reports: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = reports.clone();
    // 진행 보고 콜백은 동기 함수다(Python은 코루틴도 받는다).
    let on_progress = move |payload: Value| sink.lock().unwrap().push(payload);
    let opts = ProposeOptions { content_type: Some("book"), delay: Some(0.0), use_bookstore: Some(false), use_content_meta: Some(false), on_progress: Some(&on_progress) };

    let (result, err) = manager.propose_category_changes("A", None, opts).await;

    assert_eq!(err, None);
    let titles: Vec<&str> = result["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["ok"]);
    assert_eq!(result["failures"], json!([{"file_path": "A/bad.txt", "error": "분류 제안에 실패했습니다"}]));
    assert!(reports.lock().unwrap().iter().any(|r| r.get("updated_items").is_some()));
}

// ---- 분류 적용 ----

#[tokio::test]
async fn test_apply_awaits_async_callbacks() {
    let tmp = TempDir::new().unwrap();
    category_with_files(tmp.path(), "1_A", &["a.txt"]);
    fs::create_dir(tmp.path().join("2_B")).unwrap();
    let done: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 파일 이동 함수를 교체하는 훅
    let deps = make_deps(tmp.path(), FakeEsManager::new()).with_move_classified_file_hook(Arc::new(|_path: &Path, _target: &str| (Some(json!({"status": "moved"})), None)));
    let manager = BookManager::with_deps(deps);
    let (done_sink, progress_sink) = (done.clone(), progress.clone());
    // 콜백은 동기 함수다(Python은 코루틴도 받는다).
    let should_continue = || true;
    let on_item_done = move |payload: Value| done_sink.lock().unwrap().push(payload["apply_status"].as_str().unwrap().to_string());
    let on_progress = move |payload: Value| progress_sink.lock().unwrap().push(payload);
    let opts = ApplyOptions { should_continue: Some(&should_continue), on_item_done: Some(&on_item_done), on_progress: Some(&on_progress), ..Default::default() };
    let allowed: HashSet<String> = ["1_A/a.txt".to_string()].into();

    let (result, err) = manager.apply_category_changes_with(&[json!({"file_path": "1_A/a.txt", "target_category": "2_B"})], &allowed, opts).await;

    assert_eq!(err, None);
    assert_eq!(result["applied_count"], 1);
    assert_eq!(*done.lock().unwrap(), vec!["moving", "moved"]);
    assert_eq!(*progress.lock().unwrap(), vec![json!({"apply_total_count": 1, "applied_count": 1, "failed_count": 0})]);
}

#[tokio::test]
async fn test_apply_records_move_error_as_failed() {
    let tmp = TempDir::new().unwrap();
    category_with_files(tmp.path(), "1_A", &["a.txt"]);
    fs::create_dir(tmp.path().join("2_B")).unwrap();

    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 파일 이동 함수를 교체하는 훅
    let deps = make_deps(tmp.path(), FakeEsManager::new()).with_move_classified_file_hook(Arc::new(|_path: &Path, _target: &str| (None, Some("이동 실패".to_string()))));
    let manager = BookManager::with_deps(deps);
    let allowed: HashSet<String> = ["1_A/a.txt".to_string()].into();

    let (result, _) = manager.apply_category_changes(&[json!({"file_path": "1_A/a.txt", "target_category": "2_B"})], &allowed, Default::default()).await;

    assert_eq!(result["failed_count"], 1);
    assert_eq!(result["results"][0]["apply_error"], "이동 실패");
}

#[tokio::test]
async fn test_apply_rejects_file_when_resolve_raises_oserror() {
    let tmp = TempDir::new().unwrap();
    category_with_files(tmp.path(), "1_A", &["a.txt"]);
    fs::create_dir(tmp.path().join("2_B")).unwrap();

    // 경로 정규화(canonicalize)만 실패시킨다. a.txt만 EIO로 실패하고 나머지는 실제 파일 시스템에 맡긴다.
    let fs_ops = FaultyFs::new();
    fs_ops.set_canonicalize(|p: &Path| if p.file_name().unwrap() == "a.txt" { Some(std::io::Error::from_raw_os_error(5)) } else { None });
    let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_fs(fs_ops));
    let allowed: HashSet<String> = ["1_A/a.txt".to_string()].into();

    let (result, _) = manager.apply_category_changes(&[json!({"file_path": "1_A/a.txt", "target_category": "2_B"})], &allowed, Default::default()).await;

    assert_eq!(result["results"][0]["apply_error"], "파일 경로가 corpus 밖입니다");
    assert!(tmp.path().join("1_A").join("a.txt").exists());
}

// ---- 카테고리 파일 열거 ----

#[test]
fn test_iter_category_files_skips_subdirectories_and_outside_symlinked_category() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), FakeEsManager::new());
    category_with_files(tmp.path(), "A", &["keep.txt"]);
    fs::create_dir(tmp.path().join("A").join("sub")).unwrap();

    let names: Vec<String> = manager._iter_category_indexable_files("A", false).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["keep.txt"]);

    // 같은 부모 디렉토리 아래에 corpus 밖 디렉토리를 만든다(부모는 별도 TempDir로 둔다).
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("leak.txt"), "밖").unwrap();
    std::os::unix::fs::symlink(outside.path(), tmp.path().join("L")).unwrap();
    assert!(manager._iter_category_indexable_files("L", false).is_empty());
}

// ---- 중복 판정/번호 붙이기/이동 ----

#[test]
fn test_is_duplicate_content_treats_unreadable_file_as_not_duplicate() {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("a.txt");
    fs::write(&source, "x").unwrap();
    assert!(!BookManager::_is_duplicate_content(&source, &tmp.path().join("없음.txt")));
}

#[test]
fn test_next_numbered_path_gives_up_when_all_slots_taken() {
    // 상수 재정의 setter 대신 상한을 인자로 받는 `_next_numbered_path_with`를 쓴다.
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("book.txt");
    fs::write(tmp.path().join("book (1).txt"), "x").unwrap();
    let result = BookManager::_next_numbered_path_with(&target, 1);
    assert!(result.is_none());
}

#[tokio::test]
async fn test_move_reports_when_numbered_names_are_exhausted() {
    let tmp = TempDir::new().unwrap();
    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 값 설정을 Deps 빌더로 옮겼다): 번호 붙이기 상한을 1로 둔다
    let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_duplicate_name_max_index(1));
    category_with_files(tmp.path(), "A", &["book.txt"]);
    category_with_files(tmp.path(), "B", &["book.txt", "book (1).txt"]);
    fs::write(tmp.path().join("A").join("book.txt"), "다른 내용입니다").unwrap();

    let (result, err) = manager._move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", Default::default()).await;

    assert!(result.is_none());
    assert!(err.unwrap().contains("너무 많습니다"));
}

#[tokio::test]
async fn test_move_removes_identical_duplicate_when_clean_existing() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), es.clone());
    category_with_files(tmp.path(), "A", &["book.txt"]);
    category_with_files(tmp.path(), "B", &["book.txt"]);

    let opts = MoveOpts { clean_existing: true, source_category: Some("A".to_string()), ..Default::default() };
    let (result, err) = manager._move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", opts).await;

    assert_eq!(err, None);
    assert_eq!(result.unwrap()["status"], "duplicate_cleaned");
    assert!(!tmp.path().join("A").join("book.txt").exists());
    assert_eq!(es.deleted_file_paths(), vec![(vec!["A/book.txt".to_string()], Vec::<i64>::new())]);
}

#[tokio::test]
async fn test_move_reports_duplicate_cleanup_failure() {
    let tmp = TempDir::new().unwrap();
    category_with_files(tmp.path(), "A", &["book.txt"]);
    category_with_files(tmp.path(), "B", &["book.txt"]);

    // 파일 삭제(unlink)가 항상 PermissionError로 실패하는 파일 시스템
    let fs_ops = FaultyFs::new();
    fs_ops.set_remove_file(|_p: &Path| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "삭제 거부")));
    let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_fs(fs_ops));

    let opts = MoveOpts { clean_existing: true, ..Default::default() };
    let (result, err) = manager._move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", opts).await;

    assert!(result.is_none());
    assert!(err.unwrap().starts_with("중복 파일 정리 실패"));
}

#[tokio::test]
async fn test_move_dry_run_reports_plain_move_without_touching_files() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), FakeEsManager::new());
    category_with_files(tmp.path(), "A", &["book.txt"]);
    fs::create_dir(tmp.path().join("B")).unwrap();

    let opts = MoveOpts { dry_run: true, ..Default::default() };
    let (result, err) = manager._move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", opts).await;

    assert_eq!(err, None);
    assert_eq!(result.unwrap()["action"], "move");
    assert!(tmp.path().join("A").join("book.txt").exists());
}

#[tokio::test]
async fn test_move_rolls_back_when_loader_returns_no_single_document() {
    let tmp = TempDir::new().unwrap();
    // Loader.read_file이 빈 dict를 돌려주는 파일 읽기 대역
    let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_file_reader(FnFileReader::new(|_path: &Path, _prefix: Option<&Path>| Docs::new())));
    category_with_files(tmp.path(), "A", &["book.txt"]);
    fs::create_dir(tmp.path().join("B")).unwrap();

    let (result, err) = manager._move_classified_file(&tmp.path().join("A").join("book.txt"), "B", "book", Default::default()).await;

    assert!(result.is_none());
    assert!(err.unwrap().contains("지원하지 않는 파일 형식"));
    assert!(tmp.path().join("A").join("book.txt").exists());
}

// ---- 불일치 집계 ----

#[test]
fn test_category_mismatches_omit_categories_without_anomalies() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    es.with(|s| s.all_file_paths = vec![("A".to_string(), "A/ok.txt".to_string())]);
    let manager = make_manager(tmp.path(), es);
    category_with_files(tmp.path(), "A", &["ok.txt"]);

    let result = manager.get_category_mismatches(true);

    assert!(result["mismatches"].as_array().unwrap().iter().all(|item| item["category"] != "A"));
}

// ---- _root 재적재 ----

#[tokio::test]
async fn test_reload_root_files_reports_unreadable_root() {
    let tmp = TempDir::new().unwrap();
    // 디렉토리 열거(scandir)가 PermissionError로 실패하는 파일 시스템
    let fs_ops = FaultyFs::new();
    fs_ops.set_read_dir(|_p: &Path| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "읽기 거부")));
    let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_fs(fs_ops));

    let (result, err) = manager._reload_root_files().await;

    assert_eq!(result, json!({}));
    assert!(err.unwrap().contains("최상위 디렉토리를 읽을 수 없습니다"));
}

#[tokio::test]
async fn test_reload_root_files_reports_refresh_and_batch_failures() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    fs::write(tmp.path().join("top.txt"), "x").unwrap();
    es.fail("refresh", "refresh 불가");

    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 색인 함수를 교체하는 훅
    let deps = make_deps(tmp.path(), es).with_bulk_index_files_hook(Arc::new(|_names: &[String], _clean_existing: bool| (std::collections::BTreeMap::new(), vec![json!({"error": "색인 실패"})])));
    let manager = BookManager::with_deps(deps);

    let (result, err) = manager._reload_root_files().await;

    assert_eq!(result, json!({}));
    assert_eq!(err.as_deref(), Some("최상위 파일 재적재 실패 2건: 색인 실패"));
}

// ---- 커버 추출 ----

fn epub_with(root: &Path, members: &[(&str, &str)]) -> ZipArchive<fs::File> {
    let path = root.join("t.epub");
    let owned: Vec<(&str, &[u8])> = members.iter().map(|(n, c)| (*n, c.as_bytes())).collect();
    write_zip(&path, &owned);
    open_zip(&path)
}

#[test]
fn test_find_epub_cover_member_returns_none_without_opf() {
    let tmp = TempDir::new().unwrap();
    let mut zin = epub_with(tmp.path(), &[("readme.txt", "x")]);
    assert_eq!(BookManager::_find_epub_cover_member(&mut zin), None);
}

#[test]
fn test_find_epub_cover_member_returns_none_when_opf_member_is_missing() {
    let tmp = TempDir::new().unwrap();
    let mut zin = epub_with(tmp.path(), &[("META-INF/container.xml", CONTAINER)]);
    assert_eq!(BookManager::_find_epub_cover_member(&mut zin), None);
}

#[test]
fn test_find_epub_cover_member_returns_none_when_opf_is_not_xml() {
    let tmp = TempDir::new().unwrap();
    let mut zin = epub_with(tmp.path(), &[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", "xml 이 아닌 문자열")]);
    assert_eq!(BookManager::_find_epub_cover_member(&mut zin), None);
}

#[test]
fn test_find_epub_cover_member_ignores_comment_nodes() {
    let opf = r#"<?xml version="1.0"?>
    <package xmlns="http://www.idpf.org/2007/opf">
      <!-- 주석 노드는 태그가 문자열이 아니다 -->
      <manifest><item id="cover" href="img/c.png" media-type="image/png"/></manifest>
    </package>"#;
    let tmp = TempDir::new().unwrap();
    let mut zin = epub_with(tmp.path(), &[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", opf)]);
    assert_eq!(BookManager::_find_epub_cover_member(&mut zin).as_deref(), Some("OEBPS/img/c.png"));
}

#[test]
fn test_find_epub_cover_member_falls_back_to_first_spine_page_image() {
    // 표지 선언이 없고 meta cover 가 존재하지 않는 id 를 가리켜도 첫 spine 페이지의 이미지를 쓴다.
    let opf = r#"<?xml version="1.0"?>
    <package xmlns="http://www.idpf.org/2007/opf">
      <metadata><meta name="cover" content="cover"/></metadata>
      <manifest>
        <item id="id0" href="0.html" media-type="application/xhtml+xml"/>
        <item id="id1" href="1.html" media-type="application/xhtml+xml"/>
        <item id="img1" href="images/img1.webp" media-type="image/webp"/>
        <item id="img2" href="images/img2.webp" media-type="image/webp"/>
      </manifest>
      <spine><itemref idref="id0"/><itemref idref="id1"/></spine>
    </package>"#;
    let members = [
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", opf),
        ("OEBPS/0.html", r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="images/img1.webp"/></body></html>"#),
        ("OEBPS/1.html", r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="images/img2.webp"/></body></html>"#),
        ("OEBPS/images/img1.webp", "x"),
        ("OEBPS/images/img2.webp", "x"),
    ];
    let tmp = TempDir::new().unwrap();
    let mut zin = epub_with(tmp.path(), &members);
    assert_eq!(BookManager::_find_epub_cover_member(&mut zin).as_deref(), Some("OEBPS/images/img1.webp"));
}

#[test]
fn test_blank_cover_check_returns_false_when_image_has_no_colors() {
    // 색 목록을 얻지 못하는 이미지를 흉내 낼 수 없어 크기 0 이미지를 쓴다.
    let image = image::DynamicImage::new_rgb8(0, 0);
    assert!(!BookManager::_is_mostly_blank_cover(&image));
}

#[test]
fn test_extract_cover_thumbnail_ignores_unsupported_type() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(BookManager::_extract_cover_thumbnail(&tmp.path().join("a.txt"), "txt", None), None);
}

#[tokio::test]
async fn test_get_cover_returns_404_when_file_is_missing() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), FakeEsManager::with_doc(make_doc("gone.epub", "epub")));
    let resp = manager.get_cover(1, None).await;
    assert_eq!(resp.status_code, 404);
}

#[tokio::test]
async fn test_get_cover_returns_503_when_storage_access_fails() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("book.epub"), b"PK").unwrap();

    // is_file 확인이 book.epub에서만 EIO로 실패하고 나머지는 실제 파일 시스템에 맡긴다
    let fs_ops = FaultyFs::new();
    fs_ops.set_is_file(|p: &Path| if p.file_name().unwrap() == "book.epub" { Some(std::io::Error::from_raw_os_error(5)) } else { None });
    let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::with_doc(make_doc("book.epub", "epub"))).with_fs(fs_ops));

    let resp = manager.get_cover(1, None).await;

    assert_eq!(resp.status_code, 503);
}

// ---- 커버 목록(작품 묶음 + 커서) ----

fn cover_doc(path: &str, title: &str, category: &str, file_type: &str) -> Value {
    let mut doc = make_doc(path, file_type);
    doc.as_object_mut().unwrap().extend([("title".to_string(), json!(title)), ("category".to_string(), json!(category)), ("file_path".to_string(), json!(path))]);
    doc
}

#[derive(Default)]
struct FileCall {
    categories: Vec<String>,
    size: usize,
    exclude: Vec<String>,
}

#[derive(Default)]
struct RepCall {
    keys: Vec<String>,
    exclude: Vec<String>,
}

#[derive(Default)]
struct Calls {
    files: Vec<FileCall>,
    reps: Vec<RepCall>,
}

type Stats = HashMap<String, (usize, f64, f64)>;
type CoverEntry = (i64, Value, (i64, i64, String));

/// 커버 목록 조회용 ES 대역. `FakeEsManager`에 커버 목록 메서드가 없어 이 세 메서드만 가진 로컬 대역을 쓴다.
/// 나머지 `EsManagerApi` 메서드는 기본 구현(`Unsupported`)이다. TODO(common): `FakeEsManager`에 커버 목록 훅이 생기면 지운다.
type CoverFilesFn = dyn Fn(&[String], i64, Option<(i64, i64, String)>, &[String]) -> (Vec<(i64, Value, Vec<Value>)>, i64) + Send + Sync;
type CoverRepsFn = dyn Fn(&[String], &[String]) -> HashMap<String, Vec<(i64, Value)>> + Send + Sync;

struct CoverEs {
    stats: Stats,
    files: Box<CoverFilesFn>,
    reps: Box<CoverRepsFn>,
}

impl EsManagerApi for CoverEs {
    fn search_cover_files_paged(&self, categories: &[String], size: i64, search_after: Option<(i64, i64, String)>, exclude: &[String]) -> Result<(Vec<(i64, Value, Vec<Value>)>, i64), EsError> {
        Ok((self.files)(categories, size, search_after, exclude))
    }

    fn search_group_representatives(&self, keys: &[String], exclude: &[String]) -> Result<HashMap<String, Vec<(i64, Value)>>, EsError> {
        Ok((self.reps)(keys, exclude))
    }

    fn aggregate_cover_categories(&self) -> Result<Stats, EsError> {
        Ok(self.stats.clone())
    }
}

/// ES 없이 커서 조회가 하는 일(최신순 정렬, search_after, 전체 건수)을 흉내 낸다. 받은 인자는 calls에 모은다.
/// files는 (문서, 수정 시각) 쌍이다. 정렬값은 ES와 같은 (수정 시각, 생성 시각, 파일 경로)다.
fn fake_cover_es(stats: &Stats, files: &[(Value, i64)], reps: &HashMap<String, Vec<Value>>) -> (Arc<CoverEs>, Arc<Mutex<Calls>>) {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let mut entries: Vec<CoverEntry> = files.iter().enumerate().map(|(i, (doc, updated))| (i as i64 + 1, doc.clone(), (*updated, 0, doc["file_path"].as_str().unwrap().to_string()))).collect();
    let key = |item: &CoverEntry| (-item.2.0, -item.2.1, item.2.2.clone());
    entries.sort_by_key(key);

    let files_calls = calls.clone();
    let files_fn = move |categories: &[String], size: i64, search_after: Option<(i64, i64, String)>, exclude: &[String]| {
        let size = size as usize;
        files_calls.lock().unwrap().files.push(FileCall { categories: categories.to_vec(), size, exclude: exclude.to_vec() });
        let eligible: Vec<&CoverEntry> = entries.iter().filter(|item| categories.iter().any(|c| c == item.1["category"].as_str().unwrap())).collect();
        let window: Vec<(i64, Value, Vec<Value>)> = eligible
            .iter()
            .filter(|item| match &search_after {
                None => true,
                Some(sa) => key(item) > (-sa.0, -sa.1, sa.2.clone()),
            })
            .take(size)
            .map(|item| (item.0, item.1.clone(), vec![json!(item.2.0), json!(item.2.1), json!(item.2.2)]))
            .collect();
        (window, eligible.len() as i64)
    };
    let rep_calls = calls.clone();
    let reps = reps.clone();
    let reps_fn = move |keys: &[String], exclude: &[String]| {
        rep_calls.lock().unwrap().reps.push(RepCall { keys: keys.to_vec(), exclude: exclude.to_vec() });
        keys.iter().filter(|k| reps.contains_key(*k)).map(|k| (k.clone(), reps[k].iter().enumerate().map(|(i, d)| (900 + i as i64, d.clone())).collect::<Vec<_>>())).collect::<HashMap<_, _>>()
    };
    let es = Arc::new(CoverEs { stats: stats.clone(), files: Box::new(files_fn), reps: Box::new(reps_fn) });
    (es, calls)
}

fn titles(items: &[(Book, String)]) -> Vec<(String, String)> {
    items.iter().map(|(book, display)| (book.title.clone(), display.clone())).collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// 디렉토리 A: 직속 파일 가(수정 400), 다(수정 200)와 작품 디렉토리 나작품(최근 300), 라작품(최근 100).
type CoverFixture = (Vec<(Value, i64)>, HashMap<String, Vec<Value>>, Stats);

fn cover_fixture() -> CoverFixture {
    let files = vec![(cover_doc("A/가.epub", "가", "A", "epub"), 400), (cover_doc("A/다.pdf", "다", "A", "pdf"), 200)];
    let reps = HashMap::from([
        ("A/나작품".to_string(), vec![cover_doc("A/나작품/나작품 02권.pdf", "나작품 02권", "A/나작품", "pdf"), cover_doc("A/나작품/나작품 01권.pdf", "나작품 01권", "A/나작품", "pdf")]),
        ("A/라작품".to_string(), vec![cover_doc("A/라작품/라작품 1권.pdf", "라작품 1권", "A/라작품", "pdf")]),
    ]);
    let stats = HashMap::from([("A".to_string(), (2, 400.0, 0.0)), ("A/나작품".to_string(), (12, 300.0, 0.0)), ("A/라작품".to_string(), (1, 100.0, 0.0))]);
    (files, reps, stats)
}

fn page(category: Option<&str>, size: usize, cursor: Option<&str>, exclude: &[&str]) -> CoverPageOpts {
    CoverPageOpts { category: category.map(str::to_string), size, cursor: cursor.map(str::to_string), exclude_categories: strings(exclude) }
}

/// 원본 테스트의 `(항목, 전체 수, 다음 커서, 오류)` 쪽.
type RawPage = (Vec<(Book, String)>, usize, Option<String>, Option<String>);

/// `Result<CoverPage, _>`를 원본 테스트 모양으로 펼친다.
fn unpage(result: Result<CoverPage, String>) -> RawPage {
    match result {
        Ok(page) => (page.items, page.total.unwrap_or(0), page.next_cursor, None),
        Err(error) => (Vec::new(), 0, None, Some(error)),
    }
}

#[tokio::test]
async fn test_get_cover_books_paged_merges_files_and_work_groups_newest_first() {
    let tmp = TempDir::new().unwrap();
    let (files, reps, stats) = cover_fixture();
    let (es, calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    let (items, total, next_cursor, error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &[])).await);

    assert_eq!(error, None);
    assert_eq!(next_cursor, None);
    assert_eq!(total, 4);
    // 묶음은 1권을 대표로 하고 작품 디렉토리 이름을 display_title로 갖는다. 순서는 최신순이다.
    assert_eq!(titles(&items), pairs(&[("가", ""), ("나작품 01권", "나작품"), ("다", ""), ("라작품 1권", "라작품")]));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.files[0].categories, strings(&["A"]));
    assert_eq!(calls.reps[0].keys, strings(&["A/나작품", "A/라작품"]));
}

#[tokio::test]
async fn test_get_cover_books_paged_rejects_malformed_es_book() {
    let tmp = TempDir::new().unwrap();
    let mut doc = make_doc("A/bad.epub", "epub");
    doc.as_object_mut().unwrap().remove("category");
    let es = Arc::new(CoverEs {
        stats: HashMap::from([("A".to_string(), (1, 10.0, 0.0))]),
        files: Box::new(move |_, _, _, _| (vec![(1, doc.clone(), vec![json!(10), json!(0), json!("A/bad.epub")])], 1)),
        reps: Box::new(|_, _| HashMap::new()),
    });
    let manager = make_manager(tmp.path(), es);

    let (items, total, cursor, error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &[])).await);

    assert!(items.is_empty());
    assert_eq!(total, 0);
    assert!(cursor.is_none());
    assert!(error.unwrap().contains("category"));
}

#[tokio::test]
async fn test_get_cover_books_paged_pages_continue_without_gaps_or_repeats() {
    let tmp = TempDir::new().unwrap();
    let (files, reps, stats) = cover_fixture();
    let (es, _calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    let mut seen: Vec<(String, String)> = Vec::new();
    let mut cursor = String::new();
    let mut finished = false;
    for _ in 0..10 {
        let (items, total, cursor_next, error) = unpage(manager.get_cover_books_paged(page(Some("A"), 1, Some(&cursor), &[])).await);
        assert_eq!(error, None);
        assert_eq!(total, 4);
        seen.extend(titles(&items));
        match cursor_next {
            None => {
                finished = true;
                break;
            }
            Some(next) => cursor = next,
        }
    }
    assert!(finished, "커서가 끝나지 않는다");

    assert_eq!(seen, pairs(&[("가", ""), ("나작품 01권", "나작품"), ("다", ""), ("라작품 1권", "라작품")]));
}

#[tokio::test]
async fn test_get_cover_books_paged_root_shows_root_files_and_one_card_per_top_level_directory() {
    let tmp = TempDir::new().unwrap();
    let files = vec![(cover_doc("루트책.epub", "루트책", "_root", "epub"), 50), (cover_doc("3_판타지/가.epub", "가", "3_판타지", "epub"), 10)];
    let reps = HashMap::from([("3_판타지".to_string(), vec![files[1].0.clone()]), ("0_웹툰".to_string(), vec![cover_doc("0_웹툰/작품 01권.pdf", "작품 01권", "0_웹툰/작품", "pdf")])]);
    let stats = HashMap::from([("_root".to_string(), (1, 50.0, 0.0)), ("3_판타지".to_string(), (12, 70.0, 0.0)), ("0_웹툰/작품".to_string(), (3, 90.0, 0.0))]);
    let (es, calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    let (items, total, next_cursor, error) = unpage(manager.get_cover_books_paged(page(None, 10, None, &[])).await);

    assert_eq!(error, None);
    assert_eq!(next_cursor, None);
    // 3_판타지 직속 책은 펼치지 않고 디렉토리마다 대표 한 권만 나온다. 최신 시각이 큰 묶음부터다.
    let calls = calls.lock().unwrap();
    assert_eq!(calls.files[0].categories, strings(&["_root"]));
    assert_eq!(calls.reps[0].keys, strings(&["0_웹툰", "3_판타지"]));
    assert_eq!(total, 3);
    assert_eq!(titles(&items), pairs(&[("작품 01권", "0_웹툰"), ("가", "3_판타지"), ("루트책", "")]));
}

#[tokio::test]
async fn test_get_cover_books_paged_scopes_to_selected_directory() {
    let tmp = TempDir::new().unwrap();
    let (files, reps, stats) = cover_fixture();
    let (es, calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    let (items, total, _cursor, error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &[])).await);

    assert_eq!(error, None);
    assert_eq!(total, 4);
    assert_eq!(calls.lock().unwrap().files[0].categories, strings(&["A"]));
    let displays: Vec<&str> = items.iter().map(|(_, d)| d.as_str()).collect();
    assert_eq!(displays, vec!["", "나작품", "", "라작품"]);
}

#[tokio::test]
async fn test_get_cover_books_paged_excludes_hidden_categories_everywhere() {
    let tmp = TempDir::new().unwrap();
    let (files, reps, stats) = cover_fixture();
    let (es, calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    let (items, total, _cursor, _error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &["A/나작품"])).await);

    assert_eq!(total, 3);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.reps[0].keys, strings(&["A/라작품"]));
    assert_eq!(calls.files[0].exclude, strings(&["A/나작품"]));
    assert_eq!(calls.reps[0].exclude, strings(&["A/나작품"]));
    let displays: Vec<&str> = items.iter().map(|(_, d)| d.as_str()).collect();
    assert_eq!(displays, vec!["", "", "라작품"]);
}

#[tokio::test]
async fn test_get_cover_books_paged_skips_group_without_representative() {
    let tmp = TempDir::new().unwrap();
    let (files, mut reps, stats) = cover_fixture();
    reps.remove("A/나작품");
    let (es, _calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    let (items, _total, _cursor, _error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &[])).await);

    let displays: Vec<&str> = items.iter().map(|(_, d)| d.as_str()).collect();
    assert_eq!(displays, vec!["", "", "라작품"]);
}

#[tokio::test]
async fn test_get_cover_books_paged_keeps_paging_when_only_files_remain() {
    let tmp = TempDir::new().unwrap();
    let (files, reps, stats) = cover_fixture();
    let (es, _calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    let (items, _total, next_cursor, _error) = unpage(manager.get_cover_books_paged(page(Some("A"), 2, None, &[])).await);

    let displays: Vec<&str> = items.iter().map(|(_, d)| d.as_str()).collect();
    assert_eq!(displays, vec!["", "나작품"]);
    assert!(next_cursor.is_some());
}

#[tokio::test]
async fn test_get_cover_books_paged_rejects_malformed_cursor() {
    // parametrize: 디코딩 실패, 디코딩은 되지만 [그룹, 파일] 두 칸 형식이 아닌 두 경우
    for bad_cursor in ["bad_cursor!!!", "WzFd", "WyJ4IiwgbnVsbF0="] {
        let tmp = TempDir::new().unwrap();
        let (files, reps, stats) = cover_fixture();
        let (es, _calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        let result = unpage(manager.get_cover_books_paged(page(None, 10, Some(bad_cursor), &[])).await);

        assert!(result.0.is_empty() && result.1 == 0 && result.2.is_none() && result.3.as_deref() == Some("invalid cursor"), "cursor {bad_cursor}");
    }
}

#[tokio::test]
async fn test_get_cover_books_paged_rejects_wrong_cursor_member_shapes() {
    let tmp = TempDir::new().unwrap();
    let (files, reps, stats) = cover_fixture();
    let (es, _calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    // 묶음·파일 커서는 [수정 시각, 생성 시각, 경로] 세 칸이어야 한다.
    for member in [json!([["a"], null]), json!([null, "x"]), json!([null, [1, 2]]), json!([[1, 2, "k", "extra"], null])] {
        let cursor = encode_category_cursor(&member);
        let result = unpage(manager.get_cover_books_paged(page(None, 10, Some(&cursor), &[])).await);
        assert_eq!(result.3.as_deref(), Some("invalid cursor"), "member {member}");
    }
}

#[tokio::test]
async fn test_get_cover_books_paged_clamps_page_size() {
    let tmp = TempDir::new().unwrap();
    let (files, reps, stats) = cover_fixture();
    let (es, calls) = fake_cover_es(&stats, &files, &reps);
    let manager = make_manager(tmp.path(), es);

    unpage(manager.get_cover_books_paged(page(None, 0, None, &[])).await);
    unpage(manager.get_cover_books_paged(page(None, 1_000_000_000, None, &[])).await);

    let calls = calls.lock().unwrap();
    assert_eq!(calls.files[0].size, 1);
    assert_eq!(calls.files[1].size, MAX_CATEGORY_PAGE_SIZE as usize);
}

/// 쪽 크기 66x165pt인 빈 한 쪽짜리 PDF를 직접 만든다(pikepdf 대신).
fn blank_pdf(width: u32, height: u32) -> Vec<u8> {
    let objs = ["<< /Type /Catalog /Pages 2 0 R >>".to_string(), "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(), format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] >>")];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, obj) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{obj}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
    for off in &offsets {
        out.push_str(&format!("{off:010} 00000 n \n"));
    }
    out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1));
    out.into_bytes()
}

fn vector_pdf() -> Vec<u8> {
    let content = b"q 1 0 0 rg 40 80 180 220 re f Q\nBT /F1 36 Tf 30 300 Td (RUST COVER) Tj ET\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 450] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        [format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(), content.to_vec(), b"endstream".to_vec()].concat(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn append_large_unreferenced_stream(path: &Path, pdf: &[u8], stream_size: u64) {
    let marker = b"startxref\n";
    let start = pdf.windows(marker.len()).rposition(|window| window == marker).unwrap() + marker.len();
    let previous_xref = String::from_utf8_lossy(&pdf[start..]).lines().next().unwrap().parse::<u64>().unwrap();
    let mut file = fs::File::create(path).unwrap();
    file.write_all(pdf).unwrap();
    let object_offset = file.stream_position().unwrap();
    write!(file, "6 0 obj\n<< /Length {stream_size} >>\nstream\n").unwrap();
    file.seek(SeekFrom::Current(stream_size as i64)).unwrap();
    file.write_all(b"\nendstream\nendobj\n").unwrap();
    let xref_offset = file.stream_position().unwrap();
    write!(file, "xref\n6 1\n{object_offset:010} 00000 n \ntrailer\n<< /Size 7 /Root 1 0 R /Prev {previous_xref} >>\nstartxref\n{xref_offset}\n%%EOF\n").unwrap();
}

#[test]
fn test_extract_cover_thumbnail_scales_tall_first_page_to_exact_cover_size() {
    // 쪽 폭 66pt 의 세로로 긴 쪽은 Poppler 렌더 결과가 한 픽셀 어긋나도 썸네일 크기가 고정된다.
    let tmp = TempDir::new().unwrap();
    let pdf_path = tmp.path().join("tall.pdf");
    fs::write(&pdf_path, blank_pdf(66, 165)).unwrap();

    let data = BookManager::_extract_cover_thumbnail(&pdf_path, "pdf", Some(200)).expect("썸네일");

    assert_eq!(image::guess_format(&data).unwrap(), image::ImageFormat::Jpeg);
    let thumb = image::load_from_memory(&data).unwrap();
    // Python round()는 반올림 동점을 짝수로 보내지만 Rust round()는 0에서 먼 쪽이다. 동점이 나오는 비율이면 달라질 수 있다.
    assert_eq!((thumb.width(), thumb.height()), (200, (200.0 / BookManager::COVER_ASPECT_RATIO).round() as u32));
}

#[test]
fn test_extract_cover_thumbnail_renders_text_and_vector_content() {
    let tmp = TempDir::new().unwrap();
    let pdf_path = tmp.path().join("vector.pdf");
    fs::write(&pdf_path, vector_pdf()).unwrap();

    let data = BookManager::_extract_cover_thumbnail(&pdf_path, "pdf", Some(200)).expect("벡터 PDF 표지");
    let thumb = image::load_from_memory(&data).unwrap().to_rgb8();

    assert_eq!((thumb.width(), thumb.height()), (200, 300));
    assert!(thumb.pixels().any(|pixel| pixel[0] > 180 && pixel[1] < 100 && pixel[2] < 100), "빨간 벡터 사각형이 렌더링돼야 한다");
}

#[test]
fn test_extract_cover_thumbnail_renders_first_page_of_large_pdf() {
    let tmp = TempDir::new().unwrap();
    let pdf_path = tmp.path().join("large.pdf");
    let stream_size = 256 * 1024 * 1024 + 1;
    append_large_unreferenced_stream(&pdf_path, &vector_pdf(), stream_size);
    assert!(fs::metadata(&pdf_path).unwrap().len() > 256 * 1024 * 1024);

    let data = BookManager::_extract_cover_thumbnail(&pdf_path, "pdf", Some(200)).expect("큰 PDF 첫 쪽 표지");
    let thumb = image::load_from_memory(&data).unwrap().to_rgb8();

    assert_eq!((thumb.width(), thumb.height()), (200, 300));
    assert!(thumb.pixels().any(|pixel| pixel[0] > 180 && pixel[1] < 100 && pixel[2] < 100), "첫 쪽 벡터 표지를 렌더링해야 한다");
}
