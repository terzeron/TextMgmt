import { describe, expect, it, vi } from "vitest";

import { createCoverRequestQueue } from "../src/coverRequestQueue.js";

describe("createCoverRequestQueue", () => {
  it("keeps a cancelled running request's slot until the work settles", async () => {
    const queue = createCoverRequestQueue(1);
    const started = [];
    let finish;
    let signal;
    const cancel = queue.enqueue((requestSignal) => {
      signal = requestSignal;
      started.push("old");
      return new Promise((resolve) => { finish = resolve; });
    });
    queue.enqueue(() => started.push("new"));
    cancel();
    expect(signal.aborted).toBe(false);
    expect(started).toEqual(["old"]);
    finish();
    await vi.waitFor(() => expect(started).toEqual(["old", "new"]));
  });

  it("starts only the configured number of requests at once", async () => {
    const queue = createCoverRequestQueue(2);
    const started = [];
    const finish = [];
    Array.from({ length: 5 }, (_, index) =>
      queue.enqueue(() => {
        started.push(index);
        return new Promise((resolve) => finish.push(resolve));
      }),
    );

    expect(started).toEqual([0, 1]);
    finish.shift()();
    await vi.waitFor(() => expect(started).toEqual([0, 1, 2]));
    finish.shift()();
    await vi.waitFor(() => expect(started).toEqual([0, 1, 2, 3]));
    finish.shift()();
    finish.shift()();
    await vi.waitFor(() => expect(started).toEqual([0, 1, 2, 3, 4]));
    finish.shift()();
  });

  it("does not start a queued request after it is cancelled", async () => {
    const queue = createCoverRequestQueue(1);
    const started = [];
    let finish;
    queue.enqueue(() => new Promise((resolve) => { finish = resolve; }));
    const cancel = queue.enqueue(() => started.push("cancelled"));
    queue.enqueue(() => started.push("next"));

    cancel();
    finish();
    await vi.waitFor(() => expect(started).toEqual(["next"]));
  });
});
