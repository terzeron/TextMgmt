//! 파일 하나의 진단 결과 모델. bookdoctor가 채우고 출력한다.

use std::path::Path;

use crate::severity::Severity;
use crate::stream::Report;

/// 파일 포맷.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize), serde(rename_all = "lowercase"))]
pub enum Format {
    Epub,
    Pdf,
    Txt,
}

/// 지표의 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize), serde(rename_all = "lowercase"))]
pub enum Category {
    Security,
    Defect,
    Render,
}

/// 뷰어가 이 지표를 막는지. 판정하기 전이거나 해당 없으면 `NotApplicable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ViewerBlocked {
    #[cfg_attr(feature = "serde", serde(rename = "blocked"))]
    Blocked,
    #[cfg_attr(feature = "serde", serde(rename = "not_blocked"))]
    NotBlocked,
    #[cfg_attr(feature = "serde", serde(rename = "n/a"))]
    NotApplicable,
}

/// 지표 한 건.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Finding {
    pub category: Category,
    pub severity: Severity,
    pub code: String,
    pub location: String,
    pub detail: String,
    pub viewer_blocked: ViewerBlocked,
}

/// 파일 하나의 보고서.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileReport {
    pub path: String,
    pub format: Option<Format>,
    pub findings: Vec<Finding>,
    pub incomplete: bool,
    pub reasons: Vec<String>,
    pub likeness: Option<f64>,
}

impl FileReport {
    /// 빈 보고서. 경로는 원문 그대로 둔다(JSON은 serde가 이스케이프하고, `quarantine_broken`이 실제 파일을 찾으려면 원문이어야 한다).
    /// 터미널에 찍는 쪽이 `text::sanitize_display`로 제어·방향 문자를 지운다.
    pub fn new(path: &Path, format: Option<Format>) -> Self {
        let path = path.to_string_lossy().into_owned();
        FileReport { path, format, findings: Vec::new(), incomplete: false, reasons: Vec::new(), likeness: None }
    }

    /// 지표를 더한다. `viewer_blocked`는 나중에 채우므로 `NotApplicable`로 둔다.
    pub fn push(&mut self, category: Category, severity: Severity, code: &str, location: &str, detail: &str) {
        self.findings.push(Finding {
            category,
            severity,
            code: code.to_string(),
            location: location.to_string(),
            detail: detail.to_string(),
            viewer_blocked: ViewerBlocked::NotApplicable,
        });
    }

    /// 전체를 검사하지 못했다고 표시하고 이유를 남긴다.
    pub fn mark_incomplete(&mut self, reason: &str) {
        self.incomplete = true;
        self.reasons.push(reason.to_string());
    }

    /// 이 코드의 지표가 있는가.
    pub fn has(&self, code: &str) -> bool {
        self.findings.iter().any(|f| f.code == code)
    }
}

impl Report for FileReport {
    fn severities(&self) -> impl Iterator<Item = Severity> + '_ {
        self.findings.iter().map(|f| f.severity)
    }

    fn is_incomplete(&self) -> bool {
        self.incomplete
    }
}
