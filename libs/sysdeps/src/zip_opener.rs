//! ZIP 열기와 멤버 읽기 상한. EPUB·OOXML·HWPX를 읽는 crate가 함께 쓴다.

use std::fs::File;
use std::io::BufReader;
use std::path::{Component, Path};

use zip::ZipArchive;
use zip::read::{ArchiveOffset, Config};

/// 버퍼를 둔 ZIP 리더. 버퍼가 없으면 중앙 디렉토리를 읽느라 read/lseek가 수백만 번 난다.
pub type Zip = ZipArchive<BufReader<File>>;

fn buffered(path: &Path) -> std::io::Result<BufReader<File>> {
    Ok(BufReader::with_capacity(64 * 1024, File::open(path)?))
}

/// ZIP을 연다. 기본 설정(`Detect`)은 앞에 붙은 데이터를 찾으려고 파일 전체를 훑어 1.5MB EPUB 하나에
/// 읽기 1만 3천 번(48ms)이 든다. 앞에 데이터가 없는 보통의 경우를 먼저 `Known(0)`으로 열고
/// (0.2ms), 실패하면 `Detect`로 다시 연다. 앞에 데이터가 붙은 ZIP도 그대로 읽힌다.
pub fn open_zip(path: &Path) -> Option<Zip> {
    let fast = Config { archive_offset: ArchiveOffset::Known(0) };
    if let Ok(zip) = ZipArchive::with_config(fast, buffered(path).ok()?) {
        return Some(zip);
    }
    ZipArchive::new(buffered(path).ok()?).ok()
}

/// 멤버 하나에서 읽는 압축 해제 바이트 상한. 작게 압축된 거대 멤버(zip bomb)가 메모리를 먹지 못하게 한다.
pub const MAX_MEMBER_BYTES: u64 = 16 * 1024 * 1024;

/// 경로를 zip 항목 이름으로 바꾼다. zip crate의 `enclosed_name` 규칙을 따른다:
/// 조각은 `/`로 잇고, `.`은 버리고, `..`은 앞 조각을 지우고, 맨 앞 `/`는 뗀다.
/// `..`이 루트 밖으로 나가거나 UTF-8이 아닌 조각이 있으면 `None`이다(압축 파일 밖을 가리키는 경로).
pub fn zip_entry_name(path: impl AsRef<Path>) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.as_ref().components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Some(parts.join("/"))
}
