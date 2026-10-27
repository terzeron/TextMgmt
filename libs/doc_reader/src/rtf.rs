//! RTF를 평문으로. 원본: `striprtf.rtf_to_text`(`Loader.read_from_rtf`가 씀).
//!
//! 정규식 상태 기계를 그대로 옮겼다. 목적지 그룹(글꼴 표, 그림 등)은 건너뛰고, 16진 이스케이프는 글꼴의 문자셋으로 푼다.

use std::sync::LazyLock;

use regex::Regex;

use crate::TEXT_SIZE;
use crate::pytext::{head_chars, keep_word_chars};

const DESTINATIONS: [&str; 292] = [
    "aftncn",
    "aftnsep",
    "aftnsepc",
    "annotation",
    "atnauthor",
    "atndate",
    "atnicn",
    "atnid",
    "atnparent",
    "atnref",
    "atntime",
    "atrfend",
    "atrfstart",
    "author",
    "background",
    "bkmkend",
    "bkmkstart",
    "blipuid",
    "buptim",
    "category",
    "colorschememapping",
    "colortbl",
    "comment",
    "company",
    "creatim",
    "datafield",
    "datastore",
    "defchp",
    "defpap",
    "do",
    "doccomm",
    "docvar",
    "dptxbxtext",
    "ebcend",
    "ebcstart",
    "factoidname",
    "falt",
    "fchars",
    "ffdeftext",
    "ffentrymcr",
    "ffexitmcr",
    "ffformat",
    "ffhelptext",
    "ffl",
    "ffname",
    "ffstattext",
    "file",
    "filetbl",
    "fldinst",
    "fldtype",
    "fname",
    "fontemb",
    "fontfile",
    "fonttbl",
    "footer",
    "footerf",
    "footerl",
    "footerr",
    "footnote",
    "formfield",
    "ftncn",
    "ftnsep",
    "ftnsepc",
    "g",
    "generator",
    "gridtbl",
    "header",
    "headerf",
    "headerl",
    "headerr",
    "hl",
    "hlfr",
    "hlinkbase",
    "hlloc",
    "hlsrc",
    "hsv",
    "htmltag",
    "info",
    "keycode",
    "keywords",
    "latentstyles",
    "lchars",
    "levelnumbers",
    "leveltext",
    "lfolevel",
    "linkval",
    "list",
    "listlevel",
    "listname",
    "listoverride",
    "listoverridetable",
    "listpicture",
    "liststylename",
    "listtable",
    "listtext",
    "lsdlockedexcept",
    "macc",
    "maccPr",
    "mailmerge",
    "maln",
    "malnScr",
    "manager",
    "margPr",
    "mbar",
    "mbarPr",
    "mbaseJc",
    "mbegChr",
    "mborderBox",
    "mborderBoxPr",
    "mbox",
    "mboxPr",
    "mchr",
    "mcount",
    "mctrlPr",
    "md",
    "mdPr",
    "mdeg",
    "mdegHide",
    "mden",
    "mdiff",
    "me",
    "mendChr",
    "meqArr",
    "meqArrPr",
    "mf",
    "mfName",
    "mfPr",
    "mfunc",
    "mfuncPr",
    "mgroupChr",
    "mgroupChrPr",
    "mgrow",
    "mhideBot",
    "mhideLeft",
    "mhideRight",
    "mhideTop",
    "mhtmltag",
    "mlim",
    "mlimloc",
    "mlimlow",
    "mlimlowPr",
    "mlimupp",
    "mlimuppPr",
    "mm",
    "mmPr",
    "mmaddfieldname",
    "mmath",
    "mmathPict",
    "mmathPr",
    "mmaxdist",
    "mmc",
    "mmcJc",
    "mmcPr",
    "mmconnectstr",
    "mmconnectstrdata",
    "mmcs",
    "mmdatasource",
    "mmheadersource",
    "mmmailsubject",
    "mmodso",
    "mmodsofilter",
    "mmodsofldmpdata",
    "mmodsomappedname",
    "mmodsoname",
    "mmodsorecipdata",
    "mmodsosort",
    "mmodsosrc",
    "mmodsotable",
    "mmodsoudl",
    "mmodsoudldata",
    "mmodsouniquetag",
    "mmquery",
    "mmr",
    "mnary",
    "mnaryPr",
    "mnoBreak",
    "mnum",
    "moMath",
    "moMathPara",
    "moMathParaPr",
    "mobjDist",
    "mopEmu",
    "mphant",
    "mphantPr",
    "mplcHide",
    "mpos",
    "mr",
    "mrPr",
    "mrad",
    "mradPr",
    "msPre",
    "msPrePr",
    "msSub",
    "msSubPr",
    "msSubSup",
    "msSubSupPr",
    "msSup",
    "msSupPr",
    "msepChr",
    "mshow",
    "mshp",
    "mstrikeBLTR",
    "mstrikeH",
    "mstrikeTLBR",
    "mstrikeV",
    "msub",
    "msubHide",
    "msup",
    "msupHide",
    "mtransp",
    "mtype",
    "mvertJc",
    "mvfmf",
    "mvfml",
    "mvtof",
    "mvtol",
    "mzeroAsc",
    "mzeroDesc",
    "mzeroWid",
    "nesttableprops",
    "nextfile",
    "nonesttables",
    "objalias",
    "objclass",
    "objdata",
    "object",
    "objname",
    "objsect",
    "objtime",
    "oldcprops",
    "oldpprops",
    "oldsprops",
    "oldtprops",
    "oleclsid",
    "operator",
    "panose",
    "password",
    "passwordhash",
    "pgp",
    "pgptbl",
    "picprop",
    "pict",
    "pn",
    "pnseclvl",
    "pntext",
    "pntxta",
    "pntxtb",
    "printim",
    "private",
    "propname",
    "protend",
    "protstart",
    "protusertbl",
    "pxe",
    "result",
    "revtbl",
    "revtim",
    "rsidtbl",
    "rxe",
    "shp",
    "shpgrp",
    "shpinst",
    "shppict",
    "shprslt",
    "shptxt",
    "sn",
    "sp",
    "staticval",
    "stylesheet",
    "subject",
    "sv",
    "svb",
    "tc",
    "template",
    "themedata",
    "title",
    "txe",
    "ud",
    "upr",
    "userprops",
    "wgrffmtfilter",
    "windowcaption",
    "writereservation",
    "writereservhash",
    "xe",
    "xform",
    "xmlattrname",
    "xmlattrvalue",
    "xmlclose",
    "xmlname",
    "xmlnstbl",
    "xmlopen",
];
const CHARSETS: [(i64, &str); 31] = [
    (0, "cp1252"),
    (42, "cp1252"),
    (77, "mac_roman"),
    (78, "mac_japanese"),
    (79, "mac_chinesetrad"),
    (80, "mac_korean"),
    (81, "mac_arabic"),
    (82, "mac_hebrew"),
    (83, "mac_greek"),
    (84, "mac_cyrillic"),
    (85, "mac_chinesesimp"),
    (86, "mac_rumanian"),
    (87, "mac_ukrainian"),
    (88, "mac_thai"),
    (89, "mac_ce"),
    (128, "cp932"),
    (129, "cp949"),
    (130, "cp1361"),
    (134, "cp936"),
    (136, "cp950"),
    (161, "cp1253"),
    (162, "cp1254"),
    (163, "cp1258"),
    (177, "cp1255"),
    (178, "cp1256"),
    (186, "cp1257"),
    (204, "cp1251"),
    (222, "cp874"),
    (238, "cp1250"),
    (254, "cp437"),
    (255, "cp850"),
];

