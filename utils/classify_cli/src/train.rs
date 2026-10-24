use crate::{CliIo, Deps, args::Args, data};
use std::path::PathBuf;

/// sklearn만 Python에서 실행한다. 셸을 거치지 않아 공백·따옴표·메타문자도 경로 그대로 전달한다.
pub(super) fn run(args: &Args, io: &mut CliIo<'_>, deps: &Deps) -> Result<i32, String> {
    data::corpus(&args.path("--corpus", "tmp/classifier/corpus.jsonl"))?;
    let out = args.get("--out").map(PathBuf::from).unwrap_or_else(|| args.model());
    if out.is_file() {
        return Err("--out은 Rust 모델을 저장할 디렉토리여야 한다".into());
    }
    let mut joblib_name = out.as_os_str().to_os_string();
    joblib_name.push(".joblib");
    let joblib = PathBuf::from(joblib_name);
    let mut forwarded = Vec::new();
    let mut iter = args.original.iter();
    while let Some(arg) = iter.next() {
        if arg == "--out" {
            iter.next();
        } else if !arg.starts_with("--out=") {
            forwarded.push(arg.clone());
        }
    }
    forwarded.extend(["--out".into(), joblib.to_string_lossy().into_owned()]);
    let launch = |module: &str, params: Vec<String>| {
        let code = format!("import sys; sys.path.insert(0, sys.argv.pop(1)); from {module} import main; raise SystemExit(main(sys.argv[1:]))");
        let mut argv = vec![deps.python.to_string_lossy().into_owned(), "-c".into(), code, deps.project.to_string_lossy().into_owned()];
        argv.extend(params);
        argv
    };
    let mut train = vec!["train".into()];
    train.extend(forwarded);
    let result = deps.runner.run(&launch("utils.classify_cli", train), None).map_err(|e| format!("Python 학습 실행 실패: {e}. TM_CLASSIFIER_PYTHON을 확인할 것"))?;
    io.out.write_all(&result.stdout).map_err(|e| e.to_string())?;
    io.err.write_all(&result.stderr).map_err(|e| e.to_string())?;
    if result.returncode != 0 {
        return Ok(if result.returncode < 0 { 1 } else { result.returncode });
    }
    let export_args = vec!["--model".into(), joblib.to_string_lossy().into_owned(), "--out".into(), out.to_string_lossy().into_owned()];
    let result = deps.runner.run(&launch("backend.classifier.export", export_args), None).map_err(|e| format!("모델 export 실행 실패: {e}"))?;
    io.out.write_all(&result.stdout).map_err(|e| e.to_string())?;
    io.err.write_all(&result.stderr).map_err(|e| e.to_string())?;
    if result.returncode != 0 {
        return Ok(if result.returncode < 0 { 1 } else { result.returncode });
    }
    // export가 실제로 Rust에서 읽히는지 확인한 뒤 학습 때의 명단을 옮긴다.
    data::model(&out)?;
    let manifest = data::json_file(&data::holdout_manifest_path(&joblib))?;
    data::save_json(&data::holdout_manifest_path(&out), &manifest)?;
    writeln!(io.out, "Rust 모델 저장: {}\n홀드아웃 명단 저장: {}", out.display(), data::holdout_manifest_path(&out).display()).map_err(|e| e.to_string())?;
    Ok(0)
}
