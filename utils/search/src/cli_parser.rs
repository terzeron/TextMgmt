//! 명령행: 사용법, 인자 읽기, 실행.
//!
//! 읽기 전용이다. `_search`와 `_doc` 조회만 보내고 색인을 바꾸는 요청은 만들지 않는다.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use es_client::transport::{EsRequest, EsTransport};
use search_manager::{ConnectOptions, ESManager};
use serde_json::{Map, Value, json};

const DEFAULT_SIZE: i64 = 20;
/// ES `max_result_window` 기본값. 넘으면 ES가 오류를 낸다.
const MAX_SIZE: i64 = 10_000;
const CONNECTION_ENV: [&str; 3] = ["TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"];

/// 외부 의존. `env`는 환경변수를 읽고, `connect`는 ES 전송 계층을 만든다.
pub struct World<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub connect: &'a dyn Fn(&ConnectOptions) -> Arc<dyn EsTransport>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// 문서 번호(inode)로 `_source` 하나를 읽는다.
    Id(String),
    /// `file_path` 접두어가 같은 문서.
    Path(String),
    /// 웹 키워드 검색과 같은 질의.
    Keyword(String),
    /// `updated_time` 내림차순.
    Latest,
    /// 그대로 보낼 `_search` 본문.
    Query(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub command: Command,
    /// `book` 또는 `comics`.
    pub index: String,
    pub size: i64,
    /// `None`이면 하위 명령의 기본 필드, 빈 목록이면 모든 필드(`--fields=all`).
    pub fields: Option<Vec<String>>,
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = write!(
        out,
        "{program} - Elasticsearch 도서·만화 색인을 읽기 전용으로 조회한다(호스트 진단용).

접속 정보는 환경변수에서 읽고 화면에 내지 않는다. 색인을 바꾸는 요청은 보내지 않는다.

사용법:
  {program} [--size N] [--fields a,b | --fields all] <명령> <index_name> [인자]

명령:
  id <index_name> <doc_id>          문서 하나의 _source(기본: 모든 필드)
  path <index_name> <경로 접두어>    file_path가 접두어로 시작하는 문서, 경로순(색인 루트 기준 상대 경로)
  keyword <index_name> <검색어>      웹 키워드 검색과 같은 질의, 점수순
  latest <index_name>               updated_time 내림차순(같으면 created_time, file_path)
  query <index_name> <JSON>         _search 본문을 그대로 보낸다. 없는 size·track_total_hits·_source만 채운다

인자:
  index_name   book 또는 comics

옵션:
  --size N            최대 결과 수(1~{MAX_SIZE}, 기본 {DEFAULT_SIZE})
  --fields a,b        출력할 _source 필드. all이면 summary까지 모든 필드
                      (기본: id는 모든 필드, 나머지는 목록 필드 {list_fields})
  --                  이후 인자를 옵션으로 읽지 않는다
  -h, --help          이 도움말

출력:
  stdout에 문서 하나당 JSON 한 줄: {{\"id\": ..., \"score\": ..., \"source\": {{...}}}}
  목록 명령은 stderr에 \"total: <전체 건수> (출력 <줄 수>)\"를 낸다.

환경변수:
  TM_ES_URL, TM_ES_USER, TM_ES_PASSWORD, TM_ES_BOOK_INDEX, TM_ES_COMICS_INDEX
  HTTPS 사설 CA: TM_ES_CA_CERT (PEM 파일 경로)

예:
  {program} path comics '1_웹툰/통 시즌3/'
  {program} latest comics --size 5 --fields file_path,updated_time
  {program} query book '{{\"query\":{{\"term\":{{\"isbn\":\"9788937460449\"}}}}}}'

종료 코드: 0 정상, 1 조회 실패·문서 없음, 2 인자 오류, 255 환경변수 누락
",
        list_fields = es_client::LIST_SOURCE_FIELDS.join(",")
    );
}

fn parse_size(value: &str) -> Result<i64, String> {
    match value.trim().parse::<i64>() {
        Ok(n) if (1..=MAX_SIZE).contains(&n) => Ok(n),
        _ => Err(format!("--size는 1~{MAX_SIZE} 정수여야 한다: {value}")),
    }
}

fn parse_fields(value: &str) -> Result<Vec<String>, String> {
    if value == "all" {
        return Ok(Vec::new());
    }
    let fields: Vec<String> = value.split(',').map(str::trim).filter(|f| !f.is_empty()).map(String::from).collect();
    if fields.is_empty() {
        return Err("--fields가 비었다".into());
    }
    Ok(fields)
}

/// 옵션 값을 `--name=value` 또는 다음 인자에서 읽는다.
fn option_value<'a>(name: &str, inline: Option<&'a str>, rest: &mut impl Iterator<Item = &'a String>) -> Result<&'a str, String> {
    match inline {
        Some(value) => Ok(value),
        None => rest.next().map(String::as_str).ok_or_else(|| format!("--{name}에 값이 없다")),
    }
}

/// 인자를 읽는다. 옵션은 위치 인자 앞뒤 어디에 와도 된다. `-h`/`--help`는 `run`이 먼저 처리한다.
pub fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut size = DEFAULT_SIZE;
    let mut fields = None;
    let mut positional: Vec<&str> = Vec::new();
    let mut rest = args.iter();
    let mut options_done = false;
    while let Some(arg) = rest.next() {
        if options_done || !arg.starts_with("--") {
            positional.push(arg);
            continue;
        }
        let (name, inline) = match arg[2..].split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (&arg[2..], None),
        };
        match name {
            "" => options_done = true,
            "size" => size = parse_size(option_value(name, inline, &mut rest)?)?,
            "fields" => fields = Some(parse_fields(option_value(name, inline, &mut rest)?)?),
            _ => return Err(format!("알 수 없는 옵션: {arg}")),
        }
    }

    let [command, index, args @ ..] = positional.as_slice() else {
        return Err("<명령>과 <index_name>이 필요하다".into());
    };
    if !matches!(*index, "book" | "comics") {
        return Err(format!("index_name은 book 또는 comics여야 한다: {index}"));
    }
    let one = |what: &str| -> Result<String, String> {
        match args {
            [value] => Ok(value.to_string()),
            _ => Err(format!("{command}에는 <{what}> 인자 하나가 필요하다")),
        }
    };
    let command = match *command {
        "id" => {
            let id = one("doc_id")?;
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("doc_id는 숫자여야 한다: {id}"));
            }
            Command::Id(id)
        }
        "path" => Command::Path(one("경로 접두어")?),
        "keyword" => Command::Keyword(one("검색어")?),
        "latest" if args.is_empty() => Command::Latest,
        "latest" => return Err("latest에는 인자가 없다".into()),
        "query" => {
            let body: Value = serde_json::from_str(&one("JSON")?).map_err(|e| format!("query 본문이 JSON이 아니다: {e}"))?;
            if !body.is_object() {
                return Err("query 본문은 JSON 객체여야 한다".into());
            }
            Command::Query(body)
        }
        other => return Err(format!("알 수 없는 명령: {other}")),
    };
    Ok(Options { command, index: index.to_string(), size, fields })
}

