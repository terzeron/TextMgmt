//! 명령행. 원본 `main()`의 흐름과 출력 형식을 그대로 따른다.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::Serialize;

use crate::book_classifier::{clean_empty_parent_dirs, get_effective_filename, resolve_lenient};
use crate::{GenreResult, calculate_5_genre_scores_and_ratios, extract_content_head_tail_words};
use pyfmt::py_float;

const USAGE: &str =
    "usage: cluster_hybrid_genres [--source-dir SOURCE_DIR] [--library-root LIBRARY_ROOT] [--limit LIMIT] [--auto-move-pure] [--move-hybrids-to MOVE_HYBRIDS_TO] [--clean-existing] [--report REPORT]";
const EXTS: [&str; 5] = ["txt", "epub", "html", "htm", "pdf"];

struct Args {
    source_dir: PathBuf,
    library_root: PathBuf,
    limit: usize,
    auto_move_pure: bool,
    move_hybrids_to: String,
    report: PathBuf,
}

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args {
        source_dir: "/mnt/data/text/0_telegram".into(),
        library_root: "/mnt/data/text".into(),
        limit: 0,
        auto_move_pure: false,
        move_hybrids_to: String::new(),
        report: "/mnt/data/text/hybrid_clustering_report.json".into(),
    };
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        if arg == "-h" || arg == "--help" {
            return Ok(None);
        }
        let Some(opt) = arg.strip_prefix("--") else { return Err(format!("unrecognized arguments: {arg}")) };
        let (name, inline) = match opt.split_once('=') {
            Some((n, v)) => (n, Some(v.to_owned())),
            None => (opt, None),
        };
        match name {
            // --clean-existing은 기본값이 참인 store_true라 끌 방법이 없다. 받기만 한다.
            "auto-move-pure" => a.auto_move_pure = true,
            "clean-existing" => {}
            _ => {
                let value = inline.or_else(|| it.next().cloned()).ok_or_else(|| format!("argument --{name}: expected one argument"))?;
                match name {
                    "source-dir" => a.source_dir = value.into(),
                    "library-root" => a.library_root = value.into(),
                    "report" => a.report = value.into(),
                    "move-hybrids-to" => a.move_hybrids_to = value,
                    "limit" => a.limit = value.parse().map_err(|_| format!("argument --limit: invalid int value: '{value}'"))?,
                    _ => return Err(format!("unrecognized arguments: {arg}")),
                }
            }
        }
    }
    Ok(Some(a))
}

/// `Path.rglob("*")` 중 대상 확장자 파일. 숨김 구성요소가 하나라도 있는 경로는 뺀다.
/// 심볼릭 링크 디렉토리는 따라 들어가지 않는다. 원본은 순서가 정해져 있지 않아 여기서는 정렬한다.
fn list_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let path = entry.path();
            let is_link = entry.file_type().is_ok_and(|t| t.is_symlink());
            if !is_link && entry.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&path, out);
            } else if path.is_file() && path.extension().is_some_and(|e| EXTS.contains(&e.to_string_lossy().to_lowercase().as_str())) {
                out.push(path);
            }
        }
    }
    let hidden = |p: &Path| {
        p.components().any(|c| match c {
            std::path::Component::Normal(s) => s.to_string_lossy().starts_with('.'),
            std::path::Component::ParentDir => true,
            _ => false,
        })
    };
    let mut files = Vec::new();
    walk(root, &mut files);
    files.retain(|p| !hidden(p));
    files.sort();
    files
}

#[derive(Serialize)]
struct Item {
    file: String,
    filename: String,
    ratios: IndexMap<String, f64>,
    scores: IndexMap<String, i64>,
    total_score: i64,
}

