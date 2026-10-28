//! 원본: tests/test_pdf_delete_pages.py
//!
//! BookManager.delete_pdf_pages 단위 테스트 (ES/Docker 없이 mock 기반).
//!
//! 가정한 API (자세한 시그니처는 `common/book_fixtures.rs` 머리말을 본다. 설계는 docs/rust-test-seams.md 3.2, 3.12):
//! - `BookManager::with_deps(BookManagerDeps) -> BookManager`: ES, 책 추가, 파일 읽기, 이미지 fetch를 생성 때 꽂는다
//!   (Python fixture의 `monkeypatch.setattr`/`patch`에 대응하는 DI seam이다)
//!   - `BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>)`: 인자 순서는 (path, es)다. book_fixtures의 (es, path)를 뒤집는다
//!   - `.with_book_adder(Arc<dyn BookAdder>)`, `.with_file_reader(Arc<dyn FileReader>)`, `.with_image_fetcher(Arc<dyn ImageFetcher>)` (구 `AddBook`, `ReadFile`, `FetchUrlImage`)
//! - `BookManager::delete_pdf_pages(&self, book_id: i64, pages: &[i64], crops: Option<&[Value]>, splits: Option<&[Value]>, inserts: Option<&[Value]>, order: Option<&[i64]>) -> (Option<Value>, Option<String>)`
//! - `BookManager::preview_insert_image(&self, url: &str) -> (Option<Vec<u8>>, Option<String>)`
//! - `BookManager::preview_insert_upload(&self, data: &[u8]) -> (Option<Vec<u8>>, Option<String>)`
//! - `backend::epub_cover::MAX_IMAGE_BYTES: usize`
//! - `backend::main::{app_with, AppDeps, AuthMode}`: `app_with(AppDeps::new(AuthMode::Fixed(json!({"role": "admin"}))).with_book_manager(Arc<dyn BookManagerApi>))`.
//!   구 `app_with_overrides(Arc, Option<Value>)`. 엔드포인트 테스트의 `fx::FakeManager`가 `BookManagerApi`를 구현한다(나머지 메서드는 trait 기본 구현)
//!
//! 엔드포인트 경로는 Python과 같다: `/pdf-pages/{id}/insert-preview`, `/insert-preview-upload`, `/delete`.
//! `book_fixtures`의 `env`/`Env`/`Opts`/`Edit`/`FakeManager`/`FetchBehavior`는 아직 `#[path]`로 쓴다(TODO). PDF·이미지·편집 도구는 `common::{pdf, image, raster, edits}`에서 가져온다.

mod common;
#[path = "common/book_fixtures.rs"]
mod fx;

use std::fs;

use axum::http::Method;
use backend::direct_cover_url::FetchError;
use backend::main::{AppDeps, AuthMode, app_with};
use common::TestClient;
use common::edits;
use common::image::{assert_approx, image_size, jpeg_with_orientation, multipart_file, png_halves};
use common::pdf::{
    decoded, first_image, first_image_raw, first_image_size, flate_source, has_xobject, load, make_image_pdf, make_pixel_pdf, mediabox_of, mediabox_size, page_count, pixel_image, widths,
};
use common::raster::same_pixels;
use fx::{Edit, FetchBehavior, Opts, image_env, pixel_env};
use serde_json::{Value, json};

/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

fn has_error(error: &Option<String>) -> bool {
    error.as_deref().is_some_and(|e| !e.is_empty())
}

fn error_text(error: &Option<String>) -> &str {
    error.as_deref().unwrap_or("")
}

#[tokio::test]
async fn test_reindexed_document_gets_a_fresh_updated_time() {
    // 저장한 PDF 는 최신 책 목록(updated_time 순)에 오른다. 모킹 없이 실제 Loader.read_file 로 재색인 문서를 만든다.
    let e = fx::env(Opts::plain());

    let (_result, error) = e.run_pages(&[2]).await;

    assert_eq!(error, None);
    let calls = e.es.inserted();
    let indexed = &calls.last().unwrap()[&1];
    assert_eq!(indexed["page_count"], 4);
    let updated = indexed["updated_time"].as_str().unwrap();
    assert!(updated > "2024-01-01T00:00:00.000000", "{updated}");
    let parsed = chrono::NaiveDateTime::parse_from_str(updated, "%Y-%m-%dT%H:%M:%S%.f").unwrap();
    let delta = (chrono::Local::now().naive_local() - parsed).num_seconds().abs();
    assert!(delta < 60, "{updated}");
}

#[tokio::test]
async fn test_deletes_pages_and_keeps_original_in_trash() {
    let e = fx::env(Opts::loader());
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[2, 4]).await;

    assert_eq!(error, None);
    assert_eq!(result.unwrap(), json!({"remaining_pages": 3, "cropped_pages": 0, "split_pages": 0, "inserted_pages": 0, "backup": "0_trash/book.pdf", "backup_created": true}));
    assert_eq!(widths(&e.pdf), [100, 102, 104]);
    let backup = e.root.join("0_trash").join("book.pdf");
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert_eq!(e.es.deleted(), vec![1]);
    let calls = e.es.inserted();
    assert_eq!(calls.len(), 1);
    let indexed = &calls[0][&1];
    assert!(indexed["category"] == "cat" && indexed["file_path"] == "cat/book.pdf");
}

#[tokio::test]
async fn test_existing_backup_is_not_overwritten() {
    let e = fx::env(Opts::loader());
    let original = fs::read(&e.pdf).unwrap();

    e.run_pages(&[1]).await;
    let (result, error) = e.run_pages(&[1]).await;

    assert_eq!(error, None);
    assert_eq!(result.unwrap()["backup_created"], false);
    assert_eq!(fs::read(e.root.join("0_trash").join("book.pdf")).unwrap(), original);
    assert_eq!(widths(&e.pdf), [102, 103, 104]);
}

#[tokio::test]
async fn test_invalid_pages_leave_file_untouched() {
    let cases: [&[i64]; 4] = [&[], &[0], &[6], &[1, 2, 3, 4, 5]];
    for pages in cases {
        let e = fx::env(Opts::loader());
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run_pages(pages).await;

        assert!(result.is_none() && has_error(&error), "pages={pages:?}: {result:?} {error:?}");
        e.assert_nothing_changed(&original);
    }
}

#[tokio::test]
async fn test_duplicate_pages_count_once() {
    let e = fx::env(Opts::loader());
    let (result, error) = e.run_pages(&[2, 2, 2]).await;
    assert_eq!(error, None);
    assert_eq!(result.unwrap()["remaining_pages"], 4);
}

