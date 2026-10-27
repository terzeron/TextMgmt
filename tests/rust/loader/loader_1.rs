#![allow(clippy::type_complexity)]

//! 원본: tests/test_loader.py (전반부)
//!
//! 이 파일이 가정하는 API (실제 crate에 없는 것만, 이름은 Python 원본을 따른다):
//! - dev-dependency: `encoding_rs` (cp949/cp1252 바이트 만들기)
//!
//! 설계: docs/rust-test-seams.md 3.12, 3.13, 4.2. loader/doc_reader의 `_with`는 `*_with(args, &XxxHooks)` 구조체 하나로 통일했다(loader_2.rs와 같은 이름·필드).
//!   훅은 모두 `Option<&dyn Fn(..)>` 필드이고 `Default`로 안 쓰는 훅을 생략한다.
//! - `doc_reader::text::{TextHooks, read_from_text_with(&Path, &TextHooks) -> (TextRead, Option<String>),
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
//! - `loader::record::{ReadHooks, read_file_with_hooks(cfg, path, .., &ReadHooks)}`
//!   `ReadHooks { fast_page_count (_fast_pdf_page_count 대체), fallback_page_count (pypdf 대체), inspect (_zip_container_type 대체, 구 detect_file_type_with) }`
//! - `stat::Stat::{pdf_stage_count, pdf_stage_total_time, pdf_stage_outcome_count}` 단계별 통계 조회

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use doc_reader::TEXT_SIZE;
use doc_reader::docx::read_from_docx;
use doc_reader::epub::{read_from_epub, read_from_epub_with_extracting_zip, read_from_epub_with_extracting_zip_using};
use doc_reader::markup::{count_lines, read_from_html};
use doc_reader::mojibake::{PDF_MOJIBAKE_RUN_LIMIT, count_mojibake_runs, is_usable_text};
use doc_reader::office::{convert_with_libreoffice, find_libreoffice_with, read_from_doc, read_from_hwp, read_from_image};
use doc_reader::pdf::{
    FastCountHooks, InProcess, PDF_PAGE_LIMIT, PdftotextHooks, Stage, StageError, StageRunner, apply_png_predictor, collect_pages, fast_page_count, fast_page_count_with, find_xref_offset,
    pdf_text_by_pdftotext_with, read_from_pdf, run_stage, xref_stream_find_entry,
};
use doc_reader::rtf::read_from_rtf_text;
use doc_reader::text::{Enc, HIGH_BYTE_PAIRED_MIN, TextHooks, TextRead, read_from_text, read_from_text_with, reencode_text_file_to_utf8, reencode_text_file_to_utf8_with};
use loader::cli::print_usage;
use loader::config::Config;
use loader::flow::{get_file_list, read_files};
use loader::record::{ReadHooks, detect_file_type, read_file, read_file_with_hooks};
use serde_json::Value;
use stat::Stat;

#[test]
fn loader_binary_dispatches_the_isbn_pdf_worker_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-pdf.pdf");
    fs::write(&path, b"not a PDF").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_loader")).args(["__isbn-pdf-stage", "fallback"]).arg(path).output().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "0\n\n");
}

#[test]
fn loader_binary_dispatches_the_pdf_worker_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_loader")).arg("__pdf-stage").output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: __pdf-stage"));
}

#[test]
fn loader_binary_runs_poppler_pdf_stage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two-pages.pdf");
    fs::write(&path, minimal_pdf(2)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_loader")).args(["__pdf-stage", "poppler"]).arg(&path).output().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["pages"], 2);
}

#[test]
fn loader_binary_prints_usage_for_help_without_connecting() {
    let output = Command::new(env!("CARGO_BIN_EXE_loader")).arg("--help").output().unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("사용법:"));
}

#[test]
fn loader_binary_reports_missing_es_environment_before_connecting() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_loader"))
        .args(["--delete", "book"])
        .env("TM_BOOK_DIR", dir.path())
        .env("TM_COMICS_DIR", dir.path())
        .env("TM_ES_BOOK_INDEX", "books")
        .env_remove("TM_ES_URL")
        .env_remove("TM_ES_USER")
        .env_remove("TM_ES_PASSWORD")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(255));
    assert!(String::from_utf8_lossy(&output.stderr).contains("TM_ES_URL"));
}

#[test]
fn loader_binary_retries_es_connection_then_deletes_a_missing_index() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for status in [503, 200, 404, 200, 404] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                let Some(headers_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..headers_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok()).flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= headers_end + 4 + content_length {
                    break;
                }
            }
            let (reason, body) = match status {
                200 => ("OK", r#"{"ok":true}"#),
                404 => ("Not Found", r#"{"error":"missing"}"#),
                _ => ("Service Unavailable", r#"{"error":"retry"}"#),
            };
            write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });

    let output = Command::new(env!("CARGO_BIN_EXE_loader"))
        .args(["--delete", "book"])
        .env("TM_BOOK_DIR", dir.path())
        .env("TM_COMICS_DIR", dir.path())
        .env("TM_ES_URL", format!("http://{address}"))
        .env("TM_ES_USER", "test")
        .env("TM_ES_PASSWORD", "test")
        .env("TM_ES_BOOK_INDEX", "books")
        .env_remove("TM_ES_CA_CERT")
        .output()
        .unwrap();
    server.join().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("가 존재하지 않습니다"));
}

fn pdf_with_text(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).bytes());
    }
    let xref = pdf.len();
    pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        pdf.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    pdf
}

#[test]
fn read_file_passes_the_worker_to_isbn_extraction() {
    struct EmptySummaryRunner(PathBuf);

    impl StageRunner for EmptySummaryRunner {
        fn run(&self, _stage: Stage, _path: &Path) -> Result<(String, usize), StageError> {
            Ok((String::new(), 1))
        }

        fn worker_executable(&self) -> Option<&Path> {
            Some(&self.0)
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let books = dir.path().join("books");
    let comics = dir.path().join("comics");
    let category = books.join("A");
    fs::create_dir_all(&category).unwrap();
    fs::create_dir_all(&comics).unwrap();
    let path = category.join("book.pdf");
    fs::write(&path, pdf_with_text("ISBN 9788994492032")).unwrap();
    let cfg = Config::new(&books, &comics);
    let runner = EmptySummaryRunner(PathBuf::from(env!("CARGO_BIN_EXE_loader")));

    let result = read_file(&cfg, &path, None, false, None, &runner, &Stat::new());

    assert_eq!(result.item.unwrap().1["isbn"], "9788994492032");
}

#[test]
fn loader_isbn_pdf_worker_extracts_text_through_the_binary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.pdf");
    fs::write(&path, pdf_with_text("ISBN 9788994492032")).unwrap();

    let isbns = isbn::extract_default_with_worker(&path, None, Path::new(env!("CARGO_BIN_EXE_loader")));

    assert_eq!(isbns, ["9788994492032"]);
}
use zip::write::SimpleFileOptions;

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

fn default_config() -> Config {
    Config::from_env(&conftest_env).unwrap()
}

/// `Loader.path_prefix = book` 과 같다. 만화 prefix는 conftest 기본값이다.
fn config_with_book(book: &Path) -> Config {
    Config::new(book, tests_root().join("comics"))
}

fn write(path: &Path, data: impl AsRef<[u8]>) {
    fs::write(path, data).unwrap();
}

fn read_text(path: &Path) -> TextRead {
    read_from_text(path).0
}

/// `Loader.read_file(path, skip_text=..., path_prefix=...)`의 결과. 원본의 `{inode: doc}` 중 첫 항목을 돌려준다.
fn read_doc(cfg: &Config, path: &Path, skip_text: bool, prefix: Option<&Path>) -> Option<(i64, Value)> {
    read_file(cfg, path, None, skip_text, prefix, &InProcess, &Stat::new()).item
}

fn non_empty_str(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.is_empty())
}

fn encode_korean(s: &str) -> Vec<u8> {
    let (bytes, _, had_errors) = encoding_rs::EUC_KR.encode(s);
    assert!(!had_errors, "cp949로 인코딩할 수 없음: {s:?}");
    bytes.into_owned()
}

fn encode_cp1252(s: &str) -> Vec<u8> {
    let (bytes, _, had_errors) = encoding_rs::WINDOWS_1252.encode(s);
    assert!(!had_errors, "cp1252로 인코딩할 수 없음: {s:?}");
    bytes.into_owned()
}

/// Python `str.encode("latin-1")`: 코드포인트가 곧 바이트다.
fn encode_latin1(s: &str) -> Vec<u8> {
    s.chars().map(|c| u8::try_from(u32::from(c)).expect("latin-1 범위 밖")).collect()
}

fn decode_cp949(raw: &[u8]) -> String {
    encoding_rs::EUC_KR.decode_without_bom_handling_and_without_replacement(raw).expect("cp949 디코딩 실패").into_owned()
}

fn decode_cp1252(raw: &[u8]) -> String {
    encoding_rs::WINDOWS_1252.decode_without_bom_handling_and_without_replacement(raw).expect("cp1252 디코딩 실패").into_owned()
}

/// Python `str.encode("utf-16")`: 리틀엔디언 BOM을 붙인다.
fn encode_utf16_bom(s: &str) -> Vec<u8> {
    std::iter::once(0xfeffu16).chain(s.encode_utf16()).flat_map(u16::to_le_bytes).collect()
}

fn encode_utf16_le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn encode_utf16_be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// EUC-KR로 인코딩한 바이트를 latin-1로 잘못 해석한 글(PDF 추출 mojibake).
fn mojibake(s: &str) -> String {
    encode_korean(s).into_iter().map(char::from).collect()
}

/// 단어마다 mojibake로 바꾼 손상 구간을 정상 글 사이에 흩뿌린다.
fn scatter(words: &[&str], tail: &str) -> String {
    words.iter().map(|w| format!(" {} {tail} ", mojibake(w))).collect()
}

fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
    let mut zw = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, data) in members {
        zw.start_file(*name, SimpleFileOptions::default()).unwrap();
        zw.write_all(data).unwrap();
    }
    zw.finish().unwrap();
}

const CONTAINER_XML: &str = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
const SIMPLE_OPF: &str = r#"<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;

