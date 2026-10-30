//! 위험 점수와 조합 승격. 점수는 출력 필터가 지표를 숨기기 전의 지표 전체로 계산한다.

use crate::finding::{Category, FileReport, Finding};
use crate::severity::Severity;

/// 단독으로는 소음인 MEDIUM 지표 중 함께 나오면 HIGH로 올리는 조합: (왼쪽 코드, 함께 나오면 올리는 코드들).
/// 지표가 거의 모든 파일에 나오므로 조합은 실제 표본에서 소음이 없음을 확인한 것만 둔다.
pub const COMBINATIONS: &[(&str, &[&str])] = &[("OBFUSCATION", &["NETWORK_API", "NAVIGATION", "DYNAMIC_IMPORT"])];

/// 조합 승격으로 만드는 지표 코드.
pub const COMBINED_CODE: &str = "COMBINED_SUSPICIOUS";

fn weight(s: Severity) -> u32 {
    match s {
        Severity::Low => 10,
        Severity::Medium => 30,
        Severity::High => 70,
        Severity::Critical => 100,
    }
}

/// 0~100 점수. 지표가 없으면 0이다. 있으면 `최고 가중치 + 5 × (MEDIUM 이상 지표 수 − 1)`이고 100을 넘지 않는다.
/// 가중치는 CRITICAL 100, HIGH 70, MEDIUM 30, LOW 10이다. MEDIUM 이상이 없으면 최고 가중치만이다.
pub fn risk_score(findings: &[Finding]) -> u8 {
    let Some(max) = findings.iter().map(|f| weight(f.severity)).max() else { return 0 };
    let serious = findings.iter().filter(|f| f.severity >= Severity::Medium).count() as u32;
    let bonus = 5 * serious.saturating_sub(1);
    (max + bonus).min(100) as u8
}

/// `COMBINATIONS`의 조합이 보고서에 있으면 `COMBINED_SUSPICIOUS`(보안, HIGH)를 한 번만 더한다.
pub fn escalate(report: &mut FileReport) {
    if report.has(COMBINED_CODE) {
        return;
    }
    for (left, partners) in COMBINATIONS {
        if !report.has(left) {
            continue;
        }
        if let Some(partner) = partners.iter().find(|p| report.has(p)) {
            report.push(Category::Security, Severity::High, COMBINED_CODE, "", &format!("{left} + {partner}"));
            return;
        }
    }
}
