// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, cleanup } from "@testing-library/react";

const { mockRawJsonGetReq } = vi.hoisted(() => ({
  mockRawJsonGetReq: vi.fn(),
}));

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
});
