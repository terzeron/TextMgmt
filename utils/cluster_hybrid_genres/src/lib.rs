//! 본문 처음 500단어와 마지막 500단어, 그리고 제목을 결합 분석하여 도서의 장르별 단어 빈도
//! 비율(무협·판타지·여성향·BL·성인 %)을 계산하고, 클러스터로 묶어 확실한 클러스터는 자동
//! 분류하고 하이브리드는 보고한다. 원본: `utils/cluster_hybrid_genres.py`.
//!
//! 백엔드의 자동분류는 지도학습 모델로 바뀌었고 이 수작업 어휘 스코어러는 거기서 빠졌다.
//! 하이브리드 장르 탐색은 "무협 40% / 판타지 35%" 같은 비율 자체를 보는 도구라 모델의 단일
//! 판정으로는 대체가 안 된다.

pub mod book_classifier;
pub mod cli;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

use detect_mojibake::py_round;
use detect_mojibake::reader::{MAX_MEMBER_BYTES, decode_korean_ignore, decode_utf8_ignore, open_zip};
use fancy_regex::Regex;
use indexmap::IndexMap;

use book_classifier::extract_explicit_genre;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("잘못된 정규식 {pattern}: {e}"))
}

fn count(re: &Regex, text: &str) -> usize {
    re.find_iter(text).filter(Result::is_ok).count()
}

/// Python의 `str.isspace()`와 같은 공백 판정. `\x1c`~`\x1f`도 공백이다.
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

fn py_split(s: &str) -> Vec<&str> {
    s.split(is_py_space).filter(|w| !w.is_empty()).collect()
}

/// Python `str.splitlines()`: 줄 경계가 `\n`, `\r`, `\r\n` 외에도 있다.
fn py_splitlines(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            lines.push(&s[start..i]);
            let mut next = i + c.len_utf8();
            if c == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
                chars.next();
                next += 1;
            }
            start = next;
        }
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

/// Python 텍스트 모드의 universal newlines: `\r\n`과 `\r`을 `\n`으로 바꾼다.
fn universal_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Python의 float 표기. 정수값에도 `.0`이 붙는다.
pub fn py_float(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 { format!("{v:.1}") } else { format!("{v}") }
}

static HANGUL_RUN: LazyLock<Regex> = LazyLock::new(|| re(r"[가-힣]{3,}"));
static TAG_RE: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]+>"));

/// TXT 앞부분에서 읽는 바이트 상한. 250줄이 이보다 길면 Python과 표본이 달라질 수 있다.
const HEAD_READ_LIMIT: usize = 4 * 1024 * 1024;
const HEAD_CHUNK: usize = 64 * 1024;
const TAIL_READ_BYTES: u64 = 40_000;
const HEAD_LINES: usize = 250;

fn decode_with(raw: &[u8], enc: &str) -> String {
    if enc == "utf-8" { decode_utf8_ignore(raw) } else { decode_korean_ignore(raw, false) }
}

/// 앞에서 `lines`줄을 채우는 데 필요한 바이트만 읽는다. 줄 끝은 `\n`, `\r\n`, 단독 `\r`이다
/// (Python 텍스트 모드의 universal newlines). UTF-8의 이어지는 바이트와 CP949의 뒷바이트는
/// 모두 0x40 이상이라 바이트 단위로 세어도 인코딩과 무관하게 정확하다.
fn read_head_bytes(path: &Path, lines: usize) -> std::io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let (mut raw, mut seen, mut scanned) = (Vec::new(), 0usize, 0usize);
    loop {
        let before = raw.len();
        f.by_ref().take(HEAD_CHUNK as u64).read_to_end(&mut raw)?;
        if raw.len() == before {
            return Ok(raw);
        }
        while scanned < raw.len() {
            let b = raw[scanned];
            let lone_cr = b == b'\r' && raw.get(scanned + 1) != Some(&b'\n');
            // 청크 끝의 `\r`은 다음 바이트를 모르므로 다음 청크에서 세도록 남겨 둔다.
            if b == b'\r' && scanned + 1 == raw.len() {
                break;
            }
            if b == b'\n' || lone_cr {
                seen += 1;
                if seen == lines {
                    raw.truncate(scanned + 1);
                    return Ok(raw);
                }
            }
            scanned += 1;
        }
        if raw.len() >= HEAD_READ_LIMIT {
            return Ok(raw);
        }
    }
}