/// mimetype 없이 container/opf/ch1만 가진 최소 EPUB.
fn write_simple_epub(path: &Path, body: &str) {
    write_zip(path, &[("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("content.opf", SIMPLE_OPF.as_bytes()), ("ch1.xhtml", format!("<html><body><p>{body}</p></body></html>").as_bytes())]);
}

/// 원본의 FakeBook/FakeDoc 대신 쓰는 실제 EPUB: 챕터마다 bytes를 그대로 넣는다.
fn write_epub_with_chapters(path: &Path, metadata: &str, chapters: &[&[u8]]) {
    let items: String = (0..chapters.len()).map(|i| format!(r#"<item id="c{i}" href="ch{i}.xhtml" media-type="application/xhtml+xml"/>"#)).collect();
    let refs: String = (0..chapters.len()).map(|i| format!(r#"<itemref idref="c{i}"/>"#)).collect();
    let opf = format!(
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">{metadata}</metadata><manifest>{items}</manifest><spine>{refs}</spine></package>"#
    );
    let names: Vec<String> = (0..chapters.len()).map(|i| format!("ch{i}.xhtml")).collect();
    let mut members: Vec<(&str, &[u8])> = vec![("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("content.opf", opf.as_bytes())];
    for (name, body) in names.iter().zip(chapters) {
        members.push((name, body));
    }
    write_zip(path, &members);
}

fn write_docx(path: &Path, paragraphs: &[&str]) {
    let body: String = paragraphs.iter().map(|p| format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>")).collect();
    let document = format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#);
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    write_zip(path, &[("_rels/.rels", rels.as_bytes()), ("word/document.xml", document.as_bytes())]);
}

/// pypdf.PdfWriter로 빈 쪽을 `pages`개 만든 것과 같은 최소 PDF.
fn minimal_pdf(pages: usize) -> Vec<u8> {
    let mut objs: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
    for _ in 0..pages {
        objs.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>".into());
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref_at = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in &offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", objs.len() + 1).bytes());
    out
}

/// 단계별로 준비한 답을 돌려주고 부른 순서를 기록하는 PDF 단계 실행기. 원본의 pypdfium2/pdfplumber/pikepdf 바꿔치기를 대신한다.
struct Fake {
    answers: Vec<(Stage, Result<(String, usize), StageError>)>,
    calls: Mutex<Vec<Stage>>,
}

impl Fake {
    fn new(answers: Vec<(Stage, Result<(&str, usize), StageError>)>) -> Self {
        Fake { answers: answers.into_iter().map(|(s, r)| (s, r.map(|(t, n)| (t.to_owned(), n)))).collect(), calls: Mutex::new(Vec::new()) }
    }

    fn calls(&self) -> Vec<Stage> {
        self.calls.lock().unwrap().clone()
    }
}

impl StageRunner for Fake {
    fn run(&self, stage: Stage, _path: &Path) -> Result<(String, usize), StageError> {
        self.calls.lock().unwrap().push(stage);
        self.answers.iter().find(|(s, _)| *s == stage).map(|(_, r)| r.clone()).unwrap_or(Err(StageError::Failed("없음".into())))
    }
}

/// 원본 `_mock_pypdfium2`: pdfium 단계가 `collect_pages`로 페이지 목록을 훑고, 어느 페이지를 읽었는지 기록한다.
struct CollectRunner {
    page_texts: Vec<String>,
    visited: Mutex<Vec<usize>>,
    calls: Mutex<Vec<Stage>>,
    /// pdfium 이외 단계의 답. `None`이면 실패.
    other: Option<(String, usize)>,
}

impl CollectRunner {
    fn new(page_texts: Vec<String>) -> Self {
        CollectRunner { page_texts, visited: Mutex::new(Vec::new()), calls: Mutex::new(Vec::new()), other: None }
    }
}

impl StageRunner for CollectRunner {
    fn run(&self, stage: Stage, _path: &Path) -> Result<(String, usize), StageError> {
        self.calls.lock().unwrap().push(stage);
        if stage != Stage::Pdfium {
            return self.other.clone().ok_or(StageError::Failed("없음".into()));
        }
        collect_pages(self.page_texts.len(), |i| {
            self.visited.lock().unwrap().push(i);
            Ok::<_, StageError>(self.page_texts[i].clone())
        })
    }
}

fn error_logs(logs: &[(&'static str, String)]) -> String {
    logs.iter().filter(|(level, _)| *level == "ERROR").map(|(_, m)| m.as_str()).collect::<Vec<_>>().join("\n")
}

/// `/proc/self/fd`에서 이름이 `suffix`로 끝나는 열린 파일.
fn open_fds_ending_with(suffix: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir("/proc/self/fd") else {
        return Vec::new();
    };
    entries.flatten().filter_map(|e| fs::read_link(e.path()).ok()).map(|p| p.to_string_lossy().into_owned()).filter(|t| t.ends_with(suffix)).collect()
}

/// 가짜 LibreOffice: `--headless --convert-to FMT --outdir DIR FILE` 인자를 받아 `payload`를 DIR/<stem>.txt로 쓴다. `payload`가 `None`이면 아무것도 쓰지 않고 종료 코드 1로 끝난다.
fn fake_libreoffice(dir: &Path, payload: Option<&str>) -> PathBuf {
    let script = dir.join("fake-soffice.sh");
    let body = match payload {
        Some(text) => {
            write(&dir.join("payload.txt"), text);
            "#!/bin/sh\nname=$(basename \"$6\")\nstem=\"${name%.*}\"\ncat \"$(dirname \"$0\")/payload.txt\" > \"$5/$stem.txt\"\n"
        }
        None => "#!/bin/sh\necho error >&2\nexit 1\n",
    };
    write(&script, body);
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
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

// ---- TestReadFromText -----------------------------------------------------------------------

mod test_read_from_text {
    use super::*;

    #[test]
    fn test_read_utf8_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("test.txt");
        write(&f, "Hello World\n안녕하세요\n테스트");
        let r = read_text(&f);
        // Python의 isinstance(summary, str)은 타입으로 보장된다.
        assert_eq!(r.line_count, 3);
        assert_eq!(r.page_count, 0);
        assert!(r.raw_content.contains("Hello"));
    }

    #[test]
    fn test_read_empty_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("empty.txt");
        write(&f, "");
        let r = read_text(&f);
        assert_eq!(r.summary, "");
        assert_eq!(r.line_count, 0);
    }

    #[test]
    fn test_read_bom_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("bom.txt");
        write(&f, b"\xef\xbb\xbfBOM content");
        assert!(!read_text(&f).summary.contains('\u{feff}'));
    }

    #[test]
    fn test_read_cp949_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("cp949.txt");
        write(&f, encode_korean("안녕하세요 한글 테스트입니다"));
        let r = read_text(&f);
        assert!(r.raw_content.contains("안녕하세요"));
        assert_eq!(r.line_count, 1);
    }

    #[test]
    fn test_read_euc_kr_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("euckr.txt");
        write(&f, encode_korean("완성형 한글 테스트"));
        let r = read_text(&f);
        assert!(r.raw_content.contains("한글"));
        assert_eq!(r.line_count, 1);
    }

    #[test]
    fn test_read_utf16_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("utf16.txt");
        write(&f, encode_utf16_bom("유니코드 16 비트 테스트\n두번째 줄"));
        let r = read_text(&f);
        assert!(r.raw_content.contains("유니코드"));
        assert_eq!(r.line_count, 2);
    }

    #[test]
    fn test_read_binary_as_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("binary.txt");
        write(&f, b"\x80\x81\x82\x83");
        assert_eq!(read_text(&f).summary, "");
    }

    #[test]
    fn test_read_text_when_chardet_raises() {
        // chardet가 실패해도 기본 인코딩 순서로 읽어낸다. 판별기가 아무것도 제안하지 못하는 상황으로 옮긴다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("novel.txt");
        write(&f, encode_korean("한글 본문입니다 가나다라\n"));
        let detect = |_: &[u8]| -> Option<Enc> { None };
        let hooks = TextHooks { detect: Some(&detect), ..Default::default() };
        let (r, _) = read_from_text_with(&f, &hooks);
        assert!(r.raw_content.contains("한글 본문입니다"));
    }

    #[test]
    fn test_read_text_skips_chardet_for_plain_utf8() {
        // BOM 없는 UTF-8은 chardet을 호출하지 않고 그대로 읽는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("novel.txt");
        write(&f, "한글 본문입니다 가나다라\n".as_bytes());
        let detect = |_: &[u8]| -> Option<Enc> { panic!("chardet 호출되면 안 된다") };
        let hooks = TextHooks { detect: Some(&detect), ..Default::default() };
        let (r, _) = read_from_text_with(&f, &hooks);
        assert_eq!(r.raw_content, "한글 본문입니다 가나다라\n");
    }

    #[test]
    fn test_read_text_utf8_bom_still_uses_chardet_and_strips_bom() {
        // UTF-8 BOM 파일은 판별 경로를 유지해 BOM이 본문에 남지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("novel.txt");
        write(&f, [b"\xef\xbb\xbf".as_slice(), "한글 본문입니다 가나다라\n".as_bytes()].concat());
        assert!(!read_text(&f).raw_content.starts_with('\u{feff}'));
    }

    #[test]
    fn test_read_text_falls_back_to_replace() {
        // 모든 엄격 디코딩이 실패하면 replace로라도 읽어 적재를 이어간다. 원본 파일은 읽기 전용 경로이므로 그대로여야 한다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("broken.txt");
        let raw_bytes: Vec<u8> = [[0x80u8, 0x81, 0xFE, 0xFF, 0x00, 0x9C, 0x90].repeat(40), b"abc\n".to_vec()].concat();
        write(&f, &raw_bytes);
        let _ = read_text(&f);
        assert_eq!(fs::read(&f).unwrap(), raw_bytes);
    }

    #[test]
    fn test_read_text_replace_skips_candidate_without_letters() {
        // replace 디코딩 결과에 한글·영숫자가 전혀 없으면 그 후보를 건너뛴다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("nonletters.txt");
        let raw_bytes = [0x80u8, 0x81, 0x82, 0x83].repeat(60);
        write(&f, &raw_bytes);
        let _ = read_text(&f);
        assert_eq!(fs::read(&f).unwrap(), raw_bytes);
    }

    #[test]
    fn test_read_text_replace_skips_korean_candidate_without_letters() {
        // chardet가 한국어 인코딩을 먼저 제안했을 때만 도달하는 분기라 그 상황을 만든다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("nonletters2.txt");
        let raw_bytes = [0x80u8, 0x81, 0x82, 0x83].repeat(60);
        write(&f, &raw_bytes);
        let detect = |_: &[u8]| Some(Enc::EucKr);
        let hooks = TextHooks { detect: Some(&detect), ..Default::default() };
        let _ = read_from_text_with(&f, &hooks);
        assert_eq!(fs::read(&f).unwrap(), raw_bytes);
    }

    #[test]
    fn test_read_text_rejects_utf16_cjk_garbage() {
        // 단일바이트 파일이 utf-16으로 디코딩돼 CJK 뭉치가 되면 채택하지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("notutf16.txt");
        let raw_bytes = [0x80u8, 0x81, 0x82, 0x83].repeat(60);
        write(&f, &raw_bytes);
        let r = read_text(&f);
        // utf-16 계열로 읽힌 CJK 뭉치(膀莂..., 肁芃...)가 그대로 들어오면 안 된다
        let head: String = r.raw_content.chars().take(40).collect();
        assert!(!r.raw_content.contains("膀莂") && !r.raw_content.contains("肁芃"), "{head}");
        assert_eq!(fs::read(&f).unwrap(), raw_bytes);
    }

    #[test]
    fn test_read_corrupted_utf16_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("corrupted_utf16.txt");
        let valid = encode_utf16_bom("안녕하세요 검맥 홍파 테스트");
        let corrupted = [&valid[..20], b"\xff\xff".as_slice(), &valid[20..]].concat();
        write(&f, corrupted);
        let r = read_text(&f);
        assert!(r.raw_content.contains("안녕하세요") || r.raw_content.contains("테스트"));
    }

    #[test]
    fn test_read_utf16_le_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("utf16le.txt");
        write(&f, encode_utf16_le("한글 UTF16LE 테스트"));
        let r = read_text(&f);
        assert!(!r.raw_content.is_empty());
        assert_eq!(r.line_count, 1);
    }

    #[test]
    fn test_read_utf16_be_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("utf16be.txt");
        write(&f, encode_utf16_be("한글 UTF16BE 테스트"));
        let r = read_text(&f);
        assert!(!r.raw_content.is_empty());
        assert_eq!(r.line_count, 1);
    }

    #[test]
    fn test_read_utf8_sig_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("utf8sig.txt");
        write(&f, [b"\xef\xbb\xbf".as_slice(), "UTF-8 BOM 포함 한글 테스트".as_bytes()].concat());
        let r = read_text(&f);
        assert!(r.raw_content.contains("한글"));
        assert_eq!(r.line_count, 1);
    }
}

// ---- TestReadFromEpub -----------------------------------------------------------------------

mod test_read_from_epub {
    use super::*;

    #[test]
    fn test_read_valid_epub() {
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("book.epub");
        write_simple_epub(&epub, "테스트 내용입니다");
        // read_from_epub_with_extracting_zip 직접 테스트
        let read = read_from_epub_with_extracting_zip(&epub);
        assert!(read.summary.contains("테스트"));
    }

    #[test]
    fn test_read_bad_zip_epub() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("bad.epub");
        write(&f, "not a zip");
        let read = read_from_epub_with_extracting_zip(&f);
        assert_eq!(read.summary, "");
        assert_eq!(read.line_count, 0);
    }

    #[test]
    fn test_read_epub_no_container() {
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("nocontainer.epub");
        write_zip(&epub, &[("META-INF/container.xml", b"<container></container>")]);
        let read = read_from_epub_with_extracting_zip(&epub);
        assert_eq!(read.summary, "");
    }

    #[test]
    fn test_read_epub_chapter_exception_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("chapter-error.epub");
        write_simple_epub(&epub, "ignored");
        // 챕터 해석기가 항상 실패하는 상황. 실패한 챕터는 건너뛰고 빈 결과가 나와야 한다.
        let parse = |_: &[u8]| -> Result<(String, usize), String> { Err("boom".to_owned()) };
        let (summary, line_count) = read_from_epub_with_extracting_zip_using(&epub, &parse);
        assert_eq!(summary, "");
        assert_eq!(line_count, 0);
    }
}

// ---- TestReadFromHtml / TestReadFromImage ---------------------------------------------------

mod test_read_from_html {
    use super::*;

    #[test]
    fn test_read_html() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("test.html");
        write(&f, "<html><body><p>Hello</p></body></html>");
        let (summary, _line_count, page_count) = read_from_html(&f).unwrap();
        assert!(summary.contains("Hello"));
        assert_eq!(page_count, 0);
    }
}

mod test_read_from_image {
    use super::*;

    #[test]
    fn test_read_image_returns_empty() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("test.jpg");
        write(&f, b"\xff\xd8");
        // Rust의 read_from_image는 경로를 받지 않는다. 이미지에는 본문·줄·쪽이 없다.
        let (summary, line_count, page_count) = read_from_image();
        assert_eq!(summary, "");
        assert_eq!(line_count, 0);
        assert_eq!(page_count, 0);
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

// ---- TestGetPathPrefix ----------------------------------------------------------------------

mod test_get_path_prefix {
    use super::*;

    #[test]
    fn test_book_path() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("book.txt");
        assert_eq!(cfg.path_prefix(&f), d.path());
    }

    #[test]
    fn test_comics_path() {
        let d = tempfile::tempdir().unwrap();
        let cfg = Config::new(tests_root().join("books"), d.path());
        let f = d.path().join("comic.cbz");
        assert_eq!(cfg.path_prefix(&f), d.path());
    }
}

// ---- TestReadFile ---------------------------------------------------------------------------

mod test_read_file {
    use super::*;

    #[test]
    fn test_read_txt_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let cat_dir = d.path().join("cat");
        fs::create_dir(&cat_dir).unwrap();
        let f = cat_dir.join("[Author] Title.txt");
        write(&f, "Hello world\nLine 2");

