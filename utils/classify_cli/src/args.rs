use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct Args {
    pub command: Option<String>,
    pub help: bool,
    pub values: BTreeMap<String, Vec<String>>,
    pub paths: Vec<String>,
    pub original: Vec<String>,
}

impl Args {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key)?.last().map(String::as_str)
    }
    pub fn flag(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }
    pub fn number(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }
    pub fn path(&self, key: &str, default: &str) -> PathBuf {
        PathBuf::from(self.get(key).unwrap_or(default))
    }
    pub fn model(&self) -> PathBuf {
        self.get("--model").map(PathBuf::from).or_else(|| std::env::var_os("TM_CLASSIFIER_MODEL_DIR").map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("backend/classifier/export"))
    }
}

pub(crate) fn usage(command: &str) -> &'static str {
    match command {
        "collect" => "--out PATH --min-chars N --exclude-prefix PREFIX (반복 가능) --index NAME --library-root PATH --with-publisher --publisher-workers N",
        "train" => {
            "--corpus PATH --out DIR --config PATH --spill-dir PATH --save-config --min-per-category N --body-max-features N --body-char-max-chars N --body-min-df N --char-ngram/--no-char-ngram --C F --class-weight balanced|none --holdout F --max-iter N --n-jobs N"
        }
        "evaluate" => "--model DIR --corpus PATH",
        "score-calibration" => "--model DIR --corpus PATH --dry-run",
        "bookstore-policy" => "--model DIR --corpus PATH --sample N --sample-from refused|boundary|all --delay SEC --bands N --out PATH --reband PATH",
        "classify" => "PATH... --model DIR --library-root PATH --no-es --min-confidence F --top N",
        "reclassify" => "CATEGORY --library-root PATH --model DIR --no-es --min-confidence F --limit N --apply (기본 미리보기)",
        "info" => "--model DIR",
        _ => "",
    }
}

pub(crate) fn parse(argv: &[&str]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut iter = argv.iter().copied().peekable();
    while iter.peek().is_some_and(|s| matches!(*s, "-v" | "--verbose")) {
        iter.next();
    }
    let Some(command) = iter.next() else {
        args.help = true;
        return Ok(args);
    };
    if matches!(command, "-h" | "--help") {
        args.help = true;
        return Ok(args);
    }
    if !super::commands().contains(&command) {
        return Err(format!("알 수 없는 명령: {command}"));
    }
    args.command = Some(command.into());
    args.original = iter.clone().map(str::to_owned).collect();
    let allowed = usage(command);
    let flags = ["--with-publisher", "--save-config", "--char-ngram", "--no-char-ngram", "--dry-run", "--no-es", "--apply"];
    let mut positional = false;
    while let Some(arg) = iter.next() {
        if !positional && matches!(arg, "-h" | "--help") {
            args.help = true;
            return Ok(args);
        }
        if !positional && arg == "--" {
            positional = true;
            continue;
        }
        if !positional && arg.starts_with('-') {
            let (key, inline) = arg.split_once('=').map_or((arg, None), |(k, v)| (k, Some(v)));
            if !allowed.split([' ', '/']).any(|word| word == key) {
                return Err(format!("{command}: 알 수 없는 옵션 {key}"));
            }
            let value = if flags.contains(&key) {
                if inline.is_some() {
                    return Err(format!("{key}에는 값을 줄 수 없다"));
                }
                "true"
            } else {
                inline.or_else(|| iter.next()).filter(|v| !v.starts_with("--")).ok_or_else(|| format!("{key} 값이 없다"))?
            };
            let ints = [
                "--min-chars",
                "--publisher-workers",
                "--min-per-category",
                "--body-max-features",
                "--body-char-max-chars",
                "--body-min-df",
                "--max-iter",
                "--n-jobs",
                "--sample",
                "--bands",
                "--top",
                "--limit",
            ];
            if ints.contains(&key) {
                let n = value.parse::<usize>().map_err(|_| format!("{key}: 음수가 아닌 정수가 필요하다"))?;
                if n == 0 && !matches!(key, "--min-chars" | "--limit" | "--body-char-max-chars") {
                    return Err(format!("{key}: 0보다 커야 한다"));
                }
            }
            if matches!(key, "--C" | "--holdout" | "--delay" | "--min-confidence") {
                let n = value.parse::<f64>().map_err(|_| format!("{key}: 숫자가 필요하다"))?;
                if !n.is_finite() || n < 0.0 || (key == "--C" && n == 0.0) || (key == "--delay" && n > 86400.0) || (matches!(key, "--holdout" | "--min-confidence") && n > 1.0) {
                    return Err(format!("{key}: 범위를 벗어난 값"));
                }
            }
            if (key == "--sample-from" && !["refused", "boundary", "all"].contains(&value)) || (key == "--class-weight" && !["balanced", "none"].contains(&value)) {
                return Err(format!("{key}: 잘못된 선택 값"));
            }
            args.values.entry(key.into()).or_default().push(value.into());
        } else {
            args.paths.push(arg.into());
        }
    }
    let valid = match command {
        "classify" => !args.paths.is_empty(),
        "reclassify" => args.paths.len() == 1,
        _ => args.paths.is_empty(),
    };
    if !valid {
        return Err(format!("{command}: 위치 인자를 확인할 것. {}", usage(command)));
    }
    Ok(args)
}
