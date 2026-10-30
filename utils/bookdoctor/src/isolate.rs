//! `--isolate`: 파일마다 하위 프로세스에서 메모리(`RLIMIT_AS`)와 시간 상한을 걸고 검사한다.
//!
//! 자식은 자기 자신(`--child`)이고 필터 없는 `FileReport`를 JSON 한 줄로 낸다. 부모는 그 줄을 읽어 출력 필터를 건다.
//! 자식이 죽거나(시그널·비정상 종료) 시간 상한을 넘기거나(프로세스 그룹 전체 종료) 출력을 읽을 수 없으면 그 파일만 `incomplete`다.
//! 메모리 상한은 `sh -c 'ulimit -v …; exec …'`로 건다(`libc`·`unsafe` 없이 표준 셸 기능만 쓴다).

use std::path::Path;
use std::time::Duration;

use scan_common::FileReport;
use scan_common::text::sanitize_display;
use scan_common::Category;
use sysdeps::{CommandError, CommandRunner, SystemRunner};

use crate::cli::Options;

/// 셸이 `ulimit`을 걸지 못했을 때의 종료 코드.
const ULIMIT_FAILED: i32 = 97;
/// 실패 사유에 붙이는 자식 stderr의 끝부분(글자 수).
const STDERR_TAIL: usize = 200;

fn only_arg(opts: &Options) -> Option<String> {
    (!opts.only.is_empty()).then(|| {
        opts.only
            .iter()
            .map(|c| match c {
                Category::Security => "security",
                Category::Defect => "defect",
                Category::Render => "render",
            })
            .collect::<Vec<_>>()
            .join(",")
    })
}

/// 자식에게 넘기는 인자. `--isolate`는 넘기지 않고, 자식은 지표를 걸러내지 않도록 `--min-severity low`다.
/// 출력 필터(`--min-severity`)는 부모가 건다. `--only`는 `scan_file` 안에서 적용되므로 같이 넘긴다.
fn child_args(opts: &Options, path: &Path) -> Vec<String> {
    let mut args = vec!["--child".to_owned(), "--min-severity".into(), "low".into(), "--max-total-mb".into(), opts.max_total_mb.to_string()];
    for (on, flag) in [(opts.zip_deep, "--zip-deep"), (!opts.recurse, "--no-recurse"), (opts.pdf_text, "--pdf-text"), (opts.no_poppler, "--no-poppler")] {
        if on {
            args.push(flag.to_owned());
        }
    }
    for file in &opts.rule_files {
        args.extend(["--rules".to_owned(), file.to_string_lossy().into_owned()]);
    }
    if opts.builtin_rules {
        args.push("--builtin-rules".to_owned());
    }
    if let Some(only) = only_arg(opts) {
        args.extend(["--only".to_owned(), only]);
    }
    // `-`로 시작하는 경로가 옵션으로 읽히지 않게 한다.
    let p = path.to_string_lossy();
    args.push(if p.starts_with('-') { format!("./{p}") } else { p.into_owned() });
    args
}

fn failed(path: &Path, why: &str) -> FileReport {
    let mut r = FileReport::new(path, None);
    r.mark_incomplete(&format!("isolated scan failed: {why}"));
    r
}

fn describe_exit(code: i32) -> String {
    match code {
        ULIMIT_FAILED => "memory limit could not be set".to_owned(),
        c if c < 0 => format!("exit status {c} (signal {})", -c),
        c => format!("exit status {c}"),
    }
}

/// `path` 하나를 하위 프로세스(`exe --child …`)에서 검사한 보고서를 돌려준다. 어떤 실패도 그 파일의 `incomplete`로만 나타난다.
pub fn scan_isolated(exe: &Path, path: &Path, opts: &Options) -> FileReport {
    let kib = opts.isolate_mem_mb.saturating_mul(1024);
    let mut argv: Vec<String> = vec!["sh".into(), "-c".into(), format!("ulimit -v \"$0\" || exit {ULIMIT_FAILED}; exec \"$@\""), kib.to_string(), exe.to_string_lossy().into_owned()];
    argv.extend(child_args(opts, path));
    let output = match SystemRunner.run(&argv, Some(Duration::from_secs(opts.isolate_timeout_s))) {
        Ok(o) => o,
        Err(CommandError::Timeout) => return failed(path, &format!("timeout after {}s", opts.isolate_timeout_s)),
        Err(CommandError::NotFound) => return failed(path, "sh not found"),
        Err(CommandError::Failed(m)) => return failed(path, &sanitize_display(&m, 200, Some('…'))),
    };
    if output.returncode != 0 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr.trim().chars().rev().take(STDERR_TAIL).collect::<Vec<_>>().into_iter().rev().collect();
        let tail = sanitize_display(&tail, STDERR_TAIL, Some('…'));
        let why = if tail.is_empty() { describe_exit(output.returncode) } else { format!("{}: {tail}", describe_exit(output.returncode)) };
        return failed(path, &why);
    }
    // 정확히 한 줄이 JSON 보고서여야 한다. 깨끗한 파일도 한 줄을 내므로 빈 출력은 실패다.
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    match (lines.next(), lines.next()) {
        (Some(line), None) => serde_json::from_str::<FileReport>(line).unwrap_or_else(|_| failed(path, "unreadable child output")),
        _ => failed(path, "unreadable child output"),
    }
}