        let result = read_file(&cfg, &f, None, false, None, &InProcess, &Stat::new());
        let (_inode, doc) = result.item.expect("문서가 하나 나와야 한다");
        assert_eq!(doc["author"], "Author");
        assert_eq!(doc["title"], "Title");
        assert_eq!(doc["category"], "cat");
        assert_eq!(doc["file_type"], "txt");
        assert!(doc.get("created_time").is_some());
        assert!(non_empty_str(&doc["created_time"]));
        assert!(doc.get("created_time_source").is_some());
        assert!(non_empty_str(&doc["created_time_source"]));
    }

    #[test]
    fn test_read_file_indexes_epub_publisher() {
        // EPUB의 dc:publisher를 색인한다. 자동분류가 전집을 가르는 데 쓴다.
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let cat_dir = d.path().join("2_소설외국");
        fs::create_dir(&cat_dir).unwrap();
        let f = cat_dir.join("[작가] 제목.epub");
        write_zip(
            &f,
            &[
                ("content.opf", r#"<?xml version="1.0"?><package><metadata><dc:publisher>을유문화사</dc:publisher></metadata></package>"#.as_bytes()),
                ("ch1.xhtml", "<html><body><p>본문</p></body></html>".as_bytes()),
            ],
        );

        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        assert_eq!(doc["publisher"], "을유문화사");
    }

    #[test]
    fn test_read_file_leaves_publisher_empty_for_non_epub() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let cat_dir = d.path().join("2_소설외국");
        fs::create_dir(&cat_dir).unwrap();
        let f = cat_dir.join("[작가] 제목.txt");
        write(&f, "본문");

        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        // TXT에는 dc:publisher가 없다. 필드 자체는 늘 있어야 매핑이 흔들리지 않는다.
        assert_eq!(doc["publisher"], "");
    }

    #[test]
    fn test_read_file_accepts_explicit_path_prefix() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(&d.path().join("other"));
        let cat_dir = d.path().join("cat");
        fs::create_dir(&cat_dir).unwrap();
        let f = cat_dir.join("Title.txt");
        write(&f, "content");

        let (_, doc) = read_doc(&cfg, &f, false, Some(d.path())).unwrap();
        assert_eq!(doc["category"], "cat");
        assert_eq!(doc["file_path"], "cat/Title.txt");
    }

    #[test]
    fn test_read_file_no_author() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("NoAuthor.txt");
        write(&f, "content");

        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        assert_eq!(doc["author"], "");
        assert_eq!(doc["title"], "NoAuthor");
        assert_eq!(doc["category"], "_root");
    }

    #[test]
    fn test_read_nonexistent_file() {
        let d = tempfile::tempdir().unwrap();
        let result = read_doc(&default_config(), &d.path().join("nonexistent.txt"), false, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_read_unsupported_format() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("file.xyz");
        write(&f, "content");
        assert!(read_doc(&cfg, &f, false, None).is_none());
    }

    #[test]
    fn test_read_image_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("image.jpg");
        write(&f, b"\xff\xd8\xff\xe0");

        let (_, doc) = read_doc(&cfg, &f, false, None).expect("문서가 하나 나와야 한다");
        assert_eq!(doc["file_type"], "jpg");
        assert_eq!(doc["summary"], "");
    }

    #[test]
    fn test_read_file_skip_text() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("book.txt");
        write(&f, "content");

        let (_, doc) = read_doc(&cfg, &f, true, None).unwrap();
        assert_eq!(doc["summary"], "");
        assert!(doc.get("created_time").is_some());
        assert!(doc.get("created_time_source").is_some());
    }

    #[test]
    fn test_read_file_skip_text_unsupported() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("file.xyz");
        write(&f, "content");
        assert!(read_doc(&cfg, &f, true, None).is_none());
    }
}

// ---- TestPngPredictor -----------------------------------------------------------------------

mod test_png_predictor {
    use super::*;

    #[test]
    fn test_no_filter() {
        // filter byte = 0 (None), 2 columns of data
        let data = [0u8, 0x10, 0x20, 0, 0x30, 0x40];
        assert_eq!(apply_png_predictor(&data, 2), vec![0x10, 0x20, 0x30, 0x40]);
    }

    #[test]
    fn test_sub_filter() {
        // filter byte = 1 (Sub)
        let data = [1u8, 0x10, 0x05];
        assert_eq!(apply_png_predictor(&data, 2), vec![0x10, 0x15]);
    }

    #[test]
    fn test_up_filter() {
        // Two rows: first row no filter, second row Up filter
        let data = [0u8, 0x10, 0x20, 2, 0x01, 0x02];
        assert_eq!(apply_png_predictor(&data, 2), vec![0x10, 0x20, 0x11, 0x22]);
    }
}

// ---- TestFindXrefOffset ---------------------------------------------------------------------

mod test_find_xref_offset {
    use super::*;

    #[test]
    fn test_basic_xref() {
        let xref = b"xref\n0 3\n0000000000 65535 f \n0000000100 00000 n \n0000000200 00000 n \ntrailer\n";
        assert_eq!(find_xref_offset(xref, 1), Some(100));
        assert_eq!(find_xref_offset(xref, 2), Some(200));
        assert_eq!(find_xref_offset(xref, 5), None);
    }

    #[test]
    fn test_free_entry() {
        let xref = b"xref\n0 2\n0000000000 65535 f \n0000000100 00000 f \ntrailer\n";
        assert_eq!(find_xref_offset(xref, 1), None);
    }

    #[test]
    fn test_malformed_target_entry_returns_none() {
        let xref = b"xref\n0 2\n0000000000 65535 f \nnot-an-entry\ntrailer\n";
        assert_eq!(find_xref_offset(xref, 1), None);
    }

    #[test]
    fn test_missing_xref_keyword_returns_none() {
        assert_eq!(find_xref_offset(b"no xref here", 1), None);
    }
}

// ---- TestXrefStreamFindEntry ----------------------------------------------------------------

mod test_xref_stream_find_entry {
    use super::*;

    #[test]
    fn test_basic_entry() {
        // W=[1,2,1], index=[0,2]
        // Entry 0: type=1, offset=256, gen=0
        // Entry 1: type=1, offset=512, gen=0
        let data = [1u8, 1, 0, 0, 1, 2, 0, 0];
        assert_eq!(xref_stream_find_entry(&data, [1, 2, 1], &[0, 2], 1), Some((1, 512, 0)));
    }

    #[test]
    fn test_entry_not_found() {
        let data = [1u8, 1, 0, 0];
        assert_eq!(xref_stream_find_entry(&data, [1, 2, 1], &[0, 1], 5), None);
    }

    #[test]
    fn test_zero_entry_size() {
        assert_eq!(xref_stream_find_entry(b"", [0, 0, 0], &[0, 1], 0), None);
    }

    #[test]
    fn test_truncated_entry_returns_none() {
        assert_eq!(xref_stream_find_entry(&[1u8, 0, 10], [1, 2, 1], &[0, 1], 0), None);
    }
}
// ---- TestReadFromPdf ------------------------------------------------------------------------

/// 원본의 `shutil.which` / `subprocess.run` 바꿔치기를 대신하는 pdftotext 실행 환경. `PdftotextHooks`의 두 훅으로 바꿔 끼운다.
struct FakeEnv {
    which_result: Option<PathBuf>,
    stdout: Vec<u8>,
    which_calls: Mutex<Vec<String>>,
    commands: Mutex<Vec<Vec<String>>>,
    path_cache: std::sync::OnceLock<Option<PathBuf>>,
}

impl FakeEnv {
    fn new(which_result: Option<&str>, stdout: &str) -> Self {
        FakeEnv {
            which_result: which_result.map(PathBuf::from),
            stdout: stdout.as_bytes().to_vec(),
            which_calls: Mutex::new(Vec::new()),
            commands: Mutex::new(Vec::new()),
            path_cache: std::sync::OnceLock::new(),
        }
    }

    /// `PdftotextHooks.which`. 원본은 `shutil.which("pdftotext")`를 기록한다.
    fn which(&self) -> Option<PathBuf> {
        self.which_calls.lock().unwrap().push("pdftotext".to_owned());
        self.which_result.clone()
    }

    /// `PdftotextHooks.run`. Python subprocess와 같이 전체 인자 목록을 기록한다.
    fn run(&self, argv: &[String]) -> Option<(i32, Vec<u8>)> {
        self.commands.lock().unwrap().push(argv.to_vec());
        Some((0, self.stdout.clone()))
    }

    /// `pdf_text_by_pdftotext_with`를 이 환경으로 호출한다.
    fn text(&self, pdf: &Path) -> (String, usize) {
        let which = || self.which();
        let run = |argv: &[String]| self.run(argv);
        let hooks = PdftotextHooks { which: Some(&which), run: Some(&run), path_cache: &self.path_cache };
        pdf_text_by_pdftotext_with(pdf, &hooks)
    }
}

mod test_read_from_pdf {
    use super::*;

    #[test]
    fn test_read_valid_pdf() {
        // Create a minimal valid PDF
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("test.pdf");
        write(&pdf, minimal_pdf(1));
        let read = read_from_pdf(&pdf, &InProcess);
        assert_eq!(read.page_count, 1);
    }

    #[test]
    fn test_read_broken_pdf() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("broken.pdf");
        write(&pdf, b"%PDF-1.0\nbroken content");
        let read = read_from_pdf(&pdf, &InProcess);
        assert_eq!(read.page_count, 0);
    }

    #[test]
    fn test_read_pdf_pypdfium2_primary() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("pypdfium2_test.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");
        let fake = Fake::new(vec![(Stage::Pdfium, Ok(("pypdfium2 추출 텍스트", 1)))]);

        let read = read_from_pdf(&pdf, &fake);
        assert!(read.summary.contains("pypdfium2"), "{}", read.summary);
        assert_eq!(read.page_count, 1);
    }

    #[test]
    fn test_read_pdf_pdfplumber_fallback() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("pdfplumber_test.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");
        // pypdfium2 단계가 실패하면 다음 단계(pdfplumber)로 내려온다.
        let fake = Fake::new(vec![(Stage::Pdfium, Err(StageError::Failed("pypdfium2 fail".into()))), (Stage::Plumber, Ok(("pdfplumber 추출 텍스트", 1)))]);

        let read = read_from_pdf(&pdf, &fake);
        assert!(read.summary.contains("pdfplumber"), "{}", read.summary);
        assert_eq!(read.page_count, 1);
    }

    #[test]
    fn test_read_pdf_pdftotext_cli_helper() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("pdftotext_cli_test.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");
        let env = FakeEnv::new(Some("/usr/bin/pdftotext"), "pdftotext CLI 추출 텍스트");

        let (text, page_count) = env.text(&pdf);
        assert!(text.contains("pdftotext"), "{text}");
        assert_eq!(page_count, 0);
    }

    #[test]
    fn test_read_pdf_mojibake_falls_through() {
        // EUC-KR을 latin-1로 잘못 해석한 텍스트는 비어 있지 않아도 fallback을 타야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("mojibake.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");

        let broken = mojibake("맥(麥) 김남천 조은커뮤니티").repeat(8);
        let fake = Fake::new(vec![(Stage::Pdfium, Ok((broken.as_str(), 1))), (Stage::Plumber, Ok(("정상 추출된 한글 본문", 1)))]);

        let read = read_from_pdf(&pdf, &fake);
        assert!(read.summary.contains("정상 추출된 한글 본문"), "{}", read.summary);
        assert_eq!(read.page_count, 1);
    }
}

// ---- TestReencodeTextFileToUtf8 -------------------------------------------------------------

/// 읽은 쪽이 제안한 이름(`reason`의 첫 단어)으로 UTF-8 본문을 다시 인코딩한다.
fn encode_named(name: &str, text: &str) -> Vec<u8> {
    match name {
        "cp949" | "euc-kr" => encode_korean(text),
        "cp1252" => encode_cp1252(text),
        "utf-8" => text.as_bytes().to_vec(),
        other => panic!("테스트가 모르는 인코딩 이름: {other}"),
    }
}

/// 제자리 재인코딩을 기본 옵션으로 호출한다. 원본이 `Loader.path_prefix` 밖의 파일에 쓰는 상대 경로는 파일 이름이다.
fn reencode(f: &Path) -> doc_reader::text::Reencode {
    reencode_text_file_to_utf8(f, false, None, Path::new(f.file_name().unwrap()))
}

fn mtime_ns(path: &Path) -> (i64, i64) {
    let m = fs::metadata(path).unwrap();
    (m.mtime(), m.mtime_nsec())
}

mod test_reencode_text_file_to_utf8 {
    use super::*;

