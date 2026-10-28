#![allow(clippy::type_complexity)]

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
//! - `loader::record::{ReadHooks, read_file_with_hooks(...)}`: `fast_page_count`, `fallback_page_count`, `inspect` 대체 훅 (`Default` 구현)
//! - `loader::cli::{RunHooks, run_with_hooks(...)}`: `read_file`, `stat`, `on_worker_spawn` 대체 훅

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, UNIX_EPOCH};

use doc_reader::TEXT_SIZE;
use doc_reader::mojibake::{PDF_MOJIBAKE_RUN_LIMIT, count_mojibake_runs, count_mojibake_runs_with};
use doc_reader::office::{convert_with_libreoffice, find_libreoffice, read_from_hwp_with_fallback};
use doc_reader::pdf::{
    FastCountHooks, InProcess, PdftotextHooks, PlumberDocument, Stage, StageError, StageRunner, fast_page_count, fast_page_count_with, find_xref_offset, parse_one_xref_stream,
    pdf_text_by_pdftotext_with, pdf_text_by_plumber_with, read_from_obj_stream, read_from_pdf, run_stage,
};
use doc_reader::text::{Candidate, Enc, TextHooks, read_from_text_with, reencode_text_file_to_utf8_with};
use es_client::EsError;
use es_client::indexing::IndexStore;
use loader::cli::{RunHooks, World, parse_options, run, run_with_hooks};
use loader::config::Config;
use loader::flow::{recursive_files, report_problems, sample_files};
use loader::record::{ReadHooks, ReadOutcome, read_file, read_file_with_hooks, zip_container_type};
use parser_timeout::ParserTimeout;
use serde_json::{Value, json};
use stat::Stat;
use tempfile::TempDir;

// ---- 공통 헬퍼 ----

