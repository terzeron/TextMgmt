//! 백엔드 서버 진입점. 인자 없이 실행하면 용도와 사용법을 출력한다.

fn print_usage() {
    println!("backend: TextMgmt 백엔드 API 서버");
    println!();
    println!("사용법: backend serve [--addr HOST:PORT]");
    println!();
    println!("옵션:");
    println!("  --addr HOST:PORT  수신 주소 (기본 0.0.0.0:8000)");
    println!("  -h, --help        이 도움말");
    println!();
    println!("종료 코드: 0 정상, 2 잘못된 사용, 255 실행 오류");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return;
    }
    eprintln!("서버 실행은 아직 구현되지 않았다");
    std::process::exit(255);
}