    #[test]
    fn test_cp949_file_is_reencoded() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("cp949.txt");
        let original = "한글 소설 본문입니다.\n둘째 줄\n";
        write(&f, encode_korean(original));

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), original);
    }

    #[test]
    fn test_euckr_file_is_reencoded() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("euckr.txt");
        let original = "동양고전 한문 번역 본문\n";
        write(&f, encode_korean(original));

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), original);
    }

    #[test]
    fn test_inode_is_preserved() {
        // ES 문서 _id가 inode이므로 재인코딩이 inode를 바꾸면 문서가 갈린다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("inode.txt");
        write(&f, encode_korean("한글 본문\n"));
        let before = fs::metadata(&f).unwrap().ino();

        let r = reencode(&f);
        assert!(r.changed);
        assert_eq!(fs::metadata(&f).unwrap().ino(), before);
    }

    #[test]
    fn test_already_utf8_is_untouched() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("utf8.txt");
        let raw = "한글 본문입니다\n".as_bytes().to_vec();
        write(&f, &raw);
        let mtime = mtime_ns(&f);

        let r = reencode(&f);
        assert!(!r.changed);
        assert_eq!(r.reason, "이미 UTF-8");
        assert_eq!(fs::read(&f).unwrap(), raw);
        assert_eq!(mtime_ns(&f), mtime);
    }

    #[test]
    fn test_undecodable_file_is_untouched() {
        // 어떤 인코딩으로도 무손실 디코딩이 안 되면 파일을 건드리지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("broken.txt");
        let raw = [0x80u8, 0x81, 0xFE, 0xFF, 0x00, 0x9C, 0x90].repeat(40);
        write(&f, &raw);

        let r = reencode(&f);
        assert!(!r.changed, "{}", r.reason);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_punctuation_only_file_is_untouched() {
        // 실제 글자가 전혀 없으면 인코딩 선택이 옳다고 볼 수 없어 건드리지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("nontext.txt");
        let raw = encode_korean(&"。、※★○【】".repeat(20));
        write(&f, &raw);

        let r = reencode(&f);
        assert!(!r.changed, "{}", r.reason);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_hanja_only_file_is_reencoded() {
        // 한문 고전처럼 한글이 없어도 한자 본문이면 재인코딩 대상이다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("hanja.txt");
        let original = "君子曰 學不可以已 靑取之於藍而靑於藍\n".repeat(10);
        write(&f, encode_korean(&original));

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), original);
    }

    #[test]
    fn test_windows1252_misdetection_is_rejected() {
        // chardet가 단일바이트 인코딩을 제안해도 채택하지 않는다.
        // 단일바이트 인코딩은 어떤 바이트열이든 디코딩되고 바이트 왕복도 무조건 통과하므로,
        // 후보로 허용하면 한글 파일에 mojibake(ÇÑ±Û ¼Ò¼³...)를 영구 기록하게 된다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("korean.txt");
        let original = "한글 소설 본문 chapter 12 입니다\n".repeat(20);
        write(&f, encode_korean(&original));

        let detect = |_: &[u8]| Some(Enc::Cp1252);
        let hooks = TextHooks { detect: Some(&detect), ..Default::default() };
        let r = reencode_text_file_to_utf8_with(&f, false, None, Path::new("korean.txt"), &hooks);
        assert!(r.changed, "{}", r.reason);
        // cp949로 올바르게 복원되어야 하고, mojibake가 저장되면 안 된다
        let saved = String::from_utf8(fs::read(&f).unwrap()).unwrap();
        assert_eq!(saved, original);
        assert!(!saved.contains("ÇÑ"));
    }

    #[test]
    fn test_empty_file_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("empty.txt");
        write(&f, b"");

        let r = reencode(&f);
        assert!(!r.changed);
        assert_eq!(r.reason, "빈 파일");
        assert_eq!(fs::read(&f).unwrap(), b"");
    }

    #[test]
    fn test_unreadable_path_is_reported() {
        // 읽을 수 없는 경로는 예외를 밖으로 내보내지 않고 사유로 돌려준다.
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("adir");
        fs::create_dir(&dir).unwrap();

        let r = reencode_text_file_to_utf8(&dir, false, None, Path::new("adir"));
        assert!(!r.changed);
        assert!(r.reason.starts_with("읽기 실패"), "{}", r.reason);
    }

    #[test]
    fn test_works_when_chardet_raises() {
        // chardet가 실패해도 기본 후보 순서로 정상 변환한다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("novel.txt");
        let raw = encode_korean("한글 본문입니다\n둘째 줄\n");
        write(&f, &raw);

        let detect = |_: &[u8]| -> Option<Enc> { None };
        let hooks = TextHooks { detect: Some(&detect), ..Default::default() };
        let r = reencode_text_file_to_utf8_with(&f, false, None, Path::new("novel.txt"), &hooks);
        assert!(r.changed, "{}", r.reason);
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), decode_cp949(&raw));
    }

    #[test]
    fn test_utf16_with_bom_is_converted() {
        // BOM이 있으면 NUL 비율과 무관하게 UTF-16 후보를 유지한다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("bom.txt");
        let original = "한글 본문입니다 가나다라마바사\n".repeat(20);
        let raw = encode_utf16_bom(&original);
        assert!(raw.starts_with(b"\xff\xfe") || raw.starts_with(b"\xfe\xff"));
        write(&f, &raw);

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), original);
    }

    #[test]
    fn test_replacement_char_in_source_is_rejected() {
        // 디코딩 결과에 U+FFFD가 있으면 그 후보를 쓰지 않는다. U+FFFD가 들어간 채로 저장하면 원문 글자를 영구히 잃는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("fffd.txt");
        let raw = encode_utf16_le(&"한글 본문\u{fffd}입니다\n".repeat(30));
        write(&f, &raw);

        let r = reencode(&f);
        if r.changed {
            let saved = String::from_utf8(fs::read(&f).unwrap()).unwrap();
            assert!(!saved.contains('\u{fffd}'), "{}", r.reason);
        } else {
            assert_eq!(fs::read(&f).unwrap(), raw);
        }
    }

    #[test]
    fn test_long_text_without_newline_is_rejected() {
        // 줄바꿈이 하나도 없는 긴 텍스트는 인코딩 오선택 신호이므로 변환하지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("noline.txt");
        let raw = encode_korean(&"가나다라마바사아자차카타파하".repeat(40));
        let text = decode_cp949(&raw);
        assert!(text.chars().count() >= 200 && !text.contains('\n') && !text.contains('\r'));
        write(&f, &raw);

        let r = reencode(&f);
        assert!(!r.changed, "{}", r.reason);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_high_latin1_ratio_is_rejected() {
        // 악센트 비율이 지나치게 높으면 단일바이트 오독으로 보고 변환하지 않는다.
        // 상위바이트가 고립돼 cp1252 후보로 넘어가더라도, 정상 서양어 문서라면
        // 악센트 비율이 이렇게 높을 수 없다. 실측 cp1252 영문 파일은 0.003 수준이다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("accents.txt");
        let raw = encode_cp1252(&"a \u{e0} b \u{e9} c \u{ee} d \u{f4} e \u{f9}\r\n".repeat(40));
        write(&f, &raw);
        let sample = decode_cp1252(&raw);
        let accents = sample.chars().filter(|c| ('\u{80}'..='\u{ff}').contains(c)).count();
        assert!(accents as f64 / sample.chars().count() as f64 > 0.1);

        let r = reencode(&f);
        assert!(!r.changed, "{}", r.reason);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_adjacent_accents_stay_lossless() {
        // 알려진 한계: 악센트가 연속이면 한글 2바이트 시퀀스와 구분되지 않는다.
        // 실제 서양어 문서의 악센트는 고립돼 나타나므로(실측 인접쌍 0.000~0.0024)
        // 현실적 위험은 낮다. 어느 쪽으로 판정되든 무손실 왕복은 지켜져야 한다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("adjacent.txt");
        let raw = encode_cp1252(&"ab \u{e0}\u{e9}\u{ee}\u{f4}\u{f9}\u{e0}\u{e9}\u{ee}\u{f4}\u{f9}\r\n".repeat(40));
        write(&f, &raw);

        let r = reencode(&f);
        if r.changed {
            let enc = r.reason.split(' ').next().unwrap();
            let saved = String::from_utf8(fs::read(&f).unwrap()).unwrap();
            assert_eq!(encode_named(enc, &saved), raw, "무손실 왕복이 깨짐");
        } else {
            assert_eq!(fs::read(&f).unwrap(), raw);
        }
    }

    #[test]
    fn test_file_untouched_when_backup_content_mismatches() {
        // 백업이 잘려 쓰이면 검증에서 걸러내고 원본을 건드리지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("novel.txt");
        let raw = encode_korean("한글 본문입니다\n둘째 줄\n");
        write(&f, &raw);
        let backup = d.path().join("backup");

        let detect = |_: &[u8]| -> Option<Enc> { None };
        // 백업 디렉터리에 쓰는 바이트를 5바이트로 잘라 쓴다(원본의 `Path.write_bytes` 바꿔치기).
        let truncating_write = |dest: &Path, data: &[u8]| -> io::Result<()> { fs::write(dest, &data[..data.len().min(5)]) };
        let hooks = TextHooks { detect: Some(&detect), write_backup: Some(&truncating_write), ..Default::default() };
        let r = reencode_text_file_to_utf8_with(&f, false, Some(&backup), Path::new("novel.txt"), &hooks);
        assert!(!r.changed);
        assert!(r.reason.contains("백업 검증 실패"), "{}", r.reason);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_original_preserved_when_write_fails() {
        // 덮어쓰기가 실패하면 원본이 남고 임시 파일도 남기지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("readonly.txt");
        let raw = encode_korean("한글 본문입니다\n둘째 줄\n");
        write(&f, &raw);
        fs::set_permissions(&f, fs::Permissions::from_mode(0o444)).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let r = reencode(&f);
            assert!(!r.changed);
            assert!(r.reason.contains("쓰기 실패"), "{}", r.reason);
            assert_eq!(fs::read(&f).unwrap(), raw);
            let left: Vec<PathBuf> = fs::read_dir(d.path()).unwrap().flatten().map(|e| e.path()).collect();
            assert_eq!(left, vec![f.clone()], "임시 파일이 남음");
        }));
        fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn test_post_write_mismatch_is_detected() {
        // 저장 후 다시 읽어 확인하는 단계가 실제로 불일치를 잡아낸다.
        // 유효한 UTF-8이라 디코딩은 성공하고 길이만 달라지므로 '저장 후 불일치' 분기를 탄다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("novel.txt");
        let raw = encode_korean("한글 본문입니다\n둘째 줄\n");
        write(&f, &raw);

        let detect = |_: &[u8]| -> Option<Enc> { None };
        // 1회차는 원본 읽기, 2회차(저장 후 재읽기)에서만 내용 뒤에 유효한 UTF-8을 덧붙인다(원본의 `Path.read_bytes` 바꿔치기).
        let calls = AtomicUsize::new(0);
        let flaky_read_bytes = |path: &Path| -> io::Result<Vec<u8>> {
            let mut data = fs::read(path)?;
            if calls.fetch_add(1, Ordering::SeqCst) >= 1 {
                data.extend_from_slice("추가".as_bytes());
            }
            Ok(data)
        };
        let hooks = TextHooks { detect: Some(&detect), read_bytes: Some(&flaky_read_bytes), ..Default::default() };
        let r = reencode_text_file_to_utf8_with(&f, false, None, Path::new("novel.txt"), &hooks);
        assert!(!r.changed);
        assert!(r.reason.contains("저장 후 불일치"), "{}", r.reason);
    }

    #[test]
    fn test_backup_is_written_before_overwrite() {
        // 백업 디렉터리를 지정하면 원본이 상대 경로 구조로 보존된다.
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("book");
        fs::create_dir_all(book.join("3_판타지")).unwrap();
        let f = book.join("3_판타지").join("novel.txt");
        let raw = encode_korean("한글 본문입니다\n둘째 줄\n");
        write(&f, &raw);
        let backup = d.path().join("backup");

        // 원본은 Loader.path_prefix=book으로 상대 경로를 계산한다. Rust는 상대 경로를 인자로 받는다.
        let r = reencode_text_file_to_utf8(&f, false, Some(&backup), f.strip_prefix(&book).unwrap());
        assert!(r.changed, "{}", r.reason);
        let saved_backup = backup.join("3_판타지").join("novel.txt");
        assert!(saved_backup.is_file(), "백업이 생성되지 않음");
        assert_eq!(fs::read(&saved_backup).unwrap(), raw, "백업 내용이 원본과 다름");
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), decode_cp949(&raw));
    }

    #[test]
    fn test_file_untouched_when_backup_fails() {
        // 백업을 쓸 수 없으면 파일을 변환하지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("novel.txt");
        let raw = encode_korean("한글 본문입니다\n");
        write(&f, &raw);
        // 파일을 백업 디렉터리로 지정 -> mkdir 실패
        let blocker = d.path().join("blocker");
        write(&blocker, "not a dir");

        let r = reencode_text_file_to_utf8(&f, false, Some(&blocker), Path::new("novel.txt"));
        assert!(!r.changed);
        assert!(r.reason.contains("백업"), "{}", r.reason);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_dry_run_writes_no_backup() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("novel.txt");
        let raw = encode_korean("한글 본문입니다\n");
        write(&f, &raw);
        let backup = d.path().join("backup");

        let r = reencode_text_file_to_utf8(&f, true, Some(&backup), Path::new("novel.txt"));
        assert!(r.changed && r.reason.contains("dry-run"), "{}", r.reason);
        assert!(!backup.exists(), "dry-run인데 백업이 생성됨");
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_western_text_uses_cp1252_not_cp949() {
        // 고립된 상위바이트는 서양어 단일바이트 인코딩이다.
        // 실제로 발생한 케이스: cp1252 영문 파일의 0x97(em-dash)이 cp949에서 뒤따르는
        // ASCII까지 2바이트 시퀀스로 삼켜 영문자를 파괴했다
        // ('Don Sabas—a man' -> 'Don Sabas뾞 man', 'a' 소실).
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("ohenry.txt");
        let body = "characteristic of Don Sabas\u{97}a man at once merry, learned\r\n";
        write(&f, encode_latin1(&body.repeat(40)));

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        assert!(r.reason.starts_with("cp1252"), "cp1252가 아닌 {}", r.reason);
        let saved = String::from_utf8(fs::read(&f).unwrap()).unwrap();
        assert!(saved.contains("Don Sabas\u{2014}a man at once"));
        assert!(!saved.contains('뾞'));
    }

    #[test]
    fn test_korean_with_few_high_bytes_still_uses_cp949() {
        // 영문 위주 한글 파일(상위바이트가 적어도 인접쌍)은 cp949/euc-kr을 유지한다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("mixed.txt");
        let body = "Taegu University Press chapter one and two\r\n";
        let raw = [encode_korean(&body.repeat(40)), encode_korean("머 리 말\r\n지금 한 세기가\r\n")].concat();
        write(&f, &raw);

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        let first = r.reason.split(' ').next().unwrap();
        assert!(["cp949", "euc-kr"].contains(&first), "한글 인코딩이 아닌 {}", r.reason);
        let saved = String::from_utf8(fs::read(&f).unwrap()).unwrap();
        assert!(saved.contains("머 리 말") && saved.contains("지금 한 세기가"));
    }

    #[test]
    fn test_korean_decodable_only_by_cp1252_is_untouched() {
        // 상위바이트가 인접쌍이면 cp1252로만 디코딩되더라도 변환하지 않는다.
        // 실제 케이스: 상위바이트 비율 0.78인 한글 파일이 cp949/euc-kr 왕복은 실패하고
        // cp1252로는 디코딩됐다. 가드가 없으면 mojibake로 변환된다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("korean_broken.txt");
        // 한글 cp949 바이트 사이에 cp949로는 무효인 상위바이트 쌍을 섞는다
        let raw = [encode_korean("가나다라마바사아자차\r\n").repeat(20), b"\xff\xfe".repeat(10)].concat();
        write(&f, &raw);
        let hi: Vec<usize> = raw.iter().enumerate().filter(|(_, b)| **b >= 0x80).map(|(i, _)| i).collect();
        let seen: std::collections::HashSet<usize> = hi.iter().copied().collect();
        let paired = hi.iter().filter(|&&i| (i > 0 && seen.contains(&(i - 1))) || seen.contains(&(i + 1))).count() as f64 / hi.len() as f64;
        assert!(paired >= HIGH_BYTE_PAIRED_MIN);

        let r = reencode(&f);
        assert!(!r.changed, "한글 파일이 변환됨: {}", r.reason);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_single_byte_text_is_not_read_as_utf16() {
        // 단일바이트 텍스트를 UTF-16으로 오독해 CJK 글자 뭉치로 바꾸면 안 된다.
        // 실제로 발생한 케이스: Windows-1252 영문 파일(0x97 em-dash)이 utf-8/cp949/euc-kr
        // 디코딩에 모두 실패하고, 길이가 짝수라 utf-16-be로 디코딩되며 왕복까지 통과했다.
        // NUL 바이트가 0%이므로 UTF-16일 수 없고, 줄바꿈도 하나도 남지 않는다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("cp1252.txt");
        let body = "J. R. R. Tolkien \u{97} The Lord Of The Rings. (4/4)\r\n";
        let raw = encode_latin1(&body.repeat(60));
        assert_eq!(raw.len() % 2, 0);
        write(&f, &raw);

        let detect = |_: &[u8]| Some(Enc::Utf16Be);
        let hooks = TextHooks { detect: Some(&detect), ..Default::default() };
        let r = reencode_text_file_to_utf8_with(&f, false, None, Path::new("cp1252.txt"), &hooks);
        // UTF-16으로 오독하면 안 되고, 서양어 단일바이트로 올바르게 읽혀야 한다
        assert!(!r.reason.contains("utf-16"), "UTF-16으로 오독됨: {}", r.reason);
        assert!(r.changed && r.reason.starts_with("cp1252"), "{}", r.reason);
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), decode_cp1252(&raw));
    }

    #[test]
    fn test_utf16_not_attempted_without_nul_bytes() {
        // NUL 바이트가 거의 없으면 UTF-16 후보를 시도하지 않는다.
        // cp949 등 다른 후보로 변환되는 것은 정상이며, UTF-16으로 오독되지만 않으면 된다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("notutf16.txt");
        let raw = [encode_latin1(&"abcd efgh 1234\r\n".repeat(50)), encode_korean("가나")].concat();
        write(&f, &raw);
        assert_eq!(raw.iter().filter(|&&b| b == 0).count(), 0);

        let r = reencode(&f);
        assert!(!r.reason.contains("utf-16"), "UTF-16으로 오독됨: {}", r.reason);
        if r.changed {
            assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), decode_cp949(&raw));
        }
    }

    #[test]
    fn test_utf16_wrong_endianness_is_not_written() {
        // BOM 없는 UTF-16은 LE/BE 양쪽이 왕복을 통과하므로 타당성 검사로 걸러야 한다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("utf16le.txt");
        let original = "한글 소설 본문입니다 가나다라마바사\n".repeat(20);
        write(&f, encode_utf16_le(&original));

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), original);
    }

    #[test]
    fn test_dry_run_does_not_write() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("dry.txt");
        let raw = encode_korean("한글 본문입니다\n");
        write(&f, &raw);

        let r = reencode_text_file_to_utf8(&f, true, None, Path::new("dry.txt"));
        assert!(r.changed);
        assert!(r.reason.contains("dry-run"), "{}", r.reason);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_no_temp_file_left_behind() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("tmpcheck.txt");
        write(&f, encode_korean("한글 본문\n"));

        reencode(&f);
        let left: Vec<PathBuf> = fs::read_dir(d.path()).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(left, vec![f]);
    }

    #[test]
    fn test_char_count_and_content_match_after_write() {
        // 저장 후 실제로 다시 읽어 글자수와 내용이 원본 디코딩 결과와 일치한다.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("count.txt");
        let original: String = (0..500).map(|i| format!("{i}행 한글 본문 내용입니다 가나다라마바사\n")).collect();
        write(&f, encode_korean(&original));

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        let saved = String::from_utf8(fs::read(&f).unwrap()).unwrap();
        assert_eq!(saved.chars().count(), original.chars().count());
        assert_eq!(saved, original);
    }

    #[test]
    fn test_shrinking_content_is_fully_truncated() {
        // UTF-8이 원본보다 짧아지는 경우(잔여 바이트가 남지 않아야 함).
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("shrink.txt");
        // UTF-16은 ASCII 위주 텍스트에서 UTF-8보다 크므로 재인코딩 시 파일이 줄어든다.
        let original = "ABCDEFG hello world 12345\n".repeat(200);
        write(&f, encode_utf16_bom(&original));
        assert!(fs::metadata(&f).unwrap().len() > original.len() as u64);

        let r = reencode(&f);
        assert!(r.changed, "{}", r.reason);
        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), original);
        assert_eq!(fs::metadata(&f).unwrap().len(), original.len() as u64);
    }

    #[test]
    fn test_reencode_disabled_by_default_in_read_file() {
        // --reencode 없이는 read_file이 파일을 건드리지 않는다.
        let cfg = default_config();
        assert!(!cfg.reencode);
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("plain.txt");
        let raw = encode_korean("한글 본문\n");
        write(&f, &raw);

        let _ = read_text(&f);
        assert_eq!(fs::read(&f).unwrap(), raw);
        let _ = read_doc(&config_with_book(d.path()), &f, false, None);
        assert_eq!(fs::read(&f).unwrap(), raw);
    }
}