fn tmp() -> TempDir {
    tempfile::tempdir().unwrap()
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// zlib.compress 대응. 압축하지 않는(stored) deflate 블록이라 64KiB 미만 입력만 다룬다.
fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u16;
    let mut out = vec![0x78, 0x01, 0x01];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(data);
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
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

fn config_for(book: &Path, comics: &Path) -> Config {
    Config::new(book, comics)
}

fn read(cfg: &Config, path: &Path, skip_text: bool) -> ReadOutcome {
    read_file(cfg, path, None, skip_text, None, &InProcess, &Stat::new())
}

fn item_of(outcome: &ReadOutcome) -> &Value {
    &outcome.item.as_ref().expect("문서가 만들어져야 한다").1
}

fn messages_of(outcome: &ReadOutcome) -> String {
    outcome.messages.join("\n")
}

fn inode_of(path: &Path) -> i64 {
    fs::metadata(path).unwrap().ino() as i64
}

/// 로컬 시각 기준 해당 해 1월 1일 0시의 유닉스 시각(`datetime(y, 1, 1).timestamp()`).
fn local_timestamp(year: i32) -> u64 {
    use chrono::TimeZone;
    chrono::Local.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single().unwrap().timestamp() as u64
}

fn set_mtime(path: &Path, secs: u64) {
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
}

fn meta(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
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

// ---- HWP 헬퍼 ----

fn hwp_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/books/_hwp")
}

/// 샘플 파일이 없으면 panic한다.
fn hwp_fixture(name: &str) -> PathBuf {
    let f = hwp_dir().join(name);
    assert!(f.exists(), "HWP 샘플 파일 필요: tests/books/_hwp/{name}가 없다");
    f
}

/// 변환기를 흉내 내는 셸 스크립트. 마지막 인자(원본)의 stem으로 `--outdir` 아래에 결과를 쓴다.
fn fake_office(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fake-office");
    fs::write(
        &path,
        format!("#!/bin/sh\nout=\"\"\nwhile [ $# -gt 1 ]; do if [ \"$1\" = \"--outdir\" ]; then out=\"$2\"; fi; shift; done\nsrc=\"$1\"\nname=$(basename \"$src\")\nstem=\"${{name%.*}}\"\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// `Loader.read_from_hwp` 대응: HWP를 책 디렉토리에 복사해 `read_file`로 읽고 (요약, 줄 수, 쪽 수)를 돌려준다.
fn read_from_hwp(binary: &Path, hwp: &Path) -> (String, usize, usize) {
    let dir = tmp();
    let copy = dir.path().join(hwp.file_name().unwrap());
    fs::copy(hwp, &copy).unwrap();
    let mut cfg = config_for(dir.path(), &dir.path().join("comics"));
    cfg.libreoffice = binary.to_path_buf();
    let outcome = read(&cfg, &copy, false);
    let doc = item_of(&outcome);
    (doc["summary"].as_str().unwrap().to_owned(), doc["line_count"].as_u64().unwrap() as usize, doc["page_count"].as_u64().unwrap() as usize)
}

/// HWP 파일 헤더에서 버전 문자열을 추출한다.
fn detect_hwp_version(file_path: &Path) -> String {
    let mut header = [0u8; 30];
    let n = File::open(file_path).unwrap().read(&mut header).unwrap();
    let header = &header[..n];
    if header.starts_with(b"HWP Document Fi") {
        let text = String::from_utf8_lossy(header).into_owned();
        // 정규식 `V(\d+\.\d+)`
        for (i, _) in text.match_indices('V') {
            let rest = &text[i + 1..];
            let major: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if major.is_empty() {
                continue;
            }
            if let Some(frac) = rest[major.len()..].strip_prefix('.') {
                let minor: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !minor.is_empty() {
                    return format!("{major}.{minor}");
                }
            }
        }
    }
    if header.starts_with(b"\xd0\xcf\x11\xe0") {
        return "5.x".to_owned();
    }
    "unknown".to_owned()
}

// ---- PDF 헬퍼 ----

fn obj_stream(bytes: &[u8], obj_num: i64) -> Option<Vec<u8>> {
    let dir = tmp();
    let f = dir.path().join("fake.pdf");
    fs::write(&f, bytes).unwrap();
    let mut fh = File::open(&f).unwrap();
    read_from_obj_stream(&mut fh, 0, obj_num)
}

type XrefStream = (Vec<u8>, [usize; 3], Vec<i64>, Option<i64>);

fn xref_stream(bytes: &[u8]) -> Option<XrefStream> {
    let dir = tmp();
    let f = dir.path().join("fake.pdf");
    fs::write(&f, bytes).unwrap();
    let mut fh = File::open(&f).unwrap();
    parse_one_xref_stream(&mut fh, 0)
}

/// `startxref`가 20을 가리키는 256바이트 PDF 꼬리. 20번 위치에 `blocks`를 둔다.
fn pdf_with_blocks(blocks: &[(usize, &[u8])], size: usize) -> Vec<u8> {
    let mut data = vec![b' '; size];
    for (offset, block) in blocks {
        data[*offset..*offset + block.len()].copy_from_slice(block);
    }
    let trailer = b"startxref\n20\n%%EOF";
    let start = data.len() - trailer.len();
    data[start..].copy_from_slice(trailer);
    data
}

mod test_read_from_obj_stream {
    use super::*;

    #[test]
    fn test_missing_first_or_length() {
        // Lines 443-446: /First or /Length missing
        assert!(obj_stream(b"1 0 obj\n<< /Type /ObjStm >>\nstream\ndata\nendstream", 1).is_none());
    }

    #[test]
    fn test_indirect_length() {
        // Lines 447-448: /Length is indirect reference
        assert!(obj_stream(b"1 0 obj\n<< /First 10 /Length 5 0 R >>\nstream\ndata\nendstream", 1).is_none());
    }

    #[test]
    fn test_no_stream_marker() {
        // Lines 453-455: no stream marker
        assert!(obj_stream(b"1 0 obj\n<< /First 10 /Length 20 >>\nendobj", 1).is_none());
    }

    #[test]
    fn test_non_flatedecode_filter() {
        // Lines 462-463: unsupported filter
        let data = b"1 0 obj\n<< /First 10 /Length 5 /Filter /ASCIIHexDecode >>\nstream\nhello\nendstream";
        assert!(obj_stream(data, 1).is_none());
    }

    #[test]
    fn test_zlib_decompress_failure() {
        // Lines 466-467: zlib decompression fails
        let data = b"1 0 obj\n<< /First 10 /Length 5 /Filter /FlateDecode >>\nstream\nhello\nendstream";
        assert!(obj_stream(data, 1).is_none());
    }

    #[test]
    fn test_target_found() {
        // Lines 476-480: target object found in stream
        // Object stream: header "5 0 6 20" means obj 5 at offset 0, obj 6 at offset 20
        let header = b"5 0 6 20 ";
        let obj_data = b"<< /Type /Catalog >>";
        let mut raw = header.to_vec();
        raw.extend_from_slice(obj_data);
        raw.extend_from_slice(&[b' '; 20]);
        let compressed = zlib_compress(&raw);
        let first_offset = header.len();
        let mut stream_data = format!("1 0 obj\n<< /First {first_offset} /Length {} /Filter /FlateDecode >>\nstream\n", compressed.len()).into_bytes();
        stream_data.extend_from_slice(&compressed);
        stream_data.extend_from_slice(b"\nendstream");
        let result = obj_stream(&stream_data, 5);
        let result = result.expect("대상 객체를 찾아야 한다");
        assert!(result.windows(b"Catalog".len()).any(|w| w == b"Catalog"));
    }

    #[test]
    fn test_target_not_found() {
        // Lines 482-484: target object not in stream
        let header = b"5 0 ";
        let obj_data = b"<< /Type /Catalog >>";
        let mut raw = header.to_vec();
        raw.extend_from_slice(obj_data);
        let compressed = zlib_compress(&raw);
        let first_offset = header.len();
        let mut stream_data = format!("1 0 obj\n<< /First {first_offset} /Length {} /Filter /FlateDecode >>\nstream\n", compressed.len()).into_bytes();
        stream_data.extend_from_slice(&compressed);
        stream_data.extend_from_slice(b"\nendstream");
        assert!(obj_stream(&stream_data, 99).is_none());
    }
}

mod test_parse_one_xref_stream {
    use super::*;

    #[test]
    fn test_missing_w() {
        // Lines 493-495: /W missing
        assert!(xref_stream(b"1 0 obj\n<< /Size 10 /Length 20 >>\nstream\ndata").is_none());
    }

    #[test]
    fn test_missing_size() {
        // Lines 498-500: /Size missing
        assert!(xref_stream(b"1 0 obj\n<< /W [1 2 1] /Length 20 >>\nstream\ndata").is_none());
    }

    #[test]
    fn test_missing_length() {
        assert!(xref_stream(b"1 0 obj\n<< /W [1 2 1] /Size 10 >>\nstream\ndata").is_none());
    }

    #[test]
    fn test_indirect_length() {
        // Lines 507-508: /Length is indirect reference
        assert!(xref_stream(b"1 0 obj\n<< /W [1 2 1] /Size 10 /Length 5 0 R >>\nstream\ndata").is_none());
    }

    #[test]
    fn test_no_stream_marker() {
        // Lines 514-516: no stream marker
        assert!(xref_stream(b"1 0 obj\n<< /W [1 2 1] /Size 10 /Length 20 >>\nendobj").is_none());
    }

    #[test]
    fn test_non_flatedecode() {
        // Lines 524-525: unsupported filter
        assert!(xref_stream(b"1 0 obj\n<< /W [1 2 1] /Size 10 /Length 5 /Filter /LZW >>\nstream\nhello\nendstream").is_none());
    }

    #[test]
    fn test_zlib_failure() {
        // Lines 528-529: zlib decompression fails
        assert!(xref_stream(b"1 0 obj\n<< /W [1 2 1] /Size 10 /Length 5 /Filter /FlateDecode >>\nstream\nhello\nendstream").is_none());
    }

    #[test]
    fn test_uncompressed_stream() {
        // Lines 530-531: no filter -> raw data
        // Create an uncompressed xref stream: W=[1,2,1], Size=2, 2 entries of 4 bytes each
        let mut entry_data = vec![1u8, 0, 100, 0];
        entry_data.extend_from_slice(&[1, 0, 200, 0]); // type=1, offset=100/200, gen=0
        let mut stream_data = format!("1 0 obj\n<< /W [1 2 1] /Size 2 /Length {} >>\nstream\n", entry_data.len()).into_bytes();
        stream_data.extend_from_slice(&entry_data);
        stream_data.extend_from_slice(b"\nendstream");
        let result = xref_stream(&stream_data);
        let (_decompressed, w, _index_ranges, _prev_offset) = result.expect("xref stream을 읽어야 한다");
        assert_eq!(w, [1, 2, 1]);
    }

    #[test]
    fn test_with_decode_parms() {
        // Lines 534-540: PNG predictor with /DecodeParms
        // Each row: 1 filter byte + 4 data bytes (W=[1,2,1] -> 4 bytes per entry)
        // Filter byte 0 = None predictor
        let row1 = [0u8, 1, 0, 100, 0]; // type=1, offset=100, gen=0
        let row2 = [0u8, 1, 0, 200, 0]; // type=1, offset=200, gen=0
        let raw: Vec<u8> = row1.iter().chain(row2.iter()).copied().collect();
        let compressed = zlib_compress(&raw);
        let mut stream_data = format!("1 0 obj\n<< /W [1 2 1] /Size 2 /Length {} /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >> >>\nstream\n", compressed.len()).into_bytes();
        stream_data.extend_from_slice(&compressed);
        stream_data.extend_from_slice(b"\nendstream");
        assert!(xref_stream(&stream_data).is_some());
    }
}

mod test_read_file_edge_cases {
    use super::*;

    #[test]
    fn test_read_file_skip_text_pdf_fast_fails() {
        // Lines 710-712: skip_text PDF, fast count None, pypdf also fails
        // 원본은 _fast_pdf_page_count를 None으로 바꾼다. "not a pdf"는 실제로도 경량 읽기가 실패하므로 바꿔치기 없이 같은 경로를 탄다.
        let dir = tmp();
        let pdf = dir.path().join("cat").join("test.pdf");
        fs::create_dir(pdf.parent().unwrap()).unwrap();
        fs::write(&pdf, b"not a pdf").unwrap();
        let cfg = config_for(dir.path(), &dir.path().join("comics"));
        let result = read(&cfg, &pdf, true);
        assert!(result.item.is_some());
        assert_eq!(item_of(&result)["page_count"], 0);
    }

    #[test]
    fn test_read_file_comics_pdf() {
        // Lines 749-759: PDF in comics prefix (text extraction skipped)
        let dir = tmp();
        let pdf = dir.path().join("comics_cat").join("test.pdf");
        fs::create_dir(pdf.parent().unwrap()).unwrap();
        write_blank_pdf(&pdf);
        let cfg = config_for(dir.path(), dir.path());
        let result = read(&cfg, &pdf, false);
        assert!(result.item.is_some());
        assert!(item_of(&result)["page_count"].as_u64().is_some());
    }

    #[test]
    fn test_convert_with_libreoffice_glob_fallback() {
        // Line 305: output file stem doesn't match, falls back to glob
        let dir = tmp();
        // 원본 fake_run: 다른 이름의 출력 파일을 만든다.
        let office = fake_office(dir.path(), "printf 'converted' > \"$out/different_name.txt\"");
        let mut errors = Vec::new();
        let result = convert_with_libreoffice(&dir.path().join("input.doc"), "txt:Text", &office, &mut errors);
        assert_eq!(result, "converted");
    }
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

mod test_hwp_version_detection {
    use super::*;

    // HWP 파일 헤더 기반 버전 감지 테스트

    #[test]
    fn test_detect_v1_20() {
        let f = hwp_fixture("v1.20_부하의약혼녀.hwp");
        assert_eq!(detect_hwp_version(&f), "1.20");
    }

    #[test]
    fn test_detect_v2_00() {
        let f = hwp_fixture("v2.00_박노해_참된시작.hwp");
        assert_eq!(detect_hwp_version(&f), "2.00");
    }

    #[test]
    fn test_detect_v2_10() {
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        assert_eq!(detect_hwp_version(&f), "2.10");
    }

    #[test]
    fn test_detect_v3_00() {
        let f = hwp_fixture("v3.00_현대시사전.hwp");
        assert_eq!(detect_hwp_version(&f), "3.00");
    }

    #[test]
    fn test_detect_unknown_for_non_hwp() {
        let dir = tmp();
        let f = dir.path().join("fake.hwp");
        fs::write(&f, b"this is not an hwp file at all").unwrap();
        assert_eq!(detect_hwp_version(&f), "unknown");
    }

    #[test]
    fn test_detect_ole2_hwp5() {
        // OLE2 매직 바이트를 가진 HWP 5.x 감지
        let dir = tmp();
        let f = dir.path().join("hwp5.hwp");
        let mut bytes = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1".to_vec();
        bytes.extend_from_slice(&[0u8; 22]);
        fs::write(&f, bytes).unwrap();
        assert_eq!(detect_hwp_version(&f), "5.x");
    }
}

mod test_read_from_hwp_real_files {
    use super::*;

    // 실제 HWP 파일로 read_from_hwp 테스트 (LibreOffice 의존).
    // LibreOffice가 설치되어 있지 않거나 변환 실패 시에도 크래시 없이 빈 문자열을 반환하는지 검증한다.
    // 반환 타입(str, int)은 Rust 타입이 보장하므로 쪽 수만 값으로 확인한다.

    fn hwp_files() -> Vec<PathBuf> {
        let dir = hwp_dir();
        if !dir.exists() {
            return Vec::new();
        }
        let mut files: Vec<PathBuf> = fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "hwp")).collect();
        files.sort();
        files
    }

    fn assert_no_crash(name: &str) {
        let f = hwp_fixture(name);
        let (_summary, _line_count, page_count) = read_from_hwp(&find_libreoffice(), &f);
        assert_eq!(page_count, 0);
    }

    #[test]
    fn test_v1_20_no_crash() {
        assert_no_crash("v1.20_부하의약혼녀.hwp");
    }

    #[test]
    fn test_v2_00_no_crash() {
        assert_no_crash("v2.00_박노해_참된시작.hwp");
    }

    #[test]
    fn test_v2_10_no_crash() {
        assert_no_crash("v2.10_KYOKA.hwp");
    }

    #[test]
    fn test_v3_00_no_crash() {
        assert_no_crash("v3.00_현대시사전.hwp");
    }

    #[test]
    fn test_v3_00_noname22_no_crash() {
        assert_no_crash("v3.00_noname22.hwp");
    }

    #[test]
    fn test_all_versions_return_tuple() {
        // 모든 버전 파일에 대해 3-tuple 반환 확인
        let files = hwp_files();
        assert!(!files.is_empty(), "HWP 샘플 파일 필요: tests/books/_hwp에 .hwp 파일이 없다");
        for f in files {
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            let (_summary, _line_count, page_count) = read_from_hwp(&find_libreoffice(), &f);
            assert_eq!(page_count, 0, "파일 {name}: page_count가 0이 아님");
        }
    }
}

mod test_read_from_hwp_with_mocked_libreoffice {
    use super::*;

    // LibreOffice 변환 결과를 시뮬레이션하여 버전별 동작 검증

    #[test]
    fn test_v1_20_libreoffice_returns_empty() {
        // V1.20 파일: LibreOffice가 빈 결과를 반환하는 경우
        let f = hwp_fixture("v1.20_부하의약혼녀.hwp");
        let dir = tmp();
        let office = fake_office(dir.path(), ": > \"$out/$stem.txt\"");
        let (summary, line_count, _page_count) = read_from_hwp(&office, &f);
        assert_eq!(summary, "");
        assert_eq!(line_count, 0);
    }

    #[test]
    fn test_v2_10_libreoffice_returns_text() {
        // V2.10 파일: LibreOffice가 텍스트를 반환하는 경우
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        let dir = tmp();
        let office = fake_office(dir.path(), "printf '교카 시집\\n첫 번째 시\\n두 번째 시' > \"$out/$stem.txt\"");
        let (summary, line_count, _page_count) = read_from_hwp(&office, &f);
        assert!(summary.contains("교카"));
        assert_eq!(line_count, 3);
    }

    #[test]
    fn test_v3_00_libreoffice_raises() {
        // V3.00 파일: LibreOffice 변환 시 예외 발생. 변환기 실패(종료 코드 3)는 Rust에서 HWP3 fallback으로 이어지므로 예외 주입 seam이 필요하다.
        let f = hwp_fixture("v3.00_현대시사전.hwp");
        let dir = tmp();
        let office = fake_office(dir.path(), "echo '변환 실패' >&2; exit 3");
        let (summary, line_count, _page_count) = read_from_hwp(&office, &f);
        assert_eq!(summary, "");
        assert_eq!(line_count, 0);
    }

    #[test]
    fn test_special_chars_cleaned() {
        // 변환 결과의 특수문자가 정리되는지 확인
        let f = hwp_fixture("v2.10_PAGER.hwp");
        let dir = tmp();
        let office = fake_office(dir.path(), "printf '텍스트★내용♣특수◆문자' > \"$out/$stem.txt\"");
        let (summary, _line_count, _page_count) = read_from_hwp(&office, &f);
        assert!(!summary.contains('★'));
        assert!(summary.contains("텍스트"));
        assert!(summary.contains("내용"));
    }
}

mod test_hwp3_fallback_integration {
    use super::*;

    // LibreOffice 실패 시 hwp3 파서 fallback 테스트

    #[test]
    fn test_fallback_when_libreoffice_returns_empty() {
        // LibreOffice가 빈 문자열 -> hwp3 파서로 텍스트 추출
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        let dir = tmp();
        let office = fake_office(dir.path(), ": > \"$out/$stem.txt\"");
        let (summary, _line_count, _page_count) = read_from_hwp(&office, &f);
        assert!(!summary.is_empty());
        assert!(summary.contains("영동") || summary.contains("교가"));
    }

    #[test]
    fn test_fallback_when_libreoffice_returns_whitespace() {
        // LibreOffice가 공백만 반환 -> hwp3 파서로 fallback
        let f = hwp_fixture("v3.00_현대시사전.hwp");
        let dir = tmp();
        let office = fake_office(dir.path(), "printf '   \\n  ' > \"$out/$stem.txt\"");
        let (summary, _line_count, _page_count) = read_from_hwp(&office, &f);
        assert!(!summary.is_empty());
    }

    #[test]
    fn test_no_fallback_when_libreoffice_succeeds() {
        // LibreOffice가 텍스트 반환 -> fallback 사용 안 함
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        let dir = tmp();
        let office = fake_office(dir.path(), "printf 'LO 결과' > \"$out/$stem.txt\"");
        let (summary, _line_count, _page_count) = read_from_hwp(&office, &f);
        assert!(summary.contains("LO"));
    }
}

mod test_loader_hwp3_fallback_edge_cases {
    use super::*;

    // Loader.read_from_hwp의 hwp3 fallback 엣지 케이스

    fn empty_office(dir: &Path) -> PathBuf {
        fake_office(dir, ": > \"$out/$stem.txt\"")
    }

    #[test]
    fn test_fallback_exception_in_hwp3_parser() {
        // hwp3 파서에서 예외 발생해도 전체가 크래시하지 않음
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        let dir = tmp();
        let office = empty_office(dir.path());
        let hwp3_raises = |_: &Path| -> Result<String, String> { Err("파서 에러".to_owned()) };
        let convert_empty = |_: &Path| -> Result<String, String> { Ok(String::new()) };
        let (summary, _line_count, _errors) = read_from_hwp_with_fallback(&f, &office, &convert_empty, &hwp3_raises);
        assert_eq!(summary, "");
        // page_count == 0: read_from_hwp_with는 쪽 수를 돌려주지 않는다(항상 0).
    }

    #[test]
    fn test_fallback_text_truncated_to_text_size() {
        // fallback 결과가 TEXT_SIZE로 잘리는지 확인
        let f = hwp_fixture("v2.10_PAGER.hwp");
        let dir = tmp();
        let (summary, _line_count, _page_count) = read_from_hwp(&empty_office(dir.path()), &f);
        assert!(summary.chars().count() <= TEXT_SIZE);
    }

    #[test]
    fn test_fallback_special_chars_cleaned() {
        // fallback 결과에도 특수문자 정리가 적용되는지 확인
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        let dir = tmp();
        let (summary, _line_count, _page_count) = read_from_hwp(&empty_office(dir.path()), &f);
        // 한글, 영숫자, 공백만 남아야 함 (정규식 `[^\w\sㄱ-힣]`을 지운 결과가 원본과 같다)
        let cleaned: String = summary.chars().filter(|c| c.is_alphanumeric() || *c == '_' || c.is_whitespace() || ('ㄱ'..='힣').contains(c)).collect();
        assert_eq!(cleaned, summary);
    }

    #[test]
    fn test_fallback_line_count_correct() {
        // fallback 결과의 line_count가 정확한지 확인
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        let dir = tmp();
        let (summary, line_count, _page_count) = read_from_hwp(&empty_office(dir.path()), &f);
        if !summary.is_empty() {
            assert!(line_count > 0);
        }
    }
}

mod test_find_xref_offset_branches {
    use super::*;

    // 기존 TestFindXrefOffset 이 놓친 분기를 명시적으로 검증한다.

    #[test]
    fn test_trailer_break_and_end_return_none() {
        // obj 가 범위 밖 -> i += 1+sub_count -> trailer break -> return None (lines 366, 378-379, 381).
        // obj_num=5 는 [0,3) 범위 밖 -> skip subsection -> trailer -> None
        let xref = b"xref\n0 3\n0000000000 65535 f \n0000000100 00000 n \n0000000200 00000 n \ntrailer\n<< /Size 3 >>";
        assert_eq!(find_xref_offset(xref, 5), None);
    }

    #[test]
    fn test_entry_idx_out_of_range_returns_none() {
        // entry_idx >= len(lines) 이면 return None (line 377).
        // sub_count=2 이지만 entry 가 1개만 존재 -> obj_num=1 의 entry_idx 가 범위 밖
        let xref = b"xref\n0 2\n0000000009 00000 n \n";
        assert_eq!(find_xref_offset(xref, 1), None);
    }

    #[test]
    fn test_non_matching_line_increments_i() {
        // digits 2개가 아닌 줄은 i += 1 로 건너뛴다 (line 380-381).
        // "junk" 줄은 2-digit 패턴 불일치 -> i += 1; 그 다음 "0 1" 을 처리 -> obj 0 발견
        let xref = b"xref\njunk\n0 1\n0000000009 00000 n \ntrailer\n";
        assert_eq!(find_xref_offset(xref, 0), Some(9));
    }

    #[test]
    fn test_multiple_subsections_skips_first() {
        // 첫 subsection 에 obj 없음 -> i += 1+sub_count (lines 378-379); 두 번째에서 발견.
        let xref = b"xref\n0 1\n0000000009 00000 n \n5 1\n0000000100 00000 n \ntrailer\n";
        // obj_num=5 는 첫 subsection [0,1) 에 없음 -> skip -> 두 번째 [5,6) 에서 발견
        assert_eq!(find_xref_offset(xref, 5), Some(100));
        // obj_num=0 는 첫 subsection 에서 발견
        assert_eq!(find_xref_offset(xref, 0), Some(9));
    }
}

mod test_read_file_comics_pdf {
    use super::*;

    #[test]
    fn test_comics_pdf_fast_count_none_falls_back_to_pypdf() {
        // _fast_pdf_page_count 가 None 을 반환하면 pypdf.PdfReader 로 페이지 수를 구한다 (lines 726-729).
        // comics_path_prefix 를 tmp_path 로 설정
        let dir = tmp();
        let cfg = config_for(dir.path(), dir.path());
        let pdf_file = dir.path().join("book.pdf");
        write_blank_pdf(&pdf_file);

        // _fast_pdf_page_count 가 None 을 반환하도록 강제
        let fast_none = |_: &Path| -> Option<usize> { None };
        let hooks = ReadHooks { fast_page_count: Some(&fast_none), ..Default::default() };
        let data = read_file_with_hooks(&cfg, &pdf_file, None, false, None, &InProcess, &Stat::new(), &hooks);
        // inode 키로 결과가 반환되며 page_count 가 1
        assert!(data.item.is_some());
        assert_eq!(item_of(&data)["page_count"], 1);
    }

    #[test]
    fn test_comics_pdf_fast_count_none_pypdf_exception() {
        // pypdf.PdfReader 도 실패하면 page_count=0 으로 처리한다 (lines 726-732).
        // 원본은 두 함수를 바꿔치기한다. 깨진 PDF는 실제로도 경량 읽기와 lopdf가 모두 실패하므로 같은 경로를 탄다.
        let dir = tmp();
        let cfg = config_for(dir.path(), dir.path());
        let pdf_file = dir.path().join("broken.pdf");
        fs::write(&pdf_file, b"%PDF-broken").unwrap();

        let data = read(&cfg, &pdf_file, false);

        assert!(data.item.is_some());
        assert_eq!(item_of(&data)["page_count"], 0);
    }
}

mod test_fast_pdf_page_count_branches {
    use super::*;

    #[test]
    fn test_xref_stream_parse_none_breaks_and_returns_none() {
        // parse None -> break(603), 이후 lookup 없어 root_data None -> return None(645).
        // 원본은 _parse_one_xref_stream을 None으로 바꾼다. /W·/Size·/Length가 없는 헤더라 실제 파서도 None을 돌려준다.
        let dir = tmp();
        let pdf = dir.path().join("parse-none.pdf");
        let data = pdf_with_blocks(&[(20, b"<< /Type /XRef /Root 1 0 R >>")], 256);
        fs::write(&pdf, data).unwrap();

        assert_eq!(fast_page_count(&pdf), None);
    }

    #[test]
    fn test_xref_stream_parse_raises_is_swallowed() {
        // parse 중 예외 발생 -> except로 흡수 후 None(658-660).
        let dir = tmp();
        let pdf = dir.path().join("parse-raise.pdf");
        let data = pdf_with_blocks(&[(20, b"<< /Type /XRef /Root 1 0 R >>")], 256);
        fs::write(&pdf, data).unwrap();

        let boom = |_: &mut File, _: u64| -> Option<XrefStream> { panic!("boom") };
        let hooks = FastCountHooks { parse_one_xref_stream: Some(&boom), ..Default::default() };
        assert_eq!(fast_page_count_with(&pdf, &hooks), None);
    }

    #[test]
    fn test_mixed_traditional_and_stream_lookups_type2() {
        // traditional + stream lookup 혼재 시 type-2 조회 루프에서 비-stream은 continue(634).
        let dir = tmp();
        let pdf = dir.path().join("mixed.pdf");
        // offset 20: traditional xref (root=1, prev=200)
        let xref_block = b"xref\n0 1\n/Root 1 0 R\n/Prev 200\n";
        let data = pdf_with_blocks(&[(20, xref_block), (200, b"<< /Type /XRef >>"), (100, b"<< /Pages 2 0 R >>")], 400);
        fs::write(&pdf, data).unwrap();

        // traditional lookup: obj 1만 offset 100, 나머지는 None
        let find_offset = |_xd: &[u8], obj: i64| -> Option<i64> { if obj == 1 { Some(100) } else { None } };
        // 단일 stream xref (prev 없음)
        let parse = |_: &mut File, _: u64| -> Option<XrefStream> { Some((b"d".to_vec(), [1, 2, 1], vec![0, 3], None)) };
        let find_entry = |_d: &[u8], _w: [usize; 3], _ir: &[i64], obj_num: i64| -> Option<(i64, i64, i64)> {
            match obj_num {
                2 => Some((2, 9, 0)),   // type 2 -> object stream(컨테이너 obj 9)
                9 => Some((1, 300, 0)), // type 1 -> 파일 offset 300
                _ => None,
            }
        };
        let obj_stream_read = |_: &mut File, _: u64, _: i64| -> Option<Vec<u8>> { Some(b"<< /Count 4 >>".to_vec()) };
        let hooks =
            FastCountHooks { find_xref_offset: Some(&find_offset), parse_one_xref_stream: Some(&parse), xref_stream_find_entry: Some(&find_entry), read_from_obj_stream: Some(&obj_stream_read) };
        assert_eq!(fast_page_count_with(&pdf, &hooks), Some(4));
    }
}

mod test_loader_main_branches {
    use super::*;

    #[test]
    fn test_recursive_files_includes_file_symlinks_but_skips_directory_symlinks() {
        let dir = tmp();
        let real_file = dir.path().join("real.txt");
        fs::write(&real_file, "file").unwrap();
        let real_dir = dir.path().join("real_dir");
        fs::create_dir(&real_dir).unwrap();
        let nested_file = real_dir.join("nested.txt");
        fs::write(&nested_file, "nested").unwrap();
        std::os::unix::fs::symlink(&real_file, dir.path().join("file-link.txt")).unwrap();
        std::os::unix::fs::symlink(&real_dir, dir.path().join("dir-link")).unwrap();

        let mut expected = vec![real_file, dir.path().join("file-link.txt"), nested_file];
        expected.sort();
        assert_eq!(recursive_files(dir.path()), expected);
    }

    #[test]
    fn test_sample_files_skips_directory_symlinks() {
        let dir = tmp();
        let real_dir = dir.path().join("real_dir");
        fs::create_dir(&real_dir).unwrap();
        let first_file = real_dir.join("book.txt");
        fs::write(&first_file, "book").unwrap();
        std::os::unix::fs::symlink(&real_dir, dir.path().join("alias_dir")).unwrap();

        assert_eq!(sample_files(dir.path()), vec![("real_dir".to_string(), Some(first_file))]);
    }

    /// 정해진 inode의 `{updated_time}`을 ES가 돌려주는 상태로 `--reload`를 돌리고, 적재된 문서를 돌려준다.
    fn run_reload_with_stored_updated_time(stored_updated_time: &str, mtime_ts: u64) -> Value {
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        let file_path = dir.path().join("[author] book.txt");
        fs::write(&file_path, "content").unwrap();
        set_mtime(&file_path, mtime_ts);
        let inode = inode_of(&file_path);

        let store = FakeStore::new();
        store.update(|s| {
            s.created_meta.insert(inode, meta(&[("created_time", "2020-01-01T00:00:00"), ("updated_time", stored_updated_time)]));
        });
        let (code, _) = run_main(&["--reload", "book", &path_str(&file_path)], &env, &store);
        assert_eq!(code, 0);
        store.inserted_docs().remove(&inode).expect("문서가 적재되어야 한다")
    }

    #[test]
    fn test_ping_raises_exits() {
        // es.ping()가 예외 -> except에서 sys.exit(-1) (881-882). Rust는 접속 실패 종료 코드 255.
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        let mut store = FakeStore::new();
        store.ping_ok = false;
        let (code, _) = run_main(&["book", &path_str(dir.path())], &env, &store);
        assert_eq!(code, 255);
    }

    #[test]
    fn test_single_file_stat_oserror() {
        // get_stat가 OSError -> continue(917-918), 적재 실패 메시지(994).
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        let f = dir.path().join("[author] title.txt");
        fs::write(&f, "hello").unwrap();

        let stat_fails = |_: &Path| -> io::Result<Metadata> { Err(io::Error::other("stat failed")) };
        let hooks = RunHooks { stat: Some(&stat_fails), ..Default::default() };
        let store = FakeStore::new();
        let (code, _) = run_main_with_hooks(&["book", &path_str(&f)], &env, &store, &hooks);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_recursive_path_change_and_dedup() {
        // 경로 변경 감지(932-939), 중복 제거(961), 동기화 메시지(1004).
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        fs::write(dir.path().join("[author] book.txt"), "content").unwrap();

        let store = FakeStore::new();
        // 모든 inode를 '경로 변경됨'으로 표시 + batch에 없는 유령 inode(933 continue 경로)
        store.set_existing_paths(|inodes| {
            let mut paths: HashMap<i64, String> = inodes.iter().map(|ino| (*ino, "OLD/old.txt".to_owned())).collect();
            paths.insert(-1, "phantom/ghost.txt".to_owned());
            paths
        });
        store.update(|s| s.delete_by_paths_result = 1); // cleaned>0 -> 961
        let (code, out) = run_main(&["--recursive", "book", &path_str(dir.path())], &env, &store);
        assert_eq!(code, 0);
        assert!(out.contains("경로 변경 감지"));
        assert!(out.contains("경로 동기화"));
    }

    #[test]
    fn test_reload_preserves_existing_created_time() {
        // 강제 재적재도 ES에 저장된 최초 생성 시각을 유지한다.
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        let file_path = dir.path().join("[author] book.txt");
        fs::write(&file_path, "content").unwrap();
        let inode = inode_of(&file_path);

        let store = FakeStore::new();
        store.update(|s| {
            s.created_meta.insert(inode, meta(&[("created_time", "2020-01-01T00:00:00"), ("created_time_source", "statx_btime")]));
        });
        let (code, _) = run_main(&["--reload", "book", &path_str(&file_path)], &env, &store);
        assert_eq!(code, 0);
        let inserted = store.inserted_docs();
        assert_eq!(inserted[&inode]["created_time"], "2020-01-01T00:00:00");
        assert_eq!(inserted[&inode]["created_time_source"], "statx_btime");
    }

    #[test]
    fn test_reload_preserves_updated_time_when_file_unchanged() {
        // 파일 mtime이 저장된 updated_time보다 오래되면 재적재해도 updated_time을 유지한다.
        let doc = run_reload_with_stored_updated_time("2021-02-03T04:05:06", local_timestamp(2020));
        assert_eq!(doc["updated_time"], "2021-02-03T04:05:06");
    }

    #[test]
    fn test_reload_ignores_unparsable_stored_updated_time() {
        // 저장된 updated_time이 ISO 형식이 아니면 예외 없이 새 값으로 적재한다.
        let doc = run_reload_with_stored_updated_time("not-a-date", local_timestamp(2020));
        assert_ne!(doc["updated_time"], "not-a-date");
        assert_eq!(doc["created_time"], "2020-01-01T00:00:00");
    }

    #[test]
    fn test_reload_refreshes_updated_time_when_file_modified() {
        // 파일 mtime이 저장된 updated_time보다 새로우면 updated_time을 갱신한다.
        let doc = run_reload_with_stored_updated_time("2021-02-03T04:05:06", local_timestamp(2022));
        let updated = doc["updated_time"].as_str().unwrap();
        assert!(updated > "2021-02-03T04:05:06");
        assert_ne!(updated, "2021-02-03T04:05:06");
    }

    #[test]
    fn test_recursive_skipped() {
        // 변경 없는 기존 파일은 skip -> 건너뜀 메시지(1002).
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        let f = dir.path().join("[author] book.txt");
        fs::write(&f, "content").unwrap();
        let ino = inode_of(&f);
        let rel = f.strip_prefix(dir.path()).unwrap().to_string_lossy().into_owned();

        let store = FakeStore::new();
        store.set_existing_paths(move |_| HashMap::from([(ino, rel.clone())])); // 동일 경로 -> skip
        let (code, out) = run_main(&["--recursive", "book", &path_str(dir.path())], &env, &store);
        assert_eq!(code, 0);
        assert!(out.contains("중복 파일 건너뜀"));
    }

    #[test]
    fn test_directory_two_stage_empty_subdir_and_sync() {
        // 비재귀 2단계: 빈 하위디렉토리(1018), 샘플 동기화(1030), 2단계 skip(1038)/sync(1040).
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        let s1 = dir.path().join("s1");
        fs::create_dir(&s1).unwrap();
        fs::write(s1.join("[author] sample.txt"), "sample").unwrap();
        fs::create_dir(dir.path().join("s2")).unwrap(); // 빈 하위 디렉토리 -> 1018
        let f_r1 = dir.path().join("[author] root1.txt");
        fs::write(&f_r1, "root1").unwrap();
        fs::write(dir.path().join("[author] root2.txt"), "root2").unwrap();

        let ino_r1 = inode_of(&f_r1);
        let rel_r1 = f_r1.strip_prefix(dir.path()).unwrap().to_string_lossy().into_owned();

        let store = FakeStore::new();
        store.set_existing_paths(move |inodes| {
            inodes
                .iter()
                .map(|ino| {
                    if *ino == ino_r1 {
                        (*ino, rel_r1.clone()) // 변경 없음 -> skip(1038)
                    } else {
                        (*ino, "OLD/changed.txt".to_owned()) // 변경됨 -> sync(1030/1040)
                    }
                })
                .collect()
        });
        let (code, out) = run_main(&["book", &path_str(dir.path())], &env, &store);
        assert_eq!(code, 0);
        assert!(out.contains("(파일 없음)"));
        assert!(out.contains("경로 동기화"));
        assert!(out.contains("중복 파일 건너뜀"));
    }

    #[test]
    fn test_multiple_non_recursive_directories_skip_sample_indexing() {
        // 복수 디렉토리 인자는 샘플 파일 없이 각 대상의 직접 파일만 적재한다.
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        let d1 = dir.path().join("d1");
        let d2 = dir.path().join("d2");
        fs::create_dir(&d1).unwrap();
        fs::create_dir(&d2).unwrap();
        fs::create_dir(d1.join("nested")).unwrap();
        fs::create_dir(d2.join("nested")).unwrap();
        fs::write(d1.join("[author] direct1.txt"), "direct1").unwrap();
        fs::write(d2.join("[author] direct2.txt"), "direct2").unwrap();
        fs::write(d1.join("nested").join("[author] nested1.txt"), "nested1").unwrap();
        fs::write(d2.join("nested").join("[author] nested2.txt"), "nested2").unwrap();

        let store = FakeStore::new();
        let (code, out) = run_main(&["book", &path_str(&d1), &path_str(&d2)], &env, &store);

        assert_eq!(code, 0);
        let inserted_paths: HashSet<String> = store.inserted_docs().values().map(|v| v["file_path"].as_str().unwrap().to_owned()).collect();
        assert_eq!(inserted_paths, HashSet::from(["d1/[author] direct1.txt".to_owned(), "d2/[author] direct2.txt".to_owned()]));
        assert!(!out.contains("하위 디렉토리별 샘플 파일 등록"));
    }

    #[test]
    fn test_main_parses_batch_with_two_worker_threads() {
        // 한 배치의 일반 파일 파싱은 worker thread 2개에서 동시에 실행한다.
        let dir = tmp();
        let env = setup_loader_env(dir.path());
        fs::write(dir.path().join("[author] one.txt"), "one").unwrap();
        fs::write(dir.path().join("[author] two.txt"), "two").unwrap();

        let barrier = Barrier::new(2);
        let thread_ids: Mutex<HashSet<ThreadId>> = Mutex::new(HashSet::new());
        let root = dir.path().to_path_buf();
        let read_file_fake = |file_path: &Path, stat_result: Option<&Metadata>, _skip_text: bool| -> ReadOutcome {
            thread_ids.lock().unwrap().insert(thread::current().id());
            let name = file_path.file_name().unwrap().to_string_lossy().into_owned();
            barrier.wait();
            let st = stat_result.cloned().unwrap_or_else(|| fs::metadata(file_path).unwrap());
            let stem = file_path.file_stem().unwrap().to_string_lossy().into_owned();
            let doc = json!({
                "category": "_root", "title": stem, "author": "", "file_path": file_path.strip_prefix(&root).unwrap().to_string_lossy(), "file_type": "txt",
                "file_size": st.len(), "line_count": 1, "page_count": 0, "isbn": "", "summary": stem, "updated_time": "2026-08-11T00:00:00",
            });
            // 원본은 logging.warning("problem:%s")으로 사유를 남긴다.
            ReadOutcome { item: Some((st.ino() as i64, doc)), messages: vec![format!("problem:{name}")] }
        };
        let hooks = RunHooks { read_file: Some(&read_file_fake), ..Default::default() };
        let store = FakeStore::new();
        let (code, out) = run_main_with_hooks(&["--reload", "book", &path_str(dir.path())], &env, &store, &hooks);

        assert_eq!(code, 0);
        assert_eq!(thread_ids.lock().unwrap().len(), 2);
        assert!(out.contains("problem:[author] one.txt"));
        assert!(out.contains("problem:[author] two.txt"));
    }

    #[test]
    fn test_main_keeps_pdf_parsing_on_main_thread() {
        // PDF 파싱은 signal timeout 보호를 위해 main thread에서 유지한다.
        // 원본은 read_file을 바꿔치기한다. Rust는 PDF 단계 실행기가 도는 스레드를 기록한다(실행기는 PDF를 읽을 때만 불린다).
        struct ThreadRecorder {
            ids: Mutex<Vec<ThreadId>>,
        }
        impl StageRunner for ThreadRecorder {
            fn run(&self, _stage: Stage, _path: &Path) -> Result<(String, usize), StageError> {
                self.ids.lock().unwrap().push(thread::current().id());
                Err(StageError::Failed("테스트 실행기".to_owned()))
            }
        }

        let dir = tmp();
        let env = setup_loader_env(dir.path());
        fs::write(dir.path().join("[author] doc.pdf"), b"%PDF-1.4\n").unwrap();
        fs::write(dir.path().join("[author] note.txt"), "note").unwrap();

        let main_thread_id = thread::current().id();
        let recorder = ThreadRecorder { ids: Mutex::new(Vec::new()) };
        let store = FakeStore::new();
        let (code, _) = run_main_with(&["--reload", "book", &path_str(dir.path())], &env, &store, &recorder);

        assert_eq!(code, 0);
        let pdf_thread_ids = recorder.ids.lock().unwrap();
        assert!(!pdf_thread_ids.is_empty());
        assert!(pdf_thread_ids.iter().all(|id| *id == main_thread_id));
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

mod test_reencode_candidate_rejection {
    use super::*;

    // 후보 인코딩을 버리는 조건들.

    #[test]
    fn test_candidate_that_cannot_be_reencoded_is_skipped() {
        // 왕복 인코딩 자체가 실패하는 후보는 조용히 버린다 (256-257).
        // 디코딩은 되지만 인코딩은 실패하는 코덱(latin-1 디코딩, 인코딩 항상 실패)을 후보로 넣는다.
        let dir = tmp();
        let no_detect = |_: &[u8]| -> Option<Enc> { None };
        let hooks = TextHooks {
            detect: Some(&no_detect),
            encodings: Some(vec![Candidate::Builtin(Enc::Utf8), Candidate::Custom { name: "tmdecodeonly", decode: |data| data.iter().map(|&b| b as char).collect(), encode: |_| None }]),
            ..Default::default()
        };

        let f = dir.path().join("cannot_reencode.txt");
        let raw = b"\xc7\xd1\xb1\xdb"; // "한글".encode("euc-kr")
        fs::write(&f, raw).unwrap();

        let r = reencode_text_file_to_utf8_with(&f, false, None, Path::new("cannot_reencode.txt"), &hooks);
        assert!(!r.changed);
        assert_eq!(r.reason, "무손실 디코딩 가능한 인코딩을 찾지 못함");
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_mojibake_candidate_is_skipped() {
        // 왕복은 통과해도 mojibake 구간이 많으면 그 후보를 채택하지 않는다 (264).
        // 상위바이트가 대부분 고립되어 cp1252가 후보로 올라가지만, 그 안에 섞인 EUC-KR 2바이트 쌍이 한글로 되살아나므로 cp1252 해석은 틀린 것이다.
        let dir = tmp();
        let no_detect = |_: &[u8]| -> Option<Enc> { None };
        let hooks = TextHooks { detect: Some(&no_detect), ..Default::default() };

        let f = dir.path().join("mojibake_candidate.txt");
        let mut raw = b"a\xe9".repeat(60);
        raw.extend_from_slice(b" \xc7\xd1 \xb1\xdb \xc7\xd1 ");
        fs::write(&f, &raw).unwrap();

        // 전제 확인: cp1252 해석 결과에 한글로 복원되는 구간이 상한 이상 존재한다.
        // 이 바이트들은 0x80-0x9f 밖이라 cp1252와 latin-1 해석이 같다.
        let decoded: String = raw.iter().map(|&b| b as char).collect();
        assert!(count_mojibake_runs(&decoded) >= PDF_MOJIBAKE_RUN_LIMIT);

        let r = reencode_text_file_to_utf8_with(&f, false, None, Path::new("mojibake_candidate.txt"), &hooks);
        assert!(!r.changed);
        assert_eq!(r.reason, "무손실 디코딩 가능한 인코딩을 찾지 못함");
        assert_eq!(fs::read(&f).unwrap(), raw);
    }

    #[test]
    fn test_readback_failure_is_reported() {
        // 저장 직후 재읽기가 실패하면 성공으로 보고하지 않는다 (320-321).
        let dir = tmp();
        let f = dir.path().join("readback.txt");
        fs::write(&f, b"\xc7\xd1\xb1\xdb\x20\xba\xbb\xb9\xae\xc0\xd4\xb4\xcf\xb4\xd9\x0a").unwrap(); // "한글 본문입니다\n".encode("cp949")

        let calls = AtomicUsize::new(0);
        let flaky_read_bytes = |path: &Path| -> io::Result<Vec<u8>> {
            // 1회차는 원본 읽기, 2회차가 저장 후 재읽기
            if calls.fetch_add(1, Ordering::SeqCst) + 1 >= 2 {
                return Err(io::Error::other("EIO"));
            }
            fs::read(path)
        };
        let hooks = TextHooks { read_bytes: Some(&flaky_read_bytes), ..Default::default() };

        let r = reencode_text_file_to_utf8_with(&f, false, None, Path::new("readback.txt"), &hooks);
        assert!(!r.changed);
        assert!(r.reason.starts_with("저장 후 재읽기 실패"));
    }
}

mod test_read_from_text_replace_fallback {
    use super::*;

    // 엄격 디코딩이 전부 실패했을 때의 errors='replace' 폴백 루프.

    #[test]
    fn test_replace_candidate_without_letters_is_skipped() {
        // 글자가 하나도 없는 폴백 결과는 채택하지 않고 빈 본문으로 끝낸다 (372-373, 379-382).
        let dir = tmp();
        let no_detect = |_: &[u8]| -> Option<Enc> { None };
        let hooks = TextHooks { detect: Some(&no_detect), encodings: Some(vec![Candidate::Builtin(Enc::Utf16), Candidate::Builtin(Enc::Cp949)]), ..Default::default() };

        let f = dir.path().join("no_letters.txt");
        // 홀수 길이라 utf-16 엄격 디코딩은 실패하고, cp949는 문장부호만 만든다.
        let mut raw = b"\xa1\xa2".repeat(50);
        raw.push(0xa1);
        fs::write(&f, raw).unwrap();

        let read = read_from_text_with(&f, &hooks).unwrap_or_default();
        assert_eq!(read.summary, "");
        assert_eq!(read.raw_content, "");
        assert_eq!((read.line_count, read.page_count), (0, 0));
    }

    #[test]
    fn test_replace_read_error_is_swallowed() {
        // 폴백 읽기 자체가 I/O 오류로 죽어도 적재 루프를 세우지 않는다 (377-378).
        let dir = tmp();
        let no_detect = |_: &[u8]| -> Option<Enc> { None };
        let replace_fails = |_: &Path| -> io::Result<String> { Err(io::Error::other("EIO")) };
        let hooks =
            TextHooks { detect: Some(&no_detect), encodings: Some(vec![Candidate::Builtin(Enc::Utf16), Candidate::Builtin(Enc::Cp949)]), read_replace: Some(&replace_fails), ..Default::default() };

        let f = dir.path().join("io_error.txt");
        let mut raw = b"\xa1\xa2".repeat(50);
        raw.push(0xa1);
        fs::write(&f, raw).unwrap();

        let read = read_from_text_with(&f, &hooks).unwrap_or_default();
        assert_eq!(read.summary, "");
        assert_eq!(read.raw_content, "");
    }
}

mod test_count_mojibake_runs_guard {
    use super::*;

    #[test]
    fn test_unencodable_run_is_not_counted() {
        // latin-1로 되돌릴 수 없는 구간은 mojibake 판정에서 제외한다 (519-520).
        // 기본 정규식은 latin-1 범위만 잡으므로, 범위 밖 구간을 잡도록 바꿔 가드를 검증한다.
        assert_eq!(count_mojibake_runs_with("한글 본문입니다", "[가-힣]{2,}"), 0);
    }
}

mod test_zip_container_type {
    use super::*;

    #[test]
    fn test_non_zip_file_returns_none() {
        // ZIP이 아니면 판별 불가 (539-540).
        let dir = tmp();
        let f = dir.path().join("not_a_zip.bin");
        fs::write(&f, b"PK\x03\x04 but truncated").unwrap();

        assert_eq!(zip_container_type(&f), None);
    }

    #[test]
    fn test_unknown_zip_payload_returns_none() {
        // epub/docx/cbz 어디에도 해당하지 않는 ZIP은 판별 불가 (541).
        let dir = tmp();
        let f = dir.path().join("plain.zip");
        let mut zf = zip::ZipWriter::new(File::create(&f).unwrap());
        zf.start_file("notes.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zf.write_all(b"hello").unwrap();
        zf.finish().unwrap();

        assert_eq!(zip_container_type(&f), None);
    }
}

mod test_pikepdf_repair_round_trip {
    use super::*;

    #[test]
    fn test_repair_reopens_rewritten_buffer() {
        // pikepdf 재작성 결과를 메모리에서 그대로 다시 읽는다 (627-629).
        let dir = tmp();
        let pdf = dir.path().join("valid.pdf");
        write_blank_pdf(&pdf);

        let (text, page_count) = run_stage(Stage::Repair, &pdf).unwrap();
        assert_eq!(page_count, 1);
        assert_eq!(text, ""); // 빈 페이지라 텍스트 레이어가 없다
    }
}

mod test_pdfplumber_text_size_limit {
    use super::*;
    use std::cell::RefCell;

    struct MockPlumberPdf {
        read_pages: RefCell<Vec<usize>>,
    }

    impl PlumberDocument for MockPlumberPdf {
        fn page_count(&self) -> usize {
            5
        }
        fn extract_text(&self, idx: usize) -> Option<String> {
            self.read_pages.borrow_mut().push(idx);
            Some("가".repeat(3000))
        }
    }

    #[test]
    fn test_page_loop_stops_at_text_size() {
        // TEXT_SIZE를 채우면 남은 페이지를 읽지 않는다 (648).
        let dir = tmp();
        let pdf = dir.path().join("long.pdf");
        fs::write(&pdf, b"%PDF-1.4 mock").unwrap();

        let doc = MockPlumberPdf { read_pages: RefCell::new(Vec::new()) };
        let (text, page_count) = pdf_text_by_plumber_with(&pdf, &doc).unwrap();
        assert_eq!(page_count, 5);
        assert_eq!(*doc.read_pages.borrow(), [0, 1], "TEXT_SIZE를 넘긴 뒤에도 계속 읽었다");
        assert_eq!(text.chars().count(), 6000);
    }
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

mod test_pdf_repair_outcomes {
    use super::*;

    // 구조 복구가 성공도 타임아웃도 아닌 나머지 결과 (735-738).

    /// pypdfium2/pdfplumber(/pdftotext) 단계는 빈 결과로 끝나고, 복구 단계만 준비한 답을 돌려준다.
    struct RepairOnly {
        repair: (String, usize),
    }

    impl StageRunner for RepairOnly {
        fn run(&self, stage: Stage, _path: &Path) -> Result<(String, usize), StageError> {
            match stage {
                Stage::Repair => Ok(self.repair.clone()),
                _ => Ok((String::new(), 0)),
            }
        }
    }

    #[test]
    fn test_repair_result_is_recorded_as_damaged() {
        // 복구는 됐지만 mojibake면 damaged로 기록한다 (735-736).
        let dir = tmp();
        let pdf = dir.path().join("repair_damaged.pdf");
        fs::write(&pdf, b"%PDF-1.6 mock").unwrap();

        // "즐기는", "들어서", "필요한", "함께"를 euc-kr로 인코딩해 latin-1로 디코딩한 글자들
        let words: [&[u8]; 4] = [b"\xc1\xf1\xb1\xe2\xb4\xc2", b"\xb5\xe9\xbe\xee\xbc\xad", b"\xc7\xca\xbf\xe4\xc7\xd1", b"\xc7\xd4\xb2\xb2"];
        let damaged: String = words.iter().map(|w| format!(" {} ", w.iter().map(|&b| b as char).collect::<String>())).collect();
        let runner = RepairOnly { repair: (damaged, 7) };

        let read = read_from_pdf(&pdf, &runner);
        assert_eq!(read.page_count, 7);
        let recorded: Vec<(&str, &str)> = read.records.iter().map(|r| (r.stage.name(), r.outcome)).collect();
        assert!(recorded.contains(&("_pdf_text_by_pikepdf_repair", "damaged")), "{recorded:?}");
    }

    #[test]
    fn test_repair_result_is_recorded_as_empty() {
        // 복구본에 텍스트 레이어가 없으면 empty로 기록한다 (737-738).
        let dir = tmp();
        let pdf = dir.path().join("repair_empty.pdf");
        fs::write(&pdf, b"%PDF-1.6 mock").unwrap();

        let runner = RepairOnly { repair: ("   \n ".to_owned(), 12) };

        let read = read_from_pdf(&pdf, &runner);
        assert!(read.summary.trim().is_empty());
        assert_eq!(read.page_count, 12);
        let recorded: Vec<(&str, &str)> = read.records.iter().map(|r| (r.stage.name(), r.outcome)).collect();
        assert!(recorded.contains(&("_pdf_text_by_pikepdf_repair", "empty")), "{recorded:?}");
    }
}

mod test_read_file_page_count_timeout {
    use super::*;

    // 페이지 수만 뽑는 경로에서도 pypdf가 무한히 돌면 안 된다.

    fn pypdf_times_out(_: &Path) -> Result<usize, ParserTimeout> {
        Err(ParserTimeout { message: "pypdf/page_count".to_owned() })
    }

    #[test]
    fn test_skip_text_pdf_page_count_timeout() {
        // skip_text 경로의 pypdf 타임아웃 (1267-1268).
        let dir = tmp();
        let cfg = config_for(dir.path(), &dir.path().join("comics"));

        let pdf = dir.path().join("hang.pdf");
        fs::write(&pdf, b"%PDF-1.4 mock").unwrap();
        let fast_none = |_: &Path| -> Option<usize> { None };
        let hooks = ReadHooks { fast_page_count: Some(&fast_none), fallback_page_count: Some(&pypdf_times_out), ..Default::default() };

        let data = read_file_with_hooks(&cfg, &pdf, None, true, None, &InProcess, &Stat::new(), &hooks);

        assert_eq!(item_of(&data)["page_count"], 0);
        assert!(messages_of(&data).contains("페이지 수 추출 타임아웃"));
    }

    #[test]
    fn test_comics_pdf_page_count_timeout() {
        // comics 경로의 pypdf 타임아웃 (1312-1313).
        let dir = tmp();
        let cfg = config_for(dir.path(), dir.path());

        let pdf = dir.path().join("comic.pdf");
        fs::write(&pdf, b"%PDF-1.4 mock").unwrap();
        let fast_none = |_: &Path| -> Option<usize> { None };
        let hooks = ReadHooks { fast_page_count: Some(&fast_none), fallback_page_count: Some(&pypdf_times_out), ..Default::default() };

        let data = read_file_with_hooks(&cfg, &pdf, None, false, None, &InProcess, &Stat::new(), &hooks);

        assert_eq!(item_of(&data)["page_count"], 0);
        assert!(messages_of(&data).contains("페이지 수 추출 타임아웃"));
    }
}

mod test_read_file_reencode_mode {
    use super::*;

    // --reencode 모드에서 read_file이 적재 전에 파일을 UTF-8로 바꾼다 (1291-1297).

    #[test]
    fn test_converted_file_is_logged() {
        let dir = tmp();
        let mut cfg = config_for(dir.path(), &dir.path().join("comics"));
        cfg.reencode = true;
        cfg.reencode_dry_run = false;

        let f = dir.path().join("[작가] 제목.txt");
        let original = "한글 소설 본문입니다.\n둘째 줄\n";
        // original.encode("cp949")
        fs::write(&f, b"\xc7\xd1\xb1\xdb\x20\xbc\xd2\xbc\xb3\x20\xba\xbb\xb9\xae\xc0\xd4\xb4\xcf\xb4\xd9\x2e\x0a\xb5\xd1\xc2\xb0\x20\xc1\xd9\x0a").unwrap();

        let data = read(&cfg, &f, false);

        assert_eq!(String::from_utf8(fs::read(&f).unwrap()).unwrap(), original);
        assert!(data.item.is_some());
        assert!(messages_of(&data).contains("UTF-8 재인코딩:"));
    }

    #[test]
    fn test_skipped_file_is_logged_with_reason() {
        // 변환하지 못한 파일은 사유를 남긴다 (1296-1297).
        let dir = tmp();
        let mut cfg = config_for(dir.path(), &dir.path().join("comics"));
        cfg.reencode = true;
        cfg.reencode_dry_run = false;

        let f = dir.path().join("broken.txt");
        let raw = [0x80u8, 0x81, 0xFE, 0xFF, 0x00, 0x9C, 0x90].repeat(40);
        fs::write(&f, &raw).unwrap();

        let data = read(&cfg, &f, false);

        assert_eq!(fs::read(&f).unwrap(), raw);
        assert!(messages_of(&data).contains("UTF-8 재인코딩 건너뜀:"));
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
