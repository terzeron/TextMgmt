use crate::{CliIo, Deps, args::Args, data};
use backend::classifier::{
    bookstore_policy::{bands_from_cuts, quantile_bands},
    config,
};
use serde_json::{Value, json};

#[path = "shuffle.rs"]
mod shuffle;

/// 기존 HTTPS 클라이언트를 재사용한다. 인증서 검증은 libs/bookstore의 기본값이다.
struct LiveStores {
    service: std::sync::Mutex<backend::book_classifier::BookClassifierService>,
}

struct StoreClient {
    store: bookstore::Store,
    fetcher: bookstore::HttpFetcher,
}

impl backend::bookstore::BookstoreSearch for StoreClient {
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<backend::bookstore::SearchRow>, String, String) {
        let outcome = bookstore::search(self.store, &self.fetcher, isbn, title, author);
        (outcome.rows.into_iter().map(|r| (r.title, r.author, r.category, r.detail_url, r.search_url, r.isbn)).collect(), outcome.keyword, outcome.method.into())
    }
}

impl LiveStores {
    fn new(delay: f64) -> Self {
        use backend::classifier::{BookCategoryClassifier, ClassifierOptions};
        let root = std::path::Path::new("/mnt/data/text");
        let classifier = std::sync::Arc::new(BookCategoryClassifier::new(None, ClassifierOptions::default()));
        let mut service = backend::book_classifier::BookClassifierService::new(root, None, Some(classifier)).with_delay(delay);
        let client = |store| Box::new(StoreClient { store, fetcher: bookstore::HttpFetcher::new() });
        service.yes24 = client(bookstore::Store::Yes24);
        service.aladin = client(bookstore::Store::Aladin);
        service.kyobo = client(bookstore::Store::Kyobo);
        Self { service: std::sync::Mutex::new(service) }
    }
}

impl crate::BookstoreQuery for LiveStores {
    fn query(&self, title: &str, author: &str, raw_title: &str, _delay: f64) -> (Value, Value, Value) {
        self.service.lock().unwrap_or_else(|e| e.into_inner()).query_bookstores(title, author, raw_title, "")
    }
}

pub(super) fn run(args: &Args, io: &mut CliIo<'_>, deps: &Deps) -> Result<i32, String> {
    let (rows, source, default_out) = if let Some(path) = args.get("--reband") {
        let path = std::path::PathBuf::from(path);
        let saved = data::json_file(&path)?;
        let rows = saved["rows"].as_array().filter(|v| !v.is_empty()).ok_or("건별 결과가 없어 다시 나눌 수 없다. 서점 조회부터 다시 해야 한다.")?.clone();
        (rows, saved["sample_from"].as_str().unwrap_or("unknown").to_string(), path)
    } else {
        let model_path = args.model();
        let model = data::model(&model_path)?;
        let docs = data::corpus(&args.path("--corpus", "tmp/classifier/corpus.jsonl"))?;
        let mut hold = data::holdout(&docs, &model, &model_path, io.err)?;
        shuffle::shuffle(&mut hold, model.config["seed"].as_u64().unwrap_or(42));
        let size = args.number("--sample", 2000.0) as usize;
        hold.truncate(size.saturating_mul(20));
        let predictions = model.predict_top_k(&hold, Some(0.0), 1);
        let source = args.get("--sample-from").unwrap_or("refused");
        let floor = model.min_confidence();
        let picked: Vec<_> = hold
            .iter()
            .zip(&predictions)
            .filter(|(_, p)| match source {
                "refused" => p.confidence < floor,
                "boundary" => p.confidence < floor * 2.0,
                _ => true,
            })
            .collect();
        let where_from = match source {
            "refused" => format!("모델이 판정을 거부한 구간(확신도 < {floor:.3})"),
            "boundary" => format!("거부 구간과 그 위 (확신도 < {:.3})", floor * 2.0),
            _ => "홀드아웃 전체".into(),
        };
        if picked.is_empty() {
            return Err(format!("{where_from}에 해당하는 문서가 없다. --sample-from을 바꿀 것"));
        }
        let sample: Vec<_> = picked.iter().take(size).collect();
        writeln!(io.out, "표본 {}건을 {where_from}에서 뽑았다 (후보 {}건 / 풀 {}건)", sample.len(), picked.len(), hold.len()).map_err(|e| e.to_string())?;
        let delay = args.number("--delay", 1.0);
        let live = deps.bookstores.is_none().then(|| LiveStores::new(delay));
        let stores: &dyn crate::BookstoreQuery = deps.bookstores.as_deref().unwrap_or_else(|| live.as_ref().unwrap());
        let started = std::time::Instant::now();
        let mut rows = Vec::new();
        for (i, (doc, pred)) in sample.iter().enumerate() {
            let (author, raw_title, title) = backend::book_classifier::clean_filename_to_author_title(doc["name"].as_str().unwrap_or(""));
            let (y, a, k) = stores.query(&title, &author, &raw_title, delay);
            let votes: Vec<&str> = [&y, &a, &k].iter().filter_map(|v| v["mapped"].as_str()).filter(|s| !s.is_empty()).collect();
            let store_cat = votes.iter().copied().find(|v| votes.len() == 1 || votes.iter().filter(|w| *w == v).count() >= 2);
            let truth = config::resolve_parent(doc["cat"].as_str());
            rows.push(json!({"confidence":pred.confidence,"model_ok":config::resolve_parent(pred.category.as_deref()) == truth,"store_cat":store_cat,"store_ok":store_cat.is_some() && config::resolve_parent(store_cat) == truth,"store_answered":store_cat.is_some()}));
            if (i + 1) % 100 == 0 {
                let rate = (i + 1) as f64 / started.elapsed().as_secs_f64().max(0.001);
                writeln!(io.out, "  {}/{}  {:.0}건/분  남은 시간 약 {:.0}분", i + 1, sample.len(), rate * 60.0, (sample.len() - i - 1) as f64 / rate / 60.0).map_err(|e| e.to_string())?;
            }
        }
        (rows, source.to_string(), model_path.parent().unwrap_or(std::path::Path::new(".")).join("bookstore_policy.json"))
    };
    let report = summarize(&rows, args.number("--bands", 6.0) as usize, io)?;
    let mut report = report;
    report["sample_from"] = json!(source);
    let dest = args.get("--out").map(std::path::PathBuf::from).unwrap_or(default_out);
    data::save_json(&dest, &report)?;
    writeln!(io.out, "저장: {}", dest.display()).map_err(|e| e.to_string())?;
    Ok(0)
}