// ---- TestPdfExtractionCost ------------------------------------------------------------------

mod test_pdf_extraction_cost {
    use super::*;

    #[test]
    fn test_scanned_pdf_skips_remaining_parsers() {
        // 텍스트 레이어가 없으면 pdfplumber 이후 단계를 아예 호출하지 않아야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("scanned.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");

        let runner = CollectRunner::new(vec![String::new(); 500]);
        let read = read_from_pdf(&pdf, &runner);
        let called = runner.calls.lock().unwrap().clone();
        assert_eq!(called, vec![Stage::Pdfium], "스캔본인데 후속 파서가 호출됨: {called:?}");
        assert_eq!(read.page_count, 500);
        assert!(read.summary.trim().is_empty());
    }

    #[test]
    fn test_scanned_pdf_stops_at_page_limit() {
        // 스캔본이라도 PDF_PAGE_LIMIT 페이지까지만 훑어야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("big_scanned.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");

        let runner = CollectRunner::new(vec![String::new(); 5000]);
        read_from_pdf(&pdf, &runner);
        let visited = runner.visited.lock().unwrap().len();
        assert_eq!(visited, PDF_PAGE_LIMIT, "{visited}페이지를 읽음");
    }

    #[test]
    fn test_stops_reading_once_text_size_reached() {
        // TEXT_SIZE를 채우면 남은 페이지를 읽지 않아야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("long.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");

        let runner = CollectRunner::new(vec!["가나다라마".repeat(300); 100]);
        let read = read_from_pdf(&pdf, &runner);
        let visited = runner.visited.lock().unwrap().len();
        assert!(visited <= 4, "{visited}페이지를 읽음");
        assert_eq!(read.summary.chars().count(), TEXT_SIZE);
        assert_eq!(read.page_count, 100);
    }

    #[test]
    fn test_pdftotext_is_page_limited() {
        // pdftotext에 -l 옵션이 붙어 전체 문서를 뽑지 않아야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("cli.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");

        let env = FakeEnv::new(Some("/usr/bin/pdftotext"), "본문");
        env.text(&pdf);
        let captured = env.commands.lock().unwrap().clone();
        assert!(!captured.is_empty(), "pdftotext가 호출되지 않음");
        let cmd = &captured[0];
        let at = cmd.iter().position(|a| a == "-l").expect("-l 옵션이 없음");
        assert_eq!(cmd[at + 1], PDF_PAGE_LIMIT.to_string());
    }

    #[test]
    fn test_pdftotext_path_is_cached() {
        // pdftotext binary 탐색은 파일마다 반복하지 않아야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("cli-cache.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");

        let env = FakeEnv::new(Some("/usr/bin/pdftotext"), "본문");
        env.text(&pdf);
        env.text(&pdf);

        assert_eq!(*env.which_calls.lock().unwrap(), vec!["pdftotext".to_owned()]);
    }

    #[test]
    fn test_damaged_text_falls_back_directly_to_pdfplumber() {
        // 손상 시 기본 fallback은 pdftotext process를 건너뛰고 pdfplumber로 간다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("damaged.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");

        let broken: String = ["즐기는", "들어서", "필요한", "함께"].iter().map(|w| format!(" {} 정상 본문 ", mojibake(w))).collect();
        // Poppler 결과가 손상되면 lopdf 기반 대체 단계로 내려간다.
        let fake = Fake::new(vec![(Stage::Pdfium, Ok((broken.as_str(), 1))), (Stage::Plumber, Ok(("정상 한글 본문입니다", 1)))]);

        let read = read_from_pdf(&pdf, &fake);
        assert_eq!(fake.calls(), vec![Stage::Pdfium, Stage::Plumber]);
        assert!(read.summary.contains("정상 한글 본문입니다"), "{}", read.summary);
    }

    #[test]
    fn test_pdf_stage_metrics_are_recorded() {
        // PDF parser stage별 소요 시간이 Stat에 기록되어야 한다.
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let pdf = d.path().join("metric.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");
        let fake = Fake::new(vec![(Stage::Pdfium, Ok(("정상 본문입니다", 1)))]);
        let stat = Stat::new();

        // Rust에서는 단계 기록을 read_file이 Stat에 옮긴다.
        read_file(&cfg, &pdf, None, false, None, &fake, &stat);

        let name = "_pdf_text_by_poppler";
        assert_eq!(stat.pdf_stage_count(name), 1);
        assert!(stat.pdf_stage_total_time(name) >= 0.0);
        assert_eq!(stat.pdf_stage_outcome_count(name, "ok"), 1);
    }
}
// ---- TestPdfMojibakeDetection ---------------------------------------------------------------

/// `_is_usable_pdf_text(text)`: 손상 구간 수는 함수가 직접 센다.
fn usable(text: &str) -> bool {
    is_usable_text(text, None)
}

mod test_pdf_mojibake_detection {
    use super::*;

    #[test]
    fn test_clean_korean_is_usable() {
        assert!(usable("맥(麥) 김남천 조은커뮤니티 소설 본문입니다"));
    }

    #[test]
    fn test_empty_is_not_usable() {
        assert!(!usable("   \n\t  "));
    }

    #[test]
    fn test_full_mojibake_is_not_usable() {
        let broken = mojibake("맥(麥) 김남천 조은커뮤니티");
        assert!(!usable(&broken));
    }

    #[test]
    fn test_partial_mojibake_is_not_usable() {
        // 정상 한글이 대부분이어도 손상 구간이 섞이면 재추출 대상이다.
        // 비율 기준(10%)으로 판정하면 4096자 중 155자 손상을 놓친다.
        // 실제 부분 손상은 정상 텍스트 사이에 손상 구간이 흩어져 나타난다.
        let broken: Vec<String> = ["즐기는", "들어서", "필요한", "함께"].iter().map(|w| mojibake(w)).collect();
        let tail: String = broken.iter().map(|w| format!(" {w} 정상 한글 본문이 이어집니다 ").repeat(3)).collect();
        let partial = format!("{}{tail}", "정상 한글 본문이 길게 이어집니다 ".repeat(10));
        assert!(count_mojibake_runs(&partial) >= PDF_MOJIBAKE_RUN_LIMIT);
        assert!(!usable(&partial));
    }

    #[test]
    fn test_latin_accents_are_usable() {
        // 포르투갈어 등 라틴 악센트 문자를 손상으로 오판하지 않아야 한다.
        let text = "Songbook Lô Borges Música de Minas Transcrições Cláudio Faria Belo Horizonte";
        assert!(usable(text));
    }

