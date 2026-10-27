//! 명령행 옵션 읽기. argparse와 같은 규칙과 같은 오류 문구를 쓴다.

use std::str::FromStr;

/// 정수 옵션 값을 읽는다.
pub fn number<T: FromStr>(name: &str, value: &str) -> Result<T, String> {
    value.parse().map_err(|_| format!("argument --{name}: invalid int value: '{value}'"))
}

/// `--이름=값` 또는 `--이름 값` 꼴의 긴 옵션을 읽는다. 값이 따로 오면 `rest`의 다음 인자를 쓴다.
///
/// - `--`로 시작하지 않으면 `Ok(None)`: 옵션이 아니라 위치 인자다.
/// - `allowed`에 없는 이름이면 오류(`unrecognized arguments`). 값이 없으면 오류(`expected one argument`).
pub fn long_option<'a, I>(arg: &str, allowed: &[&str], rest: &mut I) -> Result<Option<(String, String)>, String>
where
    I: Iterator<Item = &'a String>,
{
    let Some(body) = arg.strip_prefix("--") else { return Ok(None) };
    let (name, inline) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v.to_owned())),
        None => (body, None),
    };
    if !allowed.contains(&name) {
        return Err(format!("unrecognized arguments: {arg}"));
    }
    let value = inline.or_else(|| rest.next().cloned()).ok_or_else(|| format!("argument --{name}: expected one argument"))?;
    Ok(Some((name.to_owned(), value)))
}
