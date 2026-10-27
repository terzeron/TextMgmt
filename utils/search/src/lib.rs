//! ES 제목·내용 검색 점검 도구. 원본: `utils/search.py`.
//!
//! 제목(`<title>`, `<ext>`, `<size>`)으로 찾고, 내용(`<content>`)이 있으면 요약이 비슷한 문서도 찾는다.

use std::io::Write;

use es_client::{BookSearch, EsError, Hit};
use pyfmt::py_repr;
use serde_json::Value;

fn parse_python_int(text: &str) -> Option<i64> {
    let value = text.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c));
    let (sign, digits) = match value.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => match value.strip_prefix('+') {
            Some(digits) => ("+", digits),
            None => ("", value),
        },
    };
    let chars: Vec<char> = digits.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let mut normalized = String::with_capacity(value.len());
    normalized.push_str(sign);
    for (i, c) in chars.iter().copied().enumerate() {
        if let Some(digit) = pyfmt::py_decimal_digit(c) {
            normalized.push(char::from(b'0' + digit));
        } else if c == '_' && i > 0 && i + 1 < chars.len() && pyfmt::py_decimal_digit(chars[i - 1]).is_some() && pyfmt::py_decimal_digit(chars[i + 1]).is_some() {
            continue;
        } else {
            return None;
        }
    }
    normalized.parse().ok()
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = write!(
        out,
        "Usage:\t{program}\t<title> [ <content> [ <ext> [ <size> ] ] ]

{program} - ES 도서 색인을 제목과 내용으로 검색해 결과를 보여 주는 점검 도구.

인자:
  <title>    제목으로 검색한다. <ext>·<size>가 있으면 같이 점수에 반영한다.
  <content>  요약이 비슷한 문서를 찾는다. 이 인자만으로 검색한다.
  <ext>      파일 확장자 (예: epub, txt). 생략하면 사용하지 않는다.
  <size>     파일 크기(바이트). 생략하면 0이다.

환경변수 (모두 필요):
  TM_ES_URL, TM_ES_USER, TM_ES_PASSWORD, TM_ES_BOOK_INDEX

종료 코드: 0 정상, 1 검색 실패, 255 환경변수 누락
"
    );
}

fn print_hits(hits: &[Hit], separator: bool, out: &mut dyn Write) -> std::io::Result<()> {
    for hit in hits {
        if separator {
            writeln!(out, "----------------------------------------")?;
        }
        writeln!(out, "item: {}", py_repr(&hit.source))?;
        // Python은 `item["summary"][:200]`로 바로 접근하므로 누락·비문자열 값을 오류로 처리한다.
        let summary = hit.source.get("summary").and_then(Value::as_str).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "검색 결과에 문자열 summary가 없다"))?;
        let head: String = summary.chars().take(200).collect();
        writeln!(out, "{}", head.replace('\n', " "))?;
    }
    Ok(())
}

/// 진입점. `connect`는 ES 접속을 만든다. 제목 인자가 없으면 접속하지 않고 사용법만 보인다.
/// 종료 코드: 정상 0, 환경변수 누락 255(원본 `sys.exit(-1)`), 그 밖의 실패 1.
pub fn run<'a>(program: &str, args: &[String], out: &mut dyn Write, connect: &dyn Fn() -> Result<&'a dyn BookSearch, EsError>) -> i32 {
    let Some(title) = args.first().filter(|a| !matches!(a.as_str(), "-h" | "--help")) else {
        print_usage(program, out);
        return 0;
    };
    let content = args.get(1).map_or("", String::as_str);
    let ext = args.get(2).map_or("", String::as_str);
    let size: i64 = match args.get(3).map(|s| parse_python_int(s)) {
        None => 0,
        Some(Some(n)) => n,
        Some(None) => {
            eprintln!("ERROR <size>는 정수여야 한다: {}", args[3]);
            return 1;
        }
    };

    let es = match connect() {
        Ok(es) => es,
        Err(e @ EsError::MissingEnv(_)) => {
            eprintln!("ERROR {e}");
            return 255;
        }
        Err(e) => {
            eprintln!("ERROR {e}");
            return 1;
        }
    };

    if !title.is_empty() {
        match es.search_by_title(title, ext, size) {
            Ok(hits) => {
                if let Err(e) = print_hits(&hits, true, out) {
                    eprintln!("ERROR {e}");
                    return 1;
                }
            }
            Err(e) => {
                eprintln!("ERROR {e}");
                return 1;
            }
        }
    }
    if !content.is_empty() {
        match es.search_similar_docs(content) {
            Ok(hits) => {
                if let Err(e) = print_hits(&hits, false, out) {
                    eprintln!("ERROR {e}");
                    return 1;
                }
            }
            Err(e) => {
                eprintln!("ERROR {e}");
                return 1;
            }
        }
    }
    0
}
