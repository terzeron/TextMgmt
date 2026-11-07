// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useStoredViewerOption } from "../src/useStoredViewerOption";

const isBool = (v) => typeof v === "boolean";

describe("useStoredViewerOption", () => {
  beforeEach(() => localStorage.clear());

  it("저장값이 없으면 fallback을 쓴다", () => {
    const { result } = renderHook(() => useStoredViewerOption("k", true, isBool));
    expect(result.current[0]).toBe(true);
  });

  it("값을 바꾸면 저장하고 다시 마운트해도 복원한다", () => {
    const first = renderHook(() => useStoredViewerOption("k", false, isBool));
    act(() => first.result.current[1](true));
    expect(localStorage.getItem("tm_viewer_k")).toBe("true");
    first.unmount();
    const second = renderHook(() => useStoredViewerOption("k", false, isBool));
    expect(second.result.current[0]).toBe(true);
  });

  it("함수형 갱신을 지원한다", () => {
    const { result } = renderHook(() => useStoredViewerOption("n", 1));
    act(() => result.current[1]((prev) => prev + 1));
    expect(result.current[0]).toBe(2);
    expect(localStorage.getItem("tm_viewer_n")).toBe("2");
  });

  it("검증을 통과하지 못하거나 JSON이 깨진 값은 fallback으로 돌린다", () => {
    localStorage.setItem("tm_viewer_k", "\"x\"");
    expect(renderHook(() => useStoredViewerOption("k", false, isBool)).result.current[0]).toBe(false);
    localStorage.setItem("tm_viewer_k", "{broken");
    expect(renderHook(() => useStoredViewerOption("k", false, isBool)).result.current[0]).toBe(false);
  });

  it("localStorage가 막혀도 상태는 바뀐다", () => {
    const get = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("x"); });
    const set = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("x"); });
    const { result } = renderHook(() => useStoredViewerOption("k", false, isBool));
    act(() => result.current[1](true));
    expect(result.current[0]).toBe(true);
    get.mockRestore();
    set.mockRestore();
  });
});
