//! 색인용 파일 읽기 워커. backend·loader가 파서를 자기 프로세스 밖에서 돌리려고 띄운다.
//! 파서가 죽거나 시간을 넘겨도 부모는 살아남는다. 인자 없이 실행하면 사용법을 출력한다.

use std::io::Write;
use std::process::ExitCode;

use file_indexer::child_process_runner::{PDF_STAGE_COMMAND, READ_FILE_COMMAND, run_pdf_stage_command, run_read_file_command};

fn print_usage() {
    println!("tm-index-worker: 색인용 파일 읽기 워커(backend·loader가 자식 프로세스로 띄운다)");
    println!();
    println!("사용법:");
    println!("  tm-index-worker {PDF_STAGE_COMMAND} <단계> <PDF 경로>   PDF 단계 하나를 실행하고 결과 JSON을 출력한다");
    println!("  tm-index-worker {READ_FILE_COMMAND} <책 루트> <만화 루트> <경로> <prefix 유무> <prefix> <본문 건너뜀>");
    println!("                                              파일 하나를 읽어 색인 레코드 JSON을 출력한다");
    println!();
    println!("종료 코드: 0 정상, 2 잘못된 사용");
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    let code = match args.first().map(String::as_str) {
        Some(command) if command == PDF_STAGE_COMMAND => run_pdf_stage_command(&args[1..], &mut out),
        Some(command) if command == READ_FILE_COMMAND => run_read_file_command(&args[1..], &mut out),
        None => {
            print_usage();
            0
        }
        Some(_) => {
            print_usage();
            2
        }
    };
    let _ = out.flush();
    ExitCode::from(code as u8)
}
