//! 바이트 분포의 섀넌 엔트로피. 압축·암호화된 데이터는 8에 가깝고 텍스트는 대체로 5 안팎이다.

/// 텍스트로 읽히는 확장자(소문자). 이름이 이 확장자인데 내용이 높은 엔트로피면 렌더러가 읽지 못하는 숨은 데이터일 수 있다.
const TEXT_EXTENSIONS: &[&str] = &["xml", "rels", "html", "htm", "xhtml", "js", "css", "txt", "json", "svg"];

/// 섀넌 엔트로피(비트/바이트, 0.0~8.0). 빈 입력은 0.0이다.
pub fn shannon(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &b in bytes {
        counts[usize::from(b)] += 1;
    }
    let total = bytes.len() as f64;
    counts.iter().filter(|&&c| c > 0).map(|&c| {
        let p = c as f64 / total;
        -p * p.log2()
    }).sum()
}

/// 파일 이름(경로 포함 가능)이 텍스트 확장자인가. 대소문자는 가리지 않고, 줄기가 없는 `.xml`이나 확장자 없는 이름은 아니다.
pub fn is_text_name(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => TEXT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        _ => false,
    }
}
