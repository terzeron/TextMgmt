//! backend/book_classifier.py 중 이 도구가 쓰는 함수의 이식.
//! Python backend는 원본을 그대로 쓰므로 여기서는 필요한 세 함수만 둔다.

use std::path::Path;
use std::sync::LazyLock;

use corpus_common::util::unquote;
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

pub use corpus_common::util::{clean_empty_parent_dirs, resolve_lenient};