fn txt_head(path: &Path) -> String {
    let Ok(raw) = read_head_bytes(path, HEAD_LINES) else { return String::new() };
    for enc in ["utf-8", "cp949"] {
        let text = universal_newlines(&decode_with(&raw, enc));
        let head: String = text.split_inclusive('\n').take(HEAD_LINES).collect();
        if HANGUL_RUN.is_match(&head).unwrap_or(false) {
            return head;
        }
    }
    String::new()
}

fn txt_tail(path: &Path) -> String {
    let read = || -> std::io::Result<Vec<u8>> {
        let mut f = File::open(path)?;
        let size = f.metadata()?.len();
        f.seek(SeekFrom::Start(size.saturating_sub(size.min(TAIL_READ_BYTES))))?;
        let mut raw = Vec::new();
        f.read_to_end(&mut raw)?;
        Ok(raw)
    };
    let Ok(raw) = read() else { return String::new() };
    for enc in ["utf-8", "cp949"] {
        let text = decode_with(&raw, enc);
        let lines = py_splitlines(&text);
        let kept = if lines.len() > 1 { &lines[1..] } else { &lines[..] };
        let joined = kept.join(" ");
        if HANGUL_RUN.is_match(&joined).unwrap_or(false) {
            return joined;
        }
    }
    String::new()
}

/// EPUB 첫 두 문서와 마지막 두 문서의 본문. 한 멤버라도 읽지 못하면 그 방향의 표본은 비운다.
fn epub_head_tail(path: &Path) -> (String, String) {
    let Some(mut zip) = open_zip(path) else { return Default::default() };

    let names: Vec<(usize, String)> = zip.file_names().enumerate().filter_map(|(i, n)| n.ok().map(|n| (i, n.into_owned()))).collect();
    let is_html = |n: &str| {
        let l = n.to_lowercase();
        l.ends_with(".html") || l.ends_with(".xhtml") || l.ends_with(".htm")
    };
    let mut htmls: Vec<usize> = names.iter().filter(|(_, n)| is_html(n) && !["cover", "nav", "toc", "title"].iter().any(|k| n.to_lowercase().contains(k))).map(|(i, _)| *i).collect();
    if htmls.is_empty() {
        htmls = names.iter().filter(|(_, n)| is_html(n)).map(|(i, _)| *i).collect();
    }

    let mut chunks = |idxs: &[usize]| -> Option<String> {
        let mut out = Vec::new();
        for &i in idxs {
            let entry = zip.by_index(i).ok()?;
            let mut raw = Vec::new();
            // 작게 압축된 거대 멤버(zip bomb)가 메모리를 먹지 못하게 상한을 둔다.
            entry.take(MAX_MEMBER_BYTES).read_to_end(&mut raw).ok()?;
            let clean = TAG_RE.replace_all(&decode_utf8_ignore(&raw), " ").into_owned();
            if clean.trim().chars().count() > 50 {
                out.push(clean);
            }
        }
        Some(out.join(" "))
    };
    let head_idx = &htmls[..htmls.len().min(2)];
    let tail_idx = &htmls[htmls.len().saturating_sub(2)..];
    let Some(head) = chunks(head_idx) else { return Default::default() };
    let tail = chunks(tail_idx).unwrap_or_default();
    (head, tail)
}

