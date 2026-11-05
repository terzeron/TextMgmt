//! `--sanitize`: 보안 지표가 있는 EPUB·PDF를 허용 목록으로 다시 쓴다.
//!
//! 파일을 모으고, 검사하고, `doc_checker`의 재구성(`rebuild`)과 검증(`verify`)을 거친 뒤 재검사하고 적용한다.
//! 재구성 기법은 `doc_checker::epub_sanitizer`와 `doc_checker::pdf::pdf_sanitizer`에 있다.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use doc_checker::epub_sanitizer;
use doc_checker::format_dispatcher::scan_file;
use doc_checker::pdf::pdf_sanitizer;

use crate::cli_parser::Options;
use crate::file_rewriter::{apply_and_report, has_serious_security, rescan_bytes};
use crate::output_formatter::show;
use crate::{Category, Severity};
use doc_checker::CheckOptions;

/// 재구성본을 임시 파일에 써서 다시 검사한다. 보안 지표(HIGH 이상)가 하나라도 남으면 오류다: 허용 목록으로 고치지 못한 문제가 있는 파일은 적용하지 않는다.
fn rescan_epub(bytes: &[u8], opts: &CheckOptions) -> Result<(), String> {
    let report = rescan_bytes(bytes, "epub", opts)?;
    let left: Vec<String> = report.findings.iter().filter(|f| f.category == Category::Security && f.severity >= Severity::High).map(|f| f.code.to_string()).collect();
    if left.is_empty() { Ok(()) } else { Err(format!("재검사에서 보안 지표가 남았다: {}", left.join(", "))) }
}

/// 다시 쓴 결과를 임시 파일에 써서 다시 검사한다. 보안 지표(HIGH 이상)가 남으면 오류다.
fn rescan_pdf(bytes: &[u8], opts: &CheckOptions) -> Result<(), String> {
    if has_serious_security(&rescan_bytes(bytes, "pdf", opts)?) { Err("재검사에서 보안 지표가 남았다".to_owned()) } else { Ok(()) }
}

enum Outcome {
    Done,
    Untouched,
    Skipped,
    Failed,
}

/// PDF 하나를 처리한다. `--sanitize`의 EPUB 흐름과 같은 출력 형식을 쓴다.
fn process_pdf(path: &Path, shown: &str, opts: &Options, scan_opts: &CheckOptions, out: &mut dyn Write) -> io::Result<Outcome> {
    if !has_serious_security(&scan_file(path, scan_opts)) {
        return Ok(Outcome::Untouched);
    }
    let planned = fs::read(path).map_err(|e| e.to_string()).and_then(|src| pdf_sanitizer::rebuild(&src).map(|plan| (src, plan)));
    let (src, plan) = match planned {
        Ok(v) => v,
        Err(e) => {
            writeln!(out, "SKIP {shown}: {}", show(&e))?;
            return Ok(Outcome::Skipped);
        }
    };
    if plan.removed.is_empty() {
        writeln!(out, "UNCHANGED {shown}: 지울 요소가 없다(남은 지표는 이번 정리로 해결되지 않는다)")?;
        return Ok(Outcome::Untouched);
    }
    if let Err(e) = pdf_sanitizer::verify(&src, &plan.bytes).and_then(|()| rescan_pdf(&plan.bytes, scan_opts)) {
        writeln!(out, "SKIP {shown}: 검증 실패: {}", show(&e))?;
        return Ok(Outcome::Skipped);
    }
    writeln!(out, "SANITIZED {shown}: {}개 지움", plan.removed.len())?;
    for (location, what) in &plan.removed {
        writeln!(out, "  - 제거 {} ({})", show(location), show(what))?;
    }
    Ok(if apply_and_report(path, &plan.bytes, opts, shown, out)? { Outcome::Done } else { Outcome::Failed })
}

/// `--sanitize` 진입점. 종료 코드를 돌려준다(건너뛴 파일이 있어도 0, 쓰기 실패가 있으면 1).
pub fn run(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let paths = crate::cli_parser::collect(opts, |p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("epub") || e.eq_ignore_ascii_case("pdf")));
    let scan_opts = opts.check_options();
    let (mut done, mut untouched, mut skipped, mut failed) = (0usize, 0usize, 0usize, 0usize);
    for path in &paths {
        let shown = show(&path.to_string_lossy());
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            match process_pdf(path, &shown, opts, &scan_opts, out)? {
                Outcome::Done => done += 1,
                Outcome::Untouched => untouched += 1,
                Outcome::Skipped => skipped += 1,
                Outcome::Failed => failed += 1,
            }
            continue;
        }
        let report = scan_file(path, &scan_opts);
        if !has_serious_security(&report) {
            untouched += 1;
            continue;
        }
        let planned = fs::read(path).map_err(|e| e.to_string()).and_then(|src| epub_sanitizer::rebuild(&src).map(|plan| (src, plan)));
        let (src, plan) = match planned {
            Ok(v) => v,
            Err(e) => {
                writeln!(out, "SKIP {shown}: {}", show(&e))?;
                skipped += 1;
                continue;
            }
        };
        if plan.removed.is_empty() && plan.edited.is_empty() {
            writeln!(out, "UNCHANGED {shown}: 지울 요소가 없다(남은 지표는 허용 목록으로 해결되지 않는다)")?;
            untouched += 1;
            continue;
        }
        if let Err(e) = epub_sanitizer::verify(&src, &plan.bytes).and_then(|()| rescan_epub(&plan.bytes, &scan_opts)) {
            writeln!(out, "SKIP {shown}: 검증 실패: {}", show(&e))?;
            skipped += 1;
            continue;
        }
        writeln!(out, "SANITIZED {shown}: 멤버 {}개 제거, {}개 수정", plan.removed.len(), plan.edited.len())?;
        for (name, why) in &plan.removed {
            writeln!(out, "  - 제거 {} ({why})", show(name))?;
        }
        for (name, what) in &plan.edited {
            writeln!(out, "  - 수정 {} ({what})", show(name))?;
        }
        if !apply_and_report(path, &plan.bytes, opts, &shown, out)? {
            failed += 1;
            continue;
        }
        done += 1;
    }
    writeln!(out, "검사 {}건, 재구성 {done}건, 대상 아님·변경 없음 {untouched}건, 건너뜀 {skipped}건, 실패 {failed}건", paths.len())?;
    Ok(i32::from(failed > 0))
}
