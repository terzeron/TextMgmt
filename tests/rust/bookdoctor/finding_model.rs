//! `bookdoctor::finding_model` 단위 테스트.

mod support;
use std::path::Path;
use bookdoctor::finding_model::{COMBINATIONS, escalate, risk_score, Suppressed};
use bookdoctor::{Category, FileReport, Finding, Format, Report, Severity};

#[test]
fn push_records_a_finding_and_report_trait_sees_it() {
    let mut r = FileReport::new(Path::new("a.txt"), Some(Format::Txt));
    r.push(Category::Defect, Severity::High, "NOT_UTF8", "", "encoding=cp949");
    assert!(r.has("NOT_UTF8") && !r.has("X"));
    assert_eq!(r.severities().collect::<Vec<_>>(), vec![Severity::High]);
    assert!(!r.is_incomplete());
}

#[test]
fn mark_incomplete_keeps_the_reason() {
    let mut r = FileReport::new(Path::new("a.pdf"), Some(Format::Pdf));
    r.mark_incomplete("skipped");
    assert_eq!(r.reasons, vec!["skipped".to_string()]);
    assert!(r.is_incomplete());
}

#[test]
fn path_keeps_the_original_name_so_quarantine_finds_the_real_file() {
    let r = FileReport::new(Path::new("a\u{1b}b\u{202e}.txt"), None);
    assert_eq!(r.path, "a\u{1b}b\u{202e}.txt");
}

#[test]
fn json_uses_lowercase_category_and_na_viewer() {
    let mut r = FileReport::new(Path::new("a.txt"), Some(Format::Txt));
    r.push(Category::Defect, Severity::High, "NOT_UTF8", "", "d");
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["format"], "txt");
    assert_eq!(v["findings"][0]["category"], "defect");
    assert_eq!(v["findings"][0]["severity"], "HIGH");
    assert_eq!(v["findings"][0]["viewer_blocked"], "n/a");
}

fn mixed_report() -> FileReport {
    let mut r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
    r.push(Category::Defect, Severity::Low, "L1", "", "");
    r.push(Category::Defect, Severity::Low, "L2", "", "");
    r.push(Category::Render, Severity::Medium, "M1", "", "");
    r.push(Category::Security, Severity::High, "H1", "", "");
    r
}

#[test]
fn suppress_below_removes_lower_and_counts_by_severity() {
    let mut r = mixed_report();
    r.suppress_below(Severity::High);
    assert_eq!(r.findings.len(), 1);
    assert!(r.has("H1"));
    assert_eq!(r.suppressed, Suppressed { low: 2, medium: 1, high: 0 });
    assert_eq!(r.suppressed.total(), 3);
    assert!(!r.suppressed.is_empty());
}

#[test]
fn suppress_below_low_keeps_everything() {
    let mut r = mixed_report();
    r.suppress_below(Severity::Low);
    assert_eq!(r.findings.len(), 4);
    assert!(r.suppressed.is_empty());
}

#[test]
fn suppress_below_critical_also_counts_high() {
    let mut r = mixed_report();
    r.suppress_below(Severity::Critical);
    assert!(r.findings.is_empty());
    assert_eq!(r.suppressed, Suppressed { low: 2, medium: 1, high: 1 });
}

#[test]
fn suppress_below_does_not_touch_incomplete_state() {
    let mut r = mixed_report();
    r.mark_incomplete("skipped");
    r.suppress_below(Severity::Critical);
    assert!(r.is_incomplete());
    assert_eq!(r.reasons, vec!["skipped".to_string()]);
}

#[test]
fn suppressed_is_omitted_from_json_when_empty_and_present_otherwise() {
    let mut r = mixed_report();
    let v = serde_json::to_value(&r).unwrap();
    assert!(v.get("suppressed").is_none(), "{v}");
    r.suppress_below(Severity::High);
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["suppressed"]["low"], 2);
    assert_eq!(v["suppressed"]["medium"], 1);
    assert_eq!(v["suppressed"]["high"], 0);
}

#[test]
fn encrypted_reason_markers_are_specific() {
    assert!(FileReport::is_encrypted_reason("encrypted: string values may be unreadable"));
    assert!(FileReport::is_encrypted_reason("a.bin: ZIP-encrypted entry cannot be reliably inspected"));
    // 이름에 encrypted가 들어 있을 뿐인 다른 사유는 암호화로 보지 않는다.
    assert!(!FileReport::is_encrypted_reason("encrypted.xml: OPF is not well-formed XML"));
    assert!(!FileReport::is_encrypted_reason("skipped: 5 bytes exceeds --max-total-mb"));
}

#[test]
fn encryption_codes_include_the_pdf_encrypted_code() {
    assert!(bookdoctor::finding_model::ENCRYPTION_CODES.contains(&"ENCRYPTED"));
}

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

#[test]
fn json_always_has_risk_score() {
    let r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["risk_score"], 0);
}

#[test]
fn severity_orders_from_low_to_critical_and_has_upper_case_labels() {
    assert!(Severity::Low < Severity::Medium && Severity::Medium < Severity::High && Severity::High < Severity::Critical);
    let labels: Vec<_> = [Severity::Low, Severity::Medium, Severity::High, Severity::Critical].iter().map(|s| s.label()).collect();
    assert_eq!(labels, ["LOW", "MEDIUM", "HIGH", "CRITICAL"]);
}

#[test]
fn severity_serializes_as_its_upper_case_label() {
    assert_eq!(serde_json::to_string(&Severity::Critical).unwrap(), "\"CRITICAL\"");
    assert_eq!(serde_json::to_string(&Severity::Low).unwrap(), "\"LOW\"");
}

#[test]
fn severity_parse_accepts_four_lowercase_names_and_rejects_others() {
    assert_eq!(Severity::parse("low"), Some(Severity::Low));
    assert_eq!(Severity::parse("medium"), Some(Severity::Medium));
    assert_eq!(Severity::parse("high"), Some(Severity::High));
    assert_eq!(Severity::parse("critical"), Some(Severity::Critical));
    for bad in ["", "HIGH", "High", "info", "highest", " high"] {
        assert_eq!(Severity::parse(bad), None, "{bad:?}");
    }
}