#[derive(Serialize)]
struct Summary {
    total_files: usize,
    cluster_counts: IndexMap<String, usize>,
    clusters: IndexMap<String, Vec<Item>>,
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn ratio_str(r: &GenreResult) -> String {
    r.ratios.iter().filter(|(_, v)| **v > 0.0).map(|(k, v)| format!("{}:{}%", k.rsplit('_').next().unwrap_or(k), py_float(*v))).collect::<Vec<_>>().join(", ")
}

/// 진입점. 종료 코드를 돌려준다.
pub fn run(argv: &[String], out: &mut dyn Write) -> i32 {
    if argv.is_empty() {
        let _ = print_usage(out);
        return 0;
    }
    let args = match parse(argv) {
        Ok(Some(a)) => a,
        Ok(None) => {
            let _ = print_usage(out);
            return 0;
        }
        Err(msg) => {
            eprintln!("{USAGE}\ncluster_hybrid_genres: error: {msg}");
            return 2;
        }
    };
    match execute(&args, out) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ERROR {e}");
            1
        }
    }
}

/// 파일 하나를 `target_cat` 디렉토리로 옮긴다. 이미 같은 이름이 있으면 원본을 지운다.
/// 반환값은 (신규 이동, 중복 정리) 증분이다.
fn relocate(p: &Path, library_root: &Path, target_cat: &str, source_dir: &Path) -> io::Result<(usize, usize)> {
    let dest_dir = library_root.join(target_cat);
    let dest_path = dest_dir.join(p.file_name().unwrap_or_default());
    fs::create_dir_all(&dest_dir)?;
    if resolve_lenient(&dest_path) == resolve_lenient(p) {
        return Ok((0, 0));
    }
    let parent = p.parent().unwrap_or(source_dir);
    if dest_path.exists() {
        fs::remove_file(p)?;
        clean_empty_parent_dirs(parent, source_dir);
        Ok((0, 1))
    } else {
        fs::rename(p, &dest_path)?;
        clean_empty_parent_dirs(parent, source_dir);
        Ok((1, 0))
    }
}

fn execute(args: &Args, out: &mut dyn Write) -> io::Result<()> {
    let mut files = list_files(&args.source_dir);
    if args.limit > 0 {
        files.truncate(args.limit);
    }
    let total = files.len();
    writeln!(out, "=== 장르별 단어 빈도 비율 분석 & 클러스터링 시작 (총 {total}권) ===")?;

    let mut clusters: IndexMap<String, Vec<(GenreResult, Item)>> = IndexMap::new();
    let (mut moved, mut cleaned) = (0, 0);

    for (idx, p) in files.iter().enumerate() {
        let eff_name = get_effective_filename(p, &args.source_dir);
        let content = extract_content_head_tail_words(p, 500, 500);
        let res = calculate_5_genre_scores_and_ratios(&eff_name, &content);
        let item = Item { file: p.to_string_lossy().into_owned(), filename: eff_name, ratios: res.ratios.clone(), scores: res.scores.clone(), total_score: res.total_score };

        let target = if args.auto_move_pure && res.target_cat.is_some() {
            res.target_cat.clone()
        } else if !args.auto_move_pure && !args.move_hybrids_to.is_empty() && res.cluster.starts_with("하이브리드_") {
            Some(args.move_hybrids_to.clone())
        } else {
            None
        };
        if let Some(cat) = target {
            let (m, c) = relocate(p, &args.library_root, &cat, &args.source_dir)?;
            moved += m;
            cleaned += c;
        }
        clusters.entry(res.cluster.clone()).or_default().push((res, item));

        if (idx + 1) % 1000 == 0 {
            writeln!(out, "[{}/{total}] 분석 진행 중... (클러스터 수: {})", idx + 1, clusters.len())?;
        }
    }

    print_report(&clusters, total, out)?;
    if args.auto_move_pure || !args.move_hybrids_to.is_empty() {
        writeln!(out, "\n[자동 이동 결과] 신규 이동: {moved}권, 중복 정리: {cleaned}권")?;
    }

    let summary = Summary {
        total_files: total,
        cluster_counts: clusters.iter().map(|(c, v)| (c.clone(), v.len())).collect(),
        clusters: clusters.into_iter().map(|(c, v)| (c, v.into_iter().map(|(_, item)| item).collect())).collect(),
    };
    fs::write(&args.report, serde_json::to_string_pretty(&summary).map_err(io::Error::other)?)?;
    writeln!(out, "\n상세 리포트가 {} 에 저장되었습니다.", args.report.display())?;
    Ok(())
}

