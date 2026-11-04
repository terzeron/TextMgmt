//! 출력. 텍스트 블록, NDJSON 한 줄, 요약 줄을 쓴다. 발견도 미완료도 없는 정상 파일은 건너뛴다.
//!
//! JSON은 전부 `serde_json`으로 만든다. 경로·상세에는 공격자가 정한 문자열이 들어가므로 이스케이프를 손으로 쓰지 않는다.

use std::io::{self, Write};

use crate::display::sanitize_display;
use crate::display::{Style, paint};
use crate::finding::{ENCRYPTION_CODES, Suppressed};
use crate::{Category, FileReport, Finding, Severity, Summary};

/// 출력할 내용이 있는가: 발견이 있거나 검사를 끝내지 못했다.
fn is_flagged(r: &FileReport) -> bool {
    !r.findings.is_empty() || r.incomplete
}

/// 터미널에 안전한 한 줄. 제어문자와 방향·서식 문자는 `.`로 바뀐다.
fn safe(s: &str) -> String {
    sanitize_display(s, usize::MAX, None)
}

fn io_err(e: serde_json::Error) -> io::Error {
    io::Error::other(e)
}

/// 지표 줄의 색. 암호화·DRM은 cyan, 렌더링 구조 에러(`render`·`defect`)와 CRITICAL은 red, HIGH 보안 지표는 purple이다.
pub fn finding_style(f: &Finding) -> Option<Style> {
    if ENCRYPTION_CODES.contains(&f.code.as_str()) {
        Some(Style::Cyan)
    } else if matches!(f.category, Category::Render | Category::Defect) || f.severity == Severity::Critical {
        Some(Style::Red)
    } else if f.severity == Severity::High {
        Some(Style::Purple)
    } else {
        None
    }
}

fn paint_opt(line: String, style: Option<Style>, color: bool) -> String {
    match style {
        Some(s) => paint(&line, s, color),
        None => line,
    }
}

/// 파일 1건당 텍스트 블록: 경로 한 줄, 이어서 발견과 미완료 이유를 한 줄씩 쓴다. `color`면 줄 전체에 색을 입힌다.
/// 색 코드는 `safe()`를 거친 문자열 바깥에만 붙는다.
pub fn write_text(r: &FileReport, out: &mut dyn Write, color: bool) -> io::Result<()> {
    if !is_flagged(r) {
        return Ok(());
    }
    writeln!(out, "{}", safe(&r.path))?;
    for reason in r.reasons.iter().filter(|_| r.incomplete) {
        let style = FileReport::is_encrypted_reason(reason).then_some(Style::Cyan);
        writeln!(out, "{}", paint_opt(format!("  INCOMPLETE {}", safe(reason)), style, color))?;
    }
    for f in &r.findings {
        let line = format!("  [{}] {} {} {}", f.severity.label(), safe(&f.code), safe(&f.location), safe(&f.detail));
        writeln!(out, "{}", paint_opt(line, finding_style(f), color))?;
    }
    Ok(())
}

/// NDJSON 1줄.
pub fn write_json_line(r: &FileReport, out: &mut dyn Write) -> io::Result<()> {
    if !is_flagged(r) {
        return Ok(());
    }
    serde_json::to_writer(&mut *out, r).map_err(io_err)?;
    out.write_all(b"\n")
}

/// 텍스트 모드의 마지막 요약 한 줄.
pub fn write_summary_line(agg: &Aggregate, summary: &Summary, out: &mut dyn Write) -> io::Result<()> {
    write!(out, "검사 {}건, 발견 {}건, 검사하지 못함 {}건", summary.scanned, agg.findings(), agg.incomplete())?;
    if agg.executables() > 0 {
        write!(out, ", 실행 파일 {}건", agg.executables())?;
    }
    let gone = agg.suppressed();
    let parts: Vec<String> = [("LOW", gone.low), ("MEDIUM", gone.medium), ("HIGH", gone.high)].iter().filter(|(_, n)| *n > 0).map(|(name, n)| format!("{name} {n}건")).collect();
    if !parts.is_empty() {
        write!(out, ", 억제 {}", parts.join(", "))?;
    }
    writeln!(out)
}

/// 요약 줄에 넣을 건수를 모은다. 발견이나 미완료가 있는 파일만 센다.
#[derive(Debug, Default)]
pub struct Aggregate {
    findings: usize,
    incomplete: usize,
    suppressed: Suppressed,
    executables: usize,
}

impl Aggregate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, r: &FileReport) {
        // 숨긴 지표가 있어도 출력할 것이 없는 파일은 아래에서 건너뛴다. 억제 건수는 그 전에 센다.
        self.suppressed.low += r.suppressed.low;
        self.suppressed.medium += r.suppressed.medium;
        self.suppressed.high += r.suppressed.high;
        self.executables += usize::from(r.confirmed_executable);
        if !is_flagged(r) {
            return;
        }
        self.findings += r.findings.len();
        self.incomplete += usize::from(r.incomplete);
    }

    /// 출력 필터가 숨긴 지표의 심각도별 건수.
    pub fn suppressed(&self) -> Suppressed {
        self.suppressed
    }

    /// 확장자와 매직바이트가 모두 실행 파일인 파일 수.
    pub fn executables(&self) -> usize {
        self.executables
    }

    /// 발견 총 건수.
    pub fn findings(&self) -> usize {
        self.findings
    }

    /// 검사하지 못한 파일 수.
    pub fn incomplete(&self) -> usize {
        self.incomplete
    }
}
