// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act, waitFor, cleanup } from "@testing-library/react";

const { mockRawJsonGetReq } = vi.hoisted(() => ({
  mockRawJsonGetReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  rawJsonGetReq: mockRawJsonGetReq,
}));

import {
  BROWSE_PAGE_SIZE,
  buildBrowseUrl,
  useCoverBrowse,
} from "../src/useCoverBrowse";

// 요청마다 응답을 직접 풀어 줄 수 있게 보관한다.
let pending = [];

function deferRequests() {
  pending = [];
  mockRawJsonGetReq.mockImplementation((url, resolve, reject, final) => {
    pending.push({
      url,
      resolve: (data) => {
        resolve(data);
        final();
      },
      reject: (error) => {
        reject(error);
        final();
      },
    });
  });
}

const page = (ids, nextCursor = "") => ({
  status: "success",
  result: ids.map((id) => ({ book_id: id })),
  total: 130,
  next_cursor: nextCursor,
});

describe("buildBrowseUrl", () => {
  it("category가 없으면 전체 책 URL을 만든다", () => {
    expect(buildBrowseUrl("", "", "")).toBe(
      `/browse?limit=${BROWSE_PAGE_SIZE}`,
    );
    expect(buildBrowseUrl("/comics", "", "")).toBe(
      `/comics/browse?limit=${BROWSE_PAGE_SIZE}`,
    );
  });

  it("커서는 인코딩해서 붙인다", () => {
    expect(buildBrowseUrl("", "", "a+b/c=")).toBe(
      `/browse?limit=${BROWSE_PAGE_SIZE}&cursor=a%2Bb%2Fc%3D`,
    );
  });

  it("uncovered면 uncovered=true를 맨 뒤에 붙인다", () => {
    expect(buildBrowseUrl("", "", "", true)).toBe(
      `/browse?limit=${BROWSE_PAGE_SIZE}&uncovered=true`,
    );
    expect(buildBrowseUrl("/comics", "a", "cur", true)).toBe(
      `/comics/browse?limit=${BROWSE_PAGE_SIZE}&category=a&cursor=cur&uncovered=true`,
    );
    expect(buildBrowseUrl("", "", "", false)).toBe(
      `/browse?limit=${BROWSE_PAGE_SIZE}`,
    );
  });

  it("category가 있으면 쿼리 파라미터로 붙인다", () => {
    expect(buildBrowseUrl("", "5_수학과학일반", "")).toBe(
      `/browse?limit=${BROWSE_PAGE_SIZE}&category=${encodeURIComponent("5_수학과학일반")}`,
    );
  });

  it("경로 구분자와 특수문자가 든 카테고리도 한 값으로 인코딩한다", () => {
    expect(buildBrowseUrl("/comics", "a b/c#d", "cur")).toBe(
      `/comics/browse?limit=${BROWSE_PAGE_SIZE}&category=a%20b%2Fc%23d&cursor=cur`,
    );
  });
});

