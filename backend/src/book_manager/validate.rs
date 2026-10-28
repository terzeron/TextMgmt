//! EPUB·PDF 검증. Rust `epubveri`와 Poppler 기반 PDF 검증을 제공한다.

use std::io;
use std::time::Duration;

use serde_json::{Value, json};
use sysdeps::CommandError;

use super::BookManager;
use super::util::unique_token;

/// epubveri 실행 시간 상한.
const EPUBVERI_TIMEOUT: Duration = Duration::from_secs(60);

impl BookManager {
    /// Rust epubveri로 EPUB 파일의 구조적 유효성을 검증한다.
    pub async fn validate_epub(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        log::debug!("# validate_epub(book_id={book_id})");
        let Ok(book) = self.get_book(book_id).await else { return (None, Some(format!("Book not found: {book_id}"))) };
        if book.file_type != "epub" {
            return (None, Some(format!("Not an EPUB file (type: {})", book.file_type)));
        }
        if !self.fs.is_file(&book.file_path).unwrap_or(false) {
            return (None, Some(format!("File not found: {}", book.file_path.display())));
        }

        // 임시 파일에 JSON 출력(stdout에 상태 메시지가 섞이는 문제 방지)
        let json_path = std::env::temp_dir().join(format!("epubveri-{}.json", unique_token()));
        if let Err(e) = std::fs::File::create(&json_path) {
            return (None, Some(format!("Failed to create temp file: {e}")));
        }
        let outcome = self.run_epubveri(&book.file_path, &json_path);
        // 임시 파일 정리. 실패는 무시한다.
        if let Err(e) = self.fs.remove_file(&json_path) {
            log::debug!("validate_epub: temp file cleanup failed: {e}");
        }
        let (data, returncode) = match outcome {
            Ok(v) => v,
            Err(message) => return (None, Some(message)),
        };

        // 기존 API 형식 호환을 위해 메시지 ID 키 "ID"를 읽는다.
        let mut messages: Vec<Value> = Vec::new();
        for msg in data.get("messages").and_then(Value::as_array).into_iter().flatten() {
            let loc = msg.get("locations").and_then(Value::as_array).and_then(|l| l.first()).filter(|l| l.as_object().is_none_or(|o| !o.is_empty()));
            let location = loc.map(
                |loc| json!({"path": loc.get("path").cloned().unwrap_or(json!("")), "line": loc.get("line").cloned().unwrap_or(json!(-1)), "column": loc.get("column").cloned().unwrap_or(json!(-1))}),
            );
            messages.push(json!({
                "severity": msg.get("severity").cloned().unwrap_or(json!("")), "id": msg.get("ID").cloned().unwrap_or(json!("")),
                "message": msg.get("message").cloned().unwrap_or(json!("")), "location": location,
            }));
        }

        let pub_raw = data.get("publication").filter(|p| p.as_object().is_some_and(|o| !o.is_empty()));
        let publication = pub_raw.map(|p| json!({"title": p.get("title").cloned().unwrap_or(json!("")), "creator": p.get("creator").cloned().unwrap_or(json!("")), "date": p.get("date").cloned().unwrap_or(json!("")), "publisher": p.get("publisher").cloned().unwrap_or(json!(""))}));

        // checker는 nFatal/nError/nWarning/nUsage만 준다. INFO 건수는 메시지에서 직접 센다.
        let checker = data.get("checker").cloned().unwrap_or(json!({}));
        let info_count = messages.iter().filter(|m| m["severity"] == "INFO").count();
        let count = |key: &str| checker.get(key).cloned().unwrap_or(json!(0));
        let rel_path = self.relative_to_root(&book.file_path).unwrap_or_else(|| book.file_path.display().to_string());
        let mut result = json!({
            "valid": returncode == 0, "file_path": rel_path, "messages": messages,
            "summary": {"fatal": count("nFatal"), "error": count("nError"), "warning": count("nWarning"), "usage": count("nUsage"), "info": info_count},
        });
        if let Some(publication) = publication {
            result["publication"] = publication;
        }
        (Some(result), None)
    }

    /// epubveri subprocess를 시간 제한으로 실행해 JSON 출력과 종료 코드를 돌려준다.
    fn run_epubveri(&self, epub: &std::path::Path, json_path: &std::path::Path) -> Result<(Value, i32), String> {
        let argv = vec!["epubveri".to_string(), epub.to_string_lossy().into_owned(), "--json".to_string(), json_path.to_string_lossy().into_owned()];
        let output = match self.runner.run(&argv, Some(EPUBVERI_TIMEOUT)) {
            Ok(output) => output,
            Err(CommandError::NotFound) => return Err("epubveri is not installed".to_string()),
            Err(CommandError::Timeout) => return Err("epubveri timed out (60s)".to_string()),
            Err(CommandError::Failed(message)) => return Err(format!("epubveri failed: {message}")),
        };
        let parse = || -> Result<Value, String> {
            let bytes = std::fs::read(json_path).map_err(|e: io::Error| e.to_string())?;
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())
        };
        match parse() {
            Ok(data) => Ok((data, output.returncode)),
            Err(e) => Err(format!("Failed to parse epubveri output (exit_code={}): {e}", output.returncode)),
        }
    }

    /// PDF 파일의 구문 유효성을 검증하고 메타데이터를 추출한다.
    pub async fn validate_pdf(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        log::debug!("# validate_pdf(book_id={book_id})");
        let Ok(book) = self.get_book(book_id).await else { return (None, Some(format!("Book not found: {book_id}"))) };
        if book.file_type != "pdf" {
            return (None, Some(format!("Not a PDF file (type: {})", book.file_type)));
        }
        if !self.fs.is_file(&book.file_path).unwrap_or(false) {
            return (None, Some(format!("File not found: {}", book.file_path.display())));
        }

        let pdf = match self.pdf_opener.open(&book.file_path) {
            Ok(pdf) => pdf,
            Err(e) => return (None, Some(format!("Failed to open PDF: {e}"))),
        };
        let result = (|| -> Result<Value, String> {
            let issues = pdf.check_pdf_syntax().map_err(|e| e.to_string())?;
            let messages: Vec<Value> = issues.iter().map(|m| json!({"severity": "WARNING", "message": m})).collect();

            // 메타데이터 추출
            let docinfo = pdf.docinfo();
            let mut publication = serde_json::Map::new();
            for (key, field) in [("/Title", "title"), ("/Author", "creator"), ("/Producer", "producer"), ("/CreationDate", "creation_date")] {
                if let Some(value) = docinfo.get(key).filter(|v| !v.is_empty()) {
                    publication.insert(field.to_string(), json!(value));
                }
            }
            publication.insert("page_count".to_string(), json!(pdf.page_count()));
            publication.insert("pdf_version".to_string(), json!(pdf.pdf_version()));

            let rel_path = self.relative_to_root(&book.file_path).unwrap_or_else(|| book.file_path.display().to_string());
            let mut result = json!({"valid": issues.is_empty(), "file_path": rel_path, "messages": messages, "summary": {"error": 0, "warning": issues.len()}});
            result["publication"] = Value::Object(publication);
            Ok(result)
        })();
        pdf.close();
        match result {
            Ok(result) => (Some(result), None),
            Err(e) => (None, Some(format!("Failed to open PDF: {e}"))),
        }
    }
}
