//! 출력. 텍스트 블록, NDJSON 한 줄, 리포트 파일을 쓴다. 발견도 미완료도 없는 정상 파일은 건너뛴다.
//!
//! JSON은 전부 `serde_json`으로 만든다. 경로·상세에는 공격자가 정한 문자열이 들어가므로 이스케이프를 손으로 쓰지 않는다.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use scan_common::text::sanitize_display;
use scan_common::finding::Suppressed;
use scan_common::{FileReport, Summary};
use serde::Serialize;

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

/// 파일 1건당 텍스트 블록: 경로 한 줄, 이어서 발견과 미완료 이유를 한 줄씩 쓴다.
pub fn write_text(r: &FileReport, out: &mut dyn Write) -> io::Result<()> {
    if !is_flagged(r) {
        return Ok(());
    }
    writeln!(out, "{}", safe(&r.path))?;
    for reason in r.reasons.iter().filter(|_| r.incomplete) {
        writeln!(out, "  INCOMPLETE {}", safe(reason))?;
    }
    for f in &r.findings {
        writeln!(out, "  [{}] {} {} {}", f.severity.label(), safe(&f.code), safe(&f.location), safe(&f.detail))?;
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
    let gone = agg.suppressed();
    let parts: Vec<String> = [("LOW", gone.low), ("MEDIUM", gone.medium), ("HIGH", gone.high)].iter().filter(|(_, n)| *n > 0).map(|(name, n)| format!("{name} {n}건")).collect();
    if !parts.is_empty() {
        write!(out, ", 억제 {}", parts.join(", "))?;
    }
    writeln!(out)
}

/// 리포트 파일에 넣을 보고서를 모은다. 발견이나 미완료가 있는 `FileReport`만 보관한다.
#[derive(Debug, Default)]
pub struct Aggregate {
    files: Vec<FileReport>,
    findings: usize,
    incomplete: usize,
    by_category: BTreeMap<&'static str, usize>,
    suppressed: Suppressed,
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
        if !is_flagged(r) {
            return;
        }
        self.findings += r.findings.len();
        self.incomplete += usize::from(r.incomplete);
        for f in &r.findings {
            let name = match f.category {
                scan_common::Category::Security => "security",
                scan_common::Category::Defect => "defect",
                scan_common::Category::Render => "render",
            };
            *self.by_category.entry(name).or_default() += 1;
        }
        self.files.push(r.clone());
    }

    /// 출력 필터가 숨긴 지표의 심각도별 건수.
    pub fn suppressed(&self) -> Suppressed {
        self.suppressed
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

#[derive(Serialize)]
struct SummaryOut<'a> {
    scanned: usize,
    by_severity: &'a BTreeMap<&'static str, usize>,
    by_category: &'a BTreeMap<&'static str, usize>,
    incomplete: usize,
    suppressed: Suppressed,
}

#[derive(Serialize)]
struct ReportOut<'a> {
    scanned_at: String,
    root: &'a str,
    summary: SummaryOut<'a>,
    files: &'a [FileReport],
}

/// 리포트 파일을 쓴다.
pub fn write_report_file(path: &Path, agg: &Aggregate, summary: &Summary, root: &str) -> io::Result<()> {
    let doc = ReportOut {
        scanned_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        root,
        summary: SummaryOut { scanned: summary.scanned, by_severity: &summary.by_severity, by_category: &agg.by_category, incomplete: agg.incomplete, suppressed: agg.suppressed },
        files: &agg.files,
    };
    let mut f = io::BufWriter::new(fs::File::create(path)?);
    serde_json::to_writer_pretty(&mut f, &doc).map_err(io_err)?;
    f.write_all(b"\n")?;
    f.flush()
}
