//! ES 문서에서 카테고리별 균등 무작위 표본을 뽑아 도서 분류 모델을 학습하고 파일로 쓴다.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use classifier::feature_hasher::{hash_features, BookFields, SparseVec};
use classifier::softmax_model::{ModelHeader, SoftmaxModel};
use classifier_trainer::holdout_evaluator::{evaluate, format_report};
use classifier_trainer::model_trainer::{train_with_progress, TrainConfig};
use classifier_trainer::sample_drawer::{draw, group};
use search_manager::ESManager;

const MODEL_VERSION: u32 = 1;

fn print_usage() {
    println!(
        "Usage: classifier_trainer --output <파일> [옵션]

ES 색인의 책 문서(summary·title·author)로 카테고리 분류 모델을 학습한다.
카테고리마다 최대 N권을 균등 무작위로 뽑는다. 0_ 카테고리, 9_북스캔OCR, 루트 낱개 파일은 뺀다.
ES 접속은 TM_ES_URL, TM_ES_USER, TM_ES_PASSWORD, TM_ES_BOOK_INDEX를 쓴다.

옵션:
  --output <파일>        모델 파일 경로(필수). 임시 파일에 쓴 뒤 rename한다
  --per-category <N>     카테고리당 최대 표본 수 (기본 1000)
  --seed <u64>           표본·학습 난수 seed (기본: 무작위, 출력에 기록)
  --holdout <비율>       평가용으로 떼는 표본 비율 (기본 0.1)
  --hash-bits <B>        해시 버킷 2^B (기본 17)
  --epochs <N>           학습 반복 수 (기본 8)
  -h, --help             이 도움말"
    );
}

struct Options {
    output: PathBuf,
    per_category: usize,
    seed: u64,
    holdout: f64,
    config: TrainConfig,
}

fn parse<T: std::str::FromStr>(name: &str, value: Option<String>) -> Result<T, String> {
    let value = value.ok_or_else(|| format!("{name}에 값이 없다"))?;
    value.parse().map_err(|_| format!("{name} 값을 해석하지 못했다: {value}"))
}

fn parse_args(args: Vec<String>) -> Result<Options, String> {
    let mut output = None;
    let mut seed = None;
    let (mut per_category, mut holdout, mut config) = (1000usize, 0.1f64, TrainConfig::default());
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--output" => output = Some(PathBuf::from(parse::<String>(&arg, it.next())?)),
            "--per-category" => per_category = parse(&arg, it.next())?,
            "--seed" => seed = Some(parse(&arg, it.next())?),
            "--holdout" => holdout = parse(&arg, it.next())?,
            "--hash-bits" => config.hash_bits = parse(&arg, it.next())?,
            "--epochs" => config.epochs = parse(&arg, it.next())?,
            other => return Err(format!("모르는 옵션: {other}")),
        }
    }
    if !(0.0..1.0).contains(&holdout) {
        return Err(format!("--holdout은 0 이상 1 미만이어야 한다: {holdout}"));
    }
    if !(1..=24).contains(&config.hash_bits) {
        return Err(format!("--hash-bits는 1~24여야 한다: {}", config.hash_bits));
    }
    let output = output.ok_or("--output이 필요하다")?;
    let seed = seed.unwrap_or_else(rand::random);
    config.seed = seed;
    Ok(Options { output, per_category, seed, holdout, config })
}

/// (행, 카테고리별 실제 사용 수, 뺀 수).
type Featurized = (Vec<(SparseVec, usize)>, Vec<u64>, usize);

/// 문서를 받아 특징으로 바꾼다. 본문 근거가 없는 문서는 뺀다. 반환: (행, 카테고리별 실제 사용 수, 뺀 수).
fn featurize(es: &ESManager, picked: &[(i64, String)], index: &BTreeMap<String, usize>, hash_bits: u32) -> Result<Featurized, String> {
    let ids: Vec<i64> = picked.iter().map(|(id, _)| *id).collect();
    let docs: BTreeMap<i64, serde_json::Value> = es.mget_sources(&ids, &["summary", "title", "author"]).map_err(|e| format!("ES 문서 조회 실패: {e}"))?.into_iter().collect();
    let mut rows = Vec::with_capacity(picked.len());
    let mut used = vec![0u64; index.len()];
    let mut skipped = 0usize;
    for (id, category) in picked {
        let fields = docs.get(id).map(BookFields::from_es_source).unwrap_or_default();
        if fields.is_empty() {
            skipped += 1;
            continue;
        }
        let y = index[category];
        used[y] += 1;
        rows.push((hash_features(&fields, hash_bits), y));
    }
    Ok((rows, used, skipped))
}

fn run(opts: Options) -> Result<(), String> {
    let es = ESManager::from_env().map_err(|e| format!("ES 접속 실패: {e}"))?;
    let all = es.list_ids_and_paths().map_err(|e| format!("ES 문서 열거 실패: {e}"))?;
    let population = group(&all);
    eprintln!("INFO 모집단 {}건, 대상 카테고리 {}개", population.by_category.values().map(Vec::len).sum::<usize>(), population.by_category.len());

    let sample = draw(&population, opts.per_category, opts.holdout, opts.seed);
    eprintln!("INFO 표본: 학습 {}건, 홀드아웃 {}건 (seed {})", sample.train.len(), sample.holdout.len(), opts.seed);

    let categories: Vec<String> = population.by_category.keys().cloned().collect();
    let index: BTreeMap<String, usize> = categories.iter().enumerate().map(|(i, c)| (c.clone(), i)).collect();
    let (train_rows, used, skipped_train) = featurize(&es, &sample.train, &index, opts.config.hash_bits)?;
    let (holdout_rows, _, skipped_holdout) = featurize(&es, &sample.holdout, &index, opts.config.hash_bits)?;
    eprintln!("INFO summary·title이 빈 문서를 뺐다: 학습 {skipped_train}건, 홀드아웃 {skipped_holdout}건");

    let (weights, bias) = train_with_progress(&train_rows, &categories, &opts.config, &|epoch, loss| eprintln!("INFO epoch {} 평균 손실 {loss:.4}", epoch + 1));
    let model = SoftmaxModel {
        header: ModelHeader {
            version: MODEL_VERSION,
            seed: opts.seed,
            created_unix_secs: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            hash_bits: opts.config.hash_bits,
            population_counts: categories.iter().map(|c| population.size_of(c) as u64).collect(),
            sample_counts: used,
            categories,
        },
        weights,
        bias,
    };
    println!("seed {}", opts.seed);
    print!("{}", format_report(&evaluate(&model, &holdout_rows)));

    if let Some(parent) = opts.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("출력 디렉토리를 만들지 못했다: {e}"))?;
    }
    model.save_atomic(&opts.output).map_err(|e| format!("모델을 쓰지 못했다: {e}"))?;
    eprintln!("INFO 모델을 썼다: {}", opts.output.display());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        return ExitCode::from(1);
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return ExitCode::SUCCESS;
    }
    let opts = match parse_args(args) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("ERROR {e}");
            print_usage();
            return ExitCode::from(1);
        }
    };
    match run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ERROR {e}");
            ExitCode::from(1)
        }
    }
}
