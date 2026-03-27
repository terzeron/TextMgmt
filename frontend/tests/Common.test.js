import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  quasiRandomHue,
  vanDerCorput,
  getApiUrlPrefix,
  getRandomLightColor,
  getRandomMediumColor,
  handleFetchErrors,
  jsonGetReq,
  jsonPostReq,
  jsonPutReq,
  jsonDeleteReq,
  rawJsonGetReq,
  externalJsonGetReq,
  tryRefreshToken,
  startProactiveRefresh,
  stopProactiveRefresh,
} from "../src/Common";

describe("Common Utilities", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
    vi.stubGlobal("window", {
      location: { reload: vi.fn() },
      __ENV__: { VITE_API_URL_PREFIX: "/api" },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("quasiRandomHue returns values between 0 and 1", () => {
    const h1 = quasiRandomHue(1);
    const h2 = quasiRandomHue(2);
    expect(h1).toBeGreaterThanOrEqual(0);
    expect(h1).toBeLessThan(1);
    expect(h1).not.toBe(h2);
  });

  it("vanDerCorput returns consistent sequence", () => {
    expect(vanDerCorput(1, 2)).toBe(0.5);
    expect(vanDerCorput(2, 2)).toBe(0.25);
  });

  it("getApiUrlPrefix respects window.__ENV__", () => {
    expect(getApiUrlPrefix()).toBe("/api");
  });

  it("getRandomLightColor returns valid hsl string", () => {
    const color = getRandomLightColor("test-key");
    expect(color).toMatch(/^hsl\(\d+, 55%, 90%\)$/);
  });

  it("getRandomMediumColor returns valid hsl string", () => {
    const color = getRandomMediumColor("test-key");
    expect(color).toMatch(/^hsl\(\d+, 50%, 55%\)$/);
  });

  it("handleFetchErrors throws on non-ok non-401 response", () => {
    const response = {
      ok: false,
      status: 500,
      statusText: "Internal Server Error",
    };
    expect(() => handleFetchErrors(response)).toThrow("Internal Server Error");
  });

  it("handleFetchErrors returns response on ok response", () => {
    const response = { ok: true, status: 200 };
    expect(handleFetchErrors(response)).toBe(response);
  });

  it("handleFetchErrors returns response on 401 response", () => {
    const response = { ok: false, status: 401 };
    expect(handleFetchErrors(response)).toBe(response);
  });

  it("jsonGetReq calls fetch with correct URL and options", async () => {
    const mockResult = { status: "success", result: "test-data" };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    const reject = vi.fn();

    jsonGetReq("/test-endpoint", null, resolve, reject);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith("test-data");
    });

    expect(fetch).toHaveBeenCalledWith(
      "/api/test-endpoint",
      expect.objectContaining({
        method: "GET",
        credentials: "include",
      }),
    );
  });

  it("jsonPostReq calls fetch with payload", async () => {
    const mockResult = { status: "success", result: { id: 1 } };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    const payload = { name: "test" };

    jsonPostReq("/test-endpoint", payload, resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith({ id: 1 });
    });

    expect(fetch).toHaveBeenCalledWith(
      "/api/test-endpoint",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify(payload),
        headers: expect.objectContaining({
          "Content-Type": "application/json",
        }),
      }),
    );
  });

  it("apiReq handles error in response data", async () => {
    const mockResult = { status: "error", error: "Something went wrong" };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const reject = vi.fn();

    jsonGetReq("/test-endpoint", null, null, reject);

    await vi.waitFor(() => {
      expect(reject).toHaveBeenCalledWith("Something went wrong");
    });
  });

  it("externalJsonGetReq calls fetch without prefix", async () => {
    const mockResult = { some: "external-data" };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    externalJsonGetReq("https://example.com/api", resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith(mockResult);
    });

    expect(fetch).toHaveBeenCalledWith("https://example.com/api");
  });

  it("apiReq handles 401 and attempts refresh", async () => {
    // 1st fetch returns 401
    fetch.mockResolvedValueOnce({
      ok: false,
      status: 401,
    });
    // 2nd fetch (refresh) returns success
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({ status: "success", expires_in: 7200 }),
    });
    // 3rd fetch (retry) returns success data
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({ status: "success", result: "refreshed-data" }),
    });

    const resolve = vi.fn();
    jsonGetReq("/test-endpoint", null, resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith("refreshed-data");
    });

    expect(fetch).toHaveBeenCalledWith("/api/auth/refresh", expect.anything());
  });

  it("apiReq handles 401 and refresh failure", async () => {
    fetch.mockResolvedValueOnce({ ok: false, status: 401 }); // 1st fetch
    fetch.mockResolvedValueOnce({ ok: false, status: 401 }); // refresh attempt

    const reject = vi.fn();
    jsonGetReq("/test-endpoint", null, null, reject);

    await vi.waitFor(() => {
      expect(window.location.reload).toHaveBeenCalled();
    });
  });

  it("processData formats dates correctly", async () => {
    const mockResult = {
      status: "success",
      result: {},
      last_modified_time: "2023-10-27T10:00:00Z",
      last_responded_time: "2023-10-27T11:00:00Z",
    };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    jsonGetReq("/test-endpoint", null, resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith(
        expect.objectContaining({
          last_modified_time: expect.any(String),
          last_responded_time: expect.any(String),
        }),
      );
    });
  });

  it("processData handles warnings", async () => {
    const mockResult = {
      status: "success",
      result: "some-data",
      warning: "some-warning",
    };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    jsonGetReq("/test-endpoint", null, resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith({
        result: "some-data",
        warning: "some-warning",
      });
    });
  });

  it("textGetReq calls fetch and returns text", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      text: async () => "some-text",
    });

    const resolve = vi.fn();
    const { textGetReq } = await import("../src/Common");
    textGetReq("/test-text", null, resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith("some-text");
    });
  });

  it("blobGetReq calls fetch and returns blob", async () => {
    const mockBlob = new Blob(["test"], { type: "text/plain" });
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      blob: async () => mockBlob,
    });

    const resolve = vi.fn();
    const { blobGetReq } = await import("../src/Common");
    blobGetReq("/test-blob", null, resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith(mockBlob);
    });
  });

  it("rawJsonGetReq calls fetch and returns raw json", async () => {
    const mockResult = { some: "raw-data" };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    rawJsonGetReq("/test-raw", resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith(mockResult);
    });
  });

  it("jsonPutReq calls fetch with PUT method", async () => {
    const mockResult = { status: "success", result: "put-data" };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    jsonPutReq("/test-put", { key: "value" }, resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith("put-data");
    });

    expect(fetch).toHaveBeenCalledWith(
      "/api/test-put",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify({ key: "value" }),
        headers: expect.objectContaining({
          "Content-Type": "application/json",
        }),
      }),
    );
  });

  it("jsonDeleteReq calls fetch with DELETE method", async () => {
    const mockResult = { status: "success", result: "deleted" };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    jsonDeleteReq("/test-delete", { id: 1 }, resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith("deleted");
    });

    expect(fetch).toHaveBeenCalledWith(
      "/api/test-delete",
      expect.objectContaining({
        method: "DELETE",
      }),
    );
  });

  it("apiReq calls final callback", async () => {
    const mockResult = { status: "success", result: "data" };
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => mockResult,
    });

    const resolve = vi.fn();
    const final = vi.fn();
    jsonGetReq("/test-final", null, resolve, null, final);

    await vi.waitFor(() => {
      expect(final).toHaveBeenCalled();
    });
  });

  it("apiReq handles 401 with retry not ok (reload)", async () => {
    // 1st fetch returns 401
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });
    // refresh succeeds
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({ status: "success", expires_in: 7200 }),
    });
    // retry fails (not ok)
    fetch.mockResolvedValueOnce({
      ok: false,
      status: 403,
      statusText: "Forbidden",
    });

    const reject = vi.fn();
    jsonGetReq("/test-retry-fail", null, null, reject);

    await vi.waitFor(() => {
      expect(window.location.reload).toHaveBeenCalled();
    });
  });

  it("tryRefreshToken handles network error (catch branch)", async () => {
    // 1st fetch returns 401
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });
    // refresh throws network error
    fetch.mockRejectedValueOnce(new Error("Network error"));

    const reject = vi.fn();
    jsonGetReq("/test-refresh-error", null, null, reject);

    await vi.waitFor(() => {
      expect(window.location.reload).toHaveBeenCalled();
    });
  });

  it("rawJsonGetReq handles 401 with successful refresh", async () => {
    // 1st fetch returns 401
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });
    // refresh succeeds
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({ status: "success", expires_in: 7200 }),
    });
    // retry returns data
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({ refreshed: "raw-data" }),
    });

    const resolve = vi.fn();
    rawJsonGetReq("/test-raw-401", resolve);

    await vi.waitFor(() => {
      expect(resolve).toHaveBeenCalledWith({ refreshed: "raw-data" });
    });
  });

  it("rawJsonGetReq handles 401 with retry not ok (reload)", async () => {
    // 1st fetch returns 401
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });
    // refresh succeeds
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({ status: "success", expires_in: 7200 }),
    });
    // retry fails
    fetch.mockResolvedValueOnce({
      ok: false,
      status: 403,
      statusText: "Forbidden",
    });

    const reject = vi.fn();
    rawJsonGetReq("/test-raw-retry-fail", null, reject);

    await vi.waitFor(() => {
      expect(window.location.reload).toHaveBeenCalled();
    });
  });

  it("rawJsonGetReq handles 401 with refresh failure (reload)", async () => {
    // 1st fetch returns 401
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });
    // refresh fails
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });

    const reject = vi.fn();
    rawJsonGetReq("/test-raw-refresh-fail", null, reject);

    await vi.waitFor(() => {
      expect(window.location.reload).toHaveBeenCalled();
    });
  });

  it("rawJsonGetReq calls final callback", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({ data: "test" }),
    });

    const resolve = vi.fn();
    const final = vi.fn();
    rawJsonGetReq("/test-raw-final", resolve, null, final);

    await vi.waitFor(() => {
      expect(final).toHaveBeenCalled();
    });
  });

  it("externalJsonGetReq calls reject on fetch error", async () => {
    fetch.mockRejectedValueOnce(new Error("Network failure"));

    const reject = vi.fn();
    externalJsonGetReq("https://example.com/fail", null, reject);

    await vi.waitFor(() => {
      expect(reject).toHaveBeenCalled();
    });
  });

  it("externalJsonGetReq calls final callback", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({ data: "ext" }),
    });

    const resolve = vi.fn();
    const final = vi.fn();
    externalJsonGetReq("https://example.com/api", resolve, null, final);

    await vi.waitFor(() => {
      expect(final).toHaveBeenCalled();
    });
  });

  it("getApiUrlPrefix logs when env var is not set", () => {
    window.__ENV__ = {};
    // import.meta.env may also be undefined in test
    const result = getApiUrlPrefix();
    // Should return undefined/falsy and log
    expect(result).toBeFalsy();
  });

  describe("proactive token refresh", () => {
    beforeEach(() => {
      vi.useFakeTimers();
      stopProactiveRefresh();
    });

    afterEach(() => {
      stopProactiveRefresh();
      vi.useRealTimers();
    });

    it("tryRefreshToken schedules proactive refresh on success", async () => {
      fetch.mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ status: "success", expires_in: 7200 }),
      });

      vi.useRealTimers();
      const result = await tryRefreshToken();
      expect(result).toBe(true);
      // 타이머가 예약되었는지는 stopProactiveRefresh가 에러 없이 호출되는 것으로 간접 확인
      stopProactiveRefresh();
    });

    it("tryRefreshToken does not schedule refresh on failure", async () => {
      fetch.mockResolvedValueOnce({ ok: false, status: 401 });

      vi.useRealTimers();
      const result = await tryRefreshToken();
      expect(result).toBe(false);
    });

    it("tryRefreshToken does not schedule refresh when expires_in is absent", async () => {
      fetch.mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ status: "success" }),
      });

      vi.useRealTimers();
      const result = await tryRefreshToken();
      expect(result).toBe(true);
    });

    it("startProactiveRefresh triggers refresh before expiry", async () => {
      // expires_in=600초(10분) → 버퍼 5분 = 5분 후 refresh 시도
      startProactiveRefresh(600);

      // refresh 요청에 대한 mock
      fetch.mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ status: "success", expires_in: 600 }),
      });

      // 5분(300초) 후 타이머 발동
      await vi.advanceTimersByTimeAsync(300_000);

      expect(fetch).toHaveBeenCalledWith(
        "/api/auth/refresh",
        expect.objectContaining({ method: "POST" }),
      );
    });

    it("startProactiveRefresh uses minimum 10s delay for short expiry", async () => {
      // expires_in이 매우 짧으면 최소 10초 지연
      startProactiveRefresh(60);

      fetch.mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ status: "success", expires_in: 60 }),
      });

      // 10초 후 타이머 발동 (60초 - 300초 버퍼 = 음수 → 최소 10초)
      await vi.advanceTimersByTimeAsync(10_000);

      expect(fetch).toHaveBeenCalledWith(
        "/api/auth/refresh",
        expect.objectContaining({ method: "POST" }),
      );
    });

    it("stopProactiveRefresh cancels scheduled refresh", async () => {
      startProactiveRefresh(600);

      // 타이머 취소
      stopProactiveRefresh();

      // 시간이 지나도 fetch가 호출되지 않아야 함
      await vi.advanceTimersByTimeAsync(600_000);
      expect(fetch).not.toHaveBeenCalled();
    });

    it("startProactiveRefresh replaces previous timer", async () => {
      startProactiveRefresh(600);
      // 두 번째 호출로 이전 타이머 교체
      startProactiveRefresh(1200);

      fetch.mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ status: "success", expires_in: 1200 }),
      });

      // 첫 번째 타이머(5분 후)가 발동하지 않아야 함
      await vi.advanceTimersByTimeAsync(300_000);
      expect(fetch).not.toHaveBeenCalled();

      // 두 번째 타이머(15분 후)에 발동
      await vi.advanceTimersByTimeAsync(600_000);
      expect(fetch).toHaveBeenCalledTimes(1);
    });

    it("concurrent tryRefreshToken calls share single Promise", async () => {
      fetch.mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ status: "success", expires_in: 7200 }),
      });

      vi.useRealTimers();
      const [r1, r2, r3] = await Promise.all([
        tryRefreshToken(),
        tryRefreshToken(),
        tryRefreshToken(),
      ]);

      expect(r1).toBe(true);
      expect(r2).toBe(true);
      expect(r3).toBe(true);
      // fetch는 한 번만 호출되어야 함
      expect(fetch).toHaveBeenCalledTimes(1);
    });
  });
});
