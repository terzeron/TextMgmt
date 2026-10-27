use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::Instant;

use backend::classifier::{BookCategoryClassifier, ClassifierOptions, calibration, config, corpus, training};
use serde_json::{Value, json};

use crate::{CliIo, Deps, args::Args, data};

#[path = "policy.rs"]
mod policy;

#[path = "move_file.rs"]
mod move_file;
#[path = "train.rs"]
mod train;

pub(crate) fn execute(args: &Args, io: &mut CliIo<'_>, deps: &Deps) -> Result<i32, String> {
    match args.command.as_deref().unwrap() {
        "train" => train::run(args, io, deps),
        "collect" => collect(args, io, deps),
        "info" => info(args, io),
        "import-joblib" => import_joblib(args, io),
        "evaluate" | "score-calibration" => evaluate(args, io),
        "classify" | "reclassify" => classify(args, io, deps),
        "bookstore-policy" => policy::run(args, io, deps),
        _ => unreachable!("파서가 명령을 검증한다"),
    }
}

fn import_joblib(args: &Args, io: &mut CliIo<'_>) -> Result<i32, String> {
    let input = args.get("--model").ok_or("import-joblib: --model FILE이 필요하다")?;
    let output = args.get("--out").ok_or("import-joblib: --out DIR이 필요하다")?;
    let mut summary = backend::classifier::joblib_import::import_joblib(Path::new(input), Path::new(output))?;
    let golden = args.get("--golden").unwrap_or("0").parse::<i64>().map_err(|_| "--golden: 정수가 필요하다")?;
    if golden > 0 {
        let seed = args.get("--seed").unwrap_or("0").parse::<i64>().map_err(|_| "--seed: 정수가 필요하다")?;
        let model = data::model(Path::new(output))?;
        let cases = crate::golden::write_golden(&model, Path::new(output), golden as usize, seed)?;
        summary["golden_cases"] = json!(cases);
        if let (Some(files), Ok(metadata)) = (summary.get_mut("files").and_then(Value::as_object_mut), fs::metadata(Path::new(output).join("golden.json"))) {
            files.insert("golden.json".into(), json!(metadata.len()));
        }
    }
    writeln!(io.out, "{}", serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    Ok(0)
}

fn collect(args: &Args, io: &mut CliIo<'_>, deps: &Deps) -> Result<i32, String> {
    let cfg = deps.config.clone().unwrap_or_else(config::load_config_from_env);
    let path = args.path("--out", "tmp/classifier/corpus.jsonl");
    let prefixes = args
        .values
        .get("--exclude-prefix")
        .cloned()
        .unwrap_or_else(|| cfg["corpus"]["excluded_prefixes"].as_array().map(|v| v.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default());
    let start = Instant::now();
    let pattern = cfg["corpus"]["label_pattern"].as_str().unwrap_or(corpus::DEFAULT_LABEL_PATTERN);
    let stats = corpus::collect_with_pattern(
        deps.transport(args.get("--index"))?.as_ref(),
        &path,
        corpus::CollectOptions { min_chars: args.number("--min-chars", 200.0) as usize, excluded_prefixes: prefixes, index: args.get("--index").map(str::to_owned) },
        pattern,
    )
    .map_err(|e| e.to_string())?;
    let mut cats: Vec<_> = stats["per_category"].as_object().into_iter().flat_map(|map| map.iter().map(|(cat, count)| (cat.clone(), count.clone()))).collect();
    cats.sort_by_key(|(_, n)| std::cmp::Reverse(n.as_u64().unwrap_or(0)));
    writeln!(
        io.out,
        "수집 {}건 / 제외 {}건 / 카테고리 {}개  ({:.0}초)",
        data::count(stats["kept"].as_u64().unwrap_or(0) as usize),
        data::count(stats["skipped"].as_u64().unwrap_or(0) as usize),
        stats["categories"],
        start.elapsed().as_secs_f64()
    )
    .map_err(|e| e.to_string())?;
    if args.flag("--with-publisher") {
        let cache = path.with_file_name(format!("{}.publisher.json", path.file_name().unwrap().to_string_lossy()));
        let root = args.path("--library-root", cfg["corpus"]["library_root"].as_str().unwrap_or("/mnt/data/text"));
        let stats = corpus::attach_publishers_jsonl(&path, &cache, &root, args.number("--publisher-workers", 2.0) as usize).map_err(|error| error.to_string())?;
        writeln!(io.out, "publisher: 새로 읽음 {}건, 값이 있는 문서 {}건", stats["read"], stats["with_publisher"]).map_err(|e| e.to_string())?;
    }
    writeln!(io.out, "\n  상위 10개 카테고리").map_err(|e| e.to_string())?;
    for (cat, n) in cats.iter().take(10) {
        writeln!(io.out, "  {n:>8}  {cat}").map_err(|e| e.to_string())?;
    }
    if cats.len() > 10 {
        writeln!(io.out, "  ... 하위 10개").map_err(|e| e.to_string())?;
        for (cat, n) in cats.iter().skip(cats.len() - 10) {
            writeln!(io.out, "  {n:>8}  {cat}").map_err(|e| e.to_string())?;
        }
    }
    writeln!(io.out, "\n저장: {}", path.display()).map_err(|e| e.to_string())?;
    Ok(0)
}

fn info(args: &Args, io: &mut CliIo<'_>) -> Result<i32, String> {
    let path = args.model();
    let model = data::model(&path)?;
    let meta = &model.meta;
    writeln!(
        io.out,
        "모델 디렉토리  {}\n학습 시각  {}\n설정 지문  {}\n학습 문서  {}건 / 카테고리 {}개 / 특징 {}개\n판정 임계값 {:.2}  온도 {}",
        path.display(),
        meta["trained_at"].as_str().unwrap_or("-"),
        meta["config_hash"].as_str().unwrap_or("-"),
        data::count(meta["documents"].as_u64().unwrap_or(0) as usize),
        model.classes.len(),
        data::count(model.n_features()),
        model.min_confidence(),
        model.temperature()
    )
    .map_err(|e| e.to_string())?;
    if let Some(dropped) = meta["dropped_categories"].as_array().filter(|v| !v.is_empty()) {
        writeln!(io.out, "제외 카테고리 {}개: {}{}", dropped.len(), dropped.iter().take(10).filter_map(Value::as_str).collect::<Vec<_>>().join(", "), if dropped.len() > 10 { " ..." } else { "" })
            .map_err(|e| e.to_string())?;
    }
    writeln!(io.out, "\n  필드          특징      가중치").map_err(|e| e.to_string())?;
    if let Some(blocks) = meta["blocks"].as_object() {
        for (name, block) in blocks {
            writeln!(io.out, "  {name:<12} {:>8}  {}", block["features"], block["weight"]).map_err(|e| e.to_string())?;
        }
    }
    if let Some(report) = meta.get("holdout").filter(|r| r.is_object()) {
        writeln!(io.out, "\n{}", training::format_report(report)).map_err(|e| e.to_string())?;
    }
    Ok(0)
}

fn evaluate(args: &Args, io: &mut CliIo<'_>) -> Result<i32, String> {
    let path = args.model();
    let mut model = data::model(&path)?;
    let corpus_path = args.path("--corpus", "tmp/classifier/corpus.jsonl");
    let mut docs = data::indexed_corpus(&corpus_path)?;
    let indices = data::indexed_holdout_indices(
        &docs,
        &data::holdout_manifest_path(&path),
        model.config["corpus"]["min_per_category"].as_u64().unwrap_or(30) as usize,
        model.config["holdout"].as_f64().unwrap_or(0.2),
        model.config["seed"].as_u64().unwrap_or(42),
        io.err,
    )?;
    log::info!("평가 corpus index 준비 완료: {}건 {}", docs.len(), training::rss_note());
    if args.command.as_deref() == Some("evaluate") {
        let (report, _) = training::evaluate_indexed(&model, &mut docs, &indices, Some(&config::series_to_parent())).map_err(|error| error.to_string())?;
        writeln!(io.out, "{}", training::format_report(&report)).map_err(|e| e.to_string())?;
        return Ok(0);
    }
    let (_, calibration) = training::evaluate_indexed(&model, &mut docs, &indices, Some(&config::series_to_parent())).map_err(|error| error.to_string())?;
    writeln!(io.out, "홀드아웃 {}건으로 눈금을 잰다", data::count(indices.len())).map_err(|e| e.to_string())?;
    if calibration.is_null() {
        writeln!(io.out, "표본이 모자라 눈금을 만들지 못했다").map_err(|e| e.to_string())?;
        return Ok(1);
    }
    let cal = calibration;
    writeln!(io.out, "  계단 {}칸, 표본 {}건\n  확신도   예상 정답률", cal["x"].as_array().unwrap().len(), cal["n"]).map_err(|e| e.to_string())?;
    for point in [0.02, 0.04, 0.056, 0.076, 0.10, 0.13] {
        writeln!(io.out, "  {point:>6.3}   {:.3}", calibration::expected_accuracy(Some(&cal), Some(point)).unwrap_or(0.0)).map_err(|e| e.to_string())?;
    }
    if args.flag("--dry-run") {
        writeln!(io.out, "--dry-run 이라 모델 파일은 그대로 둔다").map_err(|e| e.to_string())?;
    } else {
        model.meta["confidence_calibration"] = cal;
        // 계수 파일을 다시 쓰지 않고 manifest만 원자적으로 교체한다.
        let manifest_path = path.join("model.json");
        let mut manifest = data::json_file(&manifest_path)?;
        manifest["meta"] = model.meta;
        data::save_json(&manifest_path, &manifest)?;
        writeln!(io.out, "모델에 눈금을 넣었다: {}", path.display()).map_err(|e| e.to_string())?;
    }
    Ok(0)
}

fn classify(args: &Args, io: &mut CliIo<'_>, deps: &Deps) -> Result<i32, String> {
    let model = Arc::new(data::model(&args.model())?);
    let root = args.path("--library-root", config::load_config_from_env()["corpus"]["library_root"].as_str().unwrap_or("/mnt/data/text"));
    let es = if args.flag("--no-es") { None } else { Some(deps.transport(None)?) };
    let floor = args.get("--min-confidence").and_then(|v| v.parse().ok());
    let clf = BookCategoryClassifier::new(Some(model.clone()), ClassifierOptions { es, library_root: Some(root.clone()), min_confidence: floor });
    if args.command.as_deref() == Some("reclassify") {
        return reclassify(args, io, &root, &clf);
    }
    for path in &args.paths {
        let path = Path::new(path);
        if !path.exists() {
            writeln!(io.out, "{}: 파일이 없다", path.display()).map_err(|e| e.to_string())?;
            continue;
        }
        let doc = clf.build_document(path);
        let pred = model.predict_top_k(std::slice::from_ref(&doc), floor, args.number("--top", 3.0) as usize).remove(0);
        let text = |k: &str| doc[k].as_str().filter(|v| !v.is_empty()).unwrap_or("-");
        writeln!(
            io.out,
            "\n{}\n  특징 출처: {}  본문 {}자  제목 '{}'  저자 '{}'  publisher '{}'\n  판정: {}\n  근거: {}",
            path.display(),
            if doc["id"].as_str().is_some_and(|s| !s.is_empty()) { "ES" } else { "파일" },
            data::count(doc["text"].as_str().unwrap_or("").chars().count()),
            text("title"),
            text("author"),
            text("publisher"),
            pred.category.as_deref().unwrap_or("판정 안 함"),
            pred.reason
        )
        .map_err(|e| e.to_string())?;
        for (i, (cat, prob)) in pred.ranked.iter().enumerate() {
            writeln!(io.out, "    {}. {cat:<24} {prob:.3}", i + 1).map_err(|e| e.to_string())?;
        }
    }
    Ok(0)
}

fn category_path(root: &Path, category: &str) -> Result<std::path::PathBuf, String> {
    if category.is_empty() || !Path::new(category).components().all(|c| matches!(c, Component::Normal(_))) {
        return Err("카테고리는 루트 아래 상대 경로여야 한다".into());
    }
    let mut path = root.to_path_buf();
    for part in Path::new(category).components() {
        path.push(part);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("카테고리 심볼릭 링크는 따르지 않는다".into());
        }
    }
    Ok(path)
}

fn list_files(root: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let path = entry.path();
        if kind.is_dir() {
            list_files(&path, files)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ["txt", "epub", "pdf", "docx", "doc", "hwp", "rtf", "html"].iter().any(|e| ext.eq_ignore_ascii_case(e))) {
            files.push(path);
        }
    }
    Ok(())
}

