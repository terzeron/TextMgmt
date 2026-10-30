//! 선읽기. 회전 디스크에서는 파일을 처음 읽는 시간(탐색·전송)이 검사 시간보다 길다. 검사하는 동안 다음 파일을 미리 읽어
//! 페이지 캐시에 올려 두면 디스크 대기와 검사가 겹친다. 읽은 내용은 버린다(캐시를 채우는 것이 목적이고 새 의존성도 없다).
//!
//! 소비자가 처리한 파일 수(`advance`)보다 `ahead_files`개 또는 `ahead_bytes`바이트 이상 앞서가지 않는다. 너무 앞서가면
//! 아직 쓰지 않은 캐시가 밀려나 두 번 읽게 된다. `max_file_bytes`보다 큰 파일은 건너뛴다. 큰 PDF는 어차피 캐시에 안 담기고 다른 파일을 밀어낸다.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// 소비자를 기다리다 깨어나는 최대 간격. 알림을 놓쳐도 이 안에 다시 확인한다.
const WAIT: Duration = Duration::from_millis(50);
const CHUNK: usize = 1024 * 1024;

struct Shared {
    done: AtomicUsize,
    prefetched: AtomicUsize,
    stop: AtomicBool,
    gate: Mutex<()>,
    wake: Condvar,
}

/// 다음 파일들을 미리 읽는 스레드. `drop`하면 스레드를 멈추고 기다린다.
pub struct Prefetcher {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

impl Prefetcher {
    /// `paths`를 순서대로 미리 읽는 스레드를 시작한다.
    pub fn start(paths: Vec<PathBuf>, ahead_files: usize, ahead_bytes: u64, max_file_bytes: u64) -> Prefetcher {
        let shared = Arc::new(Shared { done: AtomicUsize::new(0), prefetched: AtomicUsize::new(0), stop: AtomicBool::new(false), gate: Mutex::new(()), wake: Condvar::new() });
        let worker = Arc::clone(&shared);
        // 스레드를 못 만들면 선읽기만 없는 것이다. 검사 결과는 같다.
        let handle = std::thread::Builder::new().name("prefetch".into()).spawn(move || run(&worker, &paths, ahead_files, ahead_bytes, max_file_bytes)).ok();
        Prefetcher { shared, handle }
    }

    /// 소비자가 `done`개 파일을 처리했음을 알린다. 줄어드는 값은 무시한다.
    pub fn advance(&self, done: usize) {
        self.shared.done.fetch_max(done, Ordering::SeqCst);
        self.shared.wake.notify_all();
    }

    /// 지금까지 실제로 읽은 파일 수(건너뛴 파일은 세지 않는다).
    pub fn prefetched(&self) -> usize {
        self.shared.prefetched.load(Ordering::SeqCst)
    }
}

impl Drop for Prefetcher {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn wait(shared: &Shared) {
    if let Ok(guard) = shared.gate.lock() {
        let _ = shared.wake.wait_timeout(guard, WAIT);
    }
}

/// 파일 하나를 끝까지 읽어 버린다. 멈춤 요청이 오면 중간에 그만둔다.
fn read_through(path: &PathBuf, buf: &mut [u8], stop: &AtomicBool) {
    let Ok(mut file) = File::open(path) else { return };
    while !stop.load(Ordering::Relaxed) {
        match file.read(buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn run(shared: &Shared, paths: &[PathBuf], ahead_files: usize, ahead_bytes: u64, max_file_bytes: u64) {
    let mut next = 0usize;
    // 읽은 파일의 (인덱스, 크기). 소비자가 지나간 파일은 앞에서 빼 "앞서간 바이트"에서 제외한다.
    let mut inflight: VecDeque<(usize, u64)> = VecDeque::new();
    let mut buf = vec![0u8; CHUNK];
    while !shared.stop.load(Ordering::Relaxed) && next < paths.len() {
        let done = shared.done.load(Ordering::SeqCst);
        while inflight.front().is_some_and(|(i, _)| *i < done) {
            inflight.pop_front();
        }
        if next >= done.saturating_add(ahead_files) {
            wait(shared);
            continue;
        }
        // 일반 파일만 읽는다. FIFO·장치는 열기만 해도 멈출 수 있고 디렉터리는 읽을 것이 없다.
        let size = match fs::metadata(&paths[next]) {
            Ok(m) if m.is_file() => m.len(),
            _ => {
                next += 1;
                continue;
            }
        };
        if size > max_file_bytes {
            next += 1;
            continue;
        }
        let flight: u64 = inflight.iter().map(|(_, s)| *s).sum();
        // 아무것도 읽어 두지 않았다면 한도보다 큰 파일도 읽는다(진행 보장).
        if flight > 0 && flight.saturating_add(size) > ahead_bytes {
            wait(shared);
            continue;
        }
        read_through(&paths[next], &mut buf, &shared.stop);
        inflight.push_back((next, size));
        shared.prefetched.fetch_add(1, Ordering::SeqCst);
        next += 1;
    }
}
