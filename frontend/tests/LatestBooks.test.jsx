// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, cleanup } from "@testing-library/react";

const { mockRawJsonGetReq, mockUseOutletContext } = vi.hoisted(() => ({
  mockRawJsonGetReq: vi.fn(),
  mockUseOutletContext: vi.fn(() => ({ role: "viewer" })),
}));

vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual("react-router-dom");
  return {
    ...actual,
    useOutletContext: mockUseOutletContext,
  };
});

vi.mock("../src/Common.js", () => ({
  rawJsonGetReq: mockRawJsonGetReq,
}));

vi.mock("../src/SearchResult", () => ({
  default: ({ results, title, showEditButton, basePath, emptyMessage }) => (
    <div
      data-testid="search-result"
      data-title={title}
      data-show-edit={String(showEditButton)}
      data-base-path={basePath}
    >
      {results.length ? results.map((book) => book.title).join(",") : emptyMessage}
    </div>
  ),
}));

import LatestBooks from "../src/LatestBooks";

describe("LatestBooks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(cleanup);

  it("최신 책 100권을 조회하고 기존 목록 컴포넌트로 렌더링한다", async () => {
    mockRawJsonGetReq.mockImplementation((url, resolve, reject, final) => {
      resolve({
        status: "success",
        result: [{ book_id: 1, title: "새 책", category: "A", file_path: "A/new.txt", file_type: "txt" }],
        total: 1,
      });
      final();
    });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalledWith(
        "/latest?limit=100",
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    const list = screen.getByTestId("search-result");
    expect(list.dataset.title).toBe("최신 책");
    expect(list.dataset.showEdit).toBe("false");
    expect(list.dataset.basePath).toBe("/book-view");
    expect(list.textContent).toContain("새 책");
  });

  it("최신 만화 100권을 조회하고 기존 목록 컴포넌트로 렌더링한다", async () => {
    mockRawJsonGetReq.mockImplementation((url, resolve, reject, final) => {
      resolve({
        status: "success",
        result: [
          {
            book_id: 1,
            title: "새 만화",
            category: "C",
            file_path: "C/new.zip",
            file_type: "zip",
          },
        ],
        total: 1,
      });
      final();
    });

    render(<LatestBooks contentType="comic" />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalledWith(
        "/comics/latest?limit=100",
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    const list = screen.getByTestId("search-result");
    expect(list.dataset.title).toBe("최신 만화");
    expect(list.dataset.showEdit).toBe("false");
    expect(list.dataset.basePath).toBe("/comics-view");
    expect(list.textContent).toContain("새 만화");
  });

  it("조회 실패 시 오류 메시지를 표시한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, _resolve, reject, final) => {
      reject("fail");
      final();
    });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(screen.getByText("최신 책 목록을 불러오지 못했습니다.")).toBeTruthy();
    });
  });

  it("최신 만화 조회 실패 시 오류 메시지를 표시한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, _resolve, reject, final) => {
      reject("fail");
      final();
    });

    render(<LatestBooks contentType="comic" />);

    await waitFor(() => {
      expect(screen.getByText("최신 만화 목록을 불러오지 못했습니다.")).toBeTruthy();
    });
  });

  it("admin 역할일 때 showEditButton을 true로 전달한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, resolve, _reject, final) => {
      resolve({
        status: "success",
        result: [{ book_id: 1, title: "새 책", category: "A", file_path: "A/new.txt", file_type: "txt" }],
        total: 1,
      });
      final();
    });

    mockUseOutletContext.mockReturnValue({ role: "admin" });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalled();
    });
    const list = screen.getByTestId("search-result");
    expect(list.dataset.showEdit).toBe("true");
  });

  it("hasSearched=true일 때 검색 결과와 최신 목록이 함께 렌더링된다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, resolve, _reject, final) => {
      resolve({
        status: "success",
        result: [{ book_id: 1, title: "최신 책 1", category: "A", file_path: "A/new.txt", file_type: "txt" }],
        total: 1,
      });
      final();
    });

    mockUseOutletContext.mockReturnValue({
      hasSearched: true,
      searchResults: [{ book_id: 2, title: "검색된 책", category: "B", file_path: "B/search.txt", file_type: "txt" }],
      role: "viewer",
      searchTotal: 1,
    });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalled();
    });

    const results = screen.getAllByTestId("search-result");
    expect(results.length).toBe(2);
    expect(results[0].textContent).toContain("검색된 책");
    expect(results[1].textContent).toContain("최신 책 1");
  });

  it("만화 컨텍스트에서 hasSearched=true일 때 만화 검색 결과와 최신 만화가 함께 렌더링된다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, resolve, _reject, final) => {
      resolve({
        status: "success",
        result: [{ book_id: 1, title: "최신 만화 1", category: "C", file_path: "C/new.zip", file_type: "zip" }],
        total: 1,
      });
      final();
    });

    mockUseOutletContext.mockReturnValue({
      hasSearched: true,
      searchResults: [{ book_id: 2, title: "검색된 만화", category: "C", file_path: "C/search.zip", file_type: "zip" }],
      role: "admin",
      searchTotal: 1,
    });

    render(<LatestBooks contentType="comic" />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalled();
    });

    const results = screen.getAllByTestId("search-result");
    expect(results.length).toBe(2);
    expect(results[0].dataset.basePath).toBe("/comics-view");
    expect(results[0].textContent).toContain("검색된 만화");
    expect(results[1].textContent).toContain("최신 만화 1");
  });
});
