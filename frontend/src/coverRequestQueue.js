export const COVER_REQUEST_CONCURRENCY = 10;

export function createCoverRequestQueue(limit = COVER_REQUEST_CONCURRENCY) {
  const pending = [];
  let activeCount = 0;

  function finish(task) {
    if (task.finished) return;
    task.finished = true;
    if (task.started) activeCount -= 1;
    pump();
  }

  function pump() {
    while (activeCount < limit && pending.length > 0) {
      const task = pending.shift();
      if (task.cancelled) continue;
      task.started = true;
      activeCount += 1;
      try {
        Promise.resolve(task.run(task.controller.signal))
          .catch(() => {})
          .finally(() => finish(task));
      } catch {
        finish(task);
      }
    }
  }

  return {
    enqueue(run) {
      const task = {
        run,
        controller: new AbortController(),
        started: false,
        cancelled: false,
        finished: false,
      };
      pending.push(task);
      pump();

      return () => {
        if (task.finished) return;
        task.cancelled = true;
        // 이미 보낸 요청은 응답까지 슬롯을 유지한다. 브라우저 취소는 서버의
        // PDF/EPUB 추출을 멈추지 않으므로 여기서 슬롯을 반환하면 작업이 겹친다.
        if (task.started) return;
        task.controller.abort();
        finish(task);
      };
    },
  };
}

export const coverRequestQueue = createCoverRequestQueue();
