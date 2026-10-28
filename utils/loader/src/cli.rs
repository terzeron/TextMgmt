//! 명령행: 사용법, getopt 방식 인자 읽기, 실행.

use std::collections::HashSet;
use std::fs::Metadata;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use doc_reader::pdf::{PDF_STAGE_TIMEOUT, Stage, StageError, StageRunner};
use es_client::EsError;
use es_client::indexing::IndexStore;
use parser_timeout::run_with_hard_timeout;
use stat::Stat;

use crate::config::Config;
use crate::flow::{FILE_ARGUMENT_BATCH_SIZE, Loader, ProcessHooks, direct_files, recursive_files_with_unreadable, sample_files};
use crate::record::ReadOutcome;

pub type ReadFileHook<'a> = dyn Fn(&Path, Option<&Metadata>, bool) -> ReadOutcome + Sync + 'a;
pub type StatHook<'a> = dyn Fn(&Path) -> io::Result<Metadata> + Sync + 'a;
#[derive(Clone, Copy, Default)]
pub struct RunHooks<'a> {
    pub read_file: Option<&'a ReadFileHook<'a>>,
    pub stat: Option<&'a StatHook<'a>>,
    pub on_worker_spawn: Option<&'a (dyn Fn() + Sync + 'a)>,
}

/// 자식 프로세스로 PDF 단계를 돌리는 숨은 부명령. Poppler 명령을 시간 상한으로 제한한다.
pub const PDF_STAGE_COMMAND: &str = "__pdf-stage";
/// ISBN PDF 파싱을 별도 프로세스에서 수행하는 숨은 부명령.
pub const ISBN_PDF_STAGE_COMMAND: &str = isbn::PDF_ISBN_WORKER_COMMAND;
/// Backend의 격리된 파일 파서 부명령.
pub const READ_FILE_COMMAND: &str = "__read-file";

const LONG_OPTIONS: [(&str, bool); 6] = [("delete", false), ("reload", false), ("recursive", false), ("reencode", false), ("reencode-dry-run", false), ("reencode-backup-dir", true)];

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = write!(
        out,
        "{program} - 파일을 읽어 Elasticsearch 색인(book 또는 comics)에 적재한다.

TXT, EPUB, PDF, DOCX, DOC, HWP, RTF, HTML과 이미지의 본문 요약·줄 수·쪽 수·ISBN을 뽑아 문서로 넣는다.
문서 번호는 파일의 inode다. 파일을 인자로 주면 이미 색인된 문서도 강제로 다시 적재한다.

사용법:
  {program} [--delete] [--reload] [--recursive] [--reencode | --reencode-dry-run] [--reencode-backup-dir=DIR] <index_name> [파일이나 디렉토리 ...]

인자:
  index_name   book 또는 comics

옵션:
  --delete                    색인을 지우고 끝낸다(파일 경로 불필요)
  --reload                    색인에 이미 있어도 강제로 다시 적재한다
  --recursive                 하위 디렉토리까지 훑는다
  --reencode                  UTF-8이 아닌 txt를 제자리에서 UTF-8로 바꾼다(검증을 통과한 파일만)
  --reencode-dry-run          --reencode가 바꿀 내용을 파일을 쓰지 않고 보여 준다
  --reencode-backup-dir=DIR   덮어쓰기 전에 원본을 DIR 아래에 복사한다(백업을 쓰고 검증하지 못하면 그 파일은 바꾸지 않는다)
  -h, --help                  이 도움말

참고:
  - 파일(디렉토리가 아닌)을 지정하면 자동으로 강제 재적재한다.
  - --reencode는 --reload나 명시한 파일 경로가 필요하다. 이미 색인된 파일은 읽지 않고 건너뛰기 때문이다.
  - --recursive --reload는 경로 아래에서 디스크에 더 이상 없는 고아 문서도 지운다.
  - 환경변수: TM_BOOK_DIR, TM_COMICS_DIR, TM_ES_URL, TM_ES_USER, TM_ES_PASSWORD, TM_ES_BOOK_INDEX, TM_ES_COMICS_INDEX
  - HTTPS 사설 CA: TM_ES_CA_CERT (PEM 파일 경로, 미설정 시 기본 CA 사용)

종료 코드: 0 정상 또는 옵션 오류 안내, 1 파일이 없는 등 일부 실패, 2 잘못된 위치 인자, 255 환경변수 누락이나 ES 접속 실패
"
    );
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub delete: bool,
    pub reload: bool,
    pub recursive: bool,
    pub reencode: bool,
    pub reencode_dry_run: bool,
    pub reencode_backup_dir: Option<PathBuf>,
    pub positional: Vec<String>,
}

