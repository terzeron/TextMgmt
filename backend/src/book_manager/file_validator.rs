//! EPUB·PDF 검증. Rust `epubveri`와 Poppler 기반 PDF 검증을 제공한다.

use std::io;
use std::time::Duration;

use common::json_field_reader::str_field;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sysdeps::CommandError;

use super::BookManager;
use super::temp_file_maker::unique_token;

/// epubveri 실행 시간 상한.
const EPUBVERI_TIMEOUT: Duration = Duration::from_secs(60);

/// 검증 메시지가 가리키는 위치. 모르는 줄·칸은 -1이다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageLocation {
    pub path: String,
    pub line: i64,
    pub column: i64,
}

/// epubveri 메시지 하나.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EpubMessage {
    pub severity: String,
    pub id: String,
    pub message: String,
    pub location: Option<MessageLocation>,
}

/// 심각도별 메시지 수.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EpubSummary {
    pub fatal: i64,
    pub error: i64,
    pub warning: i64,
    pub usage: i64,
    pub info: i64,
}

/// EPUB 패키지의 서지 정보.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EpubPublication {
    pub title: String,
    pub creator: String,
    pub date: String,
    pub publisher: String,
}

/// EPUB 검증 결과. 서지 정보가 없으면 `publication`이 없다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EpubValidation {
    pub valid: bool,
    pub file_path: String,
    pub messages: Vec<EpubMessage>,
    pub summary: EpubSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication: Option<EpubPublication>,
}

/// PDF 구문 경고 하나.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PdfMessage {
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PdfSummary {
    pub error: i64,
    pub warning: i64,
}

/// PDF 문서 정보. 문서 정보 사전에 없는 항목은 비운다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PdfPublication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<String>,
    pub page_count: usize,
    pub pdf_version: String,
}

/// PDF 검증 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PdfValidation {
    pub valid: bool,
    pub file_path: String,
    pub messages: Vec<PdfMessage>,
    pub summary: PdfSummary,
    pub publication: PdfPublication,
}

fn number_of(value: &Value, key: &str, missing: i64) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(missing)
}

impl BookManager {
    /// Rust epubveri로 EPUB 파일의 구조적 유효성을 검증한다.
    pub async fn validate_epub(&self, book_id: i64) -> Result<EpubValidation, String> {
        log::debug!("# validate_epub(book_id={book_id})");
        let Ok(book) = self.get_book(book_id).await else { return Err(format!("Book not found: {book_id}")) };
        if book.file_type != "epub" {
            return Err(format!("Not an EPUB file (type: {})", book.file_type));
        }
        if !self.fs.is_file(&book.file_path).unwrap_or(false) {
            return Err(format!("File not found: {}", book.file_path.display()));
        }

        // 임시 파일에 JSON 출력(stdout에 상태 메시지가 섞이는 문제 방지)
        let json_path = std::env::temp_dir().join(format!("epubveri-{}.json", unique_token()));
        if let Err(e) = std::fs::File::create(&json_path) {
            return Err(format!("Failed to create temp file: {e}"));
        }
        let outcome = self.run_epubveri(&book.file_path, &json_path);
        // 임시 파일 정리. 실패는 무시한다.
        if let Err(e) = self.fs.remove_file(&json_path) {
            log::debug!("validate_epub: temp file cleanup failed: {e}");
        }
        let (data, returncode) = match outcome {
            Ok(v) => v,
            Err(message) => return Err(message),
        };

        // epubveri는 메시지 ID를 "ID" 키로 준다.
        let mut messages: Vec<EpubMessage> = Vec::new();
        for msg in data.get("messages").and_then(Value::as_array).into_iter().flatten() {
            let loc = msg.get("locations").and_then(Value::as_array).and_then(|l| l.first()).filter(|l| l.as_object().is_none_or(|o| !o.is_empty()));
            let location = loc.map(|loc| MessageLocation { path: str_field(loc, "path").to_string(), line: number_of(loc, "line", -1), column: number_of(loc, "column", -1) });
            messages.push(EpubMessage { severity: str_field(msg, "severity").to_string(), id: str_field(msg, "ID").to_string(), message: str_field(msg, "message").to_string(), location });
        }

        let pub_raw = data.get("publication").filter(|p| p.as_object().is_some_and(|o| !o.is_empty()));
        let publication = pub_raw.map(|p| EpubPublication {
            title: str_field(p, "title").to_string(),
            creator: str_field(p, "creator").to_string(),
            date: str_field(p, "date").to_string(),
            publisher: str_field(p, "publisher").to_string(),
        });

        // checker는 nFatal/nError/nWarning/nUsage만 준다. INFO 건수는 메시지에서 직접 센다.
        let checker = data.get("checker").cloned().unwrap_or(Value::Null);
        let count = |key: &str| number_of(&checker, key, 0);
        let summary =
            EpubSummary { fatal: count("nFatal"), error: count("nError"), warning: count("nWarning"), usage: count("nUsage"), info: messages.iter().filter(|m| m.severity == "INFO").count() as i64 };
        let file_path = self.relative_to_root(&book.file_path).unwrap_or_else(|| book.file_path.display().to_string());
        Ok(EpubValidation { valid: returncode == 0, file_path, messages, summary, publication })
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
    pub async fn validate_pdf(&self, book_id: i64) -> Result<PdfValidation, String> {
        log::debug!("# validate_pdf(book_id={book_id})");
        let Ok(book) = self.get_book(book_id).await else { return Err(format!("Book not found: {book_id}")) };
        if book.file_type != "pdf" {
            return Err(format!("Not a PDF file (type: {})", book.file_type));
        }
        if !self.fs.is_file(&book.file_path).unwrap_or(false) {
            return Err(format!("File not found: {}", book.file_path.display()));
        }

        let pdf = match self.pdf_opener.open(&book.file_path) {
            Ok(pdf) => pdf,
            Err(e) => return Err(format!("Failed to open PDF: {e}")),
        };
        let result = (|| -> Result<PdfValidation, String> {
            let issues = pdf.check_pdf_syntax().map_err(|e| e.to_string())?;
            let messages = issues.iter().map(|m| PdfMessage { severity: "WARNING".to_string(), message: m.clone() }).collect();

            let docinfo = pdf.docinfo();
            let info = |key: &str| docinfo.get(key).filter(|v| !v.is_empty()).cloned();
            let publication = PdfPublication {
                title: info("/Title"),
                creator: info("/Author"),
                producer: info("/Producer"),
                creation_date: info("/CreationDate"),
                page_count: pdf.page_count(),
                pdf_version: pdf.pdf_version(),
            };
            let file_path = self.relative_to_root(&book.file_path).unwrap_or_else(|| book.file_path.display().to_string());
            Ok(PdfValidation { valid: issues.is_empty(), file_path, messages, summary: PdfSummary { error: 0, warning: issues.len() as i64 }, publication })
        })();
        pdf.close();
        match result {
            Ok(result) => Ok(result),
            Err(e) => Err(format!("Failed to open PDF: {e}")),
        }
    }
}