/// TXT/EPUB 파일에서 처음 N단어와 마지막 N단어 분량의 본문 텍스트를 추출해 결합한다.
pub fn extract_content_head_tail_words(path: &Path, head_n: usize, tail_n: usize) -> String {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let (head_text, tail_text) = match ext.as_deref() {
        Some("txt") => (txt_head(path), txt_tail(path)),
        Some("epub") => epub_head_tail(path),
        _ => Default::default(),
    };
    let h_words: Vec<&str> = py_split(&head_text).into_iter().take(head_n).collect();
    let t_all = py_split(&tail_text);
    let t_words = &t_all[t_all.len().saturating_sub(tail_n)..];
    format!("{} {}", h_words.join(" "), t_words.join(" ")).trim_matches(is_py_space).to_owned()
}

/// 처음 500단어와 마지막 500단어를 결합해 1000단어를 돌려준다(하위 호환).
pub fn extract_content_first_1000_words(path: &Path) -> String {
    extract_content_head_tail_words(path, 500, 500)
}

static LEGAL: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)무단\s*전재[^\n.]*(?:금합니다|금지|처벌|법적[^\n.]*책임)[^\n.]*",
        r"(?i)이\s*(?:전자)?책은\s*저작권법[^\n.]*(?:보호|금합니다|처벌|금지)[^\n.]*",
        r"(?i)저작권자의\s*(?:서면\s*)?동의\s*없이[^\n.]*(?:금합니다|처벌)[^\n.]*",
        r"(?i)(?:발행인|편집인|책임편집|디자인|제작|마케팅|펴낸곳|출판사|등록번호|isbn)\s+[:\w\s,&]+(?=[.\n]|$)",
        r"(?i)\*경고\*[^\n.]*(?:정상적인 경로의 뷰어가 아닙니다|처벌대상이 되실 수 있으니)[^\n.]*",
    ]
    .into_iter()
    .map(re)
    .collect()
});

/// 저작권 경고, 무단전재 공지, 판권지, 출판사 정보처럼 장르 판정과 무관한 문구를 분석 전에 지운다.
pub fn clean_disclaimer_and_colophon(text: &str) -> String {
    LEGAL.iter().fold(text.to_owned(), |acc, r| r.replace_all(&acc, " ").into_owned())
}

static DANJEON: LazyLock<Regex> = LazyLock::new(|| re(r"(?<!무)단전(?:[를이가의로통량석홀]?)(?:$|[^\w가-힣])"));
static JINGI: LazyLock<Regex> = LazyLock::new(|| re(r"(?<!민)진기(?!한)(?:[를가의로]?)(?:$|[^\w가-힣])"));
static GANGSU: LazyLock<Regex> = LazyLock::new(|| re(r"(?<![가-힣a-zA-Z])강수(?:[를이가의로]?)(?:$|[^\w가-힣])"));
static MANA: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[^\w가-힣])마나(?:[를이가의로통량석홀]?)(?:$|[^\w가-힣])"));
static BL: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[^a-zA-Z])bl(?:$|[^a-zA-Z])"));
static GL: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[^a-zA-Z])gl(?:$|[^a-zA-Z])"));
static BL_OR_GL: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[^a-zA-Z])(?:bl|gl)(?:$|[^a-zA-Z])"));

/// 키워드 출현 수. 오탐이 잦은 단어는 앞뒤 문맥을 보고, 나머지는 부분 문자열을 센다.
pub fn count_genre_word_patterns(text: &str, kws: &[&str]) -> usize {
    kws.iter()
        .map(|kw| match *kw {
            "단전" => count(&DANJEON, text),
            "진기" => count(&JINGI, text),
            "강수" => count(&GANGSU, text),
            "마나" => count(&MANA, text),
            "bl" => count(&BL, text),
            "gl" => count(&GL, text),
            other => text.matches(other).count(),
        })
        .sum()
}

