fn main() -> std::process::ExitCode {
    // lopdf는 1MB 넘는 입력을 rayon 풀에서 파싱한다. 스레드마다 glibc가 malloc arena를 예약해
    // CPU가 많으면 가상 주소 공간이 worker 상한(prlimit --as=512MB)을 넘고 할당이 실패한다(16코어 실측).
    // 요청 하나를 처리하는 단명 프로세스라 병렬 파싱 이득도 작다.
    // SAFETY: 다른 스레드가 생기기 전, main 첫 줄에서 한 번만 쓴다.
    unsafe { std::env::set_var("RAYON_NUM_THREADS", "1") };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    match pdf_engine::pdf_worker_client::run_worker(&args, &mut out) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::from(1)
        }
    }
}