#[tokio::test]
async fn test_non_pdf_is_rejected() {
    let e = fx::env(Opts::loader());
    e.es.set_doc(Some(json!({"category": "c", "title": "t", "author": "a", "file_path": "cat/book.epub", "file_type": "epub", "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000"})));
    let (result, error) = e.run_pages(&[1]).await;
    assert!(result.is_none() && error_text(&error).contains("Not a PDF"), "{error:?}");
}

#[tokio::test]
async fn test_unknown_book() {
    let e = fx::env(Opts::loader());
    e.es.set_doc(None);
    let (result, error) = e.run_pages(&[1]).await;
    assert!(result.is_none() && error_text(&error).to_lowercase().contains("not found"), "{error:?}");
}

#[tokio::test]
async fn test_reindex_failure_reports_backup_location() {
    let e = fx::env(Opts::loader());
    e.es.fail("insert", "boom");

    let (result, error) = e.run_pages(&[3]).await;

    assert!(result.is_none());
    assert!(error_text(&error).contains("0_trash/book.pdf"), "{error:?}");
    assert!(e.root.join("0_trash").join("book.pdf").is_file());
}

// ── 쪽 순서 바꾸기 ──

#[tokio::test]
async fn test_order_rearranges_pages_and_keeps_original_in_trash() {
    let cases: [(&[i64], [i64; 5]); 4] =
        [(&[2, 1], [101, 100, 102, 103, 104]), (&[1, 3, 2], [100, 102, 101, 103, 104]), (&[5, 4, 3, 2, 1], [104, 103, 102, 101, 100]), (&[2, 3, 1], [101, 102, 100, 103, 104])];
    for (order, expected) in cases {
        let e = fx::env(Opts::loader());
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { order: Some(order.to_vec()), ..Edit::default() }).await;

        assert_eq!(error, None, "order={order:?}");
        let result = result.unwrap();
        assert!(result["remaining_pages"] == 5 && result["backup"] == "0_trash/book.pdf", "order={order:?}: {result}");
        assert_eq!(widths(&e.pdf), expected, "order={order:?}");
        assert_eq!(fs::read(e.root.join("0_trash").join("book.pdf")).unwrap(), original, "order={order:?}");
        assert_eq!(e.es.deleted(), vec![1], "order={order:?}");
    }
}

#[tokio::test]
async fn test_order_and_delete_use_original_page_numbers() {
    let e = fx::env(Opts::loader());

    let (result, error) = e.run(&Edit { pages: vec![1], order: Some(vec![2, 1, 3]), ..Edit::default() }).await;

    assert_eq!(error, None);
    assert_eq!(result.unwrap()["remaining_pages"], 4);
    assert_eq!(widths(&e.pdf), [101, 102, 103, 104]);
}

#[tokio::test]
async fn test_invalid_order_changes_nothing() {
    let cases: [&[i64]; 5] = [&[1, 1], &[2, 3], &[0, 1], &[1, 2, 3, 4, 5, 6], &[1, 3]];
    for order in cases {
        let e = fx::env(Opts::loader());
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { order: Some(order.to_vec()), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains("순서"), "order={order:?}: {error:?}");
        e.assert_nothing_changed(&original);
    }
}

// ── 이미지 크롭 ──

