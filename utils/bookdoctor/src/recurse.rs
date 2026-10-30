//! 재귀 검사: ZIP 계열(`zip`·`docx`·`hwpx`) 안의 컨테이너 멤버(ZIP, PDF, RTF, Word·HWP OLE)를 임시 파일로 풀어 같은 분석기로 검사한다.
//!
//! 한 검사 전체가 깊이(`MAX_DEPTH`), 추출 바이트 합계(`max_total`), 추출 파일 수(`MAX_FILES`)를 공유한다. 상한에 걸리면 `incomplete`다.
//! 임시 파일 이름에 멤버 이름을 쓰지 않고, 임시 디렉터리는 소유자만 접근하며 `Drop`으로 지운다.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use corpus_common::reader::open_zip;
use ole_reader::{Limits as OleLimits, OleFile};
use scan_common::{Category, FileReport, Severity};
use scan_common::sniff::{Sniffed, sniff};
use scan_common::text::sanitize_display;

use crate::cli::Options;
use crate::epub::security::directory_limit_note;
use crate::scan::scan_file;

/// 바깥 파일 아래로 내려가는 최대 단계.
pub const MAX_DEPTH: u8 = 3;
/// 한 검사에서 풀어 검사하는 중첩 파일 수의 상한.
pub const MAX_FILES: u32 = 64;
/// 중첩 멤버 하나를 메모리에 읽는 최대 바이트.
const MAX_MEMBER: u64 = 64 * 1024 * 1024;
/// 멤버 종류를 가르려고 읽는 앞부분(PDF 서명은 앞 1KiB 안 어디든 올 수 있다).
const SNIFF_BYTES: u64 = 1024;
/// 훑는 멤버 수의 상한.
const MAX_ENTRIES: usize = 20_000;
/// 바깥 보고서에 합치는 중첩 지표 수의 상한.
const MAX_MERGED_FINDINGS: usize = 500;

static SCRATCH_COUNTER: AtomicU32 = AtomicU32::new(0);

/// 소유자만 접근하는 임시 디렉터리. 파일 이름은 `m<번호>.<확장자>`이고 멤버 이름과 무관하다. `Drop`이 디렉터리를 통째로 지운다.
pub struct ScratchDir {
    path: PathBuf,
    next: AtomicU32,
}

impl ScratchDir {
    pub fn new() -> io::Result<ScratchDir> {
        let base = std::env::temp_dir();
        for _ in 0..100 {
            let path = base.join(format!("bookdoctor-{}-{}", std::process::id(), SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed)));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(ScratchDir { path, next: AtomicU32::new(0) }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free scratch directory name"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `data`를 새 파일(0600)에 쓰고 그 경로를 돌려준다. `ext`는 영숫자 5자 이하만 쓰고 아니면 `bin`이다.
    pub fn write(&self, data: &[u8], ext: &str) -> io::Result<PathBuf> {
        let ext = if !ext.is_empty() && ext.len() <= 5 && ext.bytes().all(|b| b.is_ascii_alphanumeric()) { ext } else { "bin" };
        let file = self.path.join(format!("m{}.{ext}", self.next.fetch_add(1, Ordering::Relaxed)));
        fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&file)?.write_all(data)?;
        Ok(file)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// 한 검사 전체가 공유하는 재귀 예산.
struct State {
    depth: u8,
    bytes_left: u64,
    files_left: u32,
}

thread_local! {
    /// 바깥 `scan_file`이 만들고 중첩 `scan_file`이 이어 쓴다. 한 파일의 검사는 한 스레드에서 끝나므로 스레드 로컬이면 된다.
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// 바깥 검사가 가진 예산. 파괴되면(panic 포함) 예산을 비운다.
pub struct RootGuard(bool);

/// 최상위 `scan_file`이 부른다. 이미 예산이 있으면(중첩 검사) 아무것도 하지 않는다.
pub fn begin(opts: &Options) -> RootGuard {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_some() {
            return RootGuard(false);
        }
        *s = Some(State { depth: 0, bytes_left: opts.max_total_mb.saturating_mul(1024 * 1024), files_left: MAX_FILES });
        RootGuard(true)
    })
}

impl Drop for RootGuard {
    fn drop(&mut self) {
        if self.0 {
            STATE.with(|s| *s.borrow_mut() = None);
        }
    }
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

/// 앞부분으로 가른 중첩 멤버 종류.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Zip,
    Pdf,
    Rtf,
    Ole,
}

/// `name` 멤버의 앞 `max`바이트를 읽는다.
fn read_prefix(zip: &mut corpus_common::reader::Zip, name: &str, max: u64) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    zip.by_name(name).ok()?.take(max).read_to_end(&mut data).ok()?;
    Some(data)
}

/// 임시 파일의 ZIP이 어떤 문서 패키지인지 멤버 이름 구조로 가려 맞는 확장자를 모두 돌려준다. 멤버 이름(바깥 ZIP의 이름)은 쓰지 않는다.
/// 표지: DOCX는 `word/document.xml`, HWPX는 `Contents/content.hpf`, EPUB은 `META-INF/container.xml`이다.
/// HWPX도 `container.xml`과 `mimetype`을 갖는다. 그래서 EPUB은 `container.xml`이 있고 (`mimetype`이 없거나, `application/epub+zip`이거나,
/// `container.xml`이 `.opf`/`oebps-package`를 가리킬 때)로 본다. 정상 HWPX는 `application/hwp+zip`과 `hwpml-package`라 EPUB이 아니다.
/// 둘 이상 맞으면 한쪽 분석기만으로는 다른 쪽 지표가 가려지므로 모두 돌려주고, 하나도 안 맞으면 `zip`이다.
fn zip_extensions(file: &Path) -> Vec<&'static str> {
    let Some(mut zip) = open_zip(file) else { return vec!["zip"] };
    let (mut docx, mut hwpx, mut container, mut has_mimetype) = (false, false, false, false);
    for name in zip.file_names().take(MAX_ENTRIES).flatten() {
        match name.as_ref() {
            "word/document.xml" => docx = true,
            "Contents/content.hpf" => hwpx = true,
            "META-INF/container.xml" => container = true,
            "mimetype" => has_mimetype = true,
            _ => {}
        }
    }
    let epub = container && {
        let mime = if has_mimetype { read_prefix(&mut zip, "mimetype", 64) } else { None };
        let says_epub = mime.as_ref().is_none_or(|m| m.trim_ascii().eq_ignore_ascii_case(b"application/epub+zip"));
        let xml = read_prefix(&mut zip, "META-INF/container.xml", 4096).map(|d| String::from_utf8_lossy(&d).to_lowercase()).unwrap_or_default();
        says_epub || xml.contains(".opf") || xml.contains("oebps-package")
    };
    let mut found: Vec<&'static str> = [(docx, "docx"), (hwpx, "hwpx"), (epub, "epub")].into_iter().filter(|(on, _)| *on).map(|(_, e)| e).collect();
    if found.is_empty() {
        found.push("zip");
    }
    found
}

