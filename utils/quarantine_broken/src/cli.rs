//! 명령행. argparse와 같은 규칙으로 옵션을 읽고, 잘못된 입력은 종료 코드 2로 끝낸다.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{DEFAULT_LIBRARY_ROOT, FixResult, Log, Rec, apply_moves, fix_extension, load_corrupted, plan_moves, summarize, undo_moves};

const USAGE: &str = "usage: quarantine_broken {plan,move,undo,fix-ext} [--report REPORT] [--library-root LIBRARY_ROOT] [--manifest MANIFEST] [--limit LIMIT] [--exclude EXCLUDE] [--dry-run]";

struct Args {
    command: String,
    report: Option<PathBuf>,
    library_root: PathBuf,
    manifest: Option<PathBuf>,
    limit: usize,
    exclude: Vec<String>,
    dry_run: bool,
}

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut command = None;
    let mut args = Args { command: String::new(), report: None, library_root: PathBuf::from(DEFAULT_LIBRARY_ROOT), manifest: None, limit: 0, exclude: Vec::new(), dry_run: false };
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if a == "-h" || a == "--help" {
            return Ok(None);
        }
        if let Some(opt) = a.strip_prefix("--") {
            if opt == "dry-run" {
                args.dry_run = true;
                continue;
            }
            let (name, inline) = match opt.split_once('=') {
                Some((n, v)) => (n, Some(v.to_owned())),
                None => (opt, None),
            };
            let mut value = || inline.clone().or_else(|| it.next().cloned()).ok_or_else(|| format!("argument --{name}: expected one argument"));
            match name {
                "report" => args.report = Some(value()?.into()),
                "library-root" => args.library_root = value()?.into(),
                "manifest" => args.manifest = Some(value()?.into()),
                "exclude" => args.exclude.push(value()?),
                "limit" => {
                    let v = value()?;
                    args.limit = v.parse().map_err(|_| format!("argument --limit: invalid int value: '{v}'"))?;
                }
                _ => return Err(format!("unrecognized arguments: {a}")),
            }
        } else if command.is_none() {
            command = Some(a.clone());
        } else {
            return Err(format!("unrecognized arguments: {a}"));
        }
    }
    let command = command.ok_or("the following arguments are required: command")?;
    if !["plan", "move", "undo", "fix-ext"].contains(&command.as_str()) {
        return Err(format!("argument command: invalid choice: '{command}' (choose from 'plan', 'move', 'undo', 'fix-ext')"));
    }
    args.command = command;
    Ok(Some(args))
}

fn name_of(path: &str, max_chars: usize) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().chars().take(max_chars).collect()).unwrap_or_default()
}

fn field(rec: &Rec, key: &str) -> String {
    rec.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// Python의 `str(value)`에 맞춘 표기: null은 `None`.
fn py_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".into(),
        Some(other) => other.to_string(),
    }
}

/// 진입점. 종료 코드를 돌려준다. `repo_root`는 기본 리포트·매니페스트 위치(`<repo>/tmp`)의 기준이다.
pub fn run(argv: &[String], repo_root: &Path, out: &mut dyn Write, log: Log) -> i32 {
    let args = match parse(argv) {
        Ok(Some(a)) => a,
        Ok(None) => {
            let _ = writeln!(out, "{USAGE}");
            return 0;
        }
        Err(msg) => {
            eprintln!("{USAGE}\nquarantine_broken: error: {msg}");
            return 2;
        }
    };
    match execute(&args, repo_root, out, log) {
        Ok(code) => code,
        Err(e) => {
            log(&format!("ERROR {e}"));
            1
        }
    }
}

fn execute(args: &Args, repo_root: &Path, out: &mut dyn Write, log: Log) -> io::Result<i32> {
    let report = args.report.clone().unwrap_or_else(|| repo_root.join("tmp/mojibake_recheck.json"));
    let manifest = args.manifest.clone().unwrap_or_else(|| repo_root.join("tmp/quarantine_manifest.json"));

    match args.command.as_str() {
        "fix-ext" => {
            let (mut fixed, mut skipped) = (Vec::new(), Vec::new());
            for rec in load_corrupted(&report)? {
                match fix_extension(Path::new(&field(&rec, "path")), args.dry_run) {
                    None => {}
                    Some(e @ FixResult::Error { .. }) => skipped.push(e),
                    Some(r) => fixed.push(r),
                }
            }
            writeln!(out, "{} {}건", if args.dry_run { "교정 예정" } else { "교정 완료" }, fixed.len())?;
            for f in &fixed {
                if let FixResult::Restored { path, restored_to, before, after } = f {
                    writeln!(out, "  {}\n    -> {}  ({} -> {} 바이트)", name_of(path, 48), name_of(restored_to, 48), crate::thousands(*before), crate::thousands(*after))?;
                }
            }
            for s in &skipped {
                if let FixResult::Error { path, error } = s {
                    writeln!(out, "  건너뜀: {} - {error}", name_of(path, 44))?;
                }
            }
            Ok(0)
        }
        "undo" => {
            let res = undo_moves(&manifest, args.dry_run)?;
            writeln!(out, "원위치 {}건, 실패 {}건", crate::thousands(res.restored.len()), crate::thousands(res.failed.len()))?;
            for f in res.failed.iter().take(10) {
                writeln!(out, "  실패: {} - {}", name_of(&field(f, "dst"), 50), field(f, "error"))?;
            }
            Ok(0)
        }
        _ => {
            let mut records = load_corrupted(&report)?;
            if !args.exclude.is_empty() {
                let before = records.len();
                records.retain(|r| {
                    let name = name_of(&field(r, "path"), usize::MAX);
                    !args.exclude.iter().any(|k| name.contains(k.as_str()))
                });
                log(&format!("INFO 제외 패턴으로 {}건 제외", before - records.len()));
            }
            let mut moves = plan_moves(&records, &args.library_root, log);
            if args.limit > 0 {
                moves.truncate(args.limit);
            }

            if args.command == "plan" {
                summarize(&moves, out)?;
                writeln!(out, "\n예시 10건")?;
                for mv in moves.iter().take(10) {
                    writeln!(out, "  {:<54} likeness={}", name_of(&field(mv, "src"), 52), py_str(mv.get("likeness")))?;
                }
                return Ok(0);
            }

            summarize(&moves, out)?;
            let result = apply_moves(&moves, &manifest, args.dry_run, log);
            writeln!(out, "\n{} {}건, 실패 {}건", if args.dry_run { "이동 예정" } else { "이동 완료" }, crate::thousands(result.moved.len()), crate::thousands(result.failed.len()))?;
            for f in result.failed.iter().take(10) {
                writeln!(out, "  실패: {} - {}", name_of(&field(f, "src"), 50), field(f, "error"))?;
            }
            if !args.dry_run && !result.moved.is_empty() {
                writeln!(out, "매니페스트: {}", manifest.display())?;
                writeln!(out, "되돌리려면: quarantine_broken undo")?;
            }
            Ok(0)
        }
    }
}