    #[test]
    fn test_falls_back_to_least_damaged() {
        // 모든 파서가 손상 결과를 주면 손상이 가장 적은 것을 남긴다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("all_damaged.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");

        let heavy = scatter(&["즐기는", "들어서", "필요한", "함께", "그리고", "사람은"], "정상 본문");
        let light = format!("{} LIGHT_DAMAGE_MARKER", scatter(&["즐기는", "들어서", "필요한"], "정상 본문"));
        let fake = Fake::new(vec![(Stage::Pdfium, Ok((heavy.as_str(), 7))), (Stage::Plumber, Ok((light.as_str(), 7)))]);

        let read = read_from_pdf(&pdf, &fake);
        assert!(read.summary.contains("LIGHT_DAMAGE_MARKER"), "{}", read.summary);
        assert_eq!(read.page_count, 7);
    }
}

// ---- TestPdfParserTimeout -------------------------------------------------------------------
//
// 원본에서 이 클래스의 앞 7개(time_limit 2개 + 워커 스레드 1개, run_with_hard_timeout 4개)는 utils.parser_timeout 단위 테스트라
// tests/rust/parser_timeout/ 에 대응 테스트가 있어 옮기지 않았다.

mod test_pdf_parser_timeout {
    use super::*;

    #[test]
    fn test_hung_stage_falls_through_to_next_parser() {
        // 한 파서가 멈춰도 타임아웃 후 다음 파서로 넘어가 결과를 얻어야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("hang.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");
        let fake = Fake::new(vec![(Stage::Pdfium, Err(StageError::Timeout)), (Stage::Plumber, Ok(("정상적으로 추출된 본문입니다", 5)))]);

        let read = read_from_pdf(&pdf, &fake);
        assert!(read.summary.contains("정상적으로"), "{}", read.summary);
        assert_eq!(read.page_count, 5);
    }

    #[test]
    fn test_timeout_is_reported_as_error() {
        // 타임아웃은 조용히 넘어가지 않고 에러로 표시되어야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("hang_all.pdf");
        write(&pdf, b"%PDF-1.4 mock pdf content");
        let fake = Fake::new(vec![(Stage::Pdfium, Err(StageError::Timeout)), (Stage::Plumber, Err(StageError::Timeout))]);

        let read = read_from_pdf(&pdf, &fake);
        assert!(read.summary.trim().is_empty());
        let messages = error_logs(&read.logs);
        assert!(messages.contains("타임아웃"), "{messages}");
        // 모든 단계가 타임아웃했음이 드러나야 한다
        assert!(messages.contains("모든 파서가 타임아웃"), "{messages}");
    }

    #[test]
    fn test_pdfplumber_leaves_no_open_handle_after_parse_failure() {
        // parser 실패 뒤 fd가 GC까지 남으면 안 된다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("leak.pdf");
        write(&pdf, b"%PDF-1.4 not really a pdf");
        assert!(open_fds_ending_with("leak.pdf").is_empty());

        // Python의 mock.patch 대신 실제 Rust plumber 단계가 손상 파일을 읽은 뒤를 확인한다.
        let _ = run_stage(Stage::Plumber, &pdf);

        let left = open_fds_ending_with("leak.pdf");
        assert!(left.is_empty(), "parser 실패 뒤 파일 핸들이 남아 있음: {left:?}");
    }

    #[test]
    fn test_pypdfium2_leaves_no_open_handle_when_load_fails() {
        // 문서 로드 실패 시 pdfium이 쥔 fd가 남으면 안 된다.
        // 경로를 넘기면 로드 실패한 fd가 닫히지 않고 GC로도 회수되지 않아,
        // 손상 PDF를 만날 때마다 fd가 1개씩 영구 누적됐다(실측).
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("broken_pypdfium.pdf");
        // 헤더는 PDF지만 본문이 없어 pdfium이 문서 로드에 실패한다
        write(&pdf, b"%PDF-1.4\n%%EOF\n");
        assert!(open_fds_ending_with("broken_pypdfium.pdf").is_empty());

        let _ = run_stage(Stage::Pdfium, &pdf);

        // Rust에는 GC가 없으므로 반환 직후 곧바로 닫혀 있어야 한다
        let left = open_fds_ending_with("broken_pypdfium.pdf");
        assert!(left.is_empty(), "문서 로드 실패 후 fd가 남아 있음: {left:?}");
    }
}

// ---- TestFileTypeDetection ------------------------------------------------------------------

mod test_file_type_detection {
    use super::*;

    #[test]
    fn test_detects_pdf_by_magic_bytes() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("mislabeled.txt");
        write(&f, b"%PDF-1.6\r%\xe2\xe3\xcf\xd3\r\n");
        assert_eq!(detect_file_type(&f, "txt").as_deref(), Some("pdf"));
    }

    #[test]
    fn test_detects_epub_inside_pdf_extension() {
        // 실제 사고 케이스: EPUB이 .pdf 확장자를 달고 있음.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("actually_epub.pdf");
        write_simple_epub(&f, "테스트 내용입니다");
        assert_eq!(detect_file_type(&f, "pdf").as_deref(), Some("epub"));
    }

    #[test]
    fn test_zip_based_extension_is_not_inspected_deeper() {
        // 정상 epub/cbz/docx는 내용물을 열지 않아야 한다 (비용 0 유지).
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("normal.epub");
        write_simple_epub(&f, "테스트 내용입니다");

        let called = Mutex::new(Vec::<&str>::new());
        let inspect = |_: &Path| -> Option<&'static str> {
            called.lock().unwrap().push("inspect");
            Some("epub")
        };
        let hooks = ReadHooks { inspect: Some(&inspect), ..Default::default() };
        let cfg = config_with_book(d.path());
        let out = read_file_with_hooks(&cfg, &f, None, false, None, &InProcess, &Stat::new(), &hooks);
        assert_eq!(out.item.as_ref().map(|(_, doc)| doc["file_type"].clone()), Some(Value::from("epub")));
        assert!(called.lock().unwrap().is_empty(), "zip 기반 확장자인데 내용물을 열어봄");
    }

    #[test]
    fn test_returns_none_for_unknown_magic() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("plain.txt");
        write(&f, "그냥 텍스트 파일입니다");
        assert_eq!(detect_file_type(&f, "txt"), None);
    }

    #[test]
    fn test_missing_file_returns_none() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(detect_file_type(&d.path().join("없는파일.pdf"), "pdf"), None);
    }

    #[test]
    fn test_detects_cbz_and_docx_containers() {
        let d = tempfile::tempdir().unwrap();
        let cbz = d.path().join("comic.pdf");
        write_zip(&cbz, &[("001.jpg", b"\xff\xd8\xff\xe0fake"), ("002.png", b"\x89PNGfake")]);
        assert_eq!(detect_file_type(&cbz, "pdf").as_deref(), Some("cbz"));

        let docx = d.path().join("doc.pdf");
        write_zip(&docx, &[("word/document.xml", b"<w:document/>")]);
        assert_eq!(detect_file_type(&docx, "pdf").as_deref(), Some("docx"));
    }

    #[test]
    fn test_read_file_uses_detected_type() {
        // .pdf 확장자의 EPUB이 EPUB 파서로 가고, ES에도 epub으로 기록되어야 한다.
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("[조지 오웰] 1984년.pdf");
        write_simple_epub(&f, "빅 브라더가 당신을 보고 있다");

        let (_, record) = read_doc(&cfg, &f, false, None).expect("적재되지 않음");
        assert_eq!(record["file_type"], "epub", "{}", record["file_type"]);
        assert!(record["summary"].as_str().unwrap().contains("빅 브라더"));
    }

    #[test]
    fn test_normal_pdf_is_unaffected() {
        // 정상 PDF는 감지 로직이 끼어들어도 그대로 pdf로 처리되어야 한다.
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("normal.pdf");
        write(&f, b"%PDF-1.4 mock pdf content");
        let fake = Fake::new(vec![(Stage::Pdfium, Ok(("정상 본문입니다", 10)))]);

        let (_, record) = read_file(&cfg, &f, None, false, None, &fake, &Stat::new()).item.unwrap();
        assert_eq!(record["file_type"], "pdf");
        assert!(record["summary"].as_str().unwrap().contains("정상"));
    }
}

// ---- TestPdfStructureRepair -----------------------------------------------------------------

/// 기본 파서(pdfium, plumber)가 모두 문서를 열지 못한 상태(구조 손상)의 답.
fn all_stages_fail() -> Vec<(Stage, Result<(&'static str, usize), StageError>)> {
    vec![(Stage::Pdfium, Ok(("", 0))), (Stage::Plumber, Ok(("", 0)))]
}

mod test_pdf_structure_repair {
    use super::*;

    #[test]
    fn test_repair_recovers_text_when_all_parsers_fail() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("broken_xref.pdf");
        write(&pdf, b"%PDF-1.6 mock");
        let mut answers = all_stages_fail();
        answers.push((Stage::Repair, Ok(("복구된 본문입니다. 충분히 긴 한국어 텍스트.", 470))));
        let fake = Fake::new(answers);

        let read = read_from_pdf(&pdf, &fake);
        assert!(read.summary.contains("복구된"), "{}", read.summary);
        assert_eq!(read.page_count, 470);
    }

    #[test]
    fn test_repair_is_skipped_when_text_already_extracted() {
        // mojibake는 구조 손상이 아니므로 복구 비용을 쓰면 안 된다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("mojibake.pdf");
        write(&pdf, b"%PDF-1.6 mock");
        let damaged: String = ["즐기는", "들어서", "필요한", "함께"].iter().map(|w| format!(" {} 본문 ", mojibake(w))).collect();
        let fake = Fake::new(vec![(Stage::Pdfium, Ok((damaged.as_str(), 12))), (Stage::Plumber, Ok(("", 0))), (Stage::Repair, Ok(("", 0)))]);

        read_from_pdf(&pdf, &fake);
        assert!(!fake.calls().contains(&Stage::Repair), "텍스트가 이미 나왔는데 복구를 시도함");
    }

    #[test]
    fn test_repair_is_skipped_for_scanned_pdf() {
        // 스캔본(문서는 열리는데 텍스트 레이어 없음)은 복구 대상이 아니다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("scanned.pdf");
        write(&pdf, b"%PDF-1.6 mock");
        let fake = Fake::new(vec![(Stage::Pdfium, Ok(("", 300))), (Stage::Repair, Ok(("", 0)))]);

        let read = read_from_pdf(&pdf, &fake);
        assert!(!fake.calls().contains(&Stage::Repair), "스캔본에 복구를 시도함");
        assert_eq!(read.page_count, 300);
    }

    #[test]
    fn test_unrecoverable_file_is_reported_as_error() {
        // 객체가 소실된 파일은 복구 실패가 에러로 드러나야 한다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("unrecoverable.pdf");
        write(&pdf, b"%PDF-1.6 mock");
        let mut answers = all_stages_fail();
        answers.push((Stage::Repair, Err(StageError::Failed("unable to find /Root dictionary".into()))));
        let fake = Fake::new(answers);

        let read = read_from_pdf(&pdf, &fake);
        assert!(read.summary.trim().is_empty());
        let messages = error_logs(&read.logs);
        assert!(messages.contains("구조 복구 실패"), "{messages}");
    }

    #[test]
    fn test_repair_timeout_is_reported() {
        // 복구 단계도 무한히 돌면 안 된다.
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("hang_repair.pdf");
        write(&pdf, b"%PDF-1.6 mock");
        let mut answers = all_stages_fail();
        answers.push((Stage::Repair, Err(StageError::Timeout)));
        let fake = Fake::new(answers);

        let read = read_from_pdf(&pdf, &fake);
        let messages = error_logs(&read.logs);
        assert!(messages.contains("구조 복구 타임아웃"), "{messages}");
    }

    #[test]
    fn test_repair_reads_through_owned_handle() {
        // 복구 경로도 fd를 남기면 안 된다 (손상 파일이라 실패해도 마찬가지).
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("repair_fd.pdf");
        write(&pdf, b"%PDF-1.6\n%%EOF\n");
        assert!(open_fds_ending_with("repair_fd.pdf").is_empty());

        let _ = run_stage(Stage::Repair, &pdf);

        let left = open_fds_ending_with("repair_fd.pdf");
        assert!(left.is_empty(), "복구 시도 후 fd가 남아 있음: {left:?}");
    }
}

// ---- TestIsbnPdfTimeout ---------------------------------------------------------------------
//
// 원본 `test_isbn_pdf_extraction_has_time_limit` 1개는 utils.isbn 단위 테스트(PDF_ISBN_TIMEOUT, pypdf.PdfReader 바꿔치기)라
// tests/rust/isbn/ 에 대응 범위가 있어 옮기지 않았다.
// ---- TestReadFromRtf ------------------------------------------------------------------------

/// `Loader.read_from_rtf(path)`: 읽거나 해석하지 못하면 빈 결과다. 원본의 예외 처리와 같다.
fn read_rtf(path: &Path) -> (String, usize, usize) {
    match fs::read(path).map_err(|e| e.to_string()).and_then(|b| String::from_utf8(b).map_err(|e| e.to_string())).and_then(|c| read_from_rtf_text(&c)) {
        Ok((summary, line_count)) => (summary, line_count, 0),
        Err(_) => (String::new(), 0, 0),
    }
}

mod test_read_from_rtf {
    use super::*;

