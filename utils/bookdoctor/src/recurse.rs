//! 재귀 검사: ZIP 계열(`zip`·`docx`·`hwpx`) 안의 컨테이너 멤버(ZIP, PDF, RTF, Word·HWP OLE)를 임시 파일로 풀어 같은 분석기로 검사한다.
//!
//! 한 검사 전체가 깊이(`MAX_DEPTH`), 추출 바이트 합계(`max_total`), 추출 파일 수(`MAX_FILES`)를 공유한다. 상한에 걸리면 `incomplete`다.
//! 임시 파일 이름에 멤버 이름을 쓰지 않고, 임시 디렉터리는 소유자만 접근하며 `Drop`으로 지운다.

use std::cell::RefCell;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use corpus_common::reader::open_zip;
use ole_reader::{Limits as OleLimits, OleFile};
use scan_common::FileReport;
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
/// 중첩 ZIP의 확장자로 허용하는 이름(그 밖에는 `zip`). 멤버 이름의 나머지는 경로에 쓰지 않는다.
const ZIP_EXTENSIONS: &[&str] = &["docx", "docm", "dotx", "dotm", "hwpx", "epub", "zip"];

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

/// 멤버의 앞부분으로 중첩 검사에 쓸 임시 파일 확장자를 정한다. 재귀 대상이 아니면 `None`이다.
/// OLE는 Word·HWP 구조(`WordDocument`, `EncryptedPackage`, `FileHeader`)가 있는 것만 대상이다(`vbaProject.bin` 같은 일반 OLE는 제외).
fn nested_extension(head: &[u8], member_name: &str, full: impl FnOnce() -> Option<Vec<u8>>) -> Option<&'static str> {
    match sniff(head) {
        Sniffed::Zip => {
            let ext = member_name.rsplit(['/', '\\']).next().and_then(|f| f.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase());
            Some(ZIP_EXTENSIONS.iter().copied().find(|z| Some(*z) == ext.as_deref()).unwrap_or("zip"))
        }
        Sniffed::Pdf => Some("pdf"),
        Sniffed::Rtf => Some("rtf"),
        Sniffed::Ole => {
            let ole = OleFile::from_bytes(full()?, &OleLimits::default()).ok()?;
            if ole.find("FileHeader").is_some() {
                Some("hwp")
            } else if ole.find("WordDocument").is_some() || ole.find("EncryptedPackage").is_some() {
                Some("doc")
            } else {
                None
            }
        }
        _ => None,
    }
}

/// ZIP 계열 파일의 컨테이너 멤버를 재귀 검사하고 결과를 `report`에 합친다. `--no-recurse`면 아무것도 하지 않는다.
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
    let nested_opts = Options { only: Vec::new(), ..opts.clone() };
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
        // 컨테이너가 아니면 재귀하지 않는다. OLE만 구조를 보려고 통째로 읽는다.
        let member_cap = size.min(MAX_MEMBER);
        let mut full_cache: Option<Vec<u8>> = None;
        let read_full = |zip: &mut corpus_common::reader::Zip| -> Option<Vec<u8>> {
            let entry = zip.by_index(i).ok()?;
            let mut data = Vec::new();
            entry.take(member_cap).read_to_end(&mut data).ok()?;
            Some(data)
        };
        let ext = nested_extension(&head, &name, || {
            full_cache = read_full(&mut zip);
            full_cache.clone()
        });
        let Some(ext) = ext else { continue };
        let shown = sanitize_display(&name, 120, Some('…'));
        if size > MAX_MEMBER {
            report.mark_incomplete(&format!("{shown}: nested member larger than the {} MiB recursion limit", MAX_MEMBER / (1024 * 1024)));
            continue;
        }
        let allowed = with_state(|s| {
            if s.depth >= MAX_DEPTH {
                Err("depth")
            } else if s.files_left == 0 {
                Err("files")
            } else if size > s.bytes_left {
                Err("bytes")
            } else {
                s.files_left -= 1;
                s.bytes_left -= size;
                Ok(())
            }
        });
        match allowed {
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
        let data = match full_cache.take().or_else(|| read_full(&mut zip)) {
            Some(d) => d,
            None => {
                report.mark_incomplete(&format!("{shown}: nested member could not be read"));
                continue;
            }
        };
        if scratch.is_none() {
            scratch = ScratchDir::new().ok();
        }
        let Some(dir) = &scratch else {
            report.mark_incomplete("recursion skipped: scratch directory could not be created");
            return;
        };
        let Ok(file) = dir.write(&data, ext) else {
            report.mark_incomplete(&format!("{shown}: nested member could not be written for scanning"));
            continue;
        };
        drop(data);
        with_state(|s| s.depth += 1);
        let inner = scan_file(&file, &nested_opts);
        with_state(|s| s.depth -= 1);
        let _ = fs::remove_file(&file);
        for f in inner.findings {
            if merged >= MAX_MERGED_FINDINGS {
                report.mark_incomplete(&format!("more than {MAX_MERGED_FINDINGS} nested findings; the rest were not listed"));
                break;
            }
            merged += 1;
            let location = if f.location.is_empty() { shown.clone() } else { format!("{shown}!{}", f.location) };
            report.push(f.category, f.severity, &f.code, &location, &f.detail);
        }
        for reason in inner.reasons {
            report.mark_incomplete(&format!("{shown}: {reason}"));
        }
    }
}