fn print_report(clusters: &IndexMap<String, Vec<(GenreResult, Item)>>, total: usize, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "\n=======================================================")?;
    writeln!(out, "=== 클러스터링 분석 완료 (총 {total}권) ===")?;
    writeln!(out, "=======================================================\n")?;

    let mut names: Vec<&String> = clusters.keys().collect();
    names.sort();
    let pick = |prefix: &str| names.iter().copied().filter(|c| c.starts_with(prefix)).collect::<Vec<_>>();
    let (pure, hybrid, semi) = (pick("확실한_"), pick("하이브리드_"), pick("준확실_"));
    let other: Vec<&String> = names.iter().copied().filter(|c| !pure.contains(c) && !hybrid.contains(c) && !semi.contains(c)).collect();
    let pct = |n: usize| if total == 0 { 0.0 } else { n as f64 / total as f64 * 100.0 };
    let show = |out: &mut dyn Write, group: &[&String], limit: usize, blank: bool| -> io::Result<()> {
        for c in group {
            let items = &clusters[*c];
            if blank {
                writeln!(out, "\n  ▶ {c}: {}권", items.len())?;
            } else {
                writeln!(out, "  ▶ {c}: {}권", items.len())?;
            }
            for (res, it) in items.iter().take(limit) {
                writeln!(out, "     - \"{}\" [{}] (score: {})", truncate(&it.filename, 50), ratio_str(res), res.total_score)?;
            }
        }
        Ok(())
    };
    let count_of = |group: &[&String]| group.iter().map(|c| clusters[*c].len()).sum::<usize>();

    writeln!(out, "### 1. [가장 확실한 클러스터 (자동 분류 대상, 비율 >= 75%)]")?;
    writeln!(out, "총 {}권 ({:.1}%)", count_of(&pure), pct(count_of(&pure)))?;
    show(out, &pure, 3, true)?;

    writeln!(out, "\n-------------------------------------------------------")?;
    writeln!(out, "### 2. [하이브리드 및 복합 클러스터 (사용자 검토 및 선택 필요)]")?;
    writeln!(out, "총 {}권 ({:.1}%)", count_of(&hybrid), pct(count_of(&hybrid)))?;
    show(out, &hybrid, 5, true)?;

    writeln!(out, "\n-------------------------------------------------------")?;
    writeln!(out, "### 3. [준확실 클러스터 (60% ~ 75%)]")?;
    show(out, &semi, 3, false)?;

    writeln!(out, "\n-------------------------------------------------------")?;
    writeln!(out, "### 4. [미분류 및 기타]")?;
    for c in other {
        writeln!(out, "  ▶ {c}: {}권", clusters[c].len())?;
    }
    Ok(())
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(out: &mut dyn Write) -> io::Result<()> {
    write!(
        out,
        "cluster_hybrid_genres - 장르별 단어 빈도 비율로 도서를 클러스터링한다.

본문 처음 500단어·마지막 500단어와 제목을 합쳐 무협·판타지·여성향·BL·성인 어휘의 비율을 재고,
확실한 클러스터는 자동 분류 대상으로, 하이브리드는 사람이 검토할 목록으로 보고한다.
인자 없이 실행하면 이 도움말만 출력하고 분석하지 않는다.

사용법:
  cluster_hybrid_genres [--source-dir DIR] [--library-root DIR] [--limit N]
                        [--auto-move-pure] [--move-hybrids-to CATEGORY]
                        [--clean-existing] [--report PATH]

옵션:
  --source-dir DIR          분석할 디렉토리 (기본 /mnt/data/text/0_telegram)
  --library-root DIR        이동 대상 코퍼스 루트 (기본 /mnt/data/text)
  --limit N                 앞에서 N건만 처리한다 (0이면 전체).
  --auto-move-pure          확실한 클러스터(비율 75% 이상)를 해당 카테고리로 옮긴다.
  --move-hybrids-to CAT     하이브리드 클러스터를 지정한 카테고리로 옮긴다 (예: 3_판타지).
  --clean-existing          대상에 같은 이름이 있으면 원본을 지운다 (항상 켜져 있다).
  --report PATH             리포트 경로 (기본 /mnt/data/text/hybrid_clustering_report.json)
  -h, --help                이 도움말

종료 코드: 0 정상, 1 실행 오류, 2 잘못된 인자
"
    )
}
