//! EPUB를 잠그고 원본을 보존한 뒤 새 파일로 제자리에서 바꾼다. 쓰다 실패하면 원본으로 되돌린다.

use std::fs::{File, FileTimes, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use super::stage::{stage_cover, stage_cover_removal, stage_illustration};
use super::{CHANGED, CoverError, EpubCoverHooks, FResult, Fail, SAVING, inspect_archive, normalize_image_with, read_dimensions};

type Precheck<'a> = &'a dyn Fn(&mut ZipArchive<&File>) -> FResult<()>;
type Stage<'a> = &'a mut dyn FnMut(&File, &Path) -> FResult<()>;

/// 임시 파일. 버릴 때 지운다(Python `finally: unlink(missing_ok=True)`).
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn create_temp(dir: &Path, prefix: &str, hooks: &EpubCoverHooks) -> std::io::Result<TempFile> {
    // `tempfile.mkstemp`처럼 다른 사용자가 읽지 못하는 0600 파일을 새로 만든다.
    for _ in 0..16 {
        let path = dir.join(format!("{prefix}{}.tmp", &hooks.uuid_hex()[..16]));
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(_) => return Ok(TempFile(path)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "임시 파일 이름을 만들 수 없습니다"))
}

fn copy_all(source: &mut File, output: &mut File) -> std::io::Result<()> {
    source.seek(SeekFrom::Start(0))?;
    std::io::copy(source, output)?;
    Ok(())
}

/// `output`을 `reader` 내용으로 바꾼다(처음부터 쓰고 남는 뒷부분을 자른 뒤 `fsync`).
fn overwrite(target: &mut File, reader: &mut File, hooks: &EpubCoverHooks) -> std::io::Result<()> {
    target.seek(SeekFrom::Start(0))?;
    std::io::copy(reader, target)?;
    let end = target.stream_position()?;
    target.set_len(end)?;
    target.flush()?;
    hooks.sync(target)
}

/// EPUB를 잠그고 원본을 보존한 뒤 `stage`가 만든 새 파일로 제자리에서 바꾼다. 쓰다 실패하면 원본으로 되돌린다.
///
/// `backup_name`을 주면(휴지통 보존) 그 이름으로 두고, 같은 이름이 이미 있으면 덮어쓰지 않고 처음 원본을 지킨다(PDF 편집과 같다).
/// 주지 않으면 편집마다 고유한 이름으로 둔다(표지 관리).
pub(crate) fn rewrite_epub(
    path: &Path,
    expected_revision: &str,
    backup_dir: &Path,
    backup_name: Option<&str>,
    stage: Stage<'_>,
    precheck: Option<Precheck<'_>>,
    hooks: &EpubCoverHooks,
) -> Result<Value, CoverError> {
    rewrite_inner(path, expected_revision, backup_dir, backup_name, stage, precheck, hooks).map_err(Fail::into_cover)
}

fn rewrite_inner(path: &Path, expected_revision: &str, backup_dir: &Path, backup_name: Option<&str>, stage: Stage<'_>, precheck: Option<Precheck<'_>>, hooks: &EpubCoverHooks) -> FResult<Value> {
    let mut source = OpenOptions::new().read(true).write(true).open(path)?;
    if let Err(err) = hooks.lock_exclusive(&source) {
        return Err(if err.kind() == std::io::ErrorKind::WouldBlock { CoverError::new(SAVING).into() } else { err.into() });
    }
    if hooks.revision_of(path) != expected_revision {
        return Err(CoverError::new(CHANGED).into());
    }
    if let Some(precheck) = precheck {
        let mut archive = ZipArchive::new(&source)?;
        precheck(&mut archive)?;
    }
    let original_stat = std::fs::metadata(path)?;
    std::fs::create_dir_all(backup_dir)?;
    let named = backup_name.is_some();
    let backup = backup_dir.join(match backup_name {
        Some(name) => name.to_string(),
        None => format!("{}-{}.epub", hex(&Sha256::digest(format!("{}:{expected_revision}", path.display()).as_bytes())), hooks.uuid_hex()),
    });
    let mut backup_created = true;
    source.seek(SeekFrom::Start(0))?;
    match OpenOptions::new().write(true).create_new(true).open(&backup) {
        Ok(mut output) => {
            copy_all(&mut source, &mut output)?;
            output.flush()?;
            hooks.sync(&output)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && named => backup_created = false,
        Err(err) => return Err(err.into()),
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let staged = create_temp(parent, ".cover-", hooks)?;
    // 이미 있던 백업은 처음 원본이라 지금 원본이 아니다. 실패 때 되돌릴 지금 원본은 따로 복사해 둔다.
    let mut rollback_temp: Option<TempFile> = None;
    if !backup_created {
        let temp = create_temp(parent, ".cover-rollback-", hooks)?;
        let mut output = OpenOptions::new().write(true).truncate(true).open(&temp.0)?;
        copy_all(&mut source, &mut output)?;
        output.flush()?;
        hooks.sync(&output)?;
        rollback_temp = Some(temp);
    }
    let rollback_path = rollback_temp.as_ref().map_or(backup.as_path(), |t| t.0.as_path()).to_path_buf();
    let outcome = (|| -> FResult<()> {
        stage(&source, &staged.0)?;
        if hooks.revision_of(path) != expected_revision {
            return Err(CoverError::new(CHANGED).into());
        }
        let written = File::open(&staged.0).and_then(|mut reader| overwrite(&mut source, &mut reader, hooks));
        if let Err(err) = written {
            let restored = File::open(&rollback_path).and_then(|mut reader| overwrite(&mut source, &mut reader, hooks));
            return Err(restored.err().unwrap_or(err).into());
        }
        Ok(())
    })();
    // 수정 시각을 원래대로 되돌려 같은 inode와 mtime을 지킨다(성공·실패와 무관).
    if let (Ok(accessed), Ok(modified)) = (original_stat.accessed(), original_stat.modified()) {
        let _ = source.set_times(FileTimes::new().set_accessed(accessed).set_modified(modified));
    }
    drop(staged);
    drop(rollback_temp);
    outcome?;
    Ok(json!({
        "revision": hooks.revision_of(path),
        "backup_path": backup.to_string_lossy(),
        "backup_created": backup_created,
        "file_size": std::fs::metadata(path)?.len(),
        "book_id": original_stat.ino(),
    }))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn add_cover(path: &Path, image: &[u8], expected_revision: &str, backup_dir: &Path, replace_existing: bool) -> Result<Value, CoverError> {
    add_cover_with(path, image, expected_revision, backup_dir, replace_existing, &EpubCoverHooks::default())
}

pub fn add_cover_with(path: &Path, image: &[u8], expected_revision: &str, backup_dir: &Path, replace_existing: bool, hooks: &EpubCoverHooks) -> Result<Value, CoverError> {
    add_cover_backup_with(path, image, expected_revision, backup_dir, replace_existing, None, hooks)
}

/// 백업 이름을 정하는 표지 추가·교체. 같은 이름이 이미 있으면 처음 원본을 지킨다.
pub fn add_cover_backup_with(
    path: &Path,
    image: &[u8],
    expected_revision: &str,
    backup_dir: &Path,
    replace_existing: bool,
    backup_name: Option<&str>,
    hooks: &EpubCoverHooks,
) -> Result<Value, CoverError> {
    let image = normalize_image_with(image, Some(600), hooks)?;
    let (width, height) = read_dimensions(&image).map_err(|_| CoverError::new("표지 이미지 형식을 읽을 수 없습니다"))?;
    let image_info = json!({"width": width, "height": height, "byte_size": image.len()});
    let refuse_existing = |archive: &mut ZipArchive<&File>| -> FResult<()> {
        if inspect_archive(archive, hooks)?["has_cover"].as_bool().unwrap_or(false) && !replace_existing {
            return Err(CoverError::new("EPUB에 이미 표지가 있습니다").into());
        }
        Ok(())
    };
    let mut result = rewrite_epub(path, expected_revision, backup_dir, backup_name, &mut |source, staged| stage_cover(source, staged, &image, hooks), Some(&refuse_existing), hooks)?;
    result["cover_image"] = image_info;
    Ok(result)
}

pub fn remove_cover(path: &Path, expected_revision: &str, backup_dir: &Path) -> Result<Value, CoverError> {
    remove_cover_with(path, expected_revision, backup_dir, &EpubCoverHooks::default())
}

pub fn remove_cover_with(path: &Path, expected_revision: &str, backup_dir: &Path, hooks: &EpubCoverHooks) -> Result<Value, CoverError> {
    remove_cover_backup_with(path, expected_revision, backup_dir, None, hooks)
}

/// EPUB 표지를 뺀다. 표지가 없거나 본문과 공유하면 원본과 백업을 건드리지 않는다.
pub fn remove_cover_backup_with(path: &Path, expected_revision: &str, backup_dir: &Path, backup_name: Option<&str>, hooks: &EpubCoverHooks) -> Result<Value, CoverError> {
    use super::stage::{detach_publisher_cover, remove_tool_covers, sections};
    use super::{OPF_NS, read_package_with};
    let require_cover = |archive: &mut ZipArchive<&File>| -> FResult<()> {
        let (opf_path, mut package) = read_package_with(archive, hooks)?;
        {
            let parts = sections(&mut package);
            let (Some(manifest), Some(metadata), Some(spine)) = (parts.manifest, parts.metadata, parts.spine) else {
                return Err(CoverError::new("EPUB manifest, metadata 또는 spine이 없습니다").into());
            };
            if !inspect_archive(archive, hooks)?["has_cover"].as_bool().unwrap_or(false) {
                return Err(CoverError::new("EPUB에 표지가 없습니다").into());
            }
            remove_tool_covers(&opf_path, manifest, metadata, spine);
        }
        let _ = OPF_NS;
        detach_publisher_cover(archive, &opf_path, &mut package, true, hooks)?;
        Ok(())
    };
    let mut result = rewrite_epub(path, expected_revision, backup_dir, backup_name, &mut |source, staged| stage_cover_removal(source, staged, hooks), Some(&require_cover), hooks)?;
    result["cover_image"] = Value::Null;
    Ok(result)
}

pub fn add_illustration(path: &Path, image: &[u8], expected_revision: &str, backup_dir: &Path, before_idref: &str) -> Result<Value, CoverError> {
    add_illustration_with(path, image, expected_revision, backup_dir, before_idref, &EpubCoverHooks::default())
}

pub fn add_illustration_with(path: &Path, image: &[u8], expected_revision: &str, backup_dir: &Path, before_idref: &str, hooks: &EpubCoverHooks) -> Result<Value, CoverError> {
    add_illustration_backup_with(path, image, expected_revision, backup_dir, before_idref, None, hooks)
}

/// `before_idref` 페이지 앞에 삽화 한 장을 넣는다. 표지는 건드리지 않는다.
pub fn add_illustration_backup_with(
    path: &Path,
    image: &[u8],
    expected_revision: &str,
    backup_dir: &Path,
    before_idref: &str,
    backup_name: Option<&str>,
    hooks: &EpubCoverHooks,
) -> Result<Value, CoverError> {
    let image = normalize_image_with(image, Some(1200), hooks)?;
    let mut created = String::new();
    let mut result = rewrite_epub(
        path,
        expected_revision,
        backup_dir,
        backup_name,
        &mut |source, staged| {
            created = stage_illustration(source, staged, &image, before_idref, hooks)?;
            Ok(())
        },
        None,
        hooks,
    )?;
    result["cover_image"] = Value::Null;
    result["idref"] = json!(created);
    Ok(result)
}
