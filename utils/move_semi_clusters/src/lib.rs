//! 사용자가 승인한 준확실 클러스터 도서를 대상 장르 폴더로 옮긴다. 원본: `utils/move_semi_clusters.py`.
//!
//! 인자 없이 실행하면 사용법만 보인다. `run`을 주면 파일을 실제로 옮기고 중복 파일을 지운다. 먼저 `--dry-run`으로 건수를 확인한다.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use corpus_common::DEFAULT_LIBRARY_ROOT;
use corpus_common::util::clean_empty_parent_dirs;

const SOURCE_CATEGORY: &str = "0_telegram";
const REPORT_NAME: &str = "hybrid_clustering_report.json";
const GREEK_MYTH: &str = "이윤기의 그리스 로마 신화";
const GREEK_MYTH_CATEGORY: &str = "1_서양고전";

/// 보고서의 준확실 클러스터 이름과 기본 대상 카테고리.
const SEMI_CLUSTERS: [(&str, &str); 5] = [("준확실_3_무협", "3_무협"), ("준확실_3_판타지", "3_판타지"), ("준확실_3_여성향", "3_여성향"), ("준확실_9_BLGL", "9_BLGL"), ("준확실_9_성인", "9_성인")];

/// 카테고리별 처리 건수. 키는 대상 카테고리다.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// 새로 옮긴 파일 수
    pub moved: BTreeMap<String, usize>,
    /// 대상에 같은 이름이 이미 있어 원본만 지운 파일 수
    pub cleaned: BTreeMap<String, usize>,
    /// 소스 카테고리 밖을 가리켜서 건너뛴 보고서 항목 수
    pub skipped_outside: usize,
}

impl Outcome {
    fn new() -> Self {
        let categories = SEMI_CLUSTERS.iter().map(|(_, c)| *c).chain([GREEK_MYTH_CATEGORY]);
        let zeros: BTreeMap<String, usize> = categories.map(|c| (c.to_owned(), 0)).collect();
        Outcome { moved: zeros.clone(), cleaned: zeros, skipped_outside: 0 }
    }

    pub fn total_moved(&self) -> usize {
        self.moved.values().sum()
    }

    pub fn total_cleaned(&self) -> usize {
        self.cleaned.values().sum()
    }

    /// 건수가 있는 카테고리만 정렬해서 보여 주는 요약.
    pub fn summary(&self) -> String {
        let mut text = String::from("=== 준확실 클러스터 도서 이동 완료 ===\n");
        for (category, moved) in &self.moved {
            let cleaned = self.cleaned.get(category).copied().unwrap_or(0);
            if *moved > 0 || cleaned > 0 {
                text.push_str(&format!("  ▶ {category}: 신규 이동 {moved}권, 중복 정리 {cleaned}권 (합계 {}권)\n", moved + cleaned));
            }
        }
        let (moved, cleaned) = (self.total_moved(), self.total_cleaned());
        text.push_str(&format!("\n총 처리: 신규 이동 {moved}권, 중복 정리 {cleaned}권 (합계 {}권)\n", moved + cleaned));
        if self.skipped_outside > 0 {
            text.push_str(&format!("건너뜀: 소스 카테고리 밖 항목 {}건 (보고서의 라이브러리 경로를 확인한다)\n", self.skipped_outside));
        }
        text
    }
}

/// 파일 하나를 대상 카테고리로 옮긴다. 같은 이름이 이미 있으면 원본을 지운다.
/// `dry_run`이면 디스크를 건드리지 않고, 이번 실행에서 이미 자리를 차지한 대상 경로를 `claimed`로 따진다.
fn place(file: &Path, library_root: &Path, source_dir: &Path, category: &str, dry_run: bool, claimed: &mut HashSet<PathBuf>, outcome: &mut Outcome) -> Result<(), String> {
    let name = file.file_name().ok_or_else(|| format!("파일 이름이 없다: {}", file.display()))?;
    let dest_dir = library_root.join(category);
    let dest = dest_dir.join(name);
    let exists = dest.exists() || claimed.contains(&dest);
    if dry_run {
        claimed.insert(dest);
    } else {
        fs::create_dir_all(&dest_dir).map_err(|e| format!("{}: {e}", dest_dir.display()))?;
        if exists {
            fs::remove_file(file).map_err(|e| format!("{}: {e}", file.display()))?;
        } else {
            fs::rename(file, &dest).map_err(|e| format!("{} -> {}: {e}", file.display(), dest.display()))?;
        }
        if let Some(parent) = file.parent() {
            clean_empty_parent_dirs(parent, source_dir);
        }
    }
    let counts = if exists { &mut outcome.cleaned } else { &mut outcome.moved };
    *counts.entry(category.to_owned()).or_insert(0) += 1;
    Ok(())
}

/// 디렉토리 밑의 일반 파일 중 이름에 `needle`이 들어간 것. 디렉토리 심볼릭 링크는 따라가지 않는다.
fn files_named_like(dir: &Path, needle: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            files_named_like(&path, needle, found);
        } else if path.is_file() && entry.file_name().to_string_lossy().contains(needle) {
            found.push(path);
        }
    }
}

