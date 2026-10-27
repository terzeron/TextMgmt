//! 병렬 풀과 워커 수.

use std::path::PathBuf;

/// /mnt/data는 회전 디스크라 워커를 늘려도 I/O로 막힌다. 2개가 실측상 가장 빨랐다.
pub const DEFAULT_WORKERS: usize = 2;

pub fn pool(workers: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(workers.max(1)).build().expect("thread pool")
}

/// 앞에서 `limit`건만 남긴다. 0이면 전부.
pub fn limited(mut files: Vec<PathBuf>, limit: usize) -> Vec<PathBuf> {
    if limit > 0 {
        files.truncate(limit);
    }
    files
}

/// 워커 수를 메모리로 제한한다. 워커마다 최악의 입력 하나(`per_worker_mb`)를 동시에 해석할 수 있으므로,
/// 전체가 `budget_mb`를 넘지 않게 하되 하나는 항상 돈다. 스레드별 malloc 아레나는 한 번 늘린 메모리를 쥐고 있어서
/// 큰 작업이 돌아가며 지나가도 최대 상주 메모리가 줄지 않으므로, 동시에 도는 수 자체를 줄여야 한다.
/// 엄밀한 한도가 아니라 보수적인 동시성 상한이다.
pub fn workers_within_memory(workers: usize, per_worker_mb: u64, budget_mb: u64) -> usize {
    let by_memory = (budget_mb / per_worker_mb.max(1)).max(1);
    workers.max(1).min(usize::try_from(by_memory).unwrap_or(usize::MAX))
}