#[tokio::test]
async fn test_crop_keeps_only_the_given_region() {
    let e = image_env(Opts::loader());
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(2, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let result = result.unwrap();
    assert!(result["remaining_pages"] == 4 && result["cropped_pages"] == 1, "{result}");
    assert_eq!(fs::read(e.root.join("0_trash").join("book.pdf")).unwrap(), original);
    let img = first_image(&e.pdf, 2);
    assert_eq!((img.width, img.height), (50, 60));
    assert_approx(img.box_w, 50.0, "box_w");
    assert!(img.center[2] > 200 && img.center[0] < 60, "{:?}", img.center); // 오른쪽(파랑)만 남았다
    assert_eq!(first_image_size(&e.pdf, 1), (100, 60)); // 다른 쪽은 그대로
    assert_eq!(e.es.deleted(), vec![1]);
}

#[tokio::test]
async fn test_crop_arbitrary_rectangle_resizes_page_proportionally() {
    let e = image_env(Opts::loader());

    let (_, error) = e.run(&Edit { crops: Some(vec![edits::crop(1, 0.25, 0.25, 0.75, 0.75)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let img = first_image(&e.pdf, 1);
    assert_eq!((img.width, img.height), (50, 30));
    assert_approx(img.box_w, 50.0, "box_w");
    let doc = load(&e.pdf);
    let boxed = mediabox_of(&doc, 1);
    for (actual, expected) in boxed.iter().zip([0.0, 0.0, 50.0, 30.0]) {
        assert_approx(*actual, expected, "mediabox");
    }
}

#[tokio::test]
async fn test_crop_and_page_delete_use_original_page_numbers() {
    let e = image_env(Opts::loader());

    let (result, error) = e.run(&Edit { pages: vec![1], crops: Some(vec![edits::crop(4, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    assert_eq!(result.unwrap()["remaining_pages"], 3);
    assert_eq!(first_image_size(&e.pdf, 3), (50, 60)); // 원본 4쪽이 3쪽이 되고 오른쪽 절반만 남음
    assert_eq!(first_image_size(&e.pdf, 1), (100, 60)); // 원본 2쪽
}

#[tokio::test]
async fn test_unsupported_crop_changes_nothing() {
    let cases: Vec<(Vec<i64>, Vec<Value>, &str)> = vec![
        (vec![], vec![edits::crop(3, 0.5, 0.0, 1.0, 1.0)], "이미지 없는 쪽"),
        (vec![], vec![edits::crop(1, 0.5, 0.0, 1.0, 1.0), edits::crop(3, 0.5, 0.0, 1.0, 1.0)], "하나라도 실패하면 전부 취소"),
        (vec![2], vec![edits::crop(2, 0.5, 0.0, 1.0, 1.0)], "전체 삭제와 겹침"),
        (vec![], vec![edits::crop(9, 0.5, 0.0, 1.0, 1.0)], "쪽 범위 밖"),
        (vec![], vec![edits::crop(1, 0.6, 0.0, 0.4, 1.0)], "x0 >= x1"),
        (vec![], vec![edits::crop(1, -0.1, 0.0, 1.0, 1.0)], "쪽 밖"),
        (vec![], vec![edits::crop(1, 0.0, 0.0, 1.2, 1.0)], "쪽 밖"),
        (vec![], vec![edits::crop(1, 0.5, 0.5, 0.55, 0.55)], "너무 작음"),
        (vec![], vec![edits::crop(1, 0.0, 0.0, 0.5, 1.0), edits::crop(1, 0.5, 0.0, 1.0, 1.0)], "같은 쪽에 둘 이상"),
    ];
    for (pages, crops, label) in cases {
        let e = image_env(Opts::loader());
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { pages, crops: Some(crops), ..Edit::default() }).await;

        assert!(result.is_none() && has_error(&error), "{label}: {result:?} {error:?}");
        e.assert_nothing_changed(&original);
    }
}

#[tokio::test]
async fn test_rotated_crop_straightens_the_region() {
    let e = image_env(Opts::loader());
    let original = fs::read(&e.pdf).unwrap();

    // 시계 방향 90°: 박스의 위쪽이 화면의 오른쪽을 향한다 -> 출력 윗줄은 원본 오른쪽(파랑), 아랫줄은 왼쪽(빨강)
    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop_at(1, 0.35, 0.1, 0.65, 0.9, 90.0)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["cropped_pages"] == 1, "{error:?}");
    assert_eq!(fs::read(e.root.join("0_trash").join("book.pdf")).unwrap(), original);
    let img = first_image(&e.pdf, 1);
    assert_eq!((img.width, img.height), (30, 48));
    assert_approx(img.box_w, 30.0, "box_w");
    let image = decoded(&e.pdf, 1);
    let (top, bottom) = (image.get_pixel(15, 3).0, image.get_pixel(15, 44).0);
    assert!(top[2] > 200 && top[0] < 60, "{top:?}");
    assert!(bottom[0] > 200 && bottom[2] < 60, "{bottom:?}");
}

#[tokio::test]
async fn test_rotated_crop_output_size_follows_the_box() {
    let e = image_env(Opts::loader());

    let (_, error) = e.run(&Edit { crops: Some(vec![edits::crop_at(2, 0.4, 0.3, 0.6, 0.7, 30.0)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let img = first_image(&e.pdf, 2);
    assert_eq!((img.width, img.height), (20, 24));
    assert_approx(img.box_w, 20.0, "box_w");
    assert_eq!(first_image_size(&e.pdf, 1), (100, 60)); // 다른 쪽은 그대로
}

#[tokio::test]
async fn test_rotated_split_pages() {
    let e = image_env(Opts::loader());

    let rotated = json!({"page": 1, "first": {"x0": 0.35, "y0": 0.1, "x1": 0.65, "y1": 0.9, "angle": 90.0}, "second": {"x0": 0.1, "y0": 0.1, "x1": 0.4, "y1": 0.9}});
    let (result, error) = e.run(&Edit { splits: Some(vec![rotated]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["remaining_pages"] == 5, "{error:?}");
    assert_eq!(first_image_size(&e.pdf, 1), (30, 48));
    assert_eq!(first_image_size(&e.pdf, 2), (30, 48));
}

#[tokio::test]
async fn test_invalid_rotated_crop_changes_nothing() {
    // serde_json은 NaN을 null로 바꾼다. 각도가 null/비유한이면 오류여야 한다.
    let cases: Vec<(Value, &str)> = vec![
        (edits::crop_at(1, 0.0, 0.0, 1.0, 1.0, 10.0), "기울이면 꼭짓점이 쪽 밖으로 나간다"),
        (edits::crop_at(1, 0.05, 0.05, 0.95, 0.95, 45.0), "꼭짓점이 쪽 밖"),
        (edits::crop_at(1, 0.2, 0.2, 0.8, 0.8, 400.0), "각도 범위 밖"),
        (edits::crop_at(1, 0.2, 0.2, 0.8, 0.8, f64::NAN), "각도 NaN"),
        (edits::crop_at(1, 0.5, 0.5, 0.52, 0.52, 20.0), "너무 작음"),
    ];
    for (c, label) in cases {
        let e = image_env(Opts::loader());
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { crops: Some(vec![c]), ..Edit::default() }).await;

        assert!(result.is_none() && has_error(&error), "{label}: {result:?} {error:?}");
        assert_eq!(fs::read(&e.pdf).unwrap(), original, "{label}");
        assert!(!e.root.join("0_trash").exists(), "{label}");
        assert!(e.es.deleted().is_empty(), "{label}");
    }
}

// ── 쪽 나누기 ──

#[tokio::test]
async fn test_split_replaces_the_page_with_first_then_second() {
    let e = image_env(Opts::loader());
    let original = fs::read(&e.pdf).unwrap();

    // 앞쪽은 오른쪽(파랑), 뒤쪽은 왼쪽(빨강)으로 지정한다: 순서는 영역이 아니라 first/second 가 정한다.
    let (result, error) = e.run(&Edit { splits: Some(vec![edits::split(2, edits::RIGHT, edits::LEFT)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let result = result.unwrap();
    assert!(result["remaining_pages"] == 5 && result["split_pages"] == 1, "{result}");
    assert_eq!(page_count(&e.pdf), 5);
    assert_eq!(fs::read(e.root.join("0_trash").join("book.pdf")).unwrap(), original);
    let (first, second) = (first_image(&e.pdf, 2), first_image(&e.pdf, 3));
    assert_eq!((first.width, first.height), (50, 60));
    assert_approx(first.box_w, 50.0, "first box_w");
    assert_eq!((second.width, second.height), (50, 60));
    assert_approx(second.box_w, 50.0, "second box_w");
    assert!(first.center[2] > 200 && first.center[0] < 60, "{:?}", first.center); // 2쪽: 파랑
    assert!(second.center[0] > 200 && second.center[2] < 60, "{:?}", second.center); // 3쪽: 빨강
    assert_eq!(first_image_size(&e.pdf, 1), (100, 60)); // 앞의 쪽은 그대로
    assert_eq!(first_image_size(&e.pdf, 5), (100, 60)); // 뒤의 쪽은 한 칸 밀렸다
    assert_eq!(e.es.deleted(), vec![1]);
}

#[tokio::test]
async fn test_split_pages_move_together_with_their_page_when_reordered() {
    let e = image_env(Opts::loader());

    let (result, error) = e.run(&Edit { splits: Some(vec![edits::split(2, edits::LEFT, edits::RIGHT)]), order: Some(vec![2, 1]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["remaining_pages"] == 5, "{error:?}");
    let sizes: Vec<(i64, i64)> = (1..=3).map(|n| first_image_size(&e.pdf, n)).collect();
    assert_eq!(sizes, [(50, 60), (50, 60), (100, 60)]);
}

#[tokio::test]
async fn test_split_boxes_may_overlap() {
    let e = image_env(Opts::loader());

    let (_, error) = e.run(&Edit { splits: Some(vec![edits::split(1, [0.0, 0.0, 0.6, 1.0], [0.4, 0.0, 1.0, 1.0])]), ..Edit::default() }).await;

    assert_eq!(error, None);
    assert_eq!(first_image_size(&e.pdf, 1), (60, 60));
    assert_eq!(first_image_size(&e.pdf, 2), (60, 60));
}

#[tokio::test]
async fn test_split_and_delete_use_original_page_numbers() {
    let e = image_env(Opts::loader());

    // 원본 1쪽 삭제 + 원본 2쪽 분할 + 원본 4쪽 크롭: 편집본은 [2a, 2b, 3(text), 4(crop)]
    let (result, error) =
        e.run(&Edit { pages: vec![1], crops: Some(vec![edits::crop(4, 0.5, 0.0, 1.0, 1.0)]), splits: Some(vec![edits::split(2, edits::LEFT, edits::RIGHT)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    assert_eq!(result.unwrap()["remaining_pages"], 4);
    let sizes: Vec<(i64, i64)> = (1..=2).map(|n| first_image_size(&e.pdf, n)).collect();
    assert_eq!(sizes, [(50, 60), (50, 60)]);
    assert_eq!(first_image_size(&e.pdf, 4), (50, 60));
    let doc = load(&e.pdf);
    assert!(!has_xobject(&doc, 3)); // 3쪽은 이미지 없는 원본 3쪽
}

#[tokio::test]
async fn test_delete_after_split_removes_the_right_page() {
    let e = image_env(Opts::loader());

    let (result, error) = e.run(&Edit { pages: vec![4], splits: Some(vec![edits::split(2, edits::LEFT, edits::RIGHT)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    assert_eq!(result.unwrap()["remaining_pages"], 4); // 4 + 1 - 1
    assert_eq!(page_count(&e.pdf), 4);
}

#[tokio::test]
async fn test_unsupported_split_changes_nothing() {
    type Case = (Vec<i64>, Option<Vec<Value>>, Vec<Value>, &'static str);
    let cases: Vec<Case> = vec![
        (vec![], None, vec![edits::split(3, edits::LEFT, edits::RIGHT)], "이미지 없는 쪽"),
        (vec![], None, vec![edits::split(1, edits::LEFT, edits::RIGHT), edits::split(3, edits::LEFT, edits::RIGHT)], "하나라도 실패하면 전부 취소"),
        (vec![2], None, vec![edits::split(2, edits::LEFT, edits::RIGHT)], "삭제와 겹침"),
        (vec![], Some(vec![edits::crop(2, 0.0, 0.0, 1.0, 1.0)]), vec![edits::split(2, edits::LEFT, edits::RIGHT)], "크롭과 겹침"),
        (vec![], None, vec![edits::split(9, edits::LEFT, edits::RIGHT)], "쪽 범위 밖"),
        (vec![], None, vec![edits::split(1, [0.6, 0.0, 0.4, 1.0], edits::RIGHT)], "앞 영역이 비어 있음"),
        (vec![], None, vec![edits::split(1, edits::LEFT, [0.5, 0.5, 0.55, 0.55])], "뒤 영역이 너무 작음"),
        (vec![], None, vec![edits::split(1, edits::LEFT, edits::RIGHT), edits::split(1, edits::LEFT, edits::RIGHT)], "같은 쪽에 둘 이상"),
    ];
    for (pages, crops, splits, label) in cases {
        let e = image_env(Opts::loader());
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { pages, crops, splits: Some(splits), ..Edit::default() }).await;

        assert!(result.is_none() && has_error(&error), "{label}: {result:?} {error:?}");
        e.assert_nothing_changed(&original);
    }
}

// ── 이미지 URL 로 쪽 끼워넣기 ──

#[tokio::test]
async fn test_insert_adds_an_image_page_before_the_given_page() {
    let e = image_env(Opts::loader_fetch());
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::insert(2)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let result = result.unwrap();
    assert!(result["remaining_pages"] == 5 && result["inserted_pages"] == 1, "{result}");
    assert_eq!(*e.fetch.calls.lock().unwrap(), vec!["https://img.example.com/a.png".to_string()]);
    assert_eq!(fs::read(e.root.join("0_trash").join("book.pdf")).unwrap(), original);
    assert_eq!(page_count(&e.pdf), 5);
    // 새 2쪽: 폭은 기준 쪽(원본 2쪽, 100pt)에 맞추고 높이는 이미지 비율(200x300 -> 150pt)
    let (width, height) = mediabox_size(&e.pdf, 2);
    assert_approx(width, 100.0, "width");
    assert_approx(height, 150.0, "height");
    let image = decoded(&e.pdf, 2);
    assert_eq!(image.dimensions(), (200, 300));
    let up = image.get_pixel(100, 40).0;
    let down = image.get_pixel(100, 260).0;
    assert!(up[1] > 150 && up[0] < 80, "{up:?}"); // 위쪽: 초록
    assert!(down[0] > 180 && down[2] < 80, "{down:?}"); // 아래쪽: 노랑
    assert_eq!(first_image_size(&e.pdf, 3), (100, 60)); // 원본 2쪽은 한 칸 밀렸다
    assert_eq!(first_image_size(&e.pdf, 1), (100, 60));
    assert_eq!(e.es.deleted(), vec![1]);
}

#[tokio::test]
async fn test_inserted_page_moves_together_with_its_reference_page_when_reordered() {
    let e = image_env(Opts::loader_fetch());

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::insert(2)]), order: Some(vec![2, 1]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["remaining_pages"] == 5, "{error:?}");
    assert_approx(mediabox_size(&e.pdf, 1).1, 150.0, "1쪽 높이"); // 삽입한 쪽이 기준 쪽(원본 2쪽)과 함께 맨 앞으로 갔다
    assert_approx(mediabox_size(&e.pdf, 2).1, 60.0, "2쪽 높이");
    assert_approx(mediabox_size(&e.pdf, 3).1, 60.0, "3쪽 높이");
}

#[tokio::test]
async fn test_inserted_page_can_be_cropped_later() {
    let e = image_env(Opts::loader_fetch());

    let (_, error) = e.run(&Edit { inserts: Some(vec![edits::insert(1)]), ..Edit::default() }).await;
    assert_eq!(error, None);
    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(1, 0.0, 0.0, 0.5, 0.5)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["cropped_pages"] == 1, "{error:?}");
    assert_eq!(first_image_size(&e.pdf, 1), (100, 150));
}

#[tokio::test]
async fn test_insert_with_delete_split_and_crop_uses_original_numbers() {
    let e = image_env(Opts::loader_fetch());

    // 원본 [1, 2, 3(text), 4] -> 1 삭제, 2 앞에 삽입, 2 분할, 4 앞에 삽입
    // 기대: [삽입, 2a, 2b, 3(text), 삽입, 4]
    let (result, error) =
        e.run(&Edit { pages: vec![1], splits: Some(vec![edits::split(2, edits::LEFT, edits::RIGHT)]), inserts: Some(vec![edits::insert(2), edits::insert(4)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let result = result.unwrap();
    assert!(result["remaining_pages"] == 6 && result["inserted_pages"] == 2, "{result}");
    // 삽입 쪽의 폭은 기준 쪽의 편집 전 폭(100pt)이다: 분할로 기준 쪽이 반쪽(50pt)이 되어도 같다.
    let (w, h) = mediabox_size(&e.pdf, 1);
    assert_approx(w, 100.0, "1쪽 너비");
    assert_approx(h, 150.0, "1쪽 높이");
    assert_eq!(first_image_size(&e.pdf, 2), (50, 60));
    assert_eq!(first_image_size(&e.pdf, 3), (50, 60));
    let doc = load(&e.pdf);
    assert!(!has_xobject(&doc, 4)); // 원본 3쪽(text)
    assert_approx(mediabox_size(&e.pdf, 5).1, 150.0, "5쪽 높이");
    assert_eq!(first_image_size(&e.pdf, 6), (100, 60));
}

#[tokio::test]
async fn test_inserted_page_survives_when_the_reference_page_is_deleted() {
    let e = image_env(Opts::loader_fetch());

    let (result, error) = e.run(&Edit { pages: vec![2], inserts: Some(vec![edits::insert(2)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["remaining_pages"] == 4, "{error:?}");
    assert_approx(mediabox_size(&e.pdf, 2).1, 150.0, "2쪽 높이"); // 삽입한 쪽이 원본 2쪽 자리에 남는다
    assert_eq!(page_count(&e.pdf), 4);
    let doc = load(&e.pdf);
    assert!(!has_xobject(&doc, 3)); // 그 다음은 원본 3쪽(text)
}

#[tokio::test]
async fn test_invalid_insert_changes_nothing() {
    let cases: Vec<(Vec<Value>, &str)> = vec![
        (vec![edits::insert(9)], "쪽 범위 밖"),
        (vec![edits::insert(0)], "0쪽"),
        (vec![edits::insert(1), edits::insert_url(1, "https://img.example.com/b.png")], "같은 쪽에 둘 이상"),
        (vec![edits::insert_url(1, "   ")], "빈 URL"),
    ];
    for (inserts, label) in cases {
        let e = image_env(Opts::loader_fetch());
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { inserts: Some(inserts), ..Edit::default() }).await;

        assert!(result.is_none() && has_error(&error), "{label}: {result:?} {error:?}");
        e.assert_nothing_changed(&original);
    }
}

#[tokio::test]
async fn test_fetch_failure_changes_nothing() {
    let cases = [
        (FetchError::Value("공개 HTTP/HTTPS 이미지 URL만 사용할 수 있습니다".into()), "1쪽 앞에 넣을 이미지를 가져올 수 없습니다"),
        (FetchError::Os("Name or service not known".into()), "이미지 서버 주소를 확인하지 못했습니다"),
    ];
    for (effect, expected) in cases {
        let e = image_env(Opts::loader_fetch());
        let original = fs::read(&e.pdf).unwrap();
        e.fetch.set(FetchBehavior::Err(effect));

        let (result, error) = e.run(&Edit { inserts: Some(vec![edits::insert(1)]), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains(expected), "{expected}: {error:?}");
        assert_eq!(fs::read(&e.pdf).unwrap(), original, "{expected}");
        assert!(!e.root.join("0_trash").exists(), "{expected}");
        assert!(e.es.deleted().is_empty(), "{expected}");
    }
}

#[tokio::test]
async fn test_non_image_download_is_rejected() {
    let e = image_env(Opts::loader_fetch());
    let original = fs::read(&e.pdf).unwrap();
    e.fetch.set(FetchBehavior::Ok(b"<html>not an image</html>".to_vec()));

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::insert(1)]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("이미지"), "{error:?}");
    assert_eq!(fs::read(&e.pdf).unwrap(), original);
    assert!(!e.root.join("0_trash").exists());
}

#[tokio::test]
async fn test_unsafe_urls_are_rejected_by_the_real_fetcher() {
    for url in ["ftp://example.com/a.png", "file:///etc/passwd", "http://127.0.0.1/a.png", "http://[::1]/a.png", "http://user:pw@example.com/a.png"] {
        let e = image_env(Opts::loader());
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { inserts: Some(vec![edits::insert_url(1, url)]), ..Edit::default() }).await;

        assert!(result.is_none() && has_error(&error), "url={url}: {error:?}");
        e.assert_nothing_changed(&original);
    }
}

// ── 저장 전 미리보기 ──

#[tokio::test]
async fn test_preview_returns_the_normalized_jpeg() {
    let e = image_env(Opts::fetch());

    let jpeg = e.bm.preview_insert_image("  https://img.example.com/a.png  ").await.unwrap();

    assert_eq!(*e.fetch.calls.lock().unwrap(), vec!["https://img.example.com/a.png".to_string()]);
    assert_eq!(&jpeg[..3], b"\xff\xd8\xff"); // PNG 를 내려받아도 저장될 형식인 JPEG 로 돌려준다
    assert_eq!(image_size(&jpeg), (200, 300));
}

#[tokio::test]
async fn test_preview_matches_what_the_save_inserts() {
    let e = image_env(Opts::loader_fetch());

    let (preview, _) = split(e.bm.preview_insert_image("https://img.example.com/a.png").await);
    let (_, error) = e.run(&Edit { inserts: Some(vec![edits::insert(1)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let saved = first_image_raw(&e.pdf, 1);
    assert_eq!(Some(saved), preview); // 화면에서 본 이미지가 그대로 삽입된다
}

#[tokio::test]
async fn test_preview_rejects_empty_and_unsafe_urls() {
    for url in ["", "   ", "ftp://example.com/a.png", "http://127.0.0.1/a.png", "http://[::1]/a.png"] {
        let e = image_env(Opts::plain());

        let (jpeg, error) = split(e.bm.preview_insert_image(url).await);

        assert!(jpeg.is_none() && has_error(&error), "url={url:?}: {error:?}");
    }
}

#[tokio::test]
async fn test_preview_reports_fetch_failures() {
    let cases = [
        (FetchError::Value("내부 주소의 이미지는 사용할 수 없습니다".into()), "내부 주소의 이미지는 사용할 수 없습니다"),
        (FetchError::Os("Name or service not known".into()), "이미지 서버 주소를 확인하지 못했습니다"),
        (FetchError::Other("boom".into()), "이미지를 가져오지 못했습니다"),
    ];
    for (effect, expected) in cases {
        let e = image_env(Opts::fetch());
        e.fetch.set(FetchBehavior::Err(effect));

        let (jpeg, error) = split(e.bm.preview_insert_image("https://img.example.com/a.png").await);

        assert!(jpeg.is_none() && error.as_deref() == Some(expected), "{expected}: {error:?}");
    }
}

#[tokio::test]
async fn test_preview_rejects_non_image_download() {
    let e = image_env(Opts::fetch());
    e.fetch.set(FetchBehavior::Ok(b"<html>nope</html>".to_vec()));

    let (jpeg, error) = split(e.bm.preview_insert_image("https://img.example.com/a.png").await);

    assert!(jpeg.is_none() && error_text(&error).contains("이미지"), "{error:?}");
}

/// POST /pdf-pages/{id}/insert-preview: 관리자만 호출할 수 있고, 성공하면 JPEG, 실패하면 400 과 사유를 돌려준다.
mod test_insert_preview_endpoint {
    use super::*;

    fn client() -> (TestClient, std::sync::Arc<fx::FakeManager>) {
        let fake = fx::FakeManager::new();
        let app = app_with(AppDeps::new(AuthMode::Fixed(json!({"role": "admin"}))).with_book_manager(fake.clone()));
        (TestClient::with_app(app), fake)
    }

    #[tokio::test]
    async fn test_success_returns_jpeg_without_caching() {
        let (http, fake) = client();
        let response = http.post_json("/pdf-pages/1/insert-preview", &json!({"url": "https://img.example.com/a.png"})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-type"), Some("image/jpeg"));
        assert_eq!(response.header("cache-control"), Some("private, no-store"));
        assert_eq!(response.body, b"\xff\xd8\xffJPEG");
        assert_eq!(fake.call_count("preview_insert_image"), 1);
        assert_eq!(fake.args("preview_insert_image"), vec![json!("https://img.example.com/a.png")]);
    }

    #[tokio::test]
    async fn test_failure_returns_400_with_the_reason() {
        let (http, fake) = client();
        *fake.preview_insert_image.lock().unwrap() = (None, Some("내부 주소의 이미지는 사용할 수 없습니다".to_string()));
        let response = http.post_json("/pdf-pages/1/insert-preview", &json!({"url": "http://127.0.0.1/a.png"})).await;
        assert_eq!(response.status, 400);
        assert!(response.text().contains("내부 주소"));
    }

    #[tokio::test]
    async fn test_empty_or_oversized_url_is_a_validation_error() {
        let (http, _) = client();
        assert_eq!(http.post_json("/pdf-pages/1/insert-preview", &json!({"url": ""})).await.status, 422);
        assert_eq!(http.post_json("/pdf-pages/1/insert-preview", &json!({"url": "x".repeat(5000)})).await.status, 422);
    }

    #[tokio::test]
    async fn test_requires_admin() {
        let response = TestClient::new().post_json("/pdf-pages/1/insert-preview", &json!({"url": "https://img.example.com/a.png"})).await;
        assert!(response.status == 401 || response.status == 403, "{}", response.status);
    }
}

// ── Flate 등 JPEG 이 아닌 이미지 쪽 (무손실로 다시 저장) ──

#[tokio::test]
async fn test_flate_crop_is_lossless() {
    let cases = [(1usize, "rgb", "RGB", "/DeviceRGB"), (2, "rgb_png", "RGB", "/DeviceRGB"), (3, "gray", "L", "/DeviceGray")];
    for (page, kind, mode, cs) in cases {
        let e = pixel_env(Opts::loader());
        let rect = (0.3, 0.2, 0.9, 0.8);
        let expected = flate_source(kind).crop(30, 12, 90, 48);

        let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(page as i64, rect.0, rect.1, rect.2, rect.3)]), ..Edit::default() }).await;

        assert!(error.is_none() && result.unwrap()["cropped_pages"] == 1, "{kind}: {error:?}");
        let (image, info) = pixel_image(&e.pdf, page);
        assert_eq!((info.filter.as_str(), info.bpc, info.cs.as_str()), ("/FlateDecode", 8, cs), "{kind}"); // JPEG 로 바꾸지 않는다
        assert!(image.mode == mode && same_pixels(&image, &expected), "{kind}: 픽셀이 같지 않다"); // 픽셀이 완전히 같다
        let (w, h) = mediabox_size(&e.pdf, page);
        assert_approx(w, 60.0, "너비");
        assert_approx(h, 36.0, "높이");
        assert!(e.root.join("0_trash").join("book.pdf").is_file(), "{kind}");
        assert_eq!(pixel_image(&e.pdf, 6).1.filter, "/DCTDecode", "{kind}"); // JPEG 쪽은 그대로
    }
}

#[tokio::test]
async fn test_one_bit_crop_stays_one_bit_with_row_padding() {
    let e = pixel_env(Opts::loader());
    // 너비 50(8의 배수가 아님)이라 행 끝에 패딩 비트가 생긴다
    let (_, error) = e.run(&Edit { crops: Some(vec![edits::crop(4, 0.25, 0.0, 0.75, 1.0)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let (image, info) = pixel_image(&e.pdf, 4);
    assert!(info.bpc == 1 && info.cs == "/DeviceGray" && image.mode == "1");
    assert_eq!((image.w, image.h), (50, 60));
    assert!(same_pixels(&image, &flate_source("bw").crop(25, 0, 75, 60)));
}

#[tokio::test]
async fn test_palette_image_becomes_rgb() {
    let e = pixel_env(Opts::loader());

    let (_, error) = e.run(&Edit { crops: Some(vec![edits::crop(5, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let (image, info) = pixel_image(&e.pdf, 5);
    assert!(info.cs == "/DeviceRGB" && image.mode == "RGB");
    assert_eq!(image.pixel(10, 10), [0, 0, 255]); // 오른쪽 절반(팔레트 1번 = 파랑)
}

#[tokio::test]
async fn test_rotated_flate_crop_straightens_the_region() {
    for page in [1usize, 2, 3] {
        let e = pixel_env(Opts::loader());

        let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop_at(page as i64, 0.35, 0.1, 0.65, 0.9, 90.0)]), ..Edit::default() }).await;

        assert!(error.is_none() && result.unwrap()["cropped_pages"] == 1, "page={page}: {error:?}");
        let (image, info) = pixel_image(&e.pdf, page);
        assert!(info.filter == "/FlateDecode" && (image.w, image.h) == (30, 48), "page={page}");
        let (top, bottom) = (image.pixel(15, 3), image.pixel(15, 44));
        // rgb/rgb_png: 위 파랑, 아래 빨강. gray: 위 200(밝음), 아래 60(어두움)
        let ok = if page == 3 { top.iter().map(|&v| v as u32).sum::<u32>() > bottom.iter().map(|&v| v as u32).sum::<u32>() } else { top[2] > 200 && bottom[0] > 200 };
        assert!(ok, "page={page}: top={top:?} bottom={bottom:?}");
    }
}

#[tokio::test]
async fn test_rotated_one_bit_crop_becomes_eight_bit_gray() {
    let e = pixel_env(Opts::loader());

    let (_, error) = e.run(&Edit { crops: Some(vec![edits::crop_at(4, 0.35, 0.1, 0.65, 0.9, 90.0)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    let (image, info) = pixel_image(&e.pdf, 4);
    assert!(image.mode == "L" && info.bpc == 8 && info.cs == "/DeviceGray");
    assert!(image.gray(15, 3) > 200 && image.gray(15, 44) < 60); // 위 흰색, 아래 검정
}

#[tokio::test]
async fn test_split_works_on_flate_pages() {
    let e = pixel_env(Opts::loader());

    let (result, error) = e.run(&Edit { splits: Some(vec![edits::split(2, edits::RIGHT, edits::LEFT)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["remaining_pages"] == 7, "{error:?}");
    let (first, _) = pixel_image(&e.pdf, 2);
    let (second, _) = pixel_image(&e.pdf, 3);
    assert!(first.pixel(10, 10) == [0, 0, 255] && second.pixel(10, 10) == [255, 0, 0]);
}

#[tokio::test]
async fn test_mixed_jpeg_and_flate_pages_in_one_save() {
    let e = pixel_env(Opts::loader());

    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(1, 0.5, 0.0, 1.0, 1.0), edits::crop(6, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["cropped_pages"] == 2, "{error:?}");
    assert_eq!(pixel_image(&e.pdf, 1).1.filter, "/FlateDecode");
    assert_eq!(pixel_image(&e.pdf, 6).1.filter, "/DCTDecode");
}

#[tokio::test]
async fn test_unsupported_pixel_images_change_nothing() {
    let cases = [("cmyk", "색상 모드"), ("smask", "색상 구성"), ("jbig2", "지원하지 않는 이미지 형식"), ("corrupt", "해독")];
    for (kind, expected) in cases {
        let e = fx::env(Opts::loader());
        make_pixel_pdf(&e.pdf, &[kind]);
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(1, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains(expected), "{kind}: {error:?}");
        e.assert_nothing_changed(&original);
    }
}

#[tokio::test]
async fn test_rotated_page_is_rejected() {
    let e = fx::env(Opts::loader());
    make_image_pdf(&e.pdf, &["image", "rotated"], (100, 60));
    let (result, error) = e.run(&Edit { crops: Some(vec![edits::crop(2, 0.5, 0.0, 1.0, 1.0)]), ..Edit::default() }).await;
    assert!(result.is_none() && error_text(&error).contains("회전"), "{error:?}");
}

#[tokio::test]
async fn test_save_clears_cover_cache() {
    let e = image_env(Opts::loader());
    let cache = e.root.join(".cover_cache");
    fs::create_dir(&cache).unwrap();
    fs::write(cache.join("1.v4.jpg"), b"old").unwrap();
    fs::write(cache.join("2.v4.jpg"), b"other book").unwrap();

    let (_, error) = e.run_pages(&[1]).await;

    assert_eq!(error, None);
    assert!(!cache.join("1.v4.jpg").exists());
    assert!(cache.join("2.v4.jpg").exists());
}

// ── 올린 이미지(image_data)로 쪽 끼워넣기 ──

#[tokio::test]
async fn test_insert_uploaded_image_adds_a_page_without_fetching() {
    let e = image_env(Opts::loader_fetch());
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::upload(2, &png_halves(200, 300))]), ..Edit::default() }).await;

    assert_eq!(error, None);
    assert_eq!(result.unwrap()["inserted_pages"], 1);
    assert!(e.fetch.calls.lock().unwrap().is_empty());
    assert_eq!(fs::read(e.root.join("0_trash").join("book.pdf")).unwrap(), original);
    assert_eq!(page_count(&e.pdf), 5);
    let (width, height) = mediabox_size(&e.pdf, 2);
    assert_approx(width, 100.0, "width");
    assert_approx(height, 150.0, "height");
}

#[tokio::test]
async fn test_uploaded_and_url_inserts_can_be_mixed() {
    let e = image_env(Opts::loader_fetch());

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::insert(1), edits::upload(3, &png_halves(200, 300))]), ..Edit::default() }).await;

    assert!(error.is_none() && result.unwrap()["inserted_pages"] == 2, "{error:?}");
    assert_eq!(page_count(&e.pdf), 6);
}

/// 미리보기로 받은 JPEG 를 그대로 보내 저장하면 다시 인코딩하지 않아 화면에서 본 이미지가 삽입된다.
#[tokio::test]
async fn test_saved_upload_is_the_exact_preview_image() {
    let e = image_env(Opts::loader());

    let (preview, error) = split(e.bm.preview_insert_upload(&png_halves(200, 300)).await);
    assert_eq!(error, None);
    let preview = preview.unwrap();
    let (_, error) = e.run(&Edit { inserts: Some(vec![edits::upload(1, &preview)]), ..Edit::default() }).await;

    assert_eq!(error, None);
    assert_eq!(first_image_raw(&e.pdf, 1), preview);
}

/// 폰 카메라 사진은 회전을 EXIF 로 기록한다. 반영하지 않으면 세로 사진이 옆으로 눕는다.
#[tokio::test]
async fn test_upload_preview_applies_exif_orientation() {
    let cases = [(1u16, (200u32, 100u32)), (6, (100, 200)), (8, (100, 200)), (3, (200, 100))];
    for (orientation, expected) in cases {
        let e = image_env(Opts::plain());

        let (jpeg, error) = split(e.bm.preview_insert_upload(&jpeg_with_orientation(orientation, 200, 100)).await);

        assert_eq!(error, None, "orientation={orientation}");
        assert_eq!(image_size(&jpeg.unwrap()), expected, "orientation={orientation}");
    }
}

#[tokio::test]
async fn test_upload_preview_shrinks_a_large_image() {
    let e = image_env(Opts::plain());

    let jpeg = e.bm.preview_insert_upload(&png_halves(3000, 1500)).await.unwrap();

    assert_eq!(image_size(&jpeg), (2000, 1000));
}

#[tokio::test]
async fn test_upload_preview_rejects_empty_and_unreadable_data() {
    let cases: [&[u8]; 3] = [b"", b"not an image", b"\xff\xd8\xff garbage"];
    for data in cases {
        let e = image_env(Opts::plain());

        let (jpeg, error) = split(e.bm.preview_insert_upload(data).await);

        assert!(jpeg.is_none() && has_error(&error), "data={data:?}: {error:?}");
    }
}

#[tokio::test]
async fn test_upload_preview_rejects_oversized_data() {
    let e = image_env(Opts::plain());

    let (jpeg, error) = split(e.bm.preview_insert_upload(&vec![0u8; backend::epub_cover::MAX_IMAGE_BYTES + 1]).await);

    assert!(jpeg.is_none() && error_text(&error).contains("큽니다"), "{error:?}");
}

#[tokio::test]
async fn test_unreadable_upload_fails_without_touching_the_files() {
    let e = image_env(Opts::loader());
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::upload(2, b"not an image")]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("2쪽 앞에 넣을 이미지를 가져올 수 없습니다"), "{error:?}");
    assert_eq!(fs::read(&e.pdf).unwrap(), original);
    assert!(!e.root.join("0_trash").exists());
    assert!(e.es.deleted().is_empty());
}

#[tokio::test]
async fn test_invalid_base64_is_rejected() {
    let e = image_env(Opts::loader());
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run(&Edit { inserts: Some(vec![json!({"page": 2, "image_data": "***not-base64***"})]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("2쪽 앞에 넣을 이미지 데이터를 읽을 수 없습니다"), "{error:?}");
    assert_eq!(fs::read(&e.pdf).unwrap(), original);
    assert!(!e.root.join("0_trash").exists());
}

#[tokio::test]
async fn test_two_sources_for_the_same_page_are_rejected() {
    let e = image_env(Opts::loader_fetch());

    let (result, error) = e.run(&Edit { inserts: Some(vec![edits::insert(2), edits::upload(2, &png_halves(200, 300))]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("둘 이상"), "{error:?}");
}

/// POST /pdf-pages/{id}/insert-preview-upload 와 image_data 를 받는 저장 엔드포인트.
mod test_insert_upload_endpoint {
    use super::*;

    fn client() -> (TestClient, std::sync::Arc<fx::FakeManager>) {
        let fake = fx::FakeManager::new();
        let app = app_with(AppDeps::new(AuthMode::Fixed(json!({"role": "admin"}))).with_book_manager(fake.clone()));
        (TestClient::with_app(app), fake)
    }

    #[tokio::test]
    async fn test_upload_returns_jpeg_without_caching() {
        let (http, fake) = client();
        let body = multipart_file("tmbound", "file", "a.png", "image/png", b"PNGDATA");
        let response = http.post_multipart("/pdf-pages/1/insert-preview-upload", "tmbound", body).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-type"), Some("image/jpeg"));
        assert_eq!(response.header("cache-control"), Some("private, no-store"));
        assert_eq!(response.body, b"\xff\xd8\xffJPEG");
        assert_eq!(fake.call_count("preview_insert_upload"), 1);
        assert_eq!(fake.args("preview_insert_upload"), vec![json!(b"PNGDATA".as_slice())]);
    }

    #[tokio::test]
    async fn test_upload_failure_returns_400_with_the_reason() {
        let (http, fake) = client();
        *fake.preview_insert_upload.lock().unwrap() = (None, Some("이미지 형식을 읽을 수 없습니다".to_string()));
        let body = multipart_file("tmbound", "file", "a.txt", "text/plain", b"x");
        let response = http.post_multipart("/pdf-pages/1/insert-preview-upload", "tmbound", body).await;
        assert_eq!(response.status, 400);
        assert!(response.text().contains("형식"));
    }

    #[tokio::test]
    async fn test_delete_passes_the_new_page_order() {
        let (http, fake) = client();
        assert_eq!(http.post_json("/pdf-pages/1/delete", &json!({"order": [2, 1]})).await.json()["status"], "success");
        assert_eq!(fake.args("delete_pdf_pages")[5], json!([2, 1]));
    }

    #[tokio::test]
    async fn test_upload_without_a_file_is_a_validation_error() {
        let (http, _) = client();
        assert_eq!(http.request(Method::POST, "/pdf-pages/1/insert-preview-upload", None, Vec::new()).await.status, 422);
    }

    #[tokio::test]
    async fn test_upload_requires_admin() {
        let body = multipart_file("tmbound", "file", "a.png", "image/png", b"x");
        let response = TestClient::new().post_multipart("/pdf-pages/1/insert-preview-upload", "tmbound", body).await;
        assert!(response.status == 401 || response.status == 403, "{}", response.status);
    }

    #[tokio::test]
    async fn test_delete_failure_is_reported_as_failure_status() {
        let (http, fake) = client();
        *fake.delete_pdf_pages.lock().unwrap() = (None, Some("삭제할 쪽이 없습니다".to_string()));
        let response = http.post_json("/pdf-pages/1/delete", &json!({"pages": [1]})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status": "failure", "error": "삭제할 쪽이 없습니다"}));
    }

    #[tokio::test]
    async fn test_delete_accepts_image_data() {
        let (http, fake) = client();
        let response = http.post_json("/pdf-pages/1/delete", &json!({"inserts": [{"page": 2, "image_data": "QUJD"}]})).await;
        assert!(response.status == 200 && response.json()["status"] == "success");
        let inserts = fake.args("delete_pdf_pages")[4].clone();
        assert_eq!(inserts, json!([{"page": 2, "url": null, "image_data": "QUJD"}]));
    }

    #[tokio::test]
    async fn test_delete_requires_exactly_one_source() {
        let cases = [json!({"page": 2}), json!({"page": 2, "url": "https://a.example/a.png", "image_data": "QUJD"}), json!({"page": 2, "image_data": ""})];
        for insert in cases {
            let (http, fake) = client();
            let response = http.post_json("/pdf-pages/1/delete", &json!({"inserts": [insert]})).await;
            assert_eq!(response.status, 422, "insert={insert}");
            assert_eq!(fake.call_count("delete_pdf_pages"), 0, "insert={insert}");
        }
    }
}