/// 보고서의 준확실 클러스터를 옮기고, 소스 카테고리에 남은 그리스 로마 신화도 `1_서양고전`으로 옮긴다.
pub fn move_semi_clusters(library_root: &Path, report_path: &Path, dry_run: bool) -> Result<Outcome, String> {
    let text = fs::read_to_string(report_path).map_err(|e| format!("{}: {e}", report_path.display()))?;
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", report_path.display()))?;
    let clusters = data.get("clusters").ok_or_else(|| format!("{}: 'clusters' 키가 없다", report_path.display()))?;

    let source_dir = library_root.join(SOURCE_CATEGORY);
    let mut outcome = Outcome::new();
    let mut claimed = HashSet::new();
    let mut handled: HashSet<PathBuf> = HashSet::new();

    for (cluster, default_category) in SEMI_CLUSTERS {
        let Some(value) = clusters.get(cluster) else { continue };
        let Some(items) = value.as_array() else { return Err(format!("{cluster}: 항목이 목록이 아니다")) };
        for item in items {
            let file = item.get("file").and_then(|v| v.as_str()).ok_or_else(|| format!("{cluster}: 항목에 'file'이 없다"))?;
            let file = PathBuf::from(file);
            // 다른 루트로 만든 보고서가 라이브러리 밖 파일을 옮기거나 지우지 않게 한다. `..`는 경로 비교를 우회한다.
            if !file.starts_with(&source_dir) || file.components().any(|c| c == std::path::Component::ParentDir) {
                outcome.skipped_outside += 1;
                continue;
            }
            if !file.exists() {
                continue;
            }
            let category = if file.file_name().is_some_and(|n| n.to_string_lossy().contains(GREEK_MYTH)) { GREEK_MYTH_CATEGORY } else { default_category };
            place(&file, library_root, &source_dir, category, dry_run, &mut claimed, &mut outcome)?;
            handled.insert(file);
        }
    }

    let mut leftovers = Vec::new();
    files_named_like(&source_dir, GREEK_MYTH, &mut leftovers);
    leftovers.sort();
    for file in leftovers.into_iter().filter(|f| !handled.contains(f)) {
        place(&file, library_root, &source_dir, GREEK_MYTH_CATEGORY, dry_run, &mut claimed, &mut outcome)?;
    }
    Ok(outcome)
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = write!(
        out,
        "{program} - 준확실 클러스터 도서를 대상 장르 폴더로 옮긴다.

`cluster_hybrid_genres`가 만든 보고서(hybrid_clustering_report.json)의 준확실 클러스터를 옮긴다.
  준확실_3_무협 -> 3_무협, 준확실_3_판타지 -> 3_판타지, 준확실_3_여성향 -> 3_여성향,
  준확실_9_BLGL -> 9_BLGL, 준확실_9_성인 -> 9_성인
'{GREEK_MYTH}'는 {GREEK_MYTH_CATEGORY}로 보내고, {SOURCE_CATEGORY}에 남은 같은 이름의 파일도 함께 옮긴다.
대상에 같은 이름이 이미 있으면 원본을 지운다. 비게 된 하위 디렉토리는 청소한다.
{SOURCE_CATEGORY} 밖을 가리키는 보고서 항목은 건너뛴다.
인자 없이 실행하면 이 도움말만 보인다. 이동은 run이나 옵션을 주어야 시작한다. 먼저 --dry-run으로 건수를 본다.

사용법:
  {program} [run] [--dry-run] [--library-root DIR] [--report FILE]

명령:
  run   이동을 실행한다.

옵션:
  --dry-run          디스크를 바꾸지 않고 건수만 보여 준다
  --library-root DIR 라이브러리 루트 (기본 {DEFAULT_LIBRARY_ROOT})
  --report FILE      클러스터 보고서 (기본 <library-root>/{REPORT_NAME})
  -h, --help         이 도움말

종료 코드: 0 정상, 1 보고서를 읽지 못했거나 이동 실패, 2 잘못된 인자
"
    );
}

struct Options {
    dry_run: bool,
    library_root: PathBuf,
    report: Option<PathBuf>,
}

fn parse_options(args: &[String], default_library_root: &Path) -> Result<Options, String> {
    let mut options = Options { dry_run: false, library_root: default_library_root.to_path_buf(), report: None };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dry-run" => options.dry_run = true,
            "--library-root" => options.library_root = it.next().map(PathBuf::from).ok_or("argument --library-root: expected one argument")?,
            "--report" => options.report = Some(it.next().map(PathBuf::from).ok_or("argument --report: expected one argument")?),
            other => return Err(format!("unrecognized arguments: {other}")),
        }
    }
    Ok(options)
}

/// 진입점. 종료 코드를 돌려준다.
pub fn run(program: &str, args: &[String], out: &mut dyn Write) -> i32 {
    run_with_library_root(program, args, out, Path::new(DEFAULT_LIBRARY_ROOT))
}

/// `run`의 기본 라이브러리 경로를 주입한다. 실제 CLI와 임시 디렉토리 테스트가 같은 실행 경로를 쓴다.
pub fn run_with_library_root(program: &str, args: &[String], out: &mut dyn Write, default_library_root: &Path) -> i32 {
    let option_args = match args.first().map(String::as_str) {
        // 파일을 옮기고 지우는 도구라 인자 없이 실행하면 사용법만 보인다.
        None | Some("-h" | "--help") => {
            print_usage(program, out);
            return 0;
        }
        Some("run") => &args[1..],
        Some(arg) if arg.starts_with("--") => args,
        Some(other) => {
            eprintln!("{program}: error: argument command: invalid choice: '{other}' (choose from 'run')");
            return 2;
        }
    };
    let options = match parse_options(option_args, default_library_root) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{program}: error: {msg}");
            return 2;
        }
    };
    let report = options.report.unwrap_or_else(|| options.library_root.join(REPORT_NAME));
    match move_semi_clusters(&options.library_root, &report, options.dry_run) {
        Ok(outcome) => {
            if options.dry_run {
                let _ = writeln!(out, "[dry-run] 디스크를 바꾸지 않았다. 아래는 실행 시 예상 건수다.");
            }
            let _ = write!(out, "{}", outcome.summary());
            0
        }
        Err(msg) => {
            eprintln!("ERROR {msg}");
            1
        }
    }
}