    #[test]
    fn test_read_valid_rtf() {
        let d = tempfile::tempdir().unwrap();
        let rtf = d.path().join("test.rtf");
        write(&rtf, b"{\\rtf1 Hello World}");
        let (summary, _line_count, _page_count) = read_rtf(&rtf);
        assert!(summary.contains("Hello") || summary.is_empty()); // depends on striprtf behavior
    }

    #[test]
    fn test_read_broken_rtf() {
        let d = tempfile::tempdir().unwrap();
        let rtf = d.path().join("broken.rtf");
        write(&rtf, b"\x80\x81\x82not rtf");
        // Python의 isinstance(summary, str)은 타입으로 보장된다. 패닉 없이 끝나는지만 본다.
        let (_summary, _line_count, _page_count) = read_rtf(&rtf);
    }
}

// ---- TestReadFromDocx -----------------------------------------------------------------------

mod test_read_from_docx {
    use super::*;

    #[test]
    fn test_read_valid_docx() {
        let d = tempfile::tempdir().unwrap();
        let docx_path = d.path().join("test.docx");
        write_docx(&docx_path, &["Hello Docx World", "Line 2"]);
        let (summary, line_count) = read_from_docx(&docx_path).unwrap();
        assert!(summary.contains("Hello"), "{summary}");
        assert!(line_count >= 2, "{line_count}");
    }
}

// ---- TestCountEpubLines ---------------------------------------------------------------------
//
// 원본은 BeautifulSoup 트리를 직접 고치는 `_reference_count_epub_lines`와 대조한다. Rust에는 그 기준 구현이 없어,
// 같은 입력으로 기준 구현(bs4+lxml)을 한 번 돌려 얻은 값을 기대값으로 고정했다.

mod test_count_epub_lines {
    use super::*;

    /// (이름, HTML, 기준 구현이 낸 줄 수). 이름순이다.
    const HTML_CASES: [(&str, &str, usize); 15] = [
        ("br_splits_line", "<html><body><p>a<br/>b<br/><br/>c</p></body></html>", 3),
        ("cdata_like", "<html><body><p>a</p><![CDATA[x\ny]]></body></html>", 1),
        ("comment_ignored", "<html><body><p>a<!-- c\nd --></p><!-- e --></body></html>", 1),
        ("empty_body", "<html><body></body></html>", 0),
        ("inline_does_not_split", "<html><body><p>a<span>b</span><em>c</em> d</p></body></html>", 1),
        ("nested_pre_inline", "<html><body><pre>l1\n<b>l2\nl3</b></pre></body></html>", 3),
        ("no_body_fragment", "<p>a</p><p>b</p>", 2),
        ("paragraphs", "<html><body><p>a</p><p>b</p><p>  </p></body></html>", 2),
        ("pre_keeps_newlines", "<html><body><pre>x\n\ny\n  z</pre><p>after</p></body></html>", 4),
        ("pre_nested_block", "<html><body><div><pre>a\nb</pre><div>c</div></div></body></html>", 3),
        ("ruby_annotation", "<html><body><p><ruby>漢<rt>han</rt><rp>(</rp></ruby>字</p></body></html>", 1),
        ("skipped_tags", "<html><head><title>t</title></head><body><script>var a=1;\nb()</script><style>p{}\n</style><p>x</p><template><p>y</p></template></body></html>", 1),
        ("table_list", "<html><body><table><tr><td>a</td><td>b</td></tr></table><ul><li>1</li><li>2</li></ul></body></html>", 4),
        ("text_directly_in_body", "<html><body>a\nb<div>c</div>d</body></html>", 3),
        ("whitespace_collapse", "<html><body><p>a\n\n  b\t\tc  d</p></body></html>", 1),
    ];

    #[test]
    fn test_matches_reference_implementation() {
        for (name, html, expected) in HTML_CASES {
            assert_eq!(count_lines(html), expected, "케이스 {name}: {html:?}");
        }
    }

    #[test]
    fn test_known_counts() {
        assert_eq!(count_lines("<body><p>a</p><p>b</p><p> </p></body>"), 2);
        assert_eq!(count_lines("<body><p>a<br/>b</p></body>"), 2);
        assert_eq!(count_lines("<body><pre>x\n\ny</pre></body>"), 2);
        assert_eq!(count_lines("<body><script>a\nb</script></body>"), 0);
    }

    #[test]
    fn test_does_not_mutate_soup() {
        // 입력을 `&str`로 받으므로 고쳐 쓸 수 없다. 같은 입력을 두 번 세어도 같은 값이 나오고 입력이 그대로인지 본다.
        let html = String::from("<html><head><title>t</title></head><body><script>s</script><p>a  b</p></body></html>");
        let before = html.clone();
        let first = count_lines(&html);
        let second = count_lines(&html);
        assert_eq!(html, before);
        assert_eq!(first, second);
    }
}

// ---- TestReadFromEpubFull -------------------------------------------------------------------

mod test_read_from_epub_full {
    use super::*;

    /// 챕터 bytes 목록으로 실제 EPUB을 만들어 읽는다. 원본의 `_read_epub_with_chapters`(ebooklib 바꿔치기)를 대신한다.
    fn read_epub_with_chapters(chapters: &[&[u8]]) -> (String, usize) {
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("dummy.epub");
        write_epub_with_chapters(&epub, "", chapters);
        let read = read_from_epub(&epub);
        (read.summary, read.line_count)
    }

    #[test]
    fn test_read_epub_ebooklib_success_path() {
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("dummy.epub");
        write_epub_with_chapters(&epub, "<dc:title>테스트 제목</dc:title><dc:creator>테스트 저자</dc:creator>", &["<html><body><p>본문 줄1</p><p>줄2</p></body></html>".as_bytes()]);

        // 원본은 (summary, line_count, page_count)를 받고 page_count == 0을 확인한다. Rust의 EpubRead에는 쪽 수가 없다.
        let read = read_from_epub(&epub);
        assert!(read.summary.contains("테스트 제목"), "{}", read.summary);
        assert!(read.summary.contains("테스트 저자"), "{}", read.summary);
        assert!(read.line_count >= 1);
    }

    #[test]
    fn test_read_epub_short_utf8_chapter_bytes_not_misdetected() {
        // 짧은 UTF-8 bytes 챕터를 chardet이 Windows-1252로 오판하면 '부'가 '€'로 깨진다.
        let body = "<body xml:lang=\"ko\">\n  <h1>1부</h1>\n\n  <div class=\"img_center\"><img src=\"a.png\"/></div>\n</body>".as_bytes();
        let (summary, line_count) = read_epub_with_chapters(&[body]);
        assert!(summary.contains("1부"), "{summary}");
        assert!(!summary.contains('€'), "{summary}");
        assert_eq!(line_count, 1);
    }

    #[test]
    fn test_read_epub_nbsp_only_paragraph_is_not_counted_as_mojibake_line() {
        // NBSP만 있는 문단이 'Â'로 오판되어 행으로 세어지면 안 된다.
        let body = "<body class=\"x\">\n<p class=\"y\"><img src=\"a.png\"/></p>\n<p>\u{a0}</p>\n</body>".as_bytes();
        let (_, line_count) = read_epub_with_chapters(&[body]);
        assert_eq!(line_count, 0);
    }

    #[test]
    fn test_read_epub_utf8_bom_chapter_bytes() {
        // BOM이 있는 bytes 챕터도 BOM 글자가 본문에 남지 않는다.
        let body = [b"\xef\xbb\xbf".as_slice(), "<body><p>본문</p></body>".as_bytes()].concat();
        let (summary, line_count) = read_epub_with_chapters(&[body.as_slice()]);
        assert_eq!(summary.trim(), "본문");
        assert_eq!(line_count, 1);
    }

    #[test]
    fn test_read_epub_non_utf8_chapter_bytes_still_parsed() {
        // UTF-8이 아닌 bytes 챕터는 디코딩에 실패해도 bs4의 인코딩 감지로 읽는다.
        let body = encode_korean("<body><p>한글 본문 줄입니다 가나다라</p></body>");
        let (summary, line_count) = read_epub_with_chapters(&[body.as_slice()]);
        assert!(summary.contains("한글 본문"), "{summary}");
        assert_eq!(line_count, 1);
    }

    #[test]
    fn test_read_epub_with_ebooklib() {
        // read_from_epub using ebooklib or fallback
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("test.epub");
        write_zip(
            &epub,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
                ("content.opf", SIMPLE_OPF.as_bytes()),
                ("ch1.xhtml", "<html><body><p>테스트 내용</p></body></html>".as_bytes()),
            ],
        );
        // Python의 isinstance(summary, str)은 타입으로 보장된다. 패닉 없이 읽히는지만 본다.
        let _read = read_from_epub(&epub);
    }

    #[test]
    fn test_read_epub_exception_zip_fallback() {
        // Lines 161-176: epub read_epub fails, fallback to zip extraction
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("bad_epub.epub");
        // No mimetype -> ebooklib will fail, triggering zip fallback
        write_zip(&epub, &[("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("content.opf", SIMPLE_OPF.as_bytes()), ("ch1.xhtml", b"<html><body>Fallback content</body></html>")]);
        let _read = read_from_epub(&epub);
    }

    #[test]
    fn test_read_epub_both_primary_and_fallback_fail() {
        // 원본은 두 경로를 모두 예외로 바꿔치기한다. Rust는 zip이 아닌 파일로 두 경로를 모두 실패시킨다.
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("broken.epub");
        write(&epub, b"not a zip at all");
        let read = read_from_epub(&epub);
        assert_eq!(read.summary, "");
        assert_eq!(read.line_count, 0);
    }
}

// ---- TestReadFromDocHwp ---------------------------------------------------------------------

mod test_read_from_doc_hwp {
    use super::*;

    #[test]
    fn test_read_from_doc() {
        let d = tempfile::tempdir().unwrap();
        let doc_file = d.path().join("test.doc");
        write(&doc_file, "doc content");
        // 원본의 `_convert_with_libreoffice` 바꿔치기 대신 가짜 LibreOffice 실행 파일을 쓴다.
        let binary = fake_libreoffice(d.path(), Some("converted text\nline2"));
        let (summary, line_count, _errors) = read_from_doc(&doc_file, &binary);
        assert!(summary.contains("converted"), "{summary}");
        assert_eq!(line_count, 2);
    }

    #[test]
    fn test_read_from_doc_exception() {
        let d = tempfile::tempdir().unwrap();
        let doc_file = d.path().join("test.doc");
        write(&doc_file, "doc content");
        // 변환 실패(종료 코드 1, 출력 파일 없음)
        let binary = fake_libreoffice(d.path(), None);
        let (summary, _line_count, _errors) = read_from_doc(&doc_file, &binary);
        assert_eq!(summary, "");
    }

    #[test]
    fn test_read_from_hwp() {
        let d = tempfile::tempdir().unwrap();
        let hwp_file = d.path().join("test.hwp");
        write(&hwp_file, "hwp content");
        let binary = fake_libreoffice(d.path(), Some("hwp text"));
        let (summary, _line_count, _errors) = read_from_hwp(&hwp_file, &binary);
        assert!(summary.contains("hwp"), "{summary}");
    }
}

// ---- TestConvertWithLibreoffice -------------------------------------------------------------

mod test_convert_with_libreoffice {
    use super::*;

    #[test]
    fn test_successful_conversion() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("test.doc");
        write(&src, "doc");
        // 원본은 TemporaryDirectory와 subprocess.run을 바꿔치기한다. 가짜 실행 파일이 <outdir>/test.txt를 쓴다.
        let binary = fake_libreoffice(d.path(), Some("converted"));
        let mut errors = Vec::new();
        let result = convert_with_libreoffice(&src, "txt:Text", &binary, &mut errors);
        assert_eq!(result, "converted");
    }

    #[test]
    fn test_no_output() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("test.doc");
        write(&src, "doc");
        // 종료 코드 1, stderr "error", 출력 파일 없음
        let binary = fake_libreoffice(d.path(), None);
        let mut errors = Vec::new();
        let result = convert_with_libreoffice(&src, "txt:Text", &binary, &mut errors);
        assert_eq!(result, "");
    }
}

// ---- TestReadFileBranches -------------------------------------------------------------------

mod test_read_file_branches {
    use super::*;