fn reclassify(args: &Args, io: &mut CliIo<'_>, root: &Path, clf: &BookCategoryClassifier) -> Result<i32, String> {
    let source = category_path(root, &args.paths[0])?;
    if !source.is_dir() {
        return Err(format!("디렉토리가 없다: {}", source.display()));
    }
    let mut files = Vec::new();
    list_files(&source, &mut files)?;
    files.sort();
    let limit = args.number("--limit", 0.0) as usize;
    if limit > 0 {
        files.truncate(limit);
    }
    let apply = args.flag("--apply");
    writeln!(io.out, "대상 {}건  ({})\n{}", data::count(files.len()), source.display(), if apply { "실제로 파일을 옮긴다." } else { "미리보기다. 파일을 옮기지 않는다." })
        .map_err(|e| e.to_string())?;
    let (mut moved, mut skipped, mut failed) = (0, 0, 0);
    let mut targets = BTreeMap::<String, usize>::new();
    for (i, path) in files.iter().enumerate() {
        let pred = clf.classify_path(path);
        if let Some(cat) = pred.category {
            *targets.entry(cat.clone()).or_default() += 1;
            let dir = category_path(root, &cat)?;
            let dest = dir.join(path.file_name().ok_or("파일 이름이 없다")?);
            if apply {
                // 이미 있는 파일과 동시에 생기는 목적지를 모두 보존한다.
                if fs::symlink_metadata(&dest).is_ok() {
                    skipped += 1;
                    continue;
                }
                let result = (|| -> std::io::Result<()> {
                    fs::create_dir_all(&dir)?;
                    move_file::move_file(path, &dest)
                })();
                if let Err(e) = result {
                    failed += 1;
                    writeln!(io.err, "옮기지 못했다: {e}").map_err(|e| e.to_string())?;
                } else {
                    moved += 1;
                }
            } else {
                moved += 1;
                if moved <= 20 {
                    writeln!(io.out, "  {:.3}  {cat:<22} <- {}", pred.confidence, path.file_name().unwrap_or_default().to_string_lossy()).map_err(|e| e.to_string())?;
                }
            }
        } else {
            skipped += 1;
        }
        if (i + 1) % 500 == 0 {
            writeln!(io.out, "  {}/{} 처리", data::count(i + 1), data::count(files.len())).map_err(|e| e.to_string())?;
        }
    }
    writeln!(
        io.out,
        "\n{} {}건 / 판정 안 함 {}건{}\n\n  카테고리별 건수",
        if apply { "옮김" } else { "옮길 예정" },
        data::count(moved),
        data::count(skipped),
        if failed > 0 { format!(" / 실패 {}건", data::count(failed)) } else { String::new() }
    )
    .map_err(|e| e.to_string())?;
    let mut targets: Vec<_> = targets.into_iter().collect();
    targets.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (cat, n) in targets.iter().take(15) {
        writeln!(io.out, "  {:>6}  {cat}", data::count(*n)).map_err(|e| e.to_string())?;
    }
    if !apply {
        writeln!(io.out, "\n실제로 옮기려면 --apply 를 붙일 것.").map_err(|e| e.to_string())?;
    }
    // Python reports each failed move but keeps the command's success exit code.
    Ok(0)
}
