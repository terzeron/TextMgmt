use super::*;
use std::sync::Mutex;

fn argv(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn quiet(_: &str) {}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn parse_handles_help_all_options_and_argument_errors() {
    assert!(parse(&argv(&["--help"])).unwrap().is_none());
    assert!(matches!(parse(&[]), Err(message) if message.contains("required: command")));

    let parsed = parse(&argv(&["scan", "--root=/library", "--file", "a.txt", "--file=b.txt", "--workers", "3", "--limit=5", "--report", "out/report.json"])).unwrap().unwrap();
    assert_eq!(parsed.command, "scan");
    assert_eq!(parsed.root, PathBuf::from("/library"));
    assert_eq!(parsed.files, [PathBuf::from("a.txt"), PathBuf::from("b.txt")]);
    assert_eq!(parsed.workers, 3);
    assert_eq!(parsed.limit, 5);
    assert_eq!(parsed.report, Some(PathBuf::from("out/report.json")));

    for invalid in [argv(&["check", "--file"]), argv(&["scan", "--workers", "no"]), argv(&["scan", "--limit=bad"]), argv(&["scan", "--unknown"]), argv(&["scan", "extra"]), argv(&["unknown"])] {
        assert!(parse(&invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn run_prints_usage_and_returns_argparse_style_statuses() {
    let root = tempfile::tempdir().unwrap();
    let mut output = Vec::new();

    assert_eq!(run(&[], root.path(), &mut output, &quiet), 0);
    assert!(String::from_utf8(output.clone()).unwrap().contains("detect_mojibake"));
    output.clear();
    assert_eq!(run(&argv(&["-h"]), root.path(), &mut output, &quiet), 0);
    assert!(String::from_utf8(output).unwrap().contains("사용법:"));

    assert_eq!(run(&argv(&["bad-command"]), root.path(), &mut Vec::new(), &quiet), 2);

    let messages = Mutex::new(Vec::new());
    let log = |message: &str| messages.lock().unwrap().push(message.to_owned());
    assert_eq!(run(&argv(&["check"]), root.path(), &mut Vec::new(), &log), 1);
    assert_eq!(messages.lock().unwrap()[0], "ERROR --file 로 대상을 지정하라");
}

#[test]
fn check_writes_json_for_each_file_and_reports_output_errors() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.txt");
    let second = root.path().join("second.txt");
    fs::write(&first, "짧은 글").unwrap();
    fs::write(&second, "").unwrap();
    let files = ["check", "--file", first.to_str().unwrap(), "--file", second.to_str().unwrap()];
    let mut output = Vec::new();
    assert_eq!(run(&argv(&files), root.path(), &mut output, &quiet), 0);
    let lines: Vec<_> = String::from_utf8(output).unwrap().lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|line| line.contains("\"verdict\"")));

    let messages = Mutex::new(Vec::new());
    let log = |message: &str| messages.lock().unwrap().push(message.to_owned());
    assert_eq!(run(&argv(&files), root.path(), &mut FailingWriter, &log), 1);
    assert!(messages.lock().unwrap().iter().any(|message| message.starts_with("ERROR ")));
}

#[test]
fn scan_writes_report_creates_parent_directories_and_prints_summary() {
    let root = tempfile::tempdir().unwrap();
    let library = root.path().join("library");
    fs::create_dir(&library).unwrap();
    fs::write(library.join("short.txt"), "짧은 글").unwrap();
    let report = root.path().join("nested/reports/mojibake.json");
    let args = ["scan", "--root", library.to_str().unwrap(), "--workers=1", "--limit=1", "--report", report.to_str().unwrap()];
    let mut output = Vec::new();

    assert_eq!(run(&argv(&args), root.path(), &mut output, &quiet), 0);

    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("리포트:"));
    let report_json: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(report_json["total"], 1);
}

#[test]
fn scan_reports_file_system_and_writer_errors() {
    let root = tempfile::tempdir().unwrap();
    let library = root.path().join("library");
    fs::create_dir(&library).unwrap();
    let blocker = root.path().join("not-a-directory");
    fs::write(&blocker, "file").unwrap();
    let report = blocker.join("report.json");
    let args = argv(&["scan", "--root", library.to_str().unwrap(), "--report", report.to_str().unwrap()]);
    let messages = Mutex::new(Vec::new());
    let log = |message: &str| messages.lock().unwrap().push(message.to_owned());

    assert_eq!(run(&args, root.path(), &mut Vec::new(), &log), 1);
    assert!(messages.lock().unwrap().iter().any(|message| message.starts_with("ERROR ")));

    let mut writer = FailingWriter;
    assert!(print_usage(&mut writer).is_err());
}