/// 제어어마다 찾는다. 배열을 앞에서부터 비교하면 변환 시간의 18%가 여기에 든다.
static DESTINATION_SET: LazyLock<std::collections::HashSet<&str>> = LazyLock::new(|| DESTINATIONS.into_iter().collect());
static UNICODE_DIGIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d$").unwrap());
static HYPERLINKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)(\{\\field\{\s*\\\*\\fldinst\{.*HYPERLINK\s(".*")\}{2}\s*\{.*?\s+(.*?)\}{2,3})"#).unwrap());
static FONTTABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\f(\d+).*?\\fcharset(\d+).*?([^;]+);").unwrap());

/// RTF 토큰 하나. 아래 정규식의 캡처 그룹 하나에 대응한다.
#[derive(Debug, PartialEq)]
enum Token<'a> {
    Word(&'a str, Option<&'a str>),
    Hex(&'a str),
    Ch(&'a str),
    Brace(&'a str),
    Newlines,
    Char(&'a str),
}

/// `(?i)\\([a-z]{1,32})(-?\d{1,10})?[ ]?|\\'([0-9a-f]{2})|\\([^a-z])|([{}])|[\r\n]+|(.)`를 손으로 푼 토큰열.
/// 정규식으로 캡처하면 글자마다 역방향 탐색과 backtracking을 다시 해서 변환 시간의 80%가 여기에 든다.
struct Tokens<'a> {
    text: &'a str,
    pos: usize,
}

/// `(?i)[a-z]`: 유니코드 대소문자 접기로 `ſ`(U+017F)와 켈빈 기호(U+212A)도 들어간다.
fn is_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '\u{17f}' || c == '\u{212a}'
}

/// 유니코드 `\d`. ASCII가 아닌 숫자는 드물어서 정규식으로 확인한다.
fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || (!c.is_ascii() && UNICODE_DIGIT.is_match(c.encode_utf8(&mut [0; 4])))
}

/// `\`로 시작하는 토큰과 그 바이트 길이.
fn escape(rest: &str) -> (Token<'_>, usize) {
    let after = &rest[1..];
    match after.chars().next() {
        None => (Token::Char(&rest[..1]), 1),
        Some(c) if is_letter(c) => {
            let word_end = after.char_indices().take(32).take_while(|&(_, ch)| is_letter(ch)).last().map_or(0, |(i, ch)| i + ch.len_utf8());
            let digits_start = word_end + usize::from(after[word_end..].starts_with('-'));
            let digits_end = after[digits_start..].char_indices().take(10).take_while(|&(_, ch)| is_digit(ch)).last().map(|(i, ch)| digits_start + i + ch.len_utf8());
            let end = digits_end.unwrap_or(word_end);
            let end = end + usize::from(after[end..].starts_with(' '));
            (Token::Word(&after[..word_end], digits_end.map(|d| &after[word_end..d])), 1 + end)
        }
        Some('\'') if after.len() >= 3 && after.as_bytes()[1].is_ascii_hexdigit() && after.as_bytes()[2].is_ascii_hexdigit() => (Token::Hex(&after[1..3]), 4),
        Some(c) => (Token::Ch(&after[..c.len_utf8()]), 1 + c.len_utf8()),
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        let rest = &self.text[self.pos..];
        let c = rest.chars().next()?;
        let (token, len) = match c {
            '\\' => escape(rest),
            '{' | '}' => (Token::Brace(&rest[..1]), 1),
            '\r' | '\n' => (Token::Newlines, rest.bytes().take_while(|b| matches!(b, b'\r' | b'\n')).count()),
            _ => (Token::Char(&rest[..c.len_utf8()]), c.len_utf8()),
        };
        self.pos += len;
        Some(token)
    }
}

/// `\char` 뒤에 오는 글자와 그것이 내는 글자.
fn special(name: &str) -> Option<&'static str> {
    Some(match name {
        "line" | "row" | "\n" | "par" => "\n",
        "tab" => "\t",
        "emdash" => "\u{2014}",
        "endash" => "\u{2013}",
        "emspace" => "\u{2003}",
        "enspace" => "\u{2002}",
        "qmspace" => "\u{2005}",
        "bullet" => "\u{2022}",
        "lquote" => "\u{2018}",
        "rquote" => "\u{2019}",
        "ldblquote" => "\u{201C}",
        "rdblquote" => "\u{201D}",
        "cell" | "nestcell" => "|",
        "~" => "\u{a0}",
        "\r" => "\r",
        "{" => "{",
        "}" => "}",
        "\\" => "\\",
        "-" => "\u{ad}",
        "_" => "\u{2011}",
        "sect" | "page" => "\n\n",
        _ => return None,
    })
}

fn is_section(name: &str) -> bool {
    matches!(name, "par" | "sect" | "page")
}

/// `\pict` 그룹 안의 이진 데이터(`\binNNN`)를 건너뛴다.
fn remove_pict_groups(rtf: &str) -> String {
    if !rtf.contains("\\pict") || !rtf.contains("\\bin") {
        return rtf.to_owned();
    }
    let chars: Vec<char> = rtf.chars().collect();
    let starts = |i: usize, s: &str| chars[i..].iter().zip(s.chars()).filter(|(a, b)| *a == b).count() == s.chars().count() && chars.len() - i >= s.chars().count();
    let mut out = String::new();
    let (mut i, mut in_pict) = (0, false);
    while i < chars.len() {
        if !in_pict && starts(i, "\\pict") {
            in_pict = true;
            i += 5;
            continue;
        }
        if in_pict {
            if starts(i, "\\bin") {
                i += 4;
                let mut digits = String::new();
                while i < chars.len() && chars[i].is_ascii_digit() {
                    digits.push(chars[i]);
                    i += 1;
                }
                i += digits.parse::<usize>().unwrap_or(0);
                continue;
            } else if chars[i] == '}' {
                in_pict = false;
                i += 1;
                continue;
            }
        }
        if !in_pict {
            out.push(chars[i]);
        }
        i += 1;
    }
    out
}

/// Python 코덱 이름으로 16진 바이트를 푼다. 풀 수 없는 바이트는 버린다(`errors="ignore"`).
fn decode_hex_bytes(bytes: &[u8], encoding: &str) -> String {
    let label = match encoding {
        "cp949" => return corpus_common::reader::decode_korean_ignore(bytes, false),
        "cp1252" | "cp1250" | "cp1251" | "cp1253" | "cp1254" | "cp1255" | "cp1256" | "cp1257" | "cp1258" => format!("windows-{}", &encoding[2..]),
        "cp932" => "shift_jis".to_owned(),
        "cp936" => "gbk".to_owned(),
        "cp950" => "big5".to_owned(),
        "cp874" => "windows-874".to_owned(),
        "mac_roman" => "macintosh".to_owned(),
        "mac_cyrillic" => "x-mac-cyrillic".to_owned(),
        other => other.replace('_', "-"),
    };
    match encoding_rs::Encoding::for_label(label.as_bytes()) {
        Some(enc) => enc.decode_without_bom_handling(bytes).0.chars().filter(|&c| c != '\u{fffd}').collect(),
        None => String::from_utf8_lossy(bytes).chars().filter(|&c| c != '\u{fffd}').collect(),
    }
}

fn charset_encoding(fcharset: i64, default: &str) -> String {
    CHARSETS.iter().find(|(k, _)| *k == fcharset).map_or_else(|| default.to_owned(), |(_, v)| (*v).to_owned())
}

/// RTF 텍스트를 평문으로 바꾼다. 원본이 예외를 내는 입력(`\uc`에 인수가 없는 경우)은 `Err`.
pub fn rtf_to_text(text: &str) -> Result<String, String> {
    let text = remove_pict_groups(text);
    let text = HYPERLINKS.replace_all(&text, "$1($2)").into_owned();
    let mut encoding = "cp1252".to_owned();
    let mut fonttbl: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for caps in FONTTABLE.captures_iter(&text) {
        let fcharset: i64 = caps[2].parse().unwrap_or(0);
        fonttbl.insert(caps[1].to_owned(), charset_encoding(fcharset, &encoding));
    }

    let mut stack: Vec<(i64, bool, bool)> = Vec::new();
    let (mut default_font, mut current_font): (Option<String>, Option<String>) = (None, None);
    let (mut ignorable, mut suppress_output) = (false, false);
    let (mut ucskip, mut curskip) = (1i64, 0i64);
    let mut hexes: Option<Vec<u8>> = None;
    let mut out = String::new();

    for token in (Tokens { text: &text, pos: 0 }) {
        if !matches!(token, Token::Hex(_))
            && let Some(pending) = hexes.take()
        {
            let enc = current_font.as_ref().and_then(|f| fonttbl.get(f)).unwrap_or(&encoding);
            out.push_str(&decode_hex_bytes(&pending, enc));
        }
        match token {
            Token::Brace(brace) => {
                curskip = 0;
                if brace == "{" {
                    stack.push((ucskip, ignorable, suppress_output));
                } else if let Some(top) = stack.pop() {
                    (ucskip, ignorable, suppress_output) = top;
                } else {
                    ucskip = 0;
                    ignorable = true;
                }
            }
            Token::Ch(c) => {
                curskip = 0;
                if let Some(s) = special(c) {
                    if is_section(c) {
                        current_font = default_font.clone();
                    }
                    if !ignorable {
                        out.push_str(s);
                    }
                } else if c == "*" {
                    ignorable = true;
                }
            }
            Token::Word(word, arg) => {
                curskip = 0;
                if DESTINATION_SET.contains(word) {
                    ignorable = true;
                } else if word == "ansicpg" {
                    encoding = format!("cp{}", arg.unwrap_or(""));
                    if !matches!(arg.and_then(|a| a.parse::<u32>().ok()), Some(437 | 850 | 874 | 932 | 936 | 949 | 950 | 1250..=1258)) {
                        encoding = "utf8".into();
                    }
                }
                if ignorable || suppress_output {
                    // 건너뛴다
                } else if let Some(s) = special(word) {
                    out.push_str(s);
                } else if word == "uc" {
                    ucskip = arg.ok_or("TypeError: \\uc에 인수가 없다")?.parse().map_err(|_| "ValueError: \\uc 인수".to_owned())?;
                } else if word == "u" {
                    match arg {
                        None => curskip = ucskip,
                        Some(a) => {
                            let mut c: i64 = a.parse().map_err(|_| "ValueError: \\u 인수".to_owned())?;
                            if c < 0 {
                                c += 0x10000;
                            }
                            out.push(u32::try_from(c).ok().and_then(char::from_u32).unwrap_or('\u{fffd}'));
                            curskip = ucskip;
                        }
                    }
                } else if word == "f" {
                    current_font = arg.map(str::to_owned);
                } else if word == "deff" {
                    default_font = arg.map(str::to_owned);
                } else if word == "fonttbl" || word == "colortbl" {
                    suppress_output = true;
                }
            }
            Token::Hex(h) => {
                if curskip > 0 {
                    curskip -= 1;
                } else if !ignorable {
                    let byte = u8::from_str_radix(h, 16).map_err(|e| e.to_string())?;
                    hexes.get_or_insert_with(Vec::new).push(byte);
                }
            }
            Token::Char(t) => {
                if curskip > 0 {
                    curskip -= 1;
                } else if !ignorable && !suppress_output {
                    out.push_str(t);
                }
            }
            Token::Newlines => {}
        }
    }
    Ok(out)
}

/// RTF 파일 내용(UTF-8)에서 (요약, 줄 수). 변환에 실패하면 `Err`.
pub fn read_from_rtf_text(content: &str) -> Result<(String, usize), String> {
    let result = rtf_to_text(content)?;
    let line_count = if result.is_empty() { 0 } else { result.matches('\n').count() + 1 };
    let cleaned = keep_word_chars(&result);
    Ok((head_chars(&cleaned, TEXT_SIZE).to_owned(), line_count))
}

#[cfg(test)]
mod tests {
    use super::{Token, Tokens};
    use regex::Regex;
    use std::sync::LazyLock;

    /// 토크나이저가 대신하는 원래 정규식. 같은 입력에서 같은 토큰열을 내야 한다.
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\\([a-z]{1,32})(-?\d{1,10})?[ ]?|\\'([0-9a-f]{2})|\\([^a-z])|([{}])|[\r\n]+|(.)").unwrap());

    fn reference(text: &str) -> Vec<Token<'_>> {
        PATTERN
            .captures_iter(text)
            .map(|caps| {
                let s = |i: usize| caps.get(i).map(|m| m.as_str());
                if let Some(word) = s(1) {
                    Token::Word(word, s(2))
                } else if let Some(h) = s(3) {
                    Token::Hex(h)
                } else if let Some(c) = s(4) {
                    Token::Ch(c)
                } else if let Some(b) = s(5) {
                    Token::Brace(b)
                } else if let Some(t) = s(6) {
                    Token::Char(t)
                } else {
                    Token::Newlines
                }
            })
            .collect()
    }

    fn tokens(text: &str) -> Vec<Token<'_>> {
        Tokens { text, pos: 0 }.collect()
    }

    #[test]
    fn tokenizer_matches_the_regex_on_edge_cases() {
        let long = format!("\\{}12 x", "a".repeat(40));
        let cases = [
            r"{\rtf1\ansi\ansicpg949 Hello\par}",
            r"\'b0\'A1\'g0\'b\'",
            r"\u-1234? \u65535 \uc1 \u12345678901",
            "\\\r\n\\\n\\{\\}\\\\\\-\\_\\~\\*",
            "\\\u{17f}ect \\\u{212a}elvin \\\u{130}x \\\u{131}x",
            "\\par\u{661}\u{662} \\f-\u{663} \\b- \\i-x \\tab  x",
            "\r\n\r\r\n텍스트é\\",
            long.as_str(),
            "",
        ];
        for case in cases {
            assert_eq!(tokens(case), reference(case), "{case:?}");
        }
    }

    #[test]
    fn tokenizer_matches_the_regex_on_generated_inputs() {
        let alphabet: Vec<char> = "\\\\\\\\''abfgkszAFKSZ\u{17f}\u{212a}\u{130}\u{131}0129\u{661}- {}\r\n한é".chars().collect();
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..3000 {
            let len = (seed % 24) as usize;
            let text: String = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    alphabet[(seed % alphabet.len() as u64) as usize]
                })
                .collect();
            assert_eq!(tokens(&text), reference(&text), "{text:?}");
        }
    }
}
