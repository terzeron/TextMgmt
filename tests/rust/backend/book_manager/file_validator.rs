//! `backend::book_manager::file_validator` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use backend::book_manager::{BookManager, BookManagerDeps};
use common::es_fake::FakeEsManager;
use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// 공용 헬퍼 (전반부 정의를 복사)
// ---------------------------------------------------------------------------

/// pytest `tmp_path`에 대응한다. 반환값이 drop되면 디렉토리가 지워진다.
fn tmp_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// `asyncio.run(coro)`에 대응한다. 동기 테스트 안에서 future를 끝까지 돌린다.
fn asyncio_runner<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut)
}

/// 기본 구성의 `BookManagerDeps`. `with_*`를 더 붙인 뒤 `BookManager::with_deps`로 만든다.
fn deps(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManagerDeps {
    common::setup_env();
    BookManagerDeps::new(tmp_path.to_path_buf(), es.clone())
}

fn make_manager(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManager {
    BookManager::with_deps(deps(tmp_path, es))
}

fn make_doc(rel_path: &str, file_type: &str) -> Value {
    json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
}

// TODO(common): lopdf가 아니라 /Info 사전을 직접 쓰는 최소 PDF 생성기. common::pdf::PdfBuilder에 `info` 지원이 생기면 지운다.
/// pypdf `PdfWriter.add_blank_page`로 만든 PDF에 대응한다. `info`는 /Info 사전의 (키, 값)이다.
fn pdf_bytes(pages: usize, width: u32, height: u32, info: &[(&str, &str)]) -> Vec<u8> {
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
    for _ in 0..pages {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] >>"));
    }
    if !info.is_empty() {
        let body: String = info.iter().map(|(k, v)| format!("/{k} ({v}) ")).collect();
        objs.push(format!("<< {body}>>"));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
    }
    let xref_pos = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    let info_ref = if info.is_empty() { String::new() } else { format!(" /Info {} 0 R", 3 + pages) };
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R{info_ref} >>\nstartxref\n{xref_pos}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// 612x792 빈 페이지 `pages`장짜리 PDF.
fn blank_pdf_bytes(pages: usize, info: &[(&str, &str)]) -> Vec<u8> {
    pdf_bytes(pages, 612, 792, info)
}

// ---------------------------------------------------------------------------
// 테스트 본문 (원본 순서)
// ---------------------------------------------------------------------------
// ---- coverage: validate_pdf ----

#[test]
fn test_validate_pdf_producer_and_creation_date() {
    // Lines 395, 397: PDF with Producer and CreationDate metadata
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let pdf = tmp_path.join("A").join("test.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();

    // 최소한의 유효한 PDF를 만든다.
    fs::write(&pdf, blank_pdf_bytes(1, &[("Producer", "TestProducer"), ("CreationDate", "D:20240101"), ("Title", "Test"), ("Author", "Author")])).unwrap();

    es.set_doc(Some(make_doc("A/test.pdf", "pdf")));

    let result = asyncio_runner(manager.validate_pdf(1)).expect("result is not None");
    assert!(result["publication"].get("producer").is_some());
    assert!(result["publication"].get("creation_date").is_some());
}
