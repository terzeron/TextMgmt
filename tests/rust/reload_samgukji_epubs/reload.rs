use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use reload_samgukji_epubs::{find_epubs, loader_command, parse_dotenv, run};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"x").unwrap();
}

fn names(files: &[PathBuf], base: &Path) -> Vec<String> {
    files.iter().map(|f| f.strip_prefix(base).unwrap().to_string_lossy().into_owned()).collect()
}

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |key| map.get(key).cloned()
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn find_epubs_matches_name_and_extension_case_insensitively_and_sorts_by_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    touch(&dir.join("삼국지 2.epub"));
    touch(&dir.join("sub/[저자] 삼국지 (1).EPUB"));
    touch(&dir.join("삼국지 1.epub"));
    touch(&dir.join("삼국지 3.txt"));
    touch(&dir.join("수호지.epub"));
    touch(&dir.join("sub/deep/삼국지연의.Epub"));
    fs::create_dir_all(dir.join("삼국지 폴더.epub")).unwrap();

    let files = find_epubs(dir);

    assert_eq!(names(&files, dir), ["sub/[저자] 삼국지 (1).EPUB", "sub/deep/삼국지연의.Epub", "삼국지 1.epub", "삼국지 2.epub"]);
}

#[test]
fn find_epubs_ignores_symlinks_like_find_type_f() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    touch(&dir.join("real/삼국지.epub"));
    std::os::unix::fs::symlink(dir.join("real/삼국지.epub"), dir.join("link 삼국지.epub")).unwrap();
    std::os::unix::fs::symlink(dir.join("real"), dir.join("linkdir")).unwrap();
    assert_eq!(names(&find_epubs(dir), dir), ["real/삼국지.epub"]);
}

#[test]
fn find_epubs_of_a_missing_directory_is_empty() {
    assert!(find_epubs(Path::new("/nonexistent/dir/for/test")).is_empty());
}

#[test]
fn parse_dotenv_handles_quotes_export_comments_and_blank_lines() {
    let text = "# 주석\n\nTM_BOOK_DIR=/mnt/data/text\nexport A=\"with space\"\nB='single quoted'\n  C = spaced  \nBAD LINE\n=novalue\nD=\nE=a=b\n";
    let vars = parse_dotenv(text);
    let get = |k: &str| vars.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    assert_eq!(get("TM_BOOK_DIR"), Some("/mnt/data/text"));
    assert_eq!(get("A"), Some("with space"));
    assert_eq!(get("B"), Some("single quoted"));
    assert_eq!(get("C"), Some("spaced"));
    assert_eq!(get("D"), Some(""));
    assert_eq!(get("E"), Some("a=b"));
    assert_eq!(vars.len(), 6, "잘못된 줄은 건너뛴다");
}

#[test]
fn loader_command_runs_the_python_loader_with_each_file_as_its_own_argument() {
    let files = vec![PathBuf::from("/b/삼국지 (1) [x].epub"), PathBuf::from("/b/삼국지 2.epub")];
    let cmd = loader_command(&files, Path::new("/repo"), &[("TM_ES_URL".to_owned(), "http://h".to_owned())]);
    assert_eq!(cmd.get_program(), "uv");
    let argv: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(argv, ["run", "--no-sync", "python", "-m", "utils.loader", "book", "/b/삼국지 (1) [x].epub", "/b/삼국지 2.epub"]);
    assert_eq!(cmd.get_current_dir(), Some(Path::new("/repo")));
    let envs: HashMap<_, _> = cmd.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect();
    assert_eq!(envs["PYTHONPATH"].as_deref(), Some("/repo"));
    assert_eq!(envs["TM_ES_URL"].as_deref(), Some("http://h"));
}

fn setup() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let book_dir = tmp.path().join("books");
    touch(&book_dir.join("2_소설역사/삼국지 1.epub"));
    touch(&book_dir.join("2_소설역사/삼국지 2.epub"));
    (tmp, book_dir)
}

#[test]
fn list_prints_the_targets_and_the_apply_hint_without_launching() {
    let (tmp, book_dir) = setup();
    let env = env_of(&[("TM_BOOK_DIR", book_dir.to_str().unwrap())]);
    let mut out = Vec::new();
    let code = run("rse", &args(&["--list"]), &env, tmp.path(), &mut out, &|_| panic!("--list는 loader를 띄우지 않는다"));
    assert_eq!(code, 0);
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("대상: 2개 EPUB\n"), "{text}");
    assert!(text.contains("삼국지 1.epub\n") && text.contains("삼국지 2.epub\n"));
    assert!(text.trim_end().ends_with("재적재: rse --apply"));
}

