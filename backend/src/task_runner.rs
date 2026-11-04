//! 백그라운드 작업 실행기. 라우트와 도메인 모듈이 함께 쓴다.

use std::thread::JoinHandle;

// ---------------------------------------------------------------------------
// 백그라운드 작업
// ---------------------------------------------------------------------------

/// `TaskRunner::spawn`이 돌려주는 핸들.
pub trait TaskHandle: Send {
    fn is_finished(&self) -> bool {
        true
    }

    fn join(self: Box<Self>) {}
}

struct ThreadHandle(JoinHandle<()>);

impl TaskHandle for ThreadHandle {
    fn is_finished(&self) -> bool {
        self.0.is_finished()
    }

    fn join(self: Box<Self>) {
        let _ = self.0.join();
    }
}

struct DoneHandle;

impl TaskHandle for DoneHandle {}

fn task_handle_from_spawn(name: &str, result: std::io::Result<JoinHandle<()>>) -> Box<dyn TaskHandle> {
    match result {
        Ok(handle) => Box::new(ThreadHandle(handle)),
        Err(error) => {
            log::error!("failed to start task {name}: {error}");
            Box::new(DoneHandle)
        }
    }
}

/// 백그라운드 작업 실행기. 응답 뒤 작업과 블로킹 풀 실행을 맡는다.
pub trait TaskRunner: Send + Sync {
    /// 기본 구현은 새 스레드에서 실행한다.
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        task_handle_from_spawn(name, std::thread::Builder::new().name(name.to_string()).spawn(job))
    }

    /// 블로킹 풀로 넘어간 함수 이름을 알린다(테스트 관측용). 기본 구현은 아무것도 하지 않는다.
    fn spawn_blocking_observed(&self, _name: &str) {}
}

/// 실제 실행기: 작업마다 스레드를 띄운다.
pub struct ThreadRunner;

impl TaskRunner for ThreadRunner {}

/// 응답을 돌려주기 전에 같은 스레드에서 작업을 끝낸다(테스트용). 스레드 로컬 로그 수집에 필요하다.
pub struct InlineRunner;

impl TaskRunner for InlineRunner {
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        job();
        Box::new(DoneHandle)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct EmptyTaskHandle;
    struct EmptyTaskRunner;

    impl TaskHandle for EmptyTaskHandle {}
    impl TaskRunner for EmptyTaskRunner {}

    #[test]
    fn task_handle_and_runners_cover_inline_and_thread_execution() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let handle: Box<dyn TaskHandle> = Box::new(EmptyTaskHandle);
        assert!(handle.is_finished());
        handle.join();

        let completed = Arc::new(AtomicBool::new(false));
        let thread_completed = completed.clone();
        let handle = ThreadRunner.spawn("traits-test", Box::new(move || thread_completed.store(true, Ordering::SeqCst)));
        handle.join();
        assert!(completed.load(Ordering::SeqCst));

        let inline_completed = completed.clone();
        InlineRunner.spawn("inline-test", Box::new(move || inline_completed.store(true, Ordering::SeqCst))).join();
        assert!(completed.load(Ordering::SeqCst));
        EmptyTaskRunner.spawn_blocking_observed("blocking-test");
        let failed = task_handle_from_spawn("failed-task", Err(std::io::Error::other("thread limit")));
        assert!(failed.is_finished());
        failed.join();
    }
}