    #[test]
    fn test_read_epub_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let epub = d.path().join("[Author] Book.epub");
        write_zip(
            &epub,
            &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("content.opf", SIMPLE_OPF.as_bytes()), ("ch1.xhtml", b"<html><body>Content</body></html>")],
        );
        if let Some((_, doc)) = read_doc(&cfg, &epub, false, None) {
            assert_eq!(doc["file_type"], "epub");
            assert_eq!(doc["author"], "Author");
        }
    }

    #[test]
    fn test_read_html_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("page.html");
        write(&f, "<html><body>Hello HTML</body></html>");
        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        assert_eq!(doc["file_type"], "html");
    }

    #[test]
    fn test_read_cbz_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("comic.cbz");
        write(&f, b"cbz data");
        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        assert_eq!(doc["file_type"], "cbz");
    }

    #[test]
    fn test_read_rtf_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("doc.rtf");
        write(&f, b"{\\rtf1 Hello}");
        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        assert_eq!(doc["file_type"], "rtf");
    }

    #[test]
    fn test_read_pdf_skip_text() {
        // Lines 703-713: skip_text with PDF (fast page count)
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let pdf = d.path().join("book.pdf");
        write(&pdf, b"%PDF");
        let fast_count = |_: &Path| Some(5usize);
        let hooks = ReadHooks { fast_page_count: Some(&fast_count), ..Default::default() };
        let out = read_file_with_hooks(&cfg, &pdf, None, true, None, &InProcess, &Stat::new(), &hooks);
        let (_, doc) = out.item.unwrap();
        assert_eq!(doc["page_count"], 5);
        assert_eq!(doc["summary"], "");
    }

    #[test]
    fn test_read_pdf_skip_text_fallback() {
        // Lines 705-712: skip_text PDF fast count fails, pypdf fallback
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let pdf = d.path().join("book.pdf");
        write(&pdf, minimal_pdf(1));
        let fast_count = |_: &Path| -> Option<usize> { None };
        let hooks = ReadHooks { fast_page_count: Some(&fast_count), ..Default::default() };
        let out = read_file_with_hooks(&cfg, &pdf, None, true, None, &InProcess, &Stat::new(), &hooks);
        let (_, doc) = out.item.unwrap();
        assert_eq!(doc["page_count"], 1);
    }

    #[test]
    fn test_read_doc_file() {
        let d = tempfile::tempdir().unwrap();
        let mut cfg = config_with_book(d.path());
        cfg.libreoffice = fake_libreoffice(d.path(), Some("text"));
        let f = d.path().join("file.doc");
        write(&f, "doc");
        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        assert_eq!(doc["file_type"], "doc");
    }

    #[test]
    fn test_read_hwp_file() {
        let d = tempfile::tempdir().unwrap();
        let mut cfg = config_with_book(d.path());
        cfg.libreoffice = fake_libreoffice(d.path(), Some("text"));
        let f = d.path().join("file.hwp");
        write(&f, "hwp");
        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        assert_eq!(doc["file_type"], "hwp");
    }

    #[test]
    fn test_read_docx_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let f = d.path().join("file.docx");
        write_docx(&f, &["Hello"]);
        let (_, doc) = read_doc(&cfg, &f, false, None).unwrap();
        assert_eq!(doc["file_type"], "docx");
    }

    #[test]
    fn test_read_pdf_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config_with_book(d.path());
        let pdf = d.path().join("file.pdf");
        write(&pdf, minimal_pdf(1));
        let (_, doc) = read_doc(&cfg, &pdf, false, None).unwrap();
        assert_eq!(doc["file_type"], "pdf");
        assert_eq!(doc["page_count"], 1);
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

mod test_get_stat {
    use super::*;

    #[test]
    fn test_get_stat() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("test.txt");
        write(&f, "hello");
        // Loader.get_stat은 os.stat 래퍼다. Rust는 fs::metadata를 직접 쓴다.
        let st = fs::metadata(&f).unwrap();
        assert_eq!(st.len(), 5);
    }
}

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

// ---- TestFindLibreoffice --------------------------------------------------------------------

mod test_find_libreoffice {
    use super::*;

    #[test]
    fn test_find_via_which() {
        let which = |cmd: &str| {
            if cmd == "soffice" { Some(PathBuf::from("/usr/bin/soffice")) } else { None }
        };
        let exists = |p: &Path| p.exists();
        assert_eq!(find_libreoffice_with(&which, &exists), PathBuf::from("/usr/bin/soffice"));
    }

    #[test]
    fn test_find_mac_path() {
        let which = |_: &str| -> Option<PathBuf> { None };
        let mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice";
        let exists = |p: &Path| p == Path::new(mac_path);
        assert_eq!(find_libreoffice_with(&which, &exists), PathBuf::from(mac_path));
    }

    #[test]
    fn test_fallback() {
        let which = |_: &str| -> Option<PathBuf> { None };
        let exists = |_: &Path| false;
        assert_eq!(find_libreoffice_with(&which, &exists), PathBuf::from("libreoffice"));
    }
}

// ---- TestFastPdfPageCount -------------------------------------------------------------------

type XrefStream = (Vec<u8>, [usize; 3], Vec<i64>, Option<i64>);

/// 원본의 `_parse_one_xref_stream` / `_xref_stream_find_entry` / `_read_from_obj_stream` 바꿔치기. `FastCountHooks`의 훅으로 끼운다.
struct FakeXref {
    parsed: Option<(Vec<u8>, [usize; 3], Vec<i64>, Option<i64>)>,
    /// 객체 번호별 (type, offset, gen)
    entries: Vec<(i64, (i64, i64, i64))>,
    /// 객체 번호와 무관하게 항상 돌려주는 항목
    any_entry: Option<(i64, i64, i64)>,
    obj_stream: Option<Vec<u8>>,
}

impl FakeXref {
    fn parse_one_xref_stream(&self, _xref_offset: u64) -> Option<XrefStream> {
        self.parsed.clone()
    }

    fn xref_stream_find_entry(&self, _data: &[u8], _w: [usize; 3], _index_ranges: &[i64], obj_num: i64) -> Option<(i64, i64, i64)> {
        self.any_entry.or_else(|| self.entries.iter().find(|(n, _)| *n == obj_num).map(|(_, e)| *e))
    }

    fn read_from_obj_stream(&self, _stream_offset: u64, _target: i64) -> Option<Vec<u8>> {
        self.obj_stream.clone()
    }

    /// `fast_page_count_with`를 이 대역의 세 메서드를 훅으로 끼워 호출한다. `find_xref_offset`은 실제 구현을 쓴다.
    fn fast_page_count(&self, pdf: &Path) -> Option<usize> {
        let parse = |_: &mut fs::File, offset: u64| self.parse_one_xref_stream(offset);
        let find_entry = |data: &[u8], w: [usize; 3], index_ranges: &[i64], obj_num: i64| self.xref_stream_find_entry(data, w, index_ranges, obj_num);
        let obj_stream = |_: &mut fs::File, offset: u64, target: i64| self.read_from_obj_stream(offset, target);
        let hooks = FastCountHooks { parse_one_xref_stream: Some(&parse), xref_stream_find_entry: Some(&find_entry), read_from_obj_stream: Some(&obj_stream), ..Default::default() };
        fast_page_count_with(pdf, &hooks)
    }
}

/// 공백 256바이트에 조각을 지정한 위치로 박고, 끝에 `startxref 20` 꼬리를 붙인 PDF 본문.
fn padded_pdf(parts: &[(usize, &[u8])]) -> Vec<u8> {
    let mut data = vec![b' '; 256];
    for (at, bytes) in parts {
        data[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    let trailer = b"startxref\n20\n%%EOF";
    let n = data.len();
    data[n - trailer.len()..].copy_from_slice(trailer);
    data
}

const XREF_DICT: &[u8] = b"<< /Type /XRef /Root 1 0 R >>";
fn parsed(index: &[i64]) -> Option<(Vec<u8>, [usize; 3], Vec<i64>, Option<i64>)> {
    Some((b"decoded".to_vec(), [1, 2, 1], index.to_vec(), None))
}

mod test_fast_pdf_page_count {
    use super::*;

    #[test]
    fn test_real_pdf() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("test.pdf");
        write(&pdf, minimal_pdf(2));
        let count = fast_page_count(&pdf);
        // May return 2 or None depending on xref structure
        assert!(matches!(count, None | Some(2)), "{count:?}");
    }

    #[test]
    fn test_not_a_pdf() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("not.pdf");
        write(&f, "not a pdf");
        assert_eq!(fast_page_count(&f), None);
    }

    #[test]
    fn test_xref_stream_type1_lookup() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("stream.pdf");
        write(&pdf, padded_pdf(&[(20, XREF_DICT), (100, b"<< /Pages 2 0 R >>"), (140, b"<< /Count 7 >>")]));
        let ops = FakeXref { parsed: parsed(&[0, 1]), entries: vec![(1, (1, 100, 0)), (2, (1, 140, 0))], any_entry: None, obj_stream: None };
        assert_eq!(ops.fast_page_count(&pdf), Some(7));
    }

    #[test]
    fn test_xref_stream_type2_object_stream_lookup() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("objstream.pdf");
        write(&pdf, padded_pdf(&[(20, XREF_DICT), (100, b"<< /Pages 2 0 R >>")]));
        let ops = FakeXref { parsed: parsed(&[0, 3]), entries: vec![(1, (1, 100, 0)), (2, (2, 9, 0)), (9, (1, 180, 0))], any_entry: None, obj_stream: Some(b"<< /Type /Pages /Count 4 >>".to_vec()) };
        assert_eq!(ops.fast_page_count(&pdf), Some(4));
    }

    #[test]
    fn test_xref_stream_without_root_returns_none() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("no-root.pdf");
        write(&pdf, [b"0".repeat(64), b"startxref\n20\n%%EOF".to_vec()].concat());
        let ops = FakeXref { parsed: parsed(&[0, 1]), entries: vec![], any_entry: None, obj_stream: None };
        assert_eq!(ops.fast_page_count(&pdf), None);
    }

    #[test]
    fn test_xref_stream_missing_pages_reference_returns_none() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("missing-pages-ref.pdf");
        write(&pdf, padded_pdf(&[(20, XREF_DICT), (100, b"<< /Type /Catalog >>")]));
        let ops = FakeXref { parsed: parsed(&[0, 1]), entries: vec![], any_entry: Some((1, 100, 0)), obj_stream: None };
        assert_eq!(ops.fast_page_count(&pdf), None);
    }

    #[test]
    fn test_xref_stream_missing_pages_object_returns_none() {
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("missing-pages-object.pdf");
        write(&pdf, padded_pdf(&[(20, XREF_DICT), (100, b"<< /Pages 2 0 R >>")]));
        let ops = FakeXref { parsed: parsed(&[0, 1]), entries: vec![(1, (1, 100, 0))], any_entry: None, obj_stream: None };
        assert_eq!(ops.fast_page_count(&pdf), None);
    }
}

// ---- TestReadFromEpubEdgeCases --------------------------------------------------------------

mod test_read_from_epub_edge_cases {
    use super::*;

    #[test]
    fn test_epub_chapter_read_exception() {
        // Lines 118-119: chapter read raises exception
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf">
  <metadata/>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
        let container = r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles><rootfile full-path="content.opf"/></rootfiles>
</container>"#;
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("test.epub");
        // ch1.xhtml intentionally corrupt
        write_zip(&epub, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", container.as_bytes()), ("content.opf", opf.as_bytes()), ("ch1.xhtml", b"\x00\x01\x02\x03")]);
        // Python의 isinstance(result, str)은 타입으로 보장된다. 패닉 없이 끝나는지만 본다.
        let _read = read_from_epub_with_extracting_zip(&epub);
    }

    #[test]
    fn test_epub_both_methods_fail() {
        // Lines 171-173: both ebooklib and zip fallback fail
        let d = tempfile::tempdir().unwrap();
        let epub = d.path().join("test.epub");
        write(&epub, b"not a zip at all");
        // read_from_epub tries ebooklib (fails) then zip fallback (also fails)
        let _read = read_from_epub(&epub);
    }
}

// ---- TestReadFromPdfEdgeCases ---------------------------------------------------------------

mod test_read_from_pdf_edge_cases {
    use super::*;

    #[test]
    fn test_pdf_text_exceeds_limit() {
        // Line 198: break when text exceeds TEXT_SIZE
        let d = tempfile::tempdir().unwrap();
        let pdf = d.path().join("big.pdf");
        write(&pdf, minimal_pdf(3));
        // 원본은 pypdf.PdfReader를 세 쪽짜리 가짜로 바꾼다. 같은 내용을 pdfium 단계의 쪽 목록으로 준다.
        let runner = CollectRunner::new(vec!["x".repeat(3000); 3]);
        let read = read_from_pdf(&pdf, &runner);
        // TEXT_SIZE is 4096, so with 3000*2=6000 > 4096, should break early
        assert!(read.summary.chars().count() <= TEXT_SIZE + 3000, "{}", read.summary.chars().count());
    }
}

// ---- TestReadFromHwpEdgeCases ---------------------------------------------------------------

mod test_read_from_hwp_edge_cases {
    use super::*;

    #[test]
    fn test_hwp_conversion_exception() {
        // Lines 345-346: LibreOffice conversion raises
        let d = tempfile::tempdir().unwrap();
        let hwp = d.path().join("test.hwp");
        write(&hwp, b"fake hwp");
        let binary = fake_libreoffice(d.path(), None);
        let (summary, _line_count, _errors) = read_from_hwp(&hwp, &binary);
        assert_eq!(summary, "");
    }
}

// ---- TestFindXrefOffsetExtra ----------------------------------------------------------------

mod test_find_xref_offset_extra {
    use super::*;

    #[test]
    fn test_leading_lines_before_xref() {
        // Line 365: lines before 'xref' keyword
        let xref_data = b"some garbage\nmore garbage\nxref\n0 2\n0000000000 65535 f \n0000000100 00000 n \ntrailer\n";
        assert_eq!(find_xref_offset(xref_data, 1), Some(100));
    }

    #[test]
    fn test_empty_lines_in_xref() {
        // Lines 371-372: empty lines in xref block
        let xref_data = b"xref\n\n0 2\n0000000000 65535 f \n0000000200 00000 n \ntrailer\n";
        assert_eq!(find_xref_offset(xref_data, 1), Some(200));
    }

    #[test]
    fn test_malformed_line_in_xref() {
        // Line 388: non-subsection, non-trailer line
        let xref_data = b"xref\ngarbage_line\n0 2\n0000000000 65535 f \n0000000300 00000 n \ntrailer\n";
        assert_eq!(find_xref_offset(xref_data, 1), Some(300));
    }
}