// 5대 장르(무협, 판타지, 여성향, BLGL, 성인) 도메인 키워드 상수.
pub const WUXIA_CORE: &[&str] = &[
    "무협",
    "무림",
    "단전",
    "내공",
    "진기",
    "운기조식",
    "기경팔맥",
    "주화입마",
    "환골탈태",
    "화산파",
    "무당파",
    "소림사",
    "개방",
    "사파",
    "정파",
    "마교",
    "천마",
    "비급",
    "검법",
    "도법",
    "심법",
    "세가",
    "사천당가",
    "남궁세가",
    "제갈세가",
    "모용세가",
    "하북팽가",
    "소가주",
    "녹림",
    "암기",
    "독공",
    "독문",
    "검기",
];
const WUXIA_EXTRA: &[&str] = &["강호", "임독이맥", "종남파", "혈교", "장문인", "소교주", "맹주", "무림맹", "절기", "초식", "공자", "가주", "하오문", "점창파", "도기", "호법", "총채주"];
// 판타지: 현대 헌터물 + 중세 서양 판타지/영지물/제국물.
pub const FANTASY_CORE: &[&str] = &[
    "판타지",
    "던전",
    "몬스터",
    "헌터",
    "각성",
    "레이드",
    "게이트",
    "상태창",
    "마나",
    "마법진",
    "오크",
    "고블린",
    "드래곤",
    "엘프",
    "용사",
    "이세계",
    "귀환자",
    "만렙",
    "오러",
    "소드마스터",
    "기사단",
    "마법사",
    "마탑",
    "영지",
    "영주",
    "마력",
];
const FANTASY_EXTRA: &[&str] =
    &["길드", "시스템", "퀘스트", "스킬", "스탯", "레벨업", "플레이어", "인벤토리", "아이템", "서클", "아카데미", "골렘", "마물", "제국", "왕국", "공국", "백작", "남작", "차원이동", "환생자", "기사"];
// 여성향/로판: 서양 판타지 영지물과 겹치는 작위(황제, 공작, 영애)와 시한부는 CORE에서 뺀다.
pub const ROFAN_CORE: &[&str] = &["로맨스", "로판", "현로", "남주", "여주", "남주인공", "여주인공", "파혼", "시월드", "후회남", "집착남", "계략남", "악녀", "햇살여주", "빙의녀"];
const ROFAN_EXTRA: &[&str] = &["공작", "공작가", "황제", "황실", "사교계", "무도회", "드레스", "시녀", "집사", "약혼", "키스", "설렘", "영애", "시한부"];
// BL: 페로몬/강수/각인은 생물학과 인명 오탐을 막으려고 CORE에서 뺀다.
pub const BL_CORE: &[&str] =
    &["미인공", "미남공", "다정공", "광공", "집착공", "연하공", "연상공", "후회공", "미인수", "단정수", "지랄수", "임신수", "순진수", "오메가버스", "가이드버스", "보이즈러브", "동성애", "백합"];
const BL_EXTRA: &[&str] = &["에스퍼", "가이딩", "히트사이클", "페로몬", "노팅", "각인", "강수", "bl", "gl"];
pub const ADULT_KWS: &[&str] = &[
    "야설",
    "성인소설",
    "음란",
    "음탕",
    "육덕",
    "최면",
    "조교",
    "근친",
    "스와핑",
    "섹스",
    "자위",
    "사정액",
    "쿠퍼액",
    "애액",
    "정액",
    "자지",
    "보지에",
    "보지를",
    "보지속",
    "음순",
    "클리토리스",
    "귀두",
    "유두",
    "유륜",
    "젖가슴",
    "피스톤",
    "허리짓",
    "교성",
    "오르가즘",
    "절정에",
    "펠라치오",
];

fn with_extra(core: &[&'static str], extra: &[&'static str]) -> Vec<&'static str> {
    core.iter().chain(extra).copied().collect()
}

static WUXIA_KWS: LazyLock<Vec<&str>> = LazyLock::new(|| with_extra(WUXIA_CORE, WUXIA_EXTRA));
static FANTASY_KWS: LazyLock<Vec<&str>> = LazyLock::new(|| with_extra(FANTASY_CORE, FANTASY_EXTRA));
static ROFAN_KWS: LazyLock<Vec<&str>> = LazyLock::new(|| with_extra(ROFAN_CORE, ROFAN_EXTRA));
static BL_KWS: LazyLock<Vec<&str>> = LazyLock::new(|| with_extra(BL_CORE, BL_EXTRA));