/// Python `getopt.getopt` 방식: 첫 위치 인자를 만나면 옵션 읽기를 멈추고, 긴 옵션은 유일한 접두어로 줄여 쓸 수 있다.
pub fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            i += 1;
            break;
        }
        let Some(long) = arg.strip_prefix("--") else { break };
        let (name, inline) = match long.split_once('=') {
            Some((n, v)) => (n, Some(v.to_owned())),
            None => (long, None),
        };
        let candidates: Vec<&(&str, bool)> = LONG_OPTIONS.iter().filter(|(o, _)| o.starts_with(name)).collect();
        let exact = candidates.iter().find(|(o, _)| *o == name);
        let (opt, has_arg) = match (exact, candidates.len()) {
            (Some(e), _) => **e,
            (None, 1) => *candidates[0],
            (None, 0) => return Err(format!("option --{name} not recognized")),
            (None, _) => return Err(format!("option --{name} not a unique prefix")),
        };
        let value = if has_arg {
            match inline {
                Some(v) => Some(v),
                None => {
                    i += 1;
                    Some(args.get(i).cloned().ok_or_else(|| format!("option --{opt} requires argument"))?)
                }
            }
        } else if inline.is_some() {
            return Err(format!("option --{opt} must not have an argument"));
        } else {
            None
        };
        match opt {
            "delete" => options.delete = true,
            "reload" => options.reload = true,
            "recursive" => options.recursive = true,
            "reencode" => options.reencode = true,
            "reencode-dry-run" => {
                options.reencode = true;
                options.reencode_dry_run = true;
            }
            _ => options.reencode_backup_dir = value.map(PathBuf::from),
        }
        i += 1;
    }
    options.positional = args[i..].to_vec();
    Ok(options)
}

fn stage_key(stage: Stage) -> &'static str {
    match stage {
        Stage::Pdfium => "poppler",
        Stage::Plumber => "plumber",
        Stage::Repair => "repair",
    }
}

fn stage_from_key(key: &str) -> Option<Stage> {
    [Stage::Pdfium, Stage::Plumber, Stage::Repair].into_iter().find(|s| stage_key(*s) == key)
}

/// PDF 단계를 자식 프로세스(이 실행 파일의 숨은 부명령)로 돌리고 시간 상한이 넘으면 강제 종료한다.
pub struct ChildRunner {
    pub exe: PathBuf,
    pub timeout_seconds: f64,
}

impl ChildRunner {
    pub fn new(exe: PathBuf) -> Self {
        ChildRunner { exe, timeout_seconds: PDF_STAGE_TIMEOUT as f64 }
    }
}

impl StageRunner for ChildRunner {
    fn run(&self, stage: Stage, path: &Path) -> Result<(String, usize), StageError> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg(PDF_STAGE_COMMAND).arg(stage_key(stage)).arg(path);
        let output = run_with_hard_timeout(cmd, self.timeout_seconds).map_err(|_| StageError::Timeout)?;
        if !output.status.success() {
            return Err(StageError::Failed(String::from_utf8_lossy(&output.stderr).trim().to_owned()));
        }
        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| StageError::Failed(e.to_string()))?;
        Ok((parsed["text"].as_str().unwrap_or_default().to_owned(), parsed["pages"].as_u64().unwrap_or(0) as usize))
    }

    fn worker_executable(&self) -> Option<&Path> {
        Some(&self.exe)
    }
}

