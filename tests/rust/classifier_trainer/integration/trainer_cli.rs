//! 학습 CLI 진입점: 사용법과 인자 오류. 실제 ES 학습은 여기서 돌리지 않는다.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_classifier_trainer")).args(args).env_remove("TM_ES_URL").output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn no_arguments_prints_usage_and_exits_1() {
    let (code, stdout, _) = run(&[]);
    assert_eq!(code, 1);
    assert!(stdout.contains("Usage: classifier_trainer"), "{stdout}");
    assert!(stdout.contains("--per-category"));
}

#[test]
fn help_prints_usage_and_exits_0() {
    let (code, stdout, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Usage: classifier_trainer"));
}

#[test]
fn missing_output_exits_1() {
    let (code, stdout, stderr) = run(&["--per-category", "100"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--output"), "{stderr}");
    assert!(stdout.contains("Usage"));
}

#[test]
fn bad_number_exits_1() {
    let (code, _, stderr) = run(&["--output", "/nonexistent/m.bin", "--per-category", "많이"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--per-category"), "{stderr}");
}

#[test]
fn unknown_option_exits_1() {
    let (code, _, stderr) = run(&["--output", "m.bin", "--what"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--what"), "{stderr}");
}

#[test]
fn missing_es_env_fails_before_training() {
    let (code, _, stderr) = run(&["--output", "/nonexistent/m.bin"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("ES"), "{stderr}");
}

#[test]
fn holdout_out_of_range_exits_1() {
    let (code, _, stderr) = run(&["--output", "m.bin", "--holdout", "1.0"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--holdout"), "{stderr}");
}

#[test]
fn hash_bits_out_of_range_exits_1() {
    let (code, _, stderr) = run(&["--output", "m.bin", "--hash-bits", "25"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--hash-bits"), "{stderr}");
}

/// 요청 한 건을 끝까지 읽고 `(메서드, 경로)`를 돌려준다.
fn read_request(stream: &mut std::net::TcpStream) -> (String, String) {
    use std::io::Read;
    let mut request = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
        let headers = String::from_utf8_lossy(&request[..end]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok()).flatten()
            })
            .unwrap_or(0);
        if request.len() >= end + 4 + length {
            break;
        }
    }
    let line = String::from_utf8_lossy(&request).lines().next().unwrap_or_default().to_string();
    let mut parts = line.split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().split('?').next().unwrap_or_default().to_string();
    (method, path)
}

/// 학습에 필요한 ES 요청(info, scroll 검색, scroll, clear scroll, mget)에만 답하는 가짜 서버. 주소를 돌려준다.
fn spawn_fake_es() -> String {
    use serde_json::json;
    use std::io::Write;

    // 카테고리 둘에 6건씩. 6건이면 홀드아웃을 뗀다. 6번 문서는 summary·title이 비어 학습에서 빠진다.
    let docs: Vec<(i64, &str)> = (1..=12).map(|id| (id, if id <= 6 { "1_소설" } else { "2_만화" })).collect();
    let hits: Vec<_> = docs.iter().map(|(id, cat)| json!({"_id": id.to_string(), "_source": {"file_path": format!("{cat}/{id}.epub")}})).collect();
    let sources: Vec<_> = docs
        .iter()
        .map(|(id, cat)| {
            let source = if *id == 6 { json!({"summary": " ", "title": ""}) } else { json!({"summary": format!("{cat} 줄거리 {id}"), "title": format!("{cat} 제목"), "author": ["작가"]}) };
            json!({"_id": id.to_string(), "found": true, "_source": source})
        })
        .collect();
    let search = json!({"_scroll_id": "s1", "hits": {"hits": hits}}).to_string();
    let empty = json!({"_scroll_id": "s1", "hits": {"hits": []}}).to_string();
    let mget = json!({"docs": sources}).to_string();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let body = match read_request(&mut stream) {
                (m, p) if m == "POST" && p.ends_with("/_search") => search.clone(),
                (m, p) if m == "POST" && p == "/_search/scroll" => empty.clone(),
                (m, p) if m == "POST" && p == "/_mget" => mget.clone(),
                _ => r#"{"ok":true}"#.to_string(),
            };
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    format!("http://{address}")
}

#[test]
fn trains_from_es_and_writes_model_into_new_directory() {
    let url = spawn_fake_es();
    let dir = std::env::temp_dir().join(format!("classifier_trainer_it_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let output = dir.join("nested").join("model.bin");

    let out = Command::new(env!("CARGO_BIN_EXE_classifier_trainer"))
        .args(["--output", output.to_str().unwrap(), "--seed", "7", "--hash-bits", "8", "--epochs", "2"])
        .env("TM_ES_URL", &url)
        .env("TM_ES_USER", "u")
        .env("TM_ES_PASSWORD", "p")
        .env("TM_ES_BOOK_INDEX", "books")
        .env_remove("TM_ES_CA_CERT")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(stdout.contains("seed 7"), "{stdout}");
    assert!(stderr.contains("모집단 12건, 대상 카테고리 2개"), "{stderr}");
    assert!(stderr.contains("표본: 학습 10건, 홀드아웃 2건 (seed 7)"), "{stderr}");
    // 6번 문서는 학습·홀드아웃 중 한쪽에서만 빠진다.
    let skipped_train = if stderr.contains("빈 문서를 뺐다: 학습 1건, 홀드아웃 0건") {
        1
    } else {
        assert!(stderr.contains("빈 문서를 뺐다: 학습 0건, 홀드아웃 1건"), "{stderr}");
        0
    };
    let model = classifier::softmax_model::SoftmaxModel::load(&output).unwrap();
    assert_eq!(model.header.categories, vec!["1_소설".to_string(), "2_만화".to_string()]);
    assert_eq!(model.header.population_counts, vec![6, 6]);
    assert_eq!(model.header.sample_counts.iter().sum::<u64>(), 10 - skipped_train);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn output_directory_that_cannot_be_created_exits_1() {
    let url = spawn_fake_es();
    let blocker = std::env::temp_dir().join(format!("classifier_trainer_blocker_{}", std::process::id()));
    std::fs::write(&blocker, b"file").unwrap();
    let output = blocker.join("sub").join("model.bin");

    let out = Command::new(env!("CARGO_BIN_EXE_classifier_trainer"))
        .args(["--output", output.to_str().unwrap(), "--seed", "1", "--hash-bits", "8", "--epochs", "1"])
        .env("TM_ES_URL", &url)
        .env("TM_ES_USER", "u")
        .env("TM_ES_PASSWORD", "p")
        .env("TM_ES_BOOK_INDEX", "books")
        .env_remove("TM_ES_CA_CERT")
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&blocker);

    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("출력 디렉토리를 만들지 못했다"));
}
