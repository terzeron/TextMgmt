// 기본은 보이는 N개와 인접 ceil(N/4)개. ahead 지정 시 아래쪽 책을 그 수만큼 미리 검색한다.
export function selectViewportTargets(records, viewport, { ahead = 0 } = {}) {
  const distances = records.map((record) => {
    const clip = record.clip || viewport;
    const area = {
      top: Math.max(viewport.top, clip.top),
      bottom: Math.min(viewport.bottom, clip.bottom),
      left: Math.max(viewport.left, clip.left),
      right: Math.min(viewport.right, clip.right),
    };
    const rect = record.rect;
    const visible = rect.bottom > area.top && rect.top < area.bottom &&
      rect.right > area.left && rect.left < area.right &&
      rect.bottom > rect.top && rect.right > rect.left &&
      area.bottom > area.top && area.right > area.left;
    const distance = Math.max(0, area.top - rect.bottom, rect.top - area.bottom) +
      Math.max(0, area.left - rect.right, rect.left - area.right);
    return { record, visible, distance };
  });
  const visible = distances.filter((item) => item.visible);
  if (ahead) {
    const upcoming = distances.filter(({ record: { rect }, visible }) => !visible &&
      rect.top >= viewport.bottom && rect.bottom > rect.top && rect.right > rect.left &&
      rect.right > viewport.left && rect.left < viewport.right)
      .sort((a, b) => a.record.rect.top - b.record.rect.top).slice(0, ahead);
    return [...visible, ...upcoming].map((item) => ({ ...item.record, visible: item.visible }));
  }
  const extra = distances.filter((item) => !item.visible)
    .sort((a, b) => a.distance - b.distance).slice(0, Math.ceil(visible.length * 0.25));
  return [...visible, ...extra].map((item) => ({ ...item.record, visible: item.visible }));
}

// 모든 행의 검색과 이미지 요청을 한 queue에 넣는다. store별 동시 실행 수는 1이다.
export function createStoreQueue({ backgroundInterval = 5000 } = {}) {
  const active = new Map();
  const completed = new Set();
  let desiredKeys = new Set();
  let waiting = new Map();
  let disposed = false;
  const backgroundBooks = new Set();
  let nextBackgroundAt = Date.now() + backgroundInterval;
  let timer;

  function pump() {
    if (disposed) return;
    clearTimeout(timer);
    let deferred = false;
    const tasks = [...waiting.values()].sort((a, b) => Number(!!a.backgroundKey) - Number(!!b.backgroundKey));
    for (const task of tasks) {
      if (active.has(task.store)) continue;
      if (task.backgroundKey && !backgroundBooks.has(task.backgroundKey)) {
        if (Date.now() < nextBackgroundAt) { deferred = true; continue; }
        backgroundBooks.add(task.backgroundKey);
        nextBackgroundAt = Date.now() + backgroundInterval;
      }
      waiting.delete(task.key);
      const controller = new AbortController();
      active.set(task.store, { key: task.key, controller });
      Promise.resolve().then(() => task.run(controller.signal))
        .catch(() => {}) // 사용자에게 보여줄 오류는 task가 기록한다.
        .finally(() => {
          if (!controller.signal.aborted && desiredKeys.has(task.key)) completed.add(task.key);
          active.delete(task.store);
          pump();
        });
    }
    if (deferred) timer = setTimeout(pump, Math.max(1, nextBackgroundAt - Date.now()));
  }

  return {
    setTasks(tasks) {
      if (disposed) return;
      const desired = new Set(tasks.map((task) => task.key));
      desiredKeys = desired;
      const backgrounds = new Set(tasks.map((task) => task.backgroundKey));
      for (const key of backgroundBooks) if (!backgrounds.has(key)) backgroundBooks.delete(key);
      for (const key of completed) {
        if (!desired.has(key)) completed.delete(key);
      }
      for (const running of active.values()) {
        if (!desired.has(running.key)) running.controller.abort();
      }
      const runningKeys = new Set([...active.values()].filter((task) => !task.controller.signal.aborted).map((task) => task.key));
      waiting = new Map(tasks.filter((task) => !runningKeys.has(task.key) && !completed.has(task.key)).map((task) => [task.key, task]));
      pump();
    },
    dispose() {
      disposed = true;
      clearTimeout(timer);
      waiting.clear();
      for (const running of active.values()) running.controller.abort();
    },
  };
}