/// 숨은 부명령 `__pdf-stage <단계> <경로>`: 단계를 실행해 JSON으로 쓴다.
pub fn run_pdf_stage_command(args: &[String], out: &mut dyn Write) -> i32 {
    let (Some(stage), Some(path)) = (args.first().and_then(|k| stage_from_key(k)), args.get(1)) else {
        eprintln!("usage: {PDF_STAGE_COMMAND} <poppler|plumber|repair> <path>");
        return 2;
    };
    match doc_reader::pdf::run_stage(stage, Path::new(path)) {
        Ok((text, pages)) => {
            let _ = writeln!(out, "{}", serde_json::json!({"text": text, "pages": pages}));
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// 숨은 부명령 `__isbn-pdf-stage <poppler|fallback> <path>`.
pub fn run_isbn_pdf_stage_command(args: &[String], out: &mut dyn Write) -> i32 {
    let (Some(stage), Some(path)) = (args.first().map(String::as_str), args.get(1)) else {
        eprintln!("usage: {ISBN_PDF_STAGE_COMMAND} <poppler|fallback> <path>");
        return 2;
    };
    let result = match stage {
        "poppler" => isbn::pdf_text_auto(Path::new(path)),
        "fallback" => isbn::pdf_text_fallback(Path::new(path)).map(|text| (text, 0)),
        _ => {
            eprintln!("unknown ISBN PDF stage: {stage}");
            return 2;
        }
    };
    match result {
        Ok((text, pages)) => {
            let _ = writeln!(out, "{pages}\n{text}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

#[derive(Debug)]
pub enum ChildReadError {
    Timeout(parser_timeout::ParserTimeout),
    Other(String),
}

/// 파일 하나를 자식 프로세스에서 읽어 파서가 멈추면 프로세스 전체를 종료한다.
pub fn read_file_in_child(executable: &Path, cfg: &Config, path: &Path, path_prefix: Option<&Path>, skip_text: bool, timeout: Duration) -> Result<ReadOutcome, ChildReadError> {
    let mut command = Command::new(executable);
    command
        .arg(READ_FILE_COMMAND)
        .arg(&cfg.book_prefix)
        .arg(&cfg.comics_prefix)
        .arg(path)
        .arg(if path_prefix.is_some() { "1" } else { "0" })
        .arg(path_prefix.unwrap_or(Path::new("")))
        .arg(if skip_text { "1" } else { "0" });
    let output = run_with_hard_timeout(command, timeout.as_secs_f64()).map_err(ChildReadError::Timeout)?;
    if !output.status.success() {
        return Err(ChildReadError::Other(String::from_utf8_lossy(&output.stderr).trim().to_string()));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| ChildReadError::Other(e.to_string()))?;
    let item = match value.get("item") {
        Some(serde_json::Value::Array(parts)) if parts.len() == 2 => {
            let id = parts[0].as_i64().ok_or_else(|| ChildReadError::Other("worker returned invalid document id".into()))?;
            Some((id, parts[1].clone()))
        }
        Some(serde_json::Value::Null) | None => None,
        _ => return Err(ChildReadError::Other("worker returned invalid document".into())),
    };
    let messages = value.get("messages").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(serde_json::Value::as_str).map(str::to_owned).collect();
    Ok(ReadOutcome { item, messages })
}

/// 숨은 부명령 `__read-file`을 실행한다. 출력은 부모가 읽는 JSON 한 줄이다.
pub fn run_read_file_command(args: &[String], out: &mut dyn Write) -> i32 {
    if args.len() != 6 {
        eprintln!("usage: {READ_FILE_COMMAND} <book-root> <comics-root> <path> <has-prefix> <prefix> <skip-text>");
        return 2;
    }
    let cfg = Config::new(&args[0], &args[1]);
    let path = Path::new(&args[2]);
    let prefix = (args[3] == "1").then(|| Path::new(&args[4]));
    let runner = ChildRunner::new(std::env::current_exe().unwrap_or_else(|_| PathBuf::from("backend")));
    let result = crate::record::read_file(&cfg, path, None, args[5] == "1", prefix, &runner, Stat::global());
    let _ = writeln!(out, "{}", serde_json::json!({"item": result.item, "messages": result.messages}));
    0
}

/// 진입점이 필요로 하는 바깥 세계. 테스트가 환경변수·ES·PDF 실행기를 바꿔 끼운다.
pub type ConnectFn<'a> = dyn Fn(&str) -> Result<Box<dyn IndexStore>, String> + 'a;

pub struct World<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    /// 색인 이름으로 ES 저장소를 만든다. 접속하지 못하면 오류 메시지.
    pub connect: &'a ConnectFn<'a>,
    pub runner: &'a (dyn StageRunner + Sync),
}

/// 진입점. 종료 코드를 돌려준다.
pub fn run(program: &str, args: &[String], world: &World, out: &mut dyn Write) -> i32 {
    run_with_hooks(program, args, world, out, &RunHooks::default())
}

pub fn run_with_hooks(program: &str, args: &[String], world: &World, out: &mut dyn Write, hooks: &RunHooks<'_>) -> i32 {
    if args.is_empty() {
        print_usage(program, out);
        return 1;
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage(program, out);
        return 0;
    }
    let options = match parse_options(args) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{program}: error: {msg}");
            print_usage(program, out);
            return 0;
        }
    };
    let mut cfg = match Config::from_env(world.env) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("ERROR {msg}");
            return 255;
        }
    };
    cfg.reencode = options.reencode;
    cfg.reencode_dry_run = options.reencode_dry_run;
    cfg.reencode_backup_dir = options.reencode_backup_dir.clone();

    let Some(index_arg) = options.positional.first() else {
        print_usage(program, out);
        return 2;
    };
    let env_name = match index_arg.as_str() {
        "book" => "TM_ES_BOOK_INDEX",
        "comics" => "TM_ES_COMICS_INDEX",
        other => {
            eprintln!("ERROR 유효하지 않은 인덱스명: '{other}' (book 또는 comics만 가능)");
            print_usage(program, out);
            return 2;
        }
    };
    let Some(index) = (world.env)(env_name) else {
        eprintln!("ERROR The environment variable {env_name} is not set.");
        return 255;
    };
    let file_args = &options.positional[1..];
    if !options.delete && file_args.is_empty() {
        print_usage(program, out);
        return 2;
    }

    let started = Instant::now();
    let es = match (world.connect)(&index) {
        Ok(es) => es,
        Err(msg) => {
            eprintln!("ERROR {msg}");
            return 255;
        }
    };
    if let Err(e) = es.create_index() {
        eprintln!("ERROR {e}");
        return 255;
    }
    let _ = writeln!(out, "ES 인덱스: {index}");

    if options.delete {
        return match es.index_exists().and_then(|exists| if exists { es.delete_index().map(|()| true) } else { Ok(false) }) {
            Ok(true) => {
                let _ = writeln!(out, "인덱스 '{}' 삭제 완료", es.index_name());
                0
            }
            Ok(false) => {
                let _ = writeln!(out, "인덱스 '{}'가 존재하지 않습니다", es.index_name());
                0
            }
            Err(e) => {
                eprintln!("ERROR {e}");
                255
            }
        };
    }

    let stat = Stat::global();
    let loader = Loader { cfg: &cfg, es: es.as_ref(), runner: world.runner, stat };
    let skip_text = index_arg == "comics";
    let process_hooks = ProcessHooks { read_file: hooks.read_file, stat: hooks.stat, on_worker_spawn: hooks.on_worker_spawn };
    let mut pending: Vec<PathBuf> = Vec::new();
    let flush = |pending: &mut Vec<PathBuf>, out: &mut dyn Write| -> Result<(), EsError> {
        if pending.is_empty() {
            return Ok(());
        }
        let (processed, _, _) = loader.process_file_iter_with_hooks(out, pending.clone(), true, skip_text, None, &process_hooks)?;
        let _ = writeln!(out, "  argument 파일 {}개 중 {processed}개 적재 완료", pending.len());
        pending.clear();
        Ok(())
    };
    macro_rules! flush_or_fail {
        ($pending:expr, $out:expr) => {
            if let Err(e) = flush($pending, $out) {
                eprintln!("ERROR {e}");
                return 255;
            }
        };
    }
    macro_rules! process_batch_or_fail {
        ($batch:expr) => {
            match $batch {
                Ok(counts) => counts,
                Err(e) => {
                    eprintln!("ERROR {e}");
                    return 255;
                }
            }
        };
    }
    macro_rules! refresh_or_fail {
        ($refresh:expr) => {
            if let Err(e) = $refresh {
                eprintln!("ERROR {e}");
                return 255;
            }
        };
    }

    for arg in file_args {
        let target = PathBuf::from(arg);
        if !target.exists() {
            flush_or_fail!(&mut pending, out);
            eprintln!("ERROR can't find such a file or directory '{}'", target.display());
            return 0;
        }
        if !target.starts_with(&cfg.book_prefix) && !target.starts_with(&cfg.comics_prefix) {
            flush_or_fail!(&mut pending, out);
            eprintln!("ERROR {} is not in $TM_BOOK_DIR({}) or $TM_COMICS_DIR({}).", target.display(), cfg.book_prefix.display(), cfg.comics_prefix.display());
            continue;
        }
        let _ = writeln!(out, "====== {} ======", target.display());

        if target.is_file() {
            let _ = writeln!(out, "  [파일 강제 재적재] {}", target.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
            pending.push(target);
            if pending.len() == FILE_ARGUMENT_BATCH_SIZE {
                flush_or_fail!(&mut pending, out);
            }
            continue;
        }
        flush_or_fail!(&mut pending, out);

        if options.recursive {
            let (files, unreadable) = recursive_files_with_unreadable(&target);
            let mut seen: Option<HashSet<i64>> = options.reload.then(HashSet::new);
            let (processed, skipped, synced) = process_batch_or_fail!(loader.process_file_iter_with_hooks(out, files, options.reload, skip_text, seen.as_mut(), &process_hooks));
            let _ = writeln!(out, "  총 {processed}개 파일 처리됨");
            if skipped > 0 {
                let _ = writeln!(out, "  총 {skipped}개 중복 파일 건너뜀");
            }
            if synced > 0 {
                let _ = writeln!(out, "  총 {synced}개 경로 동기화");
            }
            // --reload: 대상 경로 아래에서 디스크에 더 이상 없는 고아 레코드를 지운다.
            // 읽지 못한 디렉토리의 책은 seen에 없어서 orphan으로 잘못 지워진다.
            if seen.is_some() && !unreadable.is_empty() {
                let _ = writeln!(out, "  [orphan 삭제 건너뜀] 읽지 못한 디렉토리 {}개:", unreadable.len());
                for path in &unreadable {
                    let _ = writeln!(out, "    {}", path.display());
                }
            } else if let Some(seen) = seen {
                let prefix = cfg.path_prefix(&target);
                let rel = target.strip_prefix(prefix).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                refresh_or_fail!(es.refresh()); // 방금 넣은 문서가 scroll에 잡히도록
                let existing = es.get_doc_ids_by_path_prefix(&rel);
                let mut orphans: Vec<i64> = existing.difference(&seen).copied().collect();
                orphans.sort_unstable();
                if orphans.is_empty() {
                    let _ = writeln!(out, "  orphan 없음 (삭제할 예전 레코드 없음)");
                } else {
                    let deleted = es.delete_by_ids(&orphans);
                    let _ = writeln!(out, "  [orphan 삭제: {deleted}개] (디스크에서 사라진 예전 레코드, 경로: {rel})");
                }
            }
        } else {
            if file_args.len() == 1 {
                let _ = writeln!(out, "  [1단계] 하위 디렉토리별 샘플 파일 등록");
                let samples = sample_files(&target);
                let mut chosen: Vec<(String, PathBuf)> = Vec::new();
                for (name, first) in samples {
                    match first {
                        Some(f) => chosen.push((name, f)),
                        None => {
                            let _ = writeln!(out, "    {name}/ -> (파일 없음)");
                        }
                    }
                }
                if !chosen.is_empty() {
                    let _ = writeln!(out, "    {}개 디렉토리에서 샘플 파일 선정 완료", chosen.len());
                    for (name, file) in &chosen {
                        let _ = writeln!(out, "      {name}/ -> {}", file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
                    }
                    let paths: Vec<PathBuf> = chosen.into_iter().map(|(_, f)| f).collect();
                    let (processed, skipped, synced) = process_batch_or_fail!(loader.process_file_iter_with_hooks(out, paths, options.reload, skip_text, None, &process_hooks));
                    let _ = writeln!(out, "    {processed}개 카테고리 샘플 저장, {skipped}개 중복 건너뜀");
                    if synced > 0 {
                        let _ = writeln!(out, "    {synced}개 경로 동기화");
                    }
                }
                let _ = writeln!(out, "  [2단계] 현재 디렉토리 파일 등록");
            } else {
                let _ = writeln!(out, "  [지정 디렉토리 파일 등록]");
            }
            let current = direct_files(&target);
            let _ = writeln!(out, "    {}개 파일 발견", current.len());
            let (_, skipped, synced) = process_batch_or_fail!(loader.process_file_iter_with_hooks(out, current, options.reload, skip_text, None, &process_hooks));
            if skipped > 0 {
                let _ = writeln!(out, "    {skipped}개 중복 파일 건너뜀");
            }
            if synced > 0 {
                let _ = writeln!(out, "    {synced}개 경로 동기화");
            }
        }
        let _ = writeln!(out, "================================");
    }
    flush_or_fail!(&mut pending, out);

    // 모든 작업이 끝나면 색인을 refresh한다.
    refresh_or_fail!(es.refresh());
    let _ = stat.set_value("index_total_time", started.elapsed().as_secs_f64());
    let _ = stat.print(out);
    0
}
