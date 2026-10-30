//! 파일 이름 규칙.

const WIN_RESERVED: &[&str] =
    &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];

/// 경로 성분이 Windows 예약 장치 이름인가. 확장자와 끝의 점·공백은 무시하고 대소문자를 구분하지 않는다(`com1.txt`, `NUL. `).
pub fn windows_reserved(component: &str) -> bool {
    let stem = component.trim_end_matches(['.', ' ']).split('.').next().unwrap_or("").to_lowercase();
    WIN_RESERVED.contains(&stem.as_str())
}
