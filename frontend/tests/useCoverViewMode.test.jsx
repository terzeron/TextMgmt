// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  renderHook,
  act,
  screen,
  cleanup,
  fireEvent,
} from "@testing-library/react";

import CoverViewToggle from "../src/CoverViewToggle";
import { useCoverViewMode } from "../src/useCoverViewMode";

describe("useCoverViewMode", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("저장된 값이 없으면 커버 보기다", () => {
    const { result } = renderHook(() => useCoverViewMode("book"));
    expect(result.current[0]).toBe("cover");
  });

  it("바꾼 값을 저장하고 같은 종류의 다음 사용에서 되살린다", () => {
    const first = renderHook(() => useCoverViewMode("book"));
    act(() => first.result.current[1]("list"));
    expect(first.result.current[0]).toBe("list");
    expect(localStorage.getItem("tm_latest_view_mode_book")).toBe("list");

    const second = renderHook(() => useCoverViewMode("book"));
    expect(second.result.current[0]).toBe("list");
  });

  it("책과 만화는 따로 기억한다", () => {
    localStorage.setItem("tm_latest_view_mode_comic", "list");
    expect(renderHook(() => useCoverViewMode("book")).result.current[0]).toBe(
      "cover",
    );
    expect(renderHook(() => useCoverViewMode("comic")).result.current[0]).toBe(
      "list",
    );
  });

  it("같은 인스턴스가 책에서 만화로 바뀌면 만화의 값을 다시 읽는다", () => {
    localStorage.setItem("tm_latest_view_mode_comic", "list");
    const { result, rerender } = renderHook(
      ({ type }) => useCoverViewMode(type),
      { initialProps: { type: "book" } },
    );
    expect(result.current[0]).toBe("cover");

    rerender({ type: "comic" });

    expect(result.current[0]).toBe("list");
  });

  it("저장소를 쓸 수 없어도 커버 보기로 시작하고 전환은 유지한다", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const { result } = renderHook(() => useCoverViewMode("book"));
    expect(result.current[0]).toBe("cover");

    act(() => result.current[1]("list"));

    expect(result.current[0]).toBe("list");
  });
});

describe("CoverViewToggle", () => {
  afterEach(cleanup);

  it("현재 모드를 선택 상태로 보이고 전환을 알린다", () => {
    const onChange = vi.fn();
    render(<CoverViewToggle id="x" viewMode="cover" onChange={onChange} />);

    expect(screen.getByLabelText("커버 보기").checked).toBe(true);
    expect(screen.getByLabelText("목록 보기").checked).toBe(false);

    fireEvent.click(screen.getByLabelText("목록 보기"));
    // react-bootstrap은 (값, 이벤트)로 부른다.
    expect(onChange.mock.calls[0][0]).toBe("list");
  });
});
