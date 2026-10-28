use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use backend::book_classifier::inspect_epub_metadata;
use serde_json::{Value, json};

fn report(path: &std::path::Path) -> Result<(Value, bool), String> {
    let validation = epubveri::validate_path(path).map_err(|error| error.to_string())?;
    let messages: Vec<Value> = validation
        .messages
        .iter()
        .map(|message| {
            let locations = message.location.as_ref().map_or_else(Vec::new, |path| {
                vec![json!({
                    "path": path,
                    "line": message.position.map_or(-1, |position| i64::from(position.line)),
                    "column": message.position.map_or(-1, |position| i64::from(position.column)),
                })]
            });
            json!({
                "severity": message.severity.to_string(),
                "ID": message.id,
                "message": message.text,
                "locations": locations,
            })
        })
        .collect();
    let metadata = inspect_epub_metadata(path);
    let mut output = json!({
        "messages": messages,
        "checker": {
            "nFatal": validation.fatals(),
            "nError": validation.errors(),
            "nWarning": validation.warnings(),
            "nUsage": validation.usages(),
            "nInfo": validation.infos(),
        },
    });
    if !metadata.title.is_empty() || !metadata.author.is_empty() {
        output["publication"] = json!({
            "title": metadata.title,
            "creator": metadata.author,
            "date": "",
            "publisher": "",
        });
    }
    Ok((output, validation.is_valid()))
}

fn parse_args() -> Result<(PathBuf, PathBuf), String> {
    let mut args = std::env::args_os().skip(1);
    let epub = args.next().map(PathBuf::from).ok_or_else(|| "usage: epubveri <epub> --json <report.json>".to_string())?;
    if args.next().as_deref() != Some(OsStr::new("--json")) {
        return Err("usage: epubveri <epub> --json <report.json>".to_string());
    }
    let json_path = args.next().map(PathBuf::from).ok_or_else(|| "usage: epubveri <epub> --json <report.json>".to_string())?;
    if args.next().is_some() {
        return Err("usage: epubveri <epub> --json <report.json>".to_string());
    }
    Ok((epub, json_path))
}

fn run() -> i32 {
    let (epub, json_path) = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let (output, valid) = match report(&epub) {
        Ok(output) => output,
        Err(error) => {
            eprintln!("epubveri validation failed: {error}");
            return 2;
        }
    };
    if let Err(error) = fs::write(&json_path, serde_json::to_vec(&output).expect("serialize validation report")) {
        eprintln!("failed to write JSON report: {error}");
        return 2;
    }
    if valid { 0 } else { 1 }
}

fn main() {
    std::process::exit(run());
}
