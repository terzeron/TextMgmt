//! ES 색인에 테스트 문서를 쓰고, 읽고, 지워 보는 점검 도구. 원본: `utils/update.py`.
//!
//! 운영 색인(`TM_ES_BOOK_INDEX`)에 실제로 쓰고 지우므로 주의한다. 인자 없이 실행하면 기본 문서로 점검한다.
//! 원본 동작과 호환하려면 `run`을 생략할 수 있다.

use std::io::Write;

use es_client::{BookStore, EsError, UpdateFields};
use pyfmt::py_repr;

/// 점검에 쓰는 문서 번호. 원본에 박혀 있던 값이다.
pub const DEFAULT_DOC_ID: i64 = 3384;

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = write!(
        out,
        "{program} - ES 색인에 테스트 문서를 쓰고 읽고 지워 보는 점검 도구.

문서 하나를 갱신하고(`_update`), 다시 읽어 출력하고, 삭제한다. 운영 색인에 실제로 쓰고 지우므로
문서 번호가 기존 문서와 겹치지 않는지 확인한 뒤 실행한다. 인자 없이 실행하면 기본 문서 번호로 점검한다.

사용법:
  {program} [run] [--doc-id N]

명령:
  run   점검을 실행한다. 생략해도 실행한다. 갱신 → 조회(출력) → 삭제 순서이고, 갱신이 실패하면 삭제하지 않는다.

옵션:
  --doc-id N   점검에 쓸 문서 번호 (기본 {DEFAULT_DOC_ID})
  -h, --help   이 도움말

환경변수 (모두 필요):
  TM_ES_URL, TM_ES_USER, TM_ES_PASSWORD, TM_ES_BOOK_INDEX

종료 코드: 0 정상, 1 요청 실패, 2 잘못된 인자, 255 갱신·삭제 실패 또는 환경변수 누락
"
    );
}

fn parse_doc_id(args: &[String]) -> Result<i64, String> {
    let mut doc_id = DEFAULT_DOC_ID;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let value = if a == "--doc-id" {
            it.next().cloned().ok_or("argument --doc-id: expected one argument")?
        } else if let Some(v) = a.strip_prefix("--doc-id=") {
            v.to_owned()
        } else {
            return Err(format!("unrecognized arguments: {a}"));
        };
        doc_id = value.parse().map_err(|_| format!("argument --doc-id: invalid int value: '{value}'"))?;
    }
    Ok(doc_id)
}

/// 진입점. 종료 코드를 돌려준다. 인자가 없으면 Python 스크립트처럼 기본 점검을 실행한다.
/// `connect`는 ES 접속을 만든다.
pub fn run<'a>(program: &str, args: &[String], out: &mut dyn Write, connect: &dyn Fn() -> Result<&'a dyn BookStore, EsError>) -> i32 {
    let command_offset = match args.first().map(String::as_str) {
        None => 0,
        Some("-h" | "--help") => {
            print_usage(program, out);
            return 0;
        }
        Some("run") => 1,
        Some("--doc-id") => 0,
        Some(arg) if arg.starts_with("--doc-id=") => 0,
        Some(other) => {
            eprintln!("{program}: error: argument command: invalid choice: '{other}' (choose from 'run')");
            return 2;
        }
    };
    let doc_id = match parse_doc_id(&args[command_offset..]) {
        Ok(id) => id,
        Err(msg) => {
            eprintln!("{program}: error: {msg}");
            return 2;
        }
    };

    let es = match connect() {
        Ok(es) => es,
        Err(e @ EsError::MissingEnv(_)) => {
            eprintln!("ERROR {e}");
            return 255;
        }
        Err(e) => {
            eprintln!("ERROR {e}");
            return 1;
        }
    };
    let fields = UpdateFields {
        category: "2_utf8".into(),
        title: "test title2".into(),
        author: "test author".into(),
        file_path: "/test/path".into(),
        file_type: "test_ext2".into(),
        file_size: 10_000_000,
        summary: "test content2".into(),
        updated_time: String::new(),
    };

    let outcome = (|| -> Result<i32, EsError> {
        if !es.update(doc_id, &fields)? {
            eprintln!("ERROR Failed to update");
            return Ok(255);
        }
        let book = es.search_by_id(doc_id)?;
        let _ = writeln!(out, "{}", py_repr(&book));
        if !es.delete(doc_id)? {
            eprintln!("ERROR Failed to delete");
            return Ok(255);
        }
        Ok(0)
    })();
    outcome.unwrap_or_else(|e| {
        eprintln!("ERROR {e}");
        1
    })
}