#[test]
fn apply_launches_the_loader_and_returns_its_exit_code() {
    let (tmp, book_dir) = setup();
    fs::create_dir_all(tmp.path().join("utils")).unwrap();
    fs::write(tmp.path().join("utils/loader.py"), "").unwrap();
    let env = env_of(&[("TM_BOOK_DIR", book_dir.to_str().unwrap())]);
    let launched = std::cell::RefCell::new(Vec::new());
    let launch = |cmd: Command| {
        launched.borrow_mut().push(cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>());
        7
    };
    let mut out = Vec::new();
    let code = run("rse", &args(&["--apply"]), &env, tmp.path(), &mut out, &launch);
    assert_eq!(code, 7);
    let launched = launched.into_inner();
    assert_eq!(launched.len(), 1);
    assert_eq!(launched[0][..6], ["run", "--no-sync", "python", "-m", "utils.loader", "book"]);
    assert_eq!(launched[0].len(), 8);
    assert!(!String::from_utf8(out).unwrap().contains("재적재:"));
}

#[test]
fn apply_reads_missing_variables_from_the_dotenv_file_but_the_environment_wins() {
    let (tmp, book_dir) = setup();
    fs::create_dir_all(tmp.path().join("utils")).unwrap();
    fs::write(tmp.path().join("utils/loader.py"), "").unwrap();
    fs::write(tmp.path().join(".env"), format!("TM_BOOK_DIR={}\nTM_ES_URL=http://from-dotenv\nTM_KEEP=dotenv\n", book_dir.display())).unwrap();
    let env = env_of(&[("TM_KEEP", "process")]);
    let seen = std::cell::RefCell::new(HashMap::new());
    let launch = |cmd: Command| {
        for (k, v) in cmd.get_envs() {
            seen.borrow_mut().insert(k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()));
        }
        0
    };
    assert_eq!(run("rse", &args(&["--apply"]), &env, tmp.path(), &mut Vec::new(), &launch), 0);
    let seen = seen.into_inner();
    assert_eq!(seen["TM_ES_URL"].as_deref(), Some("http://from-dotenv"));
    assert!(!seen.contains_key("TM_KEEP"), "이미 환경에 있는 변수는 자식에게 덮어쓰지 않는다");
}

#[test]
fn failures_exit_one_with_the_original_messages() {
    let tmp = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    // TM_BOOK_DIR 없음
    assert_eq!(run("rse", &args(&["--list"]), &env_of(&[]), tmp.path(), &mut out, &|_| 0), 1);
    // 대상 디렉토리 없음
    let env = env_of(&[("TM_BOOK_DIR", tmp.path().join("none").to_str().unwrap())]);
    assert_eq!(run("rse", &args(&["--list"]), &env, tmp.path(), &mut out, &|_| 0), 1);
    // EPUB 없음
    fs::create_dir_all(tmp.path().join("books/2_소설역사")).unwrap();
    let env = env_of(&[("TM_BOOK_DIR", tmp.path().join("books").to_str().unwrap())]);
    assert_eq!(run("rse", &args(&["--list"]), &env, tmp.path(), &mut out, &|_| 0), 1);
    // --apply인데 저장소 루트가 아님
    touch(&tmp.path().join("books/2_소설역사/삼국지.epub"));
    assert_eq!(run("rse", &args(&["--apply"]), &env, tmp.path(), &mut out, &|_| panic!("띄우면 안 된다")), 1);
}

#[test]
fn usage_without_arguments_and_exit_two_for_bad_ones() {
    let env = env_of(&[]);
    let mut out = Vec::new();
    assert_eq!(run("rse", &[], &env, Path::new("."), &mut out, &|_| 0), 0);
    assert!(String::from_utf8(out).unwrap().contains("사용법"));
    assert_eq!(run("rse", &args(&["-h"]), &env, Path::new("."), &mut Vec::new(), &|_| 0), 0);
    assert_eq!(run("rse", &args(&["--bogus"]), &env, Path::new("."), &mut Vec::new(), &|_| 0), 2);
    assert_eq!(run("rse", &args(&["--list", "--apply"]), &env, Path::new("."), &mut Vec::new(), &|_| 0), 2);
}
