//! 책 파일을 고쳐 쓰는 흐름(`--sanitize`의 EPUB·PDF, `--fix-xhtml`)이 함께 쓰는 단계.
//! 고친 바이트를 임시 파일로 다시 검사하고, 원본을 휴지통에 둔 채 같은 inode에 덮어쓴다.
//! 무엇을 고칠지와 재검사 결과의 판정은 흐름마다 다르므로 각 모듈에 둔다.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::option_model::Options;
use crate::output_formatter::show;
use crate::{Category, FileReport, Severity};
use doc_checker::CheckOptions;
use doc_checker::format_dispatcher::scan_file;

fn unique_trash_path(dir: &Path, file_name: &str) -> PathBuf {
    let first = dir.join(file_name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = file_name.rsplit_once('.').map_or((file_name, String::new()), |(s, e)| (s, format!(".{e}")));
    (1..).map(|n| dir.join(format!("{stem}.{n}{ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// 원본을 휴지통에 두고, 같은 inode에 새 내용을 덮어쓰고, mtime을 되돌린다.
pub(crate) fn apply(path: &Path, bytes: &[u8], trash_dir: &Path) -> io::Result<PathBuf> {
    let meta = fs::metadata(path)?;
    let mtime = meta.modified()?;
    fs::create_dir_all(trash_dir)?;
    let trash = unique_trash_path(trash_dir, &path.file_name().map_or_else(|| "book.epub".into(), |n| n.to_string_lossy().into_owned()));
    fs::copy(path, &trash)?;
    if fs::metadata(&trash)?.len() != meta.len() {
        return Err(io::Error::other("휴지통 사본의 크기가 원본과 다르다"));
    }
    fs::OpenOptions::new().write(true).open(&trash)?.set_modified(mtime)?;
    let mut f = fs::OpenOptions::new().write(true).open(path)?;
    f.write_all(bytes)?;
    f.set_len(bytes.len() as u64)?;
    f.sync_all()?;
    f.set_modified(mtime)?;
    if fs::metadata(path)?.ino() != meta.ino() {
        return Err(io::Error::other("inode가 바뀌었다"));
    }
    Ok(trash)
}

/// 보안 지표(HIGH 이상)가 있는가. 정리·수리 흐름이 대상을 고르고 결과를 거를 때 쓴다.
pub(crate) fn has_serious_security(report: &FileReport) -> bool {
    report.findings.iter().any(|f| f.category == Category::Security && f.severity >= Severity::High)
}

/// 임시 파일을 `O_EXCL`(`create_new`)과 모드 0600으로 만든다. 이름이 겹치거나 링크가 이미 있으면 다른 이름으로 다시 시도한다.
/// 파일을 새로 만든 경우에만 쓰므로 미리 심어 둔 심볼릭 링크를 따라가지 않는다. 검사는 확장자로 형식을 가르므로 `ext`를 맞춘다.
fn create_private_temp(bytes: &[u8], ext: &str) -> io::Result<PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    for _ in 0..100 {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let path = dir.join(format!("bookdoctor-sanitize-{}-{nanos:x}-{}.{ext}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
        match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(mut f) => {
                f.write_all(bytes)?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("임시 파일 이름을 정하지 못했다"))
}

/// 고친 바이트를 확장자 `ext`의 임시 파일에 써서 검사하고 보고서를 돌려준다. 판정은 호출자가 한다.
pub(crate) fn rescan_bytes(bytes: &[u8], ext: &str, opts: &CheckOptions) -> Result<FileReport, String> {
    let tmp = create_private_temp(bytes, ext).map_err(|e| e.to_string())?;
    let report = scan_file(&tmp, opts);
    let _ = fs::remove_file(&tmp);
    Ok(report)
}

/// 원본을 휴지통에 두고 새 내용으로 바꾼 뒤 결과를 한 줄 쓴다. 바꿨으면 `true`, 실패했으면 `FAIL` 줄을 쓰고 `false`다.
pub(crate) fn apply_and_report(path: &Path, bytes: &[u8], opts: &Options, shown: &str, out: &mut dyn Write) -> io::Result<bool> {
    match apply(path, bytes, &opts.trash_dir()) {
        Ok(saved) => {
            writeln!(out, "  - 원본 보관 {}", show(&saved.to_string_lossy()))?;
            Ok(true)
        }
        Err(e) => {
            writeln!(out, "FAIL {shown}: {}", show(&e.to_string()))?;
            Ok(false)
        }
    }
}
