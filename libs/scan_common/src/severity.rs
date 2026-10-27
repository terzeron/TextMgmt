//! 지표의 심각도.

/// 지표의 심각도. 큰 쪽이 더 위험하다.
///
/// `Critical`은 알려진 공격 수법이거나 정상 문서에 있을 이유가 없는 구조다. `High`는 "실행 가능한 구조가 있다"까지만 뜻하고
/// 정상 인터랙티브 문서에도 흔하다. 둘을 섞으면 `High` 수천 건 속에서 실제 공격이 묻힌다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize), serde(rename_all = "UPPERCASE"))]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// 출력에 쓰는 대문자 이름.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Low => "LOW",
            Severity::Medium => "MEDIUM",
            Severity::High => "HIGH",
            Severity::Critical => "CRITICAL",
        }
    }
}
