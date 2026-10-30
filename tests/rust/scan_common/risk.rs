//! scan_common::risk 테스트: 위험 점수와 조합 승격.

use std::path::Path;

use scan_common::risk::{COMBINATIONS, escalate, risk_score};
use scan_common::{Category, FileReport, Finding, Format, Severity};

fn findings(severities: &[Severity]) -> Vec<Finding> {
    let mut r = FileReport::new(Path::new("a"), None);
    for (i, s) in severities.iter().enumerate() {
        r.push(Category::Security, *s, &format!("C{i}"), "", "");
    }
    r.findings
}

#[test]
fn risk_score_empty_is_zero() {
    assert_eq!(risk_score(&[]), 0);
}

#[test]
fn risk_score_uses_the_max_weight() {
    assert_eq!(risk_score(&findings(&[Severity::Low])), 10);
    assert_eq!(risk_score(&findings(&[Severity::Medium])), 30);
    assert_eq!(risk_score(&findings(&[Severity::High])), 70);
    assert_eq!(risk_score(&findings(&[Severity::Critical])), 100);
    assert_eq!(risk_score(&findings(&[Severity::Low, Severity::High, Severity::Medium])), 70 + 5);
}

#[test]
fn risk_score_adds_five_per_extra_medium_or_higher() {
    assert_eq!(risk_score(&findings(&[Severity::High, Severity::Medium, Severity::Medium])), 80);
    assert_eq!(risk_score(&findings(&[Severity::Medium, Severity::Medium])), 35);
}

#[test]
fn risk_score_caps_at_100() {
    assert_eq!(risk_score(&findings(&[Severity::High; 10])), 100);
    assert_eq!(risk_score(&findings(&[Severity::Critical; 3])), 100);
}

#[test]
fn risk_score_ignores_low_for_the_bonus() {
    assert_eq!(risk_score(&findings(&[Severity::High, Severity::Low, Severity::Low])), 70);
}

fn report_with(codes: &[&str]) -> FileReport {
    let mut r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
    for c in codes {
        r.push(Category::Security, Severity::Medium, c, "x.js", "d");
    }
    r
}

#[test]
fn escalate_adds_combined_when_a_pair_is_present() {
    let mut r = report_with(&["OBFUSCATION", "NETWORK_API"]);
    escalate(&mut r);
    let f = r.findings.iter().find(|f| f.code == "COMBINED_SUSPICIOUS").expect("combined");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert_eq!(f.detail, "OBFUSCATION + NETWORK_API");
}

#[test]
fn escalate_covers_every_partner_in_the_table() {
    for (left, partners) in COMBINATIONS {
        for partner in *partners {
            let mut r = report_with(&[left, partner]);
            escalate(&mut r);
            assert!(r.has("COMBINED_SUSPICIOUS"), "{left} + {partner}");
        }
    }
}

#[test]
fn escalate_does_nothing_when_only_one_side_is_present() {
    for codes in [&["OBFUSCATION"][..], &["NETWORK_API"][..], &["NETWORK_API", "NAVIGATION"][..], &[][..]] {
        let mut r = report_with(codes);
        escalate(&mut r);
        assert!(!r.has("COMBINED_SUSPICIOUS"), "{codes:?}");
    }
}

#[test]
fn escalate_is_idempotent_and_adds_one_finding_for_many_partners() {
    let mut r = report_with(&["OBFUSCATION", "NETWORK_API", "NAVIGATION"]);
    escalate(&mut r);
    escalate(&mut r);
    assert_eq!(r.findings.iter().filter(|f| f.code == "COMBINED_SUSPICIOUS").count(), 1);
}

#[cfg(feature = "serde")]
#[test]
fn json_always_has_risk_score() {
    let r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["risk_score"], 0);
}