describe("useCoverBrowse", () => {
  beforeEach(() => {
    mockRawJsonGetReq.mockReset();
    deferRequests();
  });
  afterEach(cleanup);

  it("마운트하면 첫 페이지를 요청하고 목록·건수·다음 페이지 여부를 채운다", async () => {
    const { result } = renderHook(() => useCoverBrowse({ apiPrefix: "" }));
    expect(result.current.loading).toBe(true);
    expect(pending[0].url).toBe(`/browse?limit=${BROWSE_PAGE_SIZE}`);

    act(() => pending[0].resolve(page([1, 2], "cur1")));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.items.map((b) => b.book_id)).toEqual([1, 2]);
    expect(result.current.total).toBe(130);
    expect(result.current.hasMore).toBe(true);
    expect(result.current.error).toBe("");
  });

  it("loadMore는 받은 커서로 다음 페이지를 이어붙인다", async () => {
    const { result } = renderHook(() => useCoverBrowse({ apiPrefix: "" }));
    act(() => pending[0].resolve(page([1, 2], "cur1")));
    await waitFor(() => expect(result.current.hasMore).toBe(true));

    act(() => result.current.loadMore());
    expect(pending[1].url).toBe(
      `/browse?limit=${BROWSE_PAGE_SIZE}&cursor=cur1`,
    );
    act(() => pending[1].resolve(page([3], "")));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.items.map((b) => b.book_id)).toEqual([1, 2, 3]);
    expect(result.current.hasMore).toBe(false);
  });

  it("받는 중이거나 더 없으면 loadMore가 요청하지 않는다", async () => {
    const { result } = renderHook(() => useCoverBrowse({ apiPrefix: "" }));
    // 첫 페이지를 받는 중
    act(() => result.current.loadMore());
    expect(pending).toHaveLength(1);

    act(() => pending[0].resolve(page([1], "")));
    await waitFor(() => expect(result.current.loading).toBe(false));
    // 마지막 페이지까지 받음
    act(() => result.current.loadMore());
    expect(pending).toHaveLength(1);
  });

  it("uncovered가 바뀌면 목록을 비우고 미커버만 다시 받는다", async () => {
    const { result, rerender } = renderHook(
      ({ uncovered }) => useCoverBrowse({ apiPrefix: "", uncovered }),
      { initialProps: { uncovered: false } },
    );
    act(() => pending[0].resolve(page([1, 2], "cur1")));
    await waitFor(() => expect(result.current.items).toHaveLength(2));

    rerender({ uncovered: true });

    expect(result.current.items).toEqual([]);
    expect(pending[1].url).toBe(`/browse?limit=${BROWSE_PAGE_SIZE}&uncovered=true`);
    // 서버가 전체 개수를 모르면 total은 null로 남는다.
    act(() => pending[1].resolve({ status: "success", result: [{ book_id: 9 }], total: null, next_cursor: "" }));
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    expect(result.current.total).toBeNull();
  });

  it("미커버 페이지가 비어도 커서가 있으면 이어서 받는다", async () => {
    const { result } = renderHook(() =>
      useCoverBrowse({ apiPrefix: "", uncovered: true }),
    );
    act(() => pending[0].resolve(page([], "c1")));

    await waitFor(() => expect(pending).toHaveLength(2));
    expect(pending[1].url).toBe(
      `/browse?limit=${BROWSE_PAGE_SIZE}&cursor=c1&uncovered=true`,
    );
    // 이어받는 동안은 계속 로딩 중이라 빈 목록 안내가 깜박이지 않는다.
    expect(result.current.loading).toBe(true);

    act(() => pending[1].resolve(page([5], "")));
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    expect(result.current.hasMore).toBe(false);
    expect(result.current.loading).toBe(false);
    expect(pending).toHaveLength(2);
  });

  it("미커버가 아니면 빈 페이지에서 자동으로 이어받지 않는다", async () => {
    renderHook(() => useCoverBrowse({ apiPrefix: "" }));
    act(() => pending[0].resolve(page([], "c1")));

    await waitFor(() => expect(pending[0]).toBeDefined());
    expect(pending).toHaveLength(1);
  });

  it("카테고리가 바뀌면 목록을 비우고 새 카테고리로 다시 받는다", async () => {
    const { result, rerender } = renderHook(
      ({ category }) => useCoverBrowse({ apiPrefix: "", category }),
      { initialProps: { category: "" } },
    );
    act(() => pending[0].resolve(page([1, 2], "cur1")));
    await waitFor(() => expect(result.current.items).toHaveLength(2));

    rerender({ category: "소설" });

    expect(result.current.items).toEqual([]);
    expect(result.current.total).toBeNull();
    expect(pending[1].url).toBe(
      `/browse?limit=${BROWSE_PAGE_SIZE}&category=${encodeURIComponent("소설")}`,
    );
  });

  it("카테고리가 바뀐 뒤 늦게 도착한 이전 응답은 버린다", async () => {
    const { result, rerender } = renderHook(
      ({ category }) => useCoverBrowse({ apiPrefix: "", category }),
      { initialProps: { category: "A" } },
    );
    rerender({ category: "B" });

    act(() => pending[1].resolve(page([20], "")));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => pending[0].resolve(page([10], "")));

    expect(result.current.items.map((b) => b.book_id)).toEqual([20]);
  });

  it("실패하면 멈추고 오류를 알리며, reload로 처음부터 다시 받는다", async () => {
    const { result } = renderHook(() => useCoverBrowse({ apiPrefix: "" }));
    act(() => pending[0].reject(new Error("boom")));

    await waitFor(() => expect(result.current.error).not.toBe(""));
    expect(result.current.hasMore).toBe(false);
    expect(result.current.loading).toBe(false);
    // 멈췄으므로 끝 감지가 계속 호출해도 요청이 반복되지 않는다.
    act(() => result.current.loadMore());
    expect(pending).toHaveLength(1);

    act(() => result.current.reload());
    expect(pending).toHaveLength(2);
    act(() => pending[1].resolve(page([1], "")));
    await waitFor(() => expect(result.current.items).toHaveLength(1));
    expect(result.current.error).toBe("");
  });

  it("status가 success가 아닌 응답도 실패로 다룬다", async () => {
    const { result } = renderHook(() => useCoverBrowse({ apiPrefix: "" }));
    act(() => pending[0].resolve({ status: "failure", error: "x" }));

    await waitFor(() => expect(result.current.error).not.toBe(""));
    expect(result.current.items).toEqual([]);
    expect(result.current.hasMore).toBe(false);
  });

  it("건수가 응답에 없으면 null로 둔다", async () => {
    const { result } = renderHook(() => useCoverBrowse({ apiPrefix: "" }));
    act(() =>
      pending[0].resolve({ status: "success", result: [{ book_id: 1 }] }),
    );

    await waitFor(() => expect(result.current.items).toHaveLength(1));
    expect(result.current.total).toBeNull();
  });

  it("언마운트 뒤에 도착한 응답은 상태를 건드리지 않는다", async () => {
    const { result, unmount } = renderHook(() =>
      useCoverBrowse({ apiPrefix: "" }),
    );
    unmount();

    act(() => pending[0].resolve(page([1], "")));

    expect(result.current.items).toEqual([]);
  });
});