/// 임시 파일의 현재 확장자.
fn first_ext_current(file: &Path) -> String {
    file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
}

/// 임시 파일의 OLE가 검사 대상 문서인지 스트림 구조로 가려 확장자를 돌려준다. `FileHeader`는 HWP, `WordDocument`·`EncryptedPackage`는 Word·암호화 OOXML이다.
/// `vbaProject.bin` 같은 일반 OLE는 `None`이다(DOC 분석기에 넣으면 `FORMAT_MISMATCH` 소음이 생긴다).
fn ole_extension(file: &Path) -> Option<&'static str> {
    let ole = OleFile::open(file, &OleLimits::default()).ok()?;
    if ole.find("FileHeader").is_some() {
        Some("hwp")
    } else if ole.find("WordDocument").is_some() || ole.find("EncryptedPackage").is_some() {
        Some("doc")
    } else {
        None
    }
}

fn kind_of(head: &[u8]) -> Option<(Kind, &'static str)> {
    match sniff(head) {
        Sniffed::Zip => Some((Kind::Zip, "zip")),
        Sniffed::Pdf => Some((Kind::Pdf, "pdf")),
        Sniffed::Rtf => Some((Kind::Rtf, "rtf")),
        Sniffed::Ole => Some((Kind::Ole, "ole")),
        _ => None,
    }
}