/// 환경변수로 접속 정보를 만든다. 없는 변수 이름을 오류로 돌려준다.
fn connect_options(index: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<ConnectOptions, String> {
    let index_env = if index == "comics" { "TM_ES_COMICS_INDEX" } else { "TM_ES_BOOK_INDEX" };
    let get = |key: &str| env(key).filter(|v| !v.is_empty()).ok_or_else(|| format!("환경변수 {key}가 없다"));
    for key in CONNECTION_ENV.iter().chain([&index_env]) {
        get(key)?;
    }
    let url = get("TM_ES_URL")?;
    let https = url.to_lowercase().starts_with("https");
    Ok(ConnectOptions {
        hosts: vec![url],
        basic_auth: (get("TM_ES_USER")?, get("TM_ES_PASSWORD")?),
        request_timeout: 30.0,
        retry_on_timeout: false,
        verify_certs: https.then_some(true),
        ssl_show_warn: None,
        ca_cert_path: env("TM_ES_CA_CERT").map(PathBuf::from),
        index: get(index_env)?,
    })
}

/// 문서 번호는 inode라 숫자로 낸다. 숫자가 아니면 문자열 그대로 낸다.
fn id_value(id: &Value) -> Value {
    id.as_str().and_then(|s| s.parse::<i64>().ok()).map_or_else(|| id.clone(), Value::from)
}

fn source_filter(fields: &Option<Vec<String>>, default: &[&str]) -> Option<Value> {
    match fields {
        None => Some(json!(default)),
        Some(fields) if fields.is_empty() => None,
        Some(fields) => Some(json!(fields)),
    }
}

/// 하위 명령이 보낼 요청.
enum Request {
    /// `_search` 본문.
    Search(Value),
    /// `_doc` 조회할 문서 번호.
    Get(String),
}

fn request_of(options: &Options) -> Request {
    let list_fields = source_filter(&options.fields, &es_client::LIST_SOURCE_FIELDS);
    let (query, sort) = match &options.command {
        Command::Id(id) => return Request::Get(id.clone()),
        Command::Path(prefix) => (json!({"prefix": {"file_path": prefix}}), Some(json!([{"file_path": "asc"}]))),
        Command::Keyword(keyword) => (ESManager::keyword_query(keyword), None),
        Command::Latest => (json!({"match_all": {}}), Some(ESManager::latest_sort())),
        Command::Query(given) => {
            let mut body = given.as_object().cloned().unwrap_or_default();
            body.entry("size").or_insert(json!(options.size));
            body.entry("track_total_hits").or_insert(json!(true));
            if options.fields.is_some()
                && let Some(fields) = list_fields
            {
                body.entry("_source").or_insert(fields);
            }
            return Request::Search(Value::Object(body));
        }
    };
    let mut body = json!({"query": query, "size": options.size, "track_total_hits": true});
    if let Some(sort) = sort {
        body["sort"] = sort;
    }
    if let Some(fields) = list_fields {
        body["_source"] = fields;
    }
    Request::Search(body)
}

fn write_line(out: &mut dyn Write, value: &Value) -> std::io::Result<()> {
    writeln!(out, "{value}")
}

fn run_search(es: &dyn EsTransport, body: Value, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let response = match es.send(&EsRequest::search(body)) {
        Ok(response) => response,
        Err(e) => {
            let _ = writeln!(err, "ERROR 검색 실패: {e}");
            return 1;
        }
    };
    let hits = response["hits"]["hits"].as_array().cloned().unwrap_or_default();
    for hit in &hits {
        let line = json!({"id": id_value(&hit["_id"]), "score": hit.get("_score").cloned().unwrap_or(Value::Null), "source": hit.get("_source").cloned().unwrap_or_else(|| json!({}))});
        if write_line(out, &line).is_err() {
            return 1;
        }
    }
    let total = response["hits"]["total"]["value"].as_i64().map_or_else(|| "?".to_string(), |n| n.to_string());
    let _ = writeln!(err, "total: {total} (출력 {})", hits.len());
    0
}

fn run_get(es: &dyn EsTransport, id: &str, fields: &Option<Vec<String>>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let response = match es.send(&EsRequest::Get { index: None, id: id.to_string() }) {
        Ok(response) => response,
        Err(e) if e.is_not_found() => {
            let _ = writeln!(err, "ERROR 문서가 없다: {id}");
            return 1;
        }
        Err(e) => {
            let _ = writeln!(err, "ERROR 조회 실패: {e}");
            return 1;
        }
    };
    let mut source = response.get("_source").cloned().unwrap_or_else(|| json!({}));
    if let (Some(fields), Some(map)) = (fields.as_ref().filter(|f| !f.is_empty()), source.as_object()) {
        let kept: Map<String, Value> = map.iter().filter(|(key, _)| fields.contains(key)).map(|(k, v)| (k.clone(), v.clone())).collect();
        source = Value::Object(kept);
    }
    if write_line(out, &json!({"id": id_value(&json!(id)), "source": source})).is_err() {
        return 1;
    }
    0
}

/// 진입점. 종료 코드: 정상 0, 조회 실패·문서 없음 1, 인자 오류 2, 환경변수 누락 255.
pub fn run(program: &str, args: &[String], world: &World, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if args.is_empty() || args.iter().any(|a| matches!(a.as_str(), "-h" | "--help")) {
        print_usage(program, out);
        return 0;
    }
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(message) => {
            let _ = writeln!(err, "ERROR {message}\n도움말: {program} -h");
            return 2;
        }
    };
    let connect = match connect_options(&options.index, world.env) {
        Ok(connect) => connect,
        Err(message) => {
            let _ = writeln!(err, "ERROR {message}");
            return 255;
        }
    };
    let es = (world.connect)(&connect);
    match request_of(&options) {
        Request::Get(id) => run_get(es.as_ref(), &id, &options.fields, out, err),
        Request::Search(body) => run_search(es.as_ref(), body, out, err),
    }
}
