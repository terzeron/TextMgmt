import { describe, it, expect, vi } from "vitest";
import { createStoreQueue, selectViewportTargets } from "../src/coverFetchQueue";

describe("표지 fetch 제한", () => {
  it("미리 받기를 시작했던 책도 목록을 벗어났다가 돌아오면 다음 5초 기한을 지킨다", async () => {
    vi.useFakeTimers();
    const queue = createStoreQueue();
    try {
      const search = vi.fn().mockResolvedValue(undefined);
      const image = vi.fn().mockResolvedValue(undefined);
      queue.setTasks([{ key: "search", store: "yes24", backgroundKey: "book", run: search }]);
      await vi.advanceTimersByTimeAsync(5000);
      expect(search).toHaveBeenCalledOnce();
      queue.setTasks([]);
      queue.setTasks([{ key: "image", store: "yes24", backgroundKey: "book", run: image }]);
      await vi.advanceTimersByTimeAsync(4999);
      expect(image).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(image).toHaveBeenCalledOnce();
    } finally { queue.dispose(); vi.useRealTimers(); }
  });

  it("미리 받기 대기 시간이 지나도 같은 서점의 보이는 책을 먼저 처리한다", async () => {
    vi.useFakeTimers();
    const queue = createStoreQueue();
    try {
      const started = [];
      let finish;
      await vi.advanceTimersByTimeAsync(5000);
      queue.setTasks([
        { key: "background", store: "yes24", backgroundKey: "book", run: async () => { started.push("background"); } },
        { key: "visible", store: "yes24", run: () => new Promise((resolve) => { started.push("visible"); finish = resolve; }) },
        { key: "parallel", store: "aladin", run: async () => { started.push("parallel"); } },
      ]);
      await vi.advanceTimersByTimeAsync(0);
      expect(started).toEqual(["visible", "parallel"]);
      finish();
      await vi.advanceTimersByTimeAsync(0);
      expect(started).toEqual(["visible", "parallel", "background"]);
    } finally { queue.dispose(); vi.useRealTimers(); }
  });

  it("React가 반복 계산해도 미리 받기의 5초 기한을 연장하지 않는다", async () => {
    vi.useFakeTimers();
    const queue = createStoreQueue();
    try {
      const run = vi.fn().mockResolvedValue(undefined);
      const tasks = [{ key: "book", store: "aladin", backgroundKey: "book", run }];
      queue.setTasks(tasks);
      for (let count = 0; count < 4; count += 1) {
        await vi.advanceTimersByTimeAsync(1000);
        queue.setTasks(tasks);
        expect(run).not.toHaveBeenCalled();
      }
      await vi.advanceTimersByTimeAsync(1000);
      expect(run).toHaveBeenCalledOnce();
    } finally { queue.dispose(); vi.useRealTimers(); }
  });

  it("미리 받기 대상이 사라지면 timer를 취소하고 새 책만 처리한다", async () => {
    vi.useFakeTimers();
    const queue = createStoreQueue();
    try {
      const discarded = vi.fn();
      const next = vi.fn();
      queue.setTasks([{ key: "old", store: "yes24", backgroundKey: "old", run: discarded }]);
      await vi.advanceTimersByTimeAsync(2000);
      queue.setTasks([]);
      await vi.advanceTimersByTimeAsync(10000);
      expect(discarded).not.toHaveBeenCalled();
      queue.setTasks([{ key: "new", store: "yes24", backgroundKey: "new", run: next }]);
      await vi.advanceTimersByTimeAsync(0);
      expect(next).toHaveBeenCalledOnce();
    } finally { queue.dispose(); vi.useRealTimers(); }
  });

  it("이미 시작한 인접 책의 이미지도 서점별 직렬 처리를 유지한다", async () => {
    vi.useFakeTimers();
    const queue = createStoreQueue();
    try {
      const started = [];
      let finish;
      const tasks = [
        { key: "search", store: "yes24", backgroundKey: "book", run: () => new Promise((resolve) => { started.push("search"); finish = resolve; }) },
        { key: "image", store: "yes24", backgroundKey: "book", run: async () => { started.push("image"); } },
        { key: "other-book", store: "kyobo", backgroundKey: "next", run: async () => { started.push("next"); } },
      ];
      queue.setTasks(tasks);
      await vi.advanceTimersByTimeAsync(5000);
      expect(started).toEqual(["search"]);
      queue.setTasks(tasks);
      finish();
      await vi.advanceTimersByTimeAsync(0);
      expect(started).toEqual(["search", "image"]);
      await vi.advanceTimersByTimeAsync(4999);
      expect(started).toEqual(["search", "image"]);
      await vi.advanceTimersByTimeAsync(1);
      expect(started).toEqual(["search", "image", "next"]);
    } finally { queue.dispose(); vi.useRealTimers(); }
  });

  it.each([
    { top: 10, bottom: 10, left: 0, right: 100 },
    { top: 0, bottom: 100, left: 10, right: 10 },
    { top: 400, bottom: 500, left: 0, right: 100 },
  ])("크기가 없거나 화면 밖 후보만 있으면 fetch하지 않는다: %j", (rect) => {
    expect(selectViewportTargets([{ key: "hidden", rect }], { top: 0, bottom: 400, left: 0, right: 100 })).toEqual([]);
  });

  it("후보 영역 전체가 화면 밖이면 이미지를 요청하지 않는다", () => {
    const rect = { top: 0, bottom: 100, left: 0, right: 100 };
    expect(selectViewportTargets([{ key: "hidden", rect, clip: { ...rect, top: 500, bottom: 600 } }], rect)).toEqual([]);
  });
  it("보이는 책은 즉시 실행하고 보이지 않는 책은 5초 간격으로 시작한다", async () => {
    vi.useFakeTimers();
    const queue = createStoreQueue();
    try {
      const foreground = vi.fn().mockResolvedValue(undefined);
      const first = vi.fn().mockResolvedValue(undefined);
      const otherStore = vi.fn().mockResolvedValue(undefined);
      const second = vi.fn().mockResolvedValue(undefined);
      queue.setTasks([
        { key: "visible", store: "yes24", run: foreground },
        { key: "book1-yes", store: "yes24", backgroundKey: "book1", run: first },
        { key: "book1-ala", store: "aladin", backgroundKey: "book1", run: otherStore },
        { key: "book2", store: "kyobo", backgroundKey: "book2", run: second },
      ]);
      await vi.advanceTimersByTimeAsync(0);
      expect(foreground).toHaveBeenCalledOnce();
      expect(first).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(4999);
      expect(first).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(first).toHaveBeenCalledOnce();
      expect(otherStore).toHaveBeenCalledOnce();
      expect(second).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(5000);
      expect(second).toHaveBeenCalledOnce();
    } finally { queue.dispose(); vi.useRealTimers(); }
  });

  it("미리 받던 책이 화면에 들어오면 5초 대기를 건너뛰고 폐기 시 timer를 취소한다", async () => {
    vi.useFakeTimers();
    const queue = createStoreQueue();
    try {
      const run = vi.fn().mockResolvedValue(undefined);
      const task = { key: "book", store: "aladin", backgroundKey: "book", run };
      queue.setTasks([task]);
      await vi.advanceTimersByTimeAsync(1000);
      expect(run).not.toHaveBeenCalled();
      queue.setTasks([{ ...task, backgroundKey: undefined }]);
      await vi.advanceTimersByTimeAsync(0);
      expect(run).toHaveBeenCalledOnce();
      queue.setTasks([{ ...task, key: "other", backgroundKey: "other" }]);
      queue.dispose();
      await vi.advanceTimersByTimeAsync(10000);
      expect(run).toHaveBeenCalledOnce();
    } finally { queue.dispose(); vi.useRealTimers(); }
  });
  it("취소 직후 같은 후보를 다시 요청하면 새 대기 작업을 유지한다", async () => {
    const queue = createStoreQueue();
    let finish;
    let firstSignal;
    const old = { key: "same", store: "yes24", run: (signal) => { firstSignal = signal; return new Promise((resolve) => { finish = resolve; }); } };
    const retry = vi.fn().mockResolvedValue(undefined);
    queue.setTasks([old]);
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    queue.setTasks([]);
    queue.setTasks([{ key: "same", store: "yes24", run: retry }]);
    expect(firstSignal.aborted).toBe(true);
    finish();
    await vi.waitFor(() => expect(retry).toHaveBeenCalledOnce());
    queue.dispose();
  });

  it("폐기한 queue는 요청을 재실행하지 않고 task 오류를 다음 요청과 격리한다", async () => {
    const queue = createStoreQueue();
    const run = vi.fn().mockRejectedValue(new Error("download"));
    const next = vi.fn().mockResolvedValue(undefined);
    queue.setTasks([{ key: "error", store: "yes24", run }, { key: "next", store: "yes24", run: next }]);
    await vi.waitFor(() => expect(next).toHaveBeenCalledOnce());
    queue.dispose();
    queue.setTasks([{ key: "again", store: "aladin", run }]);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(run).toHaveBeenCalledOnce();
  });

  it("보이는 4개와 인접 후보 1개만 선택한다", () => {
    const records = Array.from({ length: 12 }, (_, index) => ({
      key: String(index), rect: { top: index * 100, bottom: index * 100 + 100, left: 0, right: 100 },
    }));
    const selected = selectViewportTargets(records, { top: 0, bottom: 400, left: 0, right: 100 });
    expect(selected.map((item) => item.key)).toEqual(["0", "1", "2", "3", "4"]);
    expect(selected.map((item) => item.visible)).toEqual([true, true, true, true, false]);
    expect(selectViewportTargets(records, { top: -100, bottom: -1, left: 0, right: 100 })).toEqual([]);
  });

  it("가로로 잘린 후보도 노출량 계산에서 제외한다", () => {
    const records = Array.from({ length: 10 }, (_, index) => ({
      key: String(index), rect: { top: 0, bottom: 100, left: index * 100, right: index * 100 + 100 },
      clip: { left: 0, right: 400, top: 0, bottom: 100 },
    }));
    expect(selectViewportTargets(records, { top: 0, bottom: 100, left: 0, right: 1000 }).map((item) => item.key)).toEqual(["0", "1", "2", "3", "4"]);
  });

  it("같은 서점은 직렬이고 다른 서점은 병렬이다", async () => {
    const queue = createStoreQueue();
    const release = [];
    const started = [];
    const task = (key, store) => ({ key, store, run: () => new Promise((resolve) => { started.push(key); release.push(resolve); }) });
    const tasks = [task("yes1", "yes24"), task("yes2", "yes24"), task("ala1", "aladin")];
    queue.setTasks(tasks);
    await vi.waitFor(() => expect(started).toEqual(["yes1", "ala1"]));
    release[0]();
    await vi.waitFor(() => expect(started).toEqual(["yes1", "ala1", "yes2"]));
    release.slice(1).forEach((resolve) => resolve());
    queue.dispose();
  });

  it("노출 대상에서 벗어난 대기 요청을 실행하지 않는다", async () => {
    const queue = createStoreQueue();
    let release;
    const first = vi.fn(() => new Promise((resolve) => { release = resolve; }));
    const second = vi.fn();
    queue.setTasks([{ key: "1", store: "yes24", run: first }, { key: "2", store: "yes24", run: second }]);
    await vi.waitFor(() => expect(first).toHaveBeenCalledOnce());
    queue.setTasks([]);
    release();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(second).not.toHaveBeenCalled();
    queue.dispose();
  });

  it("응답 후 React 상태 반영 전의 재계산도 같은 이미지를 중복 요청하지 않는다", async () => {
    const queue = createStoreQueue();
    const run = vi.fn(async () => {});
    const tasks = [{ key: "image:1", store: "yes24", run }];
    queue.setTasks(tasks);
    await vi.waitFor(() => expect(run).toHaveBeenCalledOnce());
    await new Promise((resolve) => setTimeout(resolve, 0));
    queue.setTasks(tasks);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(run).toHaveBeenCalledOnce();
    queue.setTasks([]);
    queue.setTasks(tasks);
    await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
    queue.dispose();
  });
});