/// ZIP 계열 파일의 컨테이너 멤버를 재귀 검사하고 결과를 `report`에 합친다. `--no-recurse`면 아무것도 하지 않는다.
/// 종류는 멤버 이름이 아니라 내용으로 정한다. 예산(깊이·파일 수·바이트)은 멤버를 읽기 전에 확인하고 읽는 양만큼 차감한다.
pub fn scan_zip_members(path: &Path, opts: &Options, report: &mut FileReport) {
    if !opts.recurse || with_state(|_| ()).is_none() {
        return;
    }
    if directory_limit_note(path).is_some() {
        return;
    }
    let Some(mut zip) = open_zip(path) else { return };
    let mut scratch: Option<ScratchDir> = None;
    let mut merged = 0usize;
    let mut depth_noted = false;
    for i in 0..zip.len().min(MAX_ENTRIES) {
        let (name, size, head) = {
            let Ok(entry) = zip.by_index(i) else { continue };
            if entry.is_dir() || entry.encrypted() || entry.size() == 0 {
                continue;
            }
            let name = entry.name().map(|n| n.into_owned()).unwrap_or_else(|_| "<invalid-name>".to_owned());
            let size = entry.size();
            let mut head = Vec::new();
            if entry.take(SNIFF_BYTES).read_to_end(&mut head).is_err() {
                continue;
            }
            (name, size, head)
        };
        let Some((kind, first_ext)) = kind_of(&head) else { continue };
        let shown = sanitize_display(&name, 120, Some('…'));
        if size > MAX_MEMBER {
            report.mark_incomplete(&format!("{shown}: nested member larger than the {} MiB recursion limit", MAX_MEMBER / (1024 * 1024)));
            continue;
        }
        // 읽기 전에 예산을 확인하고 차감한다. OLE는 대상인지 구조를 본 뒤에야 파일 수를 차감한다(대상이 아닌 OLE가 파일 수를 쓰지 않게).
        let gate = with_state(|s| {
            // OLE는 대상인지 구조를 본 뒤에 깊이를 적용한다(깊은 곳의 `vbaProject.bin` 같은 일반 OLE가 미완료 사유가 되지 않게).
            if s.depth >= MAX_DEPTH && kind != Kind::Ole {
                Err("depth")
            } else if s.files_left == 0 {
                Err("files")
            } else if size > s.bytes_left {
                Err("bytes")
            } else {
                s.bytes_left -= size;
                if kind != Kind::Ole {
                    s.files_left -= 1;
                }
                Ok(())
            }
        });
        match gate {
            Some(Ok(())) => {}
            Some(Err("depth")) => {
                if !depth_noted {
                    depth_noted = true;
                    report.mark_incomplete(&format!("{shown}: recursion depth limit ({MAX_DEPTH}) reached; nested content below was not scanned"));
                }
                continue;
            }
            Some(Err("files")) => {
                report.mark_incomplete(&format!("{shown}: recursion file limit ({MAX_FILES}) reached; the remaining members were not scanned"));
                break;
            }
            _ => {
                report.mark_incomplete(&format!("{shown}: recursion byte budget exhausted; the remaining members were not scanned"));
                break;
            }
        }
        let mut data = Vec::new();
        let read_ok = zip.by_index(i).is_ok_and(|entry| entry.take(size).read_to_end(&mut data).is_ok());
        if !read_ok {
            report.mark_incomplete(&format!("{shown}: nested member could not be read"));
            continue;
        }
        if scratch.is_none() {
            scratch = ScratchDir::new().ok();
        }
        let Some(dir) = &scratch else {
            report.mark_incomplete("recursion skipped: scratch directory could not be created");
            return;
        };
        let written = dir.write(&data, first_ext);
        drop(data);
        let Ok(mut file) = written else {
            report.mark_incomplete(&format!("{shown}: nested member could not be written for scanning"));
            continue;
        };
        // 종류는 임시 파일의 구조로 정한다. 확장자를 바꿀 뿐 내용은 그대로다.
        let final_exts: Vec<&'static str> = match kind {
            Kind::Zip => zip_extensions(&file),
            Kind::Ole => ole_extension(&file).into_iter().collect(),
            Kind::Pdf | Kind::Rtf => vec![first_ext],
        };
        if final_exts.is_empty() {
            let _ = fs::remove_file(&file);
            continue;
        }
        if kind == Kind::Ole && with_state(|s| s.depth >= MAX_DEPTH) == Some(true) {
            let _ = fs::remove_file(&file);
            if !depth_noted {
                depth_noted = true;
                report.mark_incomplete(&format!("{shown}: recursion depth limit ({MAX_DEPTH}) reached; nested content below was not scanned"));
            }
            continue;
        }
        if kind == Kind::Ole && with_state(|s| if s.files_left == 0 { false } else { s.files_left -= 1; true }) != Some(true) {
            let _ = fs::remove_file(&file);
            report.mark_incomplete(&format!("{shown}: recursion file limit ({MAX_FILES}) reached; the remaining members were not scanned"));
            break;
        }
        if final_exts.len() > 1 {
            report.push(Category::Security, Severity::Medium, "PACKAGE_POLYGLOT", &shown, &format!("member matches several document package formats: {}", final_exts.join(", ")));
        }
        // 안쪽 파일에는 남은 바이트 예산을 `--max-total-mb`로 넘긴다.
        let remaining_mb = with_state(|s| s.bytes_left / (1024 * 1024)).unwrap_or(0).max(1);
        let nested_opts = Options { only: Vec::new(), max_total_mb: remaining_mb.min(opts.max_total_mb), ..opts.clone() };
        // 맞는 분석기마다 한 번씩 본다. 같은 지표가 두 번 합쳐지지 않게 (코드, 위치, 설명)으로 거른다.
        let mut seen: HashSet<(String, String, String)> = HashSet::new();
        for ext in final_exts {
            if ext != first_ext_current(&file) {
                let renamed = file.with_extension(ext);
                if fs::rename(&file, &renamed).is_ok() {
                    file = renamed;
                }
            }
            with_state(|s| s.depth += 1);
            let inner = scan_file(&file, &nested_opts);
            with_state(|s| s.depth -= 1);
            for f in inner.findings {
                if !seen.insert((f.code.clone(), f.location.clone(), f.detail.clone())) {
                    continue;
                }
                if merged >= MAX_MERGED_FINDINGS {
                    report.mark_incomplete(&format!("more than {MAX_MERGED_FINDINGS} nested findings; the rest were not listed"));
                    break;
                }
                merged += 1;
                let location = if f.location.is_empty() { shown.clone() } else { format!("{shown}!{}", f.location) };
                report.push(f.category, f.severity, &f.code, &location, &f.detail);
            }
            for reason in inner.reasons {
                let note = format!("{shown}: {reason}");
                if !report.reasons.contains(&note) {
                    report.mark_incomplete(&note);
                }
            }
        }
        let _ = fs::remove_file(&file);
    }
}