/// 장르 분석 결과. `scores`와 `ratios`는 카테고리 순서(무협, 판타지, 여성향, BLGL, 성인)를 지킨다.
#[derive(Debug, Clone, PartialEq)]
pub struct GenreResult {
    pub valid: bool,
    pub reason: Option<String>,
    pub scores: IndexMap<String, i64>,
    pub ratios: IndexMap<String, f64>,
    pub total_score: i64,
    pub cluster: String,
    pub target_cat: Option<String>,
}

const CATS: [&str; 5] = ["3_무협", "3_판타지", "3_여성향", "9_BLGL", "9_성인"];

fn invalid(reason: &str, scores: IndexMap<String, i64>, cluster: &str) -> GenreResult {
    let ratios = scores.keys().map(|k| (k.clone(), 0.0)).collect();
    GenreResult { valid: false, reason: Some(reason.into()), scores, ratios, total_score: 0, cluster: cluster.into(), target_cat: None }
}

fn any_in(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

/// 도서 제목과 텍스트(Head 500 + Tail 500)에서 5대 장르의 빈도 점수, 비율(%), 클러스터를 산출한다.
pub fn calculate_5_genre_scores_and_ratios(title: &str, text: &str) -> GenreResult {
    let cleaned = clean_disclaimer_and_colophon(text);
    let combined = format!("{title} {cleaned}").to_lowercase();
    let total_chars = combined.chars().count();
    let hangul = combined.chars().filter(|&c| ('가'..='힣').contains(&c)).count();

    if total_chars < 15 || (hangul as f64 / total_chars as f64) < 0.20 {
        return invalid("not_enough_hangul", IndexMap::new(), "미분류_비문학영문");
    }

    // 핵심 어휘가 하나라도 있을 때만 해당 장르의 점수를 센다.
    let gated = |core: &[&str], kws: &[&str]| if any_in(&combined, core) { count_genre_word_patterns(&combined, kws) as i64 } else { 0 };
    let w_score = gated(WUXIA_CORE, &WUXIA_KWS);
    let f_score = gated(FANTASY_CORE, &FANTASY_KWS);
    // 여성향: 결정적 로맨스 관계성 어휘(ROFAN_CORE)가 있을 때만 인정한다.
    let r_score = gated(ROFAN_CORE, &ROFAN_KWS);
    // BL: 결정적 BL 어휘나 독립 단어 bl/gl이 있을 때만 인정한다(단독 페로몬/강수 오탐 방지).
    let has_bl_core = any_in(&combined, BL_CORE) || BL_OR_GL.is_match(&combined).unwrap_or(false);
    let b_score = if has_bl_core { count_genre_word_patterns(&combined, &BL_KWS) as i64 } else { 0 };

    let matched_adult = ADULT_KWS.iter().filter(|kw| combined.contains(**kw)).count();
    let adult_hits: usize = ADULT_KWS.iter().map(|kw| combined.matches(kw).count()).sum();
    let a_score = if matched_adult >= 3 && adult_hits >= 5 { adult_hits as i64 } else { 0 };

    let mut scores: IndexMap<String, i64> = CATS.iter().zip([w_score, f_score, r_score, b_score, a_score]).map(|(c, s)| (c.to_string(), s)).collect();

    // 파일명에 명시된 장르를 최우선으로 보호하고 가중치를 준다.
    if let Some(explicit) = extract_explicit_genre(title)
        && let Some(s) = scores.get_mut(explicit)
    {
        *s += 5;
        if explicit == "3_무협" || explicit == "3_판타지" {
            // 우연한 노이즈로 BL/여성향으로 이탈하는 것을 막는다.
            scores["9_BLGL"] = 0;
            scores["3_여성향"] = 0;
        }
    }

    let total_score: i64 = scores.values().sum();
    if total_score == 0 {
        return invalid("zero_score", scores, "미분류_키워드부족");
    }

    let ratios: IndexMap<String, f64> = scores.iter().map(|(k, v)| (k.clone(), py_round(*v as f64 / total_score as f64 * 100.0, 1))).collect();
    let (cluster, target_cat) = decide_cluster(&scores, &ratios);
    GenreResult { valid: true, reason: None, scores, ratios, total_score, cluster, target_cat }
}

/// 두 장르가 함께 두드러지는 하이브리드인지: 비율 합 70% 이상, 각 비율과 점수 하한을 넘는 경우.
fn is_pair(scores: &IndexMap<String, i64>, ratios: &IndexMap<String, f64>, a: &str, b: &str, min_ratio: f64) -> bool {
    ratios[a] + ratios[b] >= 70.0 && ratios[a] >= min_ratio && ratios[b] >= min_ratio && scores[a] >= 2 && scores[b] >= 2 && scores[a] + scores[b] >= 5
}

fn decide_cluster(scores: &IndexMap<String, i64>, ratios: &IndexMap<String, f64>) -> (String, Option<String>) {
    // 1. 단일 압도적 클러스터(75% 이상, 점수 5 이상).
    for (cat, r) in ratios {
        if *r >= 75.0 && scores[cat] >= 5 {
            // 무협인데 판타지 키워드가 2건 이상이면 순수무협이 아니라 하이브리드로 보고 3_판타지로 보낸다.
            return if cat == "3_무협" && scores["3_판타지"] >= 2 {
                ("하이브리드_무협_판타지".into(), Some("3_판타지".into()))
            } else {
                (format!("확실한_{cat}"), Some(cat.clone()))
            };
        }
    }

    // 2. 하이브리드. 3_무협은 순수무협만 유지하고 하이브리드는 3_판타지로 보낸다.
    let wf = (ratios["3_무협"] + ratios["3_판타지"] >= 70.0 && ratios["3_무협"] >= 15.0 && ratios["3_판타지"] >= 15.0 && scores["3_판타지"] >= 2)
        || (scores["3_무협"] >= 2 && scores["3_판타지"] >= 2 && scores["3_무협"] + scores["3_판타지"] >= 5);
    if wf {
        return ("하이브리드_무협_판타지".into(), Some("3_판타지".into()));
    }
    for (a, b, name) in [
        ("3_판타지", "3_여성향", "하이브리드_판타지_로판"),
        ("3_판타지", "9_성인", "하이브리드_판타지_성인"),
        ("3_여성향", "9_성인", "하이브리드_로맨스_성인"),
        ("3_무협", "9_성인", "하이브리드_무협_성인"),
        ("9_BLGL", "9_성인", "하이브리드_BL_성인"),
        ("9_BLGL", "3_판타지", "하이브리드_BL_판타지"),
    ] {
        if is_pair(scores, ratios, a, b, 20.0) {
            return (name.into(), None);
        }
    }

    // 3. 60% 이상 및 점수 4점 이상이면 준확실. 노이즈 3점 이하는 미분류/보류다.
    for (cat, r) in ratios {
        if *r >= 60.0 && scores[cat] >= 4 {
            return (format!("준확실_{cat}"), Some(cat.clone()));
        }
    }
    ("기타_복합".into(), None)
}

/// 하위 호환 래퍼: (본문 표본, "").
pub fn extract_head_tail_words(path: &Path, head_n: usize, tail_n: usize) -> (String, String) {
    (extract_content_head_tail_words(path, head_n, tail_n), String::new())
}

/// 하위 호환 래퍼: 머리와 꼬리 텍스트를 합쳐 분석한다.
pub fn calculate_genre_frequencies(title: &str, head_text: &str, tail_text: &str) -> GenreResult {
    calculate_5_genre_scores_and_ratios(title, format!("{head_text} {tail_text}").trim_matches(is_py_space))
}