fn summarize(rows: &[Value], count: usize, io: &mut CliIo<'_>) -> Result<Value, String> {
    let confidences: Vec<f64> = rows.iter().map(|r| r["confidence"].as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or("잘못된 confidence 값")).collect::<Result<_, _>>()?;
    if rows.iter().any(|r| ["model_ok", "store_ok", "store_answered"].iter().any(|k| !r[k].is_boolean())) {
        return Err("서점 측정 행의 정오 값은 bool이어야 한다".into());
    }
    let mut report = Vec::new();
    let mut threshold: f64 = 0.0;
    writeln!(io.out, "\n  확신도 구간   건수   모델 정답률   서점 응답률   서점 정답률").map_err(|e| e.to_string())?;
    for (lo, hi) in bands_from_cuts(&quantile_bands(&confidences, count)) {
        let band: Vec<_> = rows.iter().zip(&confidences).filter(|(_, c)| lo <= **c && **c < hi).map(|(r, _)| r).collect();
        if band.is_empty() {
            continue;
        }
        let answered: Vec<_> = band.iter().filter(|r| r["store_answered"] == true).collect();
        let model_accuracy = band.iter().filter(|r| r["model_ok"] == true).count() as f64 / band.len() as f64;
        let store_coverage = answered.len() as f64 / band.len() as f64;
        let store_accuracy = if answered.is_empty() { 0.0 } else { answered.iter().filter(|r| r["store_ok"] == true).count() as f64 / answered.len() as f64 };
        if store_accuracy > model_accuracy {
            threshold = threshold.max(hi);
        }
        report.push(json!({"low":lo,"high":hi,"n":band.len(),"model_accuracy":model_accuracy,"store_coverage":store_coverage,"store_accuracy":store_accuracy}));
        writeln!(io.out, "  {lo:.3}-{hi:.3} {:>5}   {:>9.1}%   {:>9.1}%   {:>9.1}%", band.len(), model_accuracy * 100.0, store_coverage * 100.0, store_accuracy * 100.0).map_err(|e| e.to_string())?;
    }
    let max = confidences.iter().copied().fold(0.0, f64::max);
    threshold = threshold.min(max);
    writeln!(io.out, "\n  서점으로 갈아탈 확신도 상한: {threshold:.3}  (측정 범위 {max:.3} 까지)").map_err(|e| e.to_string())?;
    Ok(json!({"sample":rows.len(),"measured_max_confidence":max,"bands":report,"bookstore_override_below":threshold,"rows":rows}))
}
