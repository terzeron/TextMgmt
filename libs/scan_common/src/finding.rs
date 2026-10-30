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
    /// 확장자가 실행 파일인 것. 매직바이트까지 맞으면 `confirmed_executable`이다.
    Executable,
    Zip,
    Docx,
    Hwpx,
    /// 구형 `.doc`·`.dot`(OLE 복합 파일).
    Doc,
    /// `.hwp`(HWP 5.x OLE, 서명만 보는 HWP 3.x 이하).
    Hwp,
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

/// 암호화·DRM을 뜻하는 지표 코드. 출력에서 cyan으로 칠한다.
pub const ENCRYPTION_CODES: &[&str] = &["ENCRYPTED"];

/// 암호화·DRM 때문에 검사하지 못했다는 사유의 표지 문구. 사유 앞에 위치(멤버 이름)가 붙을 수 있어 포함 여부로 본다.
/// 이름에 `encrypted`가 들어 있을 뿐인 다른 사유와 섞이지 않도록 알려진 문구만 쓴다.
const ENCRYPTED_REASON_MARKERS: &[&str] = &["encrypted: ", "ZIP-encrypted entry"];

/// 출력 필터가 지운 지표의 심각도별 건수. 점수·요약에서 "숨겼을 뿐 있었다"를 남기려고 센다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Suppressed {
    pub low: usize,
    pub medium: usize,
    pub high: usize,
}

impl Suppressed {
    /// 지운 지표가 하나도 없는가.
    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    /// 지운 지표의 총 건수.
    pub fn total(&self) -> usize {
        self.low + self.medium + self.high
    }
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
    /// `suppress_below`가 지운 지표의 건수. 지운 것이 없으면 JSON에 나오지 않는다.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Suppressed::is_empty"))]
    pub suppressed: Suppressed,
    /// 확장자와 매직바이트가 모두 실행 파일이다. 출력 필터가 지표를 지워도 집계가 남도록 따로 둔다.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "std::ops::Not::not"))]
    pub confirmed_executable: bool,
    /// 0~100 위험 점수. 출력 필터가 숨긴 지표도 포함해 `scan_file`이 정한다.
    pub risk_score: u8,
}

impl FileReport {
    /// 빈 보고서. 경로는 원문 그대로 둔다(JSON은 serde가 이스케이프하고, `quarantine_broken`이 실제 파일을 찾으려면 원문이어야 한다).
    /// 터미널에 찍는 쪽이 `text::sanitize_display`로 제어·방향 문자를 지운다.
    pub fn new(path: &Path, format: Option<Format>) -> Self {
        let path = path.to_string_lossy().into_owned();
        FileReport { path, format, findings: Vec::new(), incomplete: false, reasons: Vec::new(), likeness: None, suppressed: Suppressed::default(), confirmed_executable: false, risk_score: 0 }
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

    /// 이 미완료 사유가 암호화·DRM 때문인가.
    pub fn is_encrypted_reason(reason: &str) -> bool {
        ENCRYPTED_REASON_MARKERS.iter().any(|m| reason.contains(m))
    }

    /// `min`보다 낮은 심각도의 지표를 지우고 심각도별 건수를 `suppressed`에 더한다. 미완료 상태는 건드리지 않는다.
    pub fn suppress_below(&mut self, min: Severity) {
        let mut gone = Suppressed::default();
        self.findings.retain(|f| {
            if f.severity >= min {
                return true;
            }
            match f.severity {
                Severity::Low => gone.low += 1,
                Severity::Medium => gone.medium += 1,
                Severity::High => gone.high += 1,
                Severity::Critical => {}
            }
            false
        });
        self.suppressed.low += gone.low;
        self.suppressed.medium += gone.medium;
        self.suppressed.high += gone.high;
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
