//! `2_소설역사`의 삼국지 EPUB을 찾아 Python loader로 다시 적재한다. 원본: `scripts/reload_samgukji_epubs.sh`.
//!
//! 기본은 대상 확인(`--list`)이고, `--apply`가 loader를 띄운다. loader가 아직 Python이라 자식 프로세스로 부른다.

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET_CATEGORY: &str = "2_소설역사";
const NAME_MARK: &str = "삼국지";
const BOOK_DIR_ENV: &str = "TM_BOOK_DIR";

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = write!(
        out,
        "{program} - {TARGET_CATEGORY}의 삼국지 EPUB을 찾아 loader로 다시 적재한다.

${BOOK_DIR_ENV}/{TARGET_CATEGORY} 아래에서 이름에 '{NAME_MARK}'가 들어간 .epub을 모두 찾는다.
--apply는 찾은 파일을 개별 인자로 넘겨 기존 문서도 강제로 다시 적재한다(디렉토리 전체는 넘기지 않는다).
loader는 현재 디렉토리(저장소 루트)에서 `uv run --no-sync python -m utils.loader book ...`로 실행한다.

사용법:
  {program} --list
  {program} --apply

옵션:
  --list       대상 파일만 보여 준다. 아무것도 바꾸지 않는다
  --apply      대상 파일을 loader로 다시 적재한다
  -h, --help   이 도움말

환경변수:
  {BOOK_DIR_ENV}   도서 루트. 없으면 현재 디렉토리의 .env에서 읽는다
  그 밖의 .env 변수는 환경에 없을 때만 loader에 넘긴다

종료 코드: 0 정상, 1 대상 없음·환경 누락·저장소 루트가 아님(loader가 실패하면 loader의 종료 코드), 2 잘못된 인자
"
    );
}

/// `.env`를 읽는다. `KEY=VALUE` 줄만 인정하고, `export` 접두어와 따옴표를 벗긴다.
/// 셸의 변수 치환·명령 치환은 하지 않는다.
pub fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue;
        }
        let value = value.trim();
        let unquoted = ['"', '\''].iter().find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q))).unwrap_or(value);
        vars.push((key.to_owned(), unquoted.to_owned()));
    }
    vars
}

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        // `find -type f`처럼 심볼릭 링크는 따라가지 않는다.
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            collect(&path, found);
        } else if kind.is_file() && is_target_name(&entry.file_name()) {
            found.push(path);
        }
    }
}

fn is_target_name(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    bytes.windows(NAME_MARK.len()).any(|w| w == NAME_MARK.as_bytes()) && bytes.len() >= 5 && bytes[bytes.len() - 5..].eq_ignore_ascii_case(b".epub")
}

/// 디렉토리 밑에서 이름에 '삼국지'가 들어간 .epub(대소문자 무관)을 경로 바이트 순으로 돌려준다.
pub fn find_epubs(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(dir, &mut found);
    found.sort_by(|a, b| a.as_os_str().as_bytes().cmp(b.as_os_str().as_bytes()));
    found
}

/// loader를 띄울 명령. 파일마다 인자 하나로 넘기므로 공백·괄호가 있어도 안전하다.
pub fn loader_command(files: &[PathBuf], repo_root: &Path, extra_env: &[(String, String)]) -> Command {
    let mut cmd = Command::new("uv");
    cmd.args(["run", "--no-sync", "python", "-m", "utils.loader", "book"]).args(files).current_dir(repo_root).env("PYTHONPATH", repo_root);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd
}

#[derive(PartialEq)]
enum Mode {
    List,
    Apply,
}

/// 진입점. `env`는 환경변수 조회, `cwd`는 저장소 루트, `launch`는 loader 실행이다. 종료 코드를 돌려준다.
pub fn run(program: &str, args: &[String], env: &dyn Fn(&str) -> Option<String>, cwd: &Path, out: &mut dyn Write, launch: &dyn Fn(Command) -> i32) -> i32 {
    let mode = match args {
        [] => {
            print_usage(program, out);
            return 0;
        }
        [a] if a == "-h" || a == "--help" => {
            print_usage(program, out);
            return 0;
        }
        [a] if a == "--list" => Mode::List,
        [a] if a == "--apply" => Mode::Apply,
        _ => {
            eprintln!("{program}: error: 인자는 --list 또는 --apply 하나여야 한다: {}", args.join(" "));
            return 2;
        }
    };

    let dotenv = fs::read_to_string(cwd.join(".env")).map(|t| parse_dotenv(&t)).unwrap_or_default();
    let book_dir = env(BOOK_DIR_ENV).or_else(|| dotenv.iter().find(|(k, _)| k == BOOK_DIR_ENV).map(|(_, v)| v.clone())).filter(|v| !v.is_empty());
    let Some(book_dir) = book_dir else {
        eprintln!("{BOOK_DIR_ENV}가 없다. 환경변수나 {}/.env에 지정한다", cwd.display());
        return 1;
    };
    let target_dir = Path::new(&book_dir).join(TARGET_CATEGORY);
    if !target_dir.is_dir() {
        eprintln!("대상 디렉토리를 찾을 수 없습니다: {}", target_dir.display());
        return 1;
    }
    let files = find_epubs(&target_dir);
    if files.is_empty() {
        eprintln!("{NAME_MARK} EPUB이 없습니다: {}", target_dir.display());
        return 1;
    }
    let _ = writeln!(out, "대상: {}개 EPUB", files.len());
    for file in &files {
        let _ = writeln!(out, "{}", file.display());
    }

    if mode == Mode::List {
        let _ = writeln!(out, "재적재: {program} --apply");
        return 0;
    }
    if !cwd.join("utils/loader.py").is_file() {
        eprintln!("저장소 루트에서 실행해야 한다(utils/loader.py가 없다): {}", cwd.display());
        return 1;
    }
    // 이미 환경에 있는 변수는 그대로 두고, 없는 것만 .env 값을 넘긴다.
    let extra: Vec<(String, String)> = dotenv.into_iter().filter(|(k, _)| env(k).is_none()).collect();
    let _ = out.flush();
    launch(loader_command(&files, cwd, &extra))
}
