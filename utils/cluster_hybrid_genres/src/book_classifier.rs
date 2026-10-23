//! backend/book_classifier.py 중 이 도구가 쓰는 함수의 이식.
//! Python backend는 원본을 그대로 쓰므로 여기서는 필요한 세 함수만 둔다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use detect_mojibake::util::{posix_normpath, unquote};
use fancy_regex::Regex;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("잘못된 정규식 {pattern}: {e}"))
}

// 파일명에 명시된 장르 태그. 위에 있을수록 우선한다.
static EXPLICIT_TAGS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"(?i)\[[^\s\]]*(?:로맨스판타지|로맨스|로판|현대로맨스|현로|로맨틱판타지|동양로맨스)[^\s\]]*\]|\([^\s\)]*(?:로맨스판타지|로맨스|로판|현대로맨스|현로|로맨틱판타지|동양로맨스)[^\s\)]*\)|^(?:동양)?(?:로맨스판타지|로맨스|로판|현대로맨스|현로)[0-9a-zA-Z가-힣\s\(\)\]【】]*[\)\s\]]", "3_여성향"),
        (r"(?i)\[[^\s\]]*(?:BL|GL|백합|오메가버스)[^\s\]]*\]|\([^\s\)]*(?:BL|GL|백합|오메가버스)[^\s\)]*\)|^\bBL\b", "9_BLGL"),
        (r"(?i)\[[^\s\]]*(?:신무협|정통무협|무협소설|무협|선협)[^\s\]]*\]|\([^\s\)]*(?:신무협|정통무협|무협소설|무협|선협)[^\s\)]*\)|^(?:미완\)|단편\))?무협[0-9a-zA-Z가-힣\s\(\)\]【】]*[\)\s\]]", "3_무협"),
        (r"(?i)\[[^\s\]]*(?:퓨전판타지|현대판타지|게임판타지|퓨판|현판|겜판|판타지소설|판타지|레이드물|헌터물)[^\s\]]*\]|\([^\s\)]*(?:퓨전판타지|현대판타지|게임판타지|퓨판|현판|겜판|판타지소설|판타지)[^\s\)]*\)|^(?:현판|퓨판|겜판|판타지)[0-9a-zA-Z가-힣\s\(\)\]【】]*[\)\s\]]", "3_판타지"),
        (r"(?i)\[(?:TS|티에스)\]|\((?:TS|티에스)\)|(?:^|[^a-zA-Z0-9가-힣])TS(?:물|소설|됐|은|는|이|가|도|[0-9\s\]\)_]|$)|^TS[가-힣\s]", "3_판타지"),
        (r"(?i)\[[^\s\]]*(?:라이트노벨|라노벨)[^\s\]]*\]|\([^\s\)]*(?:라이트노벨|라노벨)[^\s\)]*\)", "3_라이트노벨"),
        (r"(?i)\[[^\s\]]*(?:SF소설|과학소설|SF)[^\s\]]*\]|\([^\s\)]*(?:SF소설|과학소설|SF)[^\s\)]*\)|^\bSF\b[\s\)]", "3_SF"),
        (r"(?i)\[[^\s\]]*(?:추리소설|미스터리소설|스릴러소설|추리|미스터리|스릴러)[^\s\]]*\]|\([^\s\)]*(?:추리소설|미스터리소설|스릴러소설|추리|미스터리|스릴러)[^\s\)]*\)", "3_스릴러"),
        (r"(?i)\[[^\s\]]*(?:그래픽노블|만화|코믹스|웹툰)[^\s\]]*\]|\([^\s\)]*(?:그래픽노블|만화|코믹스|웹툰)[^\s\)]*\)", "3_그래픽노블"),
        (r"(?i)\[[^\s\]]*(?:소설|한국소설)[^\s\]]*\]|\([^\s\)]*(?:소설|한국소설)[^\s\)]*\)", "2_소설한국"),
    ]
    .into_iter()
    .map(|(p, c)| (re(p), c))
    .collect()
});

/// 파일명(URL 디코딩 후)에 명시된 장르 태그나 접두어를 표준 카테고리로 돌려준다.
pub fn extract_explicit_genre(filename: &str) -> Option<&'static str> {
    let name = unquote(filename);
    EXPLICIT_TAGS.iter().find(|(pat, _)| pat.is_match(&name).unwrap_or(false)).map(|(_, cat)| *cat)
}

static SHORT_STEM: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(\d+|[0-9~]+권|[0-9~]+화|[0-9~]+부|\bvol\.?\s*\d+|\bv\d+)$"));
static AUTHOR_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"(\[[^\]]+\]|@[^\s]+)"));

/// 서브디렉토리 안 파일명이 단순 번호나 권수(01.txt, 1권.epub)이면 부모 디렉토리명(작품명)을
/// 앞에 붙여 고유한 도서 파일명을 만든다. 부모에 [저자]나 @저자가 있고 파일명에 없으면
/// 부모의 저자 정보를 이어 붙인다.
pub fn get_effective_filename(fpath: &Path, target_dir: &Path) -> String {
    let fname = fpath.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if fpath.parent() == Some(target_dir) {
        return fname;
    }
    let parent_name = fpath.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = fpath.file_stem().map(|s| s.to_string_lossy().trim().to_owned()).unwrap_or_default();
    if SHORT_STEM.is_match(&stem).unwrap_or(false) || stem.chars().count() <= 3 {
        return format!("{parent_name} {fname}");
    }
    let parent_tag = AUTHOR_TAG.captures(&parent_name).ok().flatten().map(|c| c[1].to_owned());
    let file_has_tag = AUTHOR_TAG.is_match(&fname).unwrap_or(false);
    match parent_tag {
        Some(tag) if !file_has_tag => format!("{tag} {fname}"),
        _ => fname,
    }
}

/// `Path.resolve(strict=False)`: 있는 만큼은 심볼릭 링크를 풀고, 없는 꼬리는 그대로 붙인다.
pub fn resolve_lenient(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let normalized = PathBuf::from(posix_normpath(&abs.to_string_lossy()));
    let mut head = normalized.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !head.exists() {
        let Some(name) = head.file_name().map(|n| n.to_owned()) else { return normalized };
        tail.push(name);
        if !head.pop() {
            return normalized;
        }
    }
    let mut resolved = head.canonicalize().unwrap_or(head);
    for name in tail.into_iter().rev() {
        resolved.push(name);
    }
    resolved
}

/// 파일 이동·삭제 뒤 비어 있는 상위 서브디렉토리를 안전하게 재귀 청소한다.
///
/// `stop_dir` 자신과 그 바깥은 절대 지우지 않는다. 두 경로를 정규화하지 않고 비교하면 `..`가 낀
/// 표기로는 `stop_dir`를 알아보지 못해 조상 디렉토리까지 올라간다. 비었는지 판정도 rmdir과
/// 같은 기준(숨김 파일 포함)으로 본다.
pub fn clean_empty_parent_dirs(parent_dir: &Path, stop_dir: &Path) {
    let mut curr = resolve_lenient(parent_dir);
    let stop = resolve_lenient(stop_dir);
    while curr != stop && curr.starts_with(&stop) && curr.is_dir() {
        // rmdir과 같은 기준: 숨김 파일을 포함해 항목이 하나라도 있으면 멈춘다.
        let empty = fs::read_dir(&curr).is_ok_and(|mut it| it.next().is_none());
        if !empty || fs::remove_dir(&curr).is_err() {
            break;
        }
        if !curr.pop() {
            break;
        }
    }
}
