// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  reportClientError,
  initGlobalErrorLogging,
  removeGlobalErrorLogging,
  _resetDedupCacheForTesting,
} from "../src/clientLogger";

describe("clientLogger", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    _resetDedupCacheForTesting();
    removeGlobalErrorLogging();
  });

  afterEach(() => {
    removeGlobalErrorLogging();
  });

  it("sendBeacon 지원 시 sendBeacon을 사용하여 에러를 전송한다", () => {
    const sendBeaconMock = vi.fn().mockReturnValue(true);
    navigator.sendBeacon = sendBeaconMock;

    const result = reportClientError({
      errorType: "REACT_RENDER_ERROR",
      message: "Test render error",
      stack: "Error: Test render error\n  at App.jsx:10",
      componentStack: "\n  at App",
    });

    expect(result).toBe(true);
    expect(sendBeaconMock).toHaveBeenCalledTimes(1);
    expect(sendBeaconMock.mock.calls[0][0]).toContain("/logs/client-error");
  });

  it("sendBeacon 실패 시 fetch로 폴백하여 전송한다", async () => {
    navigator.sendBeacon = vi.fn().mockImplementation(() => {
      throw new Error("sendBeacon failed");
    });
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    global.fetch = fetchMock;

    const result = reportClientError({
      errorType: "WINDOW_ERROR",
      message: "Test window error",
    });

    expect(result).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0][0]).toContain("/logs/client-error");
    const sentBody = JSON.parse(fetchMock.mock.calls[0][1].body);
    expect(sentBody.error_type).toBe("WINDOW_ERROR");
    expect(sentBody.message).toBe("Test window error");
  });

  it("5초 이내 동일한 에러 발생 시 중복 전송을 방지(dedup)한다", () => {
    navigator.sendBeacon = vi.fn().mockReturnValue(true);

    const first = reportClientError({
      errorType: "CUSTOM_ERROR",
      message: "Duplicate message",
    });
    const second = reportClientError({
      errorType: "CUSTOM_ERROR",
      message: "Duplicate message",
    });

    expect(first).toBe(true);
    expect(second).toBe(false);
    expect(navigator.sendBeacon).toHaveBeenCalledTimes(1);
  });

  it("initGlobalErrorLogging 등록 시 window error 이벤트를 포착하여 보고한다", () => {
    navigator.sendBeacon = vi.fn().mockReturnValue(true);
    initGlobalErrorLogging();

    const errorEvent = new ErrorEvent("error", {
      message: "Uncaught script error",
      filename: "http://localhost:3000/main.js",
      lineno: 12,
      colno: 34,
      error: new Error("Uncaught script error"),
    });
    window.dispatchEvent(errorEvent);

    expect(navigator.sendBeacon).toHaveBeenCalledTimes(1);
  });

  it("initGlobalErrorLogging 등록 시 unhandledrejection 이벤트를 포착하여 보고한다", () => {
    navigator.sendBeacon = vi.fn().mockReturnValue(true);
    initGlobalErrorLogging();

    const promiseEvent = new CustomEvent("unhandledrejection", {
      detail: { reason: new Error("Async promise rejected") },
    });
    Object.defineProperty(promiseEvent, "reason", {
      value: new Error("Async promise rejected"),
    });
    window.dispatchEvent(promiseEvent);

    expect(navigator.sendBeacon).toHaveBeenCalledTimes(1);
  });

  it("removeGlobalErrorLogging 호출 시 리스너가 정상 해제된다", () => {
    navigator.sendBeacon = vi.fn().mockReturnValue(true);
    const cleanup = initGlobalErrorLogging();
    cleanup();

    const errorEvent = new ErrorEvent("error", {
      message: "Should not report",
    });
    window.dispatchEvent(errorEvent);

    expect(navigator.sendBeacon).not.toHaveBeenCalled();
  });

  it("getApiUrlPrefix가 있을 때 해당 prefix를 엔드포인트에 반영한다", () => {
    window.__ENV__ = { VITE_API_URL_PREFIX: "http://api.test.com" };
    navigator.sendBeacon = vi.fn().mockReturnValue(true);

    reportClientError({
      errorType: "CUSTOM_ERROR",
      message: "Prefix test",
    });

    expect(navigator.sendBeacon).toHaveBeenCalledWith(
      "http://api.test.com/logs/client-error",
      expect.any(Blob)
    );
    delete window.__ENV__;
  });

  it("MAX_RECENT_KEYS 초과 시 가장 오래된 키를 제거한다", () => {
    navigator.sendBeacon = vi.fn().mockReturnValue(true);

    for (let i = 0; i < 110; i++) {
      reportClientError({
        errorType: "CUSTOM_ERROR",
        message: `Unique error message ${i}`,
      });
    }
    // 최초의 키는 지워졌으므로 다시 전송 가능
    const reSent = reportClientError({
      errorType: "CUSTOM_ERROR",
      message: "Unique error message 0",
    });
    expect(reSent).toBe(true);
  });

  it("unhandledrejection에서 reason이 문자열이거나 빈 값일 때도 안전하게 처리된다", () => {
    navigator.sendBeacon = vi.fn().mockReturnValue(true);
    initGlobalErrorLogging();

    const promiseEvent = new CustomEvent("unhandledrejection");
    Object.defineProperty(promiseEvent, "reason", {
      value: "String rejection reason",
    });
    window.dispatchEvent(promiseEvent);

    expect(navigator.sendBeacon).toHaveBeenCalledTimes(1);
  });
});
