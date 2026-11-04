use crate::{CliIo, Deps, arg_parser::Args, data_loader};
use classifier::{
    bookstore_policy::{bands_from_cuts, quantile_bands},
    classifier_config,
};
use serde_json::{Value, json};

/// backend와 같은 서점 검색(`bookstore::store_clients`의 기본 구현)을 쓴다. 인증서 검증은 libs/bookstore의 기본값이다.
struct LiveStores {
    service: std::sync::Mutex<classifier::book_classifier::BookClassifierService>,
}

impl LiveStores {
    fn new(delay: f64) -> Self {
        use classifier::{BookCategoryClassifier, ClassifierOptions};
        let root = std::path::Path::new("/mnt/data/text");
        let classifier = std::sync::Arc::new(BookCategoryClassifier::new(None, ClassifierOptions::default()));
        let service = classifier::book_classifier::BookClassifierService::new(root, None, Some(classifier)).with_delay(delay).with_keyword_loader(classifier::bookstore_policy::keyword_loader(|content_type| {
            let mapping = db::category_mapping::CategoryMapping::new().map_err(|e| e.to_string())?;
            let all = mapping.get_all_mappings(content_type).map_err(|e| e.to_string())?;
            Ok(serde_json::json!(all))
        }));
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
        if !path.exists() {
            return Err(format!("측정 결과 파일이 없다: {}", path.display()));
        }
        let saved = data_loader::json_file(&path)?;
        let rows = saved["rows"].as_array().filter(|v| !v.is_empty()).ok_or_else(|| format!("건별 결과가 없어 다시 나눌 수 없다: {}\n서점 조회부터 다시 해야 한다.", path.display()))?.clone();
        (rows, saved["sample_from"].as_str().unwrap_or("unknown").to_string(), path)
    } else {
        let model_path = args.model();
        let model = data_loader::model(&model_path)?;
        let corpus_path = args.path("--corpus", "tmp/classifier/corpus.jsonl");
        let mut docs = data_loader::indexed_corpus(&corpus_path)?;
        let mut hold = data_loader::indexed_holdout_indices(
            &docs,
            &data_loader::holdout_manifest_path(&model_path),
            model.config["corpus"]["min_per_category"].as_u64().unwrap_or(30) as usize,
            model.config["holdout"].as_f64().unwrap_or(0.2),
            model.config["seed"].as_u64().unwrap_or(42),
            io.err,
        )?;
        shuffle(&mut hold, model.config["seed"].as_u64().unwrap_or(42));
        let size = args.number("--sample", 2000.0) as usize;
        hold.truncate(size.saturating_mul(20));
        let source = args.get("--sample-from").unwrap_or("refused");
        let floor = model.min_confidence();
        let mut picked_count = 0usize;
        let mut sample = Vec::with_capacity(size);
        for index in hold.iter().copied() {
            let doc = docs.read_document(index).map_err(|error| format!("홀드아웃 JSONL 문서를 읽지 못했다: {error}"))?;
            let prediction = model.predict_top_k(std::slice::from_ref(&doc), Some(0.0), 1).into_iter().next().ok_or("모델이 홀드아웃 문서를 예측하지 못했다")?;
            let selected = match source {
                "refused" => prediction.confidence < floor,
                "boundary" => prediction.confidence < floor * 2.0,
                _ => true,
            };
            if selected {
                picked_count += 1;
                if sample.len() < size {
                    sample.push((doc, prediction));
                }
            }
        }
        let where_from = match source {
            "refused" => format!("모델이 판정을 거부한 구간(확신도 < {floor:.3})"),
            "boundary" => format!("거부 구간과 그 위 (확신도 < {:.3})", floor * 2.0),
            _ => "홀드아웃 전체".into(),
        };
        if picked_count == 0 {
            return Err(format!("{where_from}에 해당하는 문서가 없다. --sample-from을 바꿀 것"));
        }
        writeln!(io.out, "표본 {}건을 {where_from}에서 뽑았다 (후보 {}건 / 풀 {}건)", sample.len(), picked_count, hold.len()).map_err(|e| e.to_string())?;
        let delay = args.number("--delay", 1.0);
        let live = deps.bookstores.is_none().then(|| LiveStores::new(delay));
        let stores: &dyn crate::BookstoreQuery = deps.bookstores.as_deref().unwrap_or_else(|| live.as_ref().unwrap());
        let started = std::time::Instant::now();
        let mut rows = Vec::new();
        for (i, (doc, pred)) in sample.iter().enumerate() {
            let (author, raw_title, title) = classifier::book_classifier::clean_filename_to_author_title(doc["name"].as_str().unwrap_or(""));
            let (y, a, k) = stores.query(&title, &author, &raw_title, delay);
            let votes: Vec<&str> = [&y, &a, &k].iter().filter_map(|v| v["mapped"].as_str()).filter(|s| !s.is_empty()).collect();
            let store_cat = votes.iter().copied().find(|v| votes.len() == 1 || votes.iter().filter(|w| *w == v).count() >= 2);
            let truth = classifier_config::resolve_parent(doc["cat"].as_str());
            rows.push(json!({"confidence":pred.confidence,"model_ok":classifier_config::resolve_parent(pred.category.as_deref()) == truth,"store_cat":store_cat,"store_ok":store_cat.is_some() && classifier_config::resolve_parent(store_cat) == truth,"store_answered":store_cat.is_some()}));
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
    data_loader::save_json(&dest, &report)?;
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
    let note = if threshold == 0.0 { "  (서점이 모델을 이긴 구간이 없다)".to_string() } else { format!("  (측정 범위 {max:.3} 까지)") };
    writeln!(io.out, "\n  서점으로 갈아탈 확신도 상한: {threshold:.3}{note}").map_err(|e| e.to_string())?;
    Ok(json!({"sample":rows.len(),"measured_max_confidence":max,"bands":report,"bookstore_override_below":threshold,"rows":rows}))
}

// ---- Python random.Random(integer).shuffle와 같은 MT19937 순서. 표본을 바꾸지 않기 위해 사용한다. ----
struct Random {
    mt: [u32; 624],
    index: usize,
}
impl Random {
    fn new(seed: u64) -> Self {
        let mut r = Self { mt: [0; 624], index: 624 };
        r.mt[0] = 19650218;
        for i in 1..624 {
            r.mt[i] = 1812433253u32.wrapping_mul(r.mt[i - 1] ^ (r.mt[i - 1] >> 30)).wrapping_add(i as u32);
        }
        let mut key = vec![seed as u32];
        if seed >> 32 != 0 {
            key.push((seed >> 32) as u32);
        }
        let (mut i, mut j) = (1, 0);
        for _ in 0..624 {
            r.mt[i] = (r.mt[i] ^ (r.mt[i - 1] ^ (r.mt[i - 1] >> 30)).wrapping_mul(1664525)).wrapping_add(key[j]).wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                r.mt[0] = r.mt[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            r.mt[i] = (r.mt[i] ^ (r.mt[i - 1] ^ (r.mt[i - 1] >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                r.mt[0] = r.mt[623];
                i = 1;
            }
        }
        r.mt[0] = 0x80000000;
        r
    }
    fn next(&mut self) -> u32 {
        if self.index == 624 {
            for i in 0..624 {
                let y = (self.mt[i] & 0x80000000) | (self.mt[(i + 1) % 624] & 0x7fffffff);
                self.mt[i] = self.mt[(i + 397) % 624] ^ (y >> 1) ^ if y & 1 != 0 { 0x9908b0df } else { 0 };
            }
            self.index = 0;
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;
        y
    }
    fn below(&mut self, n: usize) -> usize {
        let bits = usize::BITS - n.leading_zeros();
        loop {
            let value = if bits <= 32 { (self.next() >> (32 - bits)) as u64 } else { u64::from(self.next()) | (u64::from(self.next() >> (64 - bits)) << 32) };
            if value < n as u64 {
                return value as usize;
            }
        }
    }
}
fn shuffle<T>(values: &mut [T], seed: u64) {
    let mut random = Random::new(seed);
    for i in (1..values.len()).rev() {
        let j = random.below(i + 1);
        values.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::{Random, shuffle, summarize};
    use crate::CliIo;
    use serde_json::{Value, json};

    fn summarize_rows(rows: &[Value]) -> (Result<Value, String>, String) {
        let mut out = Vec::new();
        let result = summarize(rows, 6, &mut CliIo { out: &mut out, err: &mut Vec::new() });
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn summary_rejects_confidences_outside_the_probability_range() {
        for confidence in [json!("0.5"), json!(-0.01), json!(1.01), Value::Null] {
            let rows = [json!({"confidence":confidence,"model_ok":true,"store_ok":false,"store_answered":false})];
            assert_eq!(summarize_rows(&rows).0.unwrap_err(), "잘못된 confidence 값");
        }
    }

    #[test]
    fn summary_requires_boolean_outcomes_for_each_measurement() {
        for field in ["model_ok", "store_ok", "store_answered"] {
            let mut row = json!({"confidence":0.5,"model_ok":true,"store_ok":false,"store_answered":false});
            row[field] = json!("yes");
            assert_eq!(summarize_rows(&[row]).0.unwrap_err(), "서점 측정 행의 정오 값은 bool이어야 한다");
        }
    }

    #[test]
    fn summary_handles_unanswered_stores_and_caps_override_at_measured_maximum() {
        let (unanswered, output) = summarize_rows(&[json!({"confidence":0.3,"model_ok":true,"store_ok":false,"store_answered":false})]);
        let unanswered = unanswered.unwrap();
        assert_eq!(unanswered["bands"][0]["store_coverage"], 0.0);
        assert_eq!(unanswered["bands"][0]["store_accuracy"], 0.0);
        assert!(output.contains("서점이 모델을 이긴 구간이 없다"));

        let (wins, _) = summarize_rows(&[json!({"confidence":0.3,"model_ok":false,"store_ok":true,"store_answered":true})]);
        let wins = wins.unwrap();
        assert_eq!(wins["measured_max_confidence"], 0.3);
        assert_eq!(wins["bookstore_override_below"], 0.3);
    }

    #[test]
    fn summary_returns_an_empty_report_for_zero_rows() {
        let (report, output) = summarize_rows(&[]);
        let report = report.unwrap();

        assert_eq!(report["sample"], 0);
        assert_eq!(report["bands"], json!([]));
        assert_eq!(report["bookstore_override_below"], 0.0);
        assert!(output.contains("서점이 모델을 이긴 구간이 없다"));
    }

    #[test]
    fn summary_uses_store_accuracy_only_in_answered_bands() {
        let rows = [
            json!({"confidence":0.1,"model_ok":true,"store_ok":false,"store_answered":false}),
            json!({"confidence":0.2,"model_ok":true,"store_ok":false,"store_answered":false}),
            json!({"confidence":0.3,"model_ok":true,"store_ok":false,"store_answered":false}),
            json!({"confidence":0.4,"model_ok":true,"store_ok":false,"store_answered":false}),
            json!({"confidence":0.5,"model_ok":true,"store_ok":false,"store_answered":false}),
            json!({"confidence":0.9,"model_ok":false,"store_ok":true,"store_answered":true}),
        ];

        let report = summarize_rows(&rows).0.unwrap();

        let bands = report["bands"].as_array().unwrap();
        assert!(bands.len() > 1);
        let unanswered = bands.iter().find(|band| band["low"] == 0.0).unwrap();
        let answered = bands.iter().find(|band| band["low"].as_f64().unwrap() > 0.5).unwrap();
        assert_eq!(unanswered["store_coverage"], 0.0);
        assert_eq!(unanswered["store_accuracy"], 0.0);
        assert_eq!(answered["store_coverage"], 1.0);
        assert_eq!(answered["store_accuracy"], 1.0);
        assert_eq!(report["bookstore_override_below"], 0.9);
    }

    #[test]
    fn shuffle_matches_python_random_integer_seeds() {
        // Python 3.13 random.Random(seed).shuffle(list(range(20)))로 생성했다.
        for (seed, expected) in [
            (0, [10, 18, 16, 14, 0, 17, 11, 2, 3, 9, 5, 7, 4, 19, 6, 15, 8, 1, 13, 12]),
            (42, [19, 5, 14, 4, 9, 13, 15, 18, 6, 12, 17, 10, 1, 11, 2, 16, 7, 8, 0, 3]),
            (20260914, [7, 9, 6, 5, 1, 2, 16, 19, 12, 3, 8, 4, 10, 18, 15, 17, 0, 11, 14, 13]),
            (1099511627783, [17, 0, 10, 3, 1, 2, 9, 12, 7, 13, 5, 11, 14, 6, 16, 4, 8, 15, 18, 19]),
        ] {
            let mut values: Vec<_> = (0..20).collect();
            shuffle(&mut values, seed);
            assert_eq!(values, expected);
        }
    }

    #[test]
    fn random_range_matches_python_seeded_draws() {
        let mut random = Random::new(42);
        assert_eq!(random.below(41), 40);
        assert_eq!(random.below(3), 0);
        assert_eq!(10 + random.below(99_999_990), 3_356_896);
    }
}
