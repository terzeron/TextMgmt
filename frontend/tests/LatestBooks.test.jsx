// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  cleanup,
  fireEvent,
} from "@testing-library/react";

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
  default: ({
    results,
    title,
    showEditButton,
    basePath,
    emptyMessage,
    viewMode,
    headerActions,
  }) => (
    <>
      {headerActions}
      <div
        data-testid="search-result"
        data-title={title}
        data-show-edit={String(showEditButton)}
        data-base-path={basePath}
        data-view-mode={viewMode}
        data-book-ids={results.map((book) => book.book_id).join(",")}
      >
        {results.length
          ? results.map((book) => book.display_title || book.title).join(",")
          : emptyMessage}
      </div>
    </>
  ),
}));

import LatestBooks from "../src/LatestBooks";

describe("LatestBooks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  afterEach(cleanup);

  it("최신 책 2000권을 조회하고 기존 목록 컴포넌트로 렌더링한다", async () => {
    mockRawJsonGetReq.mockImplementation((url, resolve, reject, final) => {
      resolve({
        status: "success",
        result: [
          {
            book_id: 1,
            title: "새 책",
            category: "A",
            file_path: "A/new.txt",
            file_type: "txt",
          },
        ],
        total: 1,
      });
      final();
    });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalledWith(
        "/latest?limit=2000",
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

  it("최신 만화 2000권을 조회하고 기존 목록 컴포넌트로 렌더링한다", async () => {
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
        "/comics/latest?limit=2000",
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

  it("최신 만화를 책 디렉토리별 1권이나 1화로 모아 표시한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, resolve, _reject, final) => {
      resolve({
        status: "success",
        result: [
          {
            book_id: 11,
            title: "작품 A 2권",
            category: "1_액션/작품 A",
            file_path: "1_액션/작품 A/02.pdf",
            file_type: "pdf",
          },
          {
            book_id: 10,
            title: "작품 A 1권",
            category: "1_액션/작품 A",
            file_path: "1_액션/작품 A/01.pdf",
            file_type: "pdf",
          },
          {
            book_id: 12,
            title: "작품 A 부록",
            category: "1_액션/작품 A/부록",
            file_path: "1_액션/작품 A/부록/extra.pdf",
            file_type: "pdf",
          },
          {
            book_id: 20,
            title: "다른 작품 10화",
            category: "2_SF/다른 작품",
            file_path: "2_SF/다른 작품/10화.pdf",
            file_type: "pdf",
          },
          {
            book_id: 21,
            title: "다른 작품 1화",
            category: "2_SF/다른 작품",
            file_path: "2_SF/다른 작품/1화.pdf",
            file_type: "pdf",
          },
        ],
      });
      final();
    });

    render(<LatestBooks contentType="comic" />);

    await waitFor(() => {
      expect(screen.getByTestId("search-result").dataset.bookIds).toBe("10,21");
    });
    const list = screen.getByTestId("search-result");
    expect(list.textContent).toBe("작품 A,다른 작품");
  });

  it("조회 실패 시 오류 메시지를 표시한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, _resolve, reject, final) => {
      reject("fail");
      final();
    });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(
        screen.getByText("최신 책 목록을 불러오지 못했습니다."),
      ).toBeTruthy();
    });
  });

  it("최신 만화 조회 실패 시 오류 메시지를 표시한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, _resolve, reject, final) => {
      reject("fail");
      final();
    });

    render(<LatestBooks contentType="comic" />);

    await waitFor(() => {
      expect(
        screen.getByText("최신 만화 목록을 불러오지 못했습니다."),
      ).toBeTruthy();
    });
  });

  it("admin 역할일 때 showEditButton을 true로 전달한다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, resolve, _reject, final) => {
      resolve({
        status: "success",
        result: [
          {
            book_id: 1,
            title: "새 책",
            category: "A",
            file_path: "A/new.txt",
            file_type: "txt",
          },
        ],
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
        result: [
          {
            book_id: 1,
            title: "최신 책 1",
            category: "A",
            file_path: "A/new.txt",
            file_type: "txt",
          },
        ],
        total: 1,
      });
      final();
    });

    mockUseOutletContext.mockReturnValue({
      hasSearched: true,
      searchResults: [
        {
          book_id: 2,
          title: "검색된 책",
          category: "B",
          file_path: "B/search.txt",
          file_type: "txt",
        },
      ],
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

  it("알 수 없는 contentType이면 기본 book 설정으로 대체한다", async () => {
    mockUseOutletContext.mockReturnValue({ role: "viewer" });
    mockRawJsonGetReq.mockImplementation((url, resolve, _reject, final) => {
      resolve({
        status: "success",
        result: [
          {
            book_id: 1,
            title: "새 책",
            category: "A",
            file_path: "A/new.txt",
            file_type: "txt",
          },
        ],
        total: 1,
      });
      final();
    });

    render(<LatestBooks contentType="unknown-type" />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalledWith(
        "/latest?limit=2000",
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    const list = screen.getByTestId("search-result");
    expect(list.dataset.title).toBe("최신 책");
    expect(list.dataset.basePath).toBe("/book-view");
  });

  it("useOutletContext가 값을 반환하지 않으면 기본값으로 동작한다", async () => {
    mockUseOutletContext.mockReturnValue(undefined);
    mockRawJsonGetReq.mockImplementation((url, resolve, _reject, final) => {
      resolve({
        status: "success",
        result: [
          {
            book_id: 1,
            title: "새 책",
            category: "A",
            file_path: "A/new.txt",
            file_type: "txt",
          },
        ],
        total: 1,
      });
      final();
    });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalled();
    });
    // hasSearched 기본값 false이므로 검색 결과 목록은 렌더링되지 않는다
    expect(screen.getAllByTestId("search-result").length).toBe(1);
    const list = screen.getByTestId("search-result");
    expect(list.dataset.showEdit).toBe("false");
  });

  it("성공 응답에 result가 없으면 빈 목록으로 처리한다", async () => {
    mockUseOutletContext.mockReturnValue({ role: "viewer" });
    mockRawJsonGetReq.mockImplementation((url, resolve, _reject, final) => {
      resolve({ status: "success" });
      final();
    });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalled();
    });
    const list = screen.getByTestId("search-result");
    expect(list.textContent).toBe("최신 책이 없습니다.");
  });

  it("응답 status가 success가 아니면 오류 메시지를 표시하고 목록을 비운다", async () => {
    mockUseOutletContext.mockReturnValue({ role: "viewer" });
    mockRawJsonGetReq.mockImplementation((url, resolve, _reject, final) => {
      resolve({ status: "fail" });
      final();
    });

    render(<LatestBooks />);

    await waitFor(() => {
      expect(
        screen.getByText("최신 책 목록을 불러오지 못했습니다."),
      ).toBeTruthy();
    });
    const list = screen.getByTestId("search-result");
    expect(list.textContent).toBe("최신 책이 없습니다.");
  });

  it("만화 컨텍스트에서 hasSearched=true일 때 만화 검색 결과와 최신 만화가 함께 렌더링된다", async () => {
    mockRawJsonGetReq.mockImplementation((_url, resolve, _reject, final) => {
      resolve({
        status: "success",
        result: [
          {
            book_id: 1,
            title: "최신 만화 1",
            category: "C",
            file_path: "C/new.zip",
            file_type: "zip",
          },
        ],
        total: 1,
      });
      final();
    });

    mockUseOutletContext.mockReturnValue({
      hasSearched: true,
      searchResults: [
        {
          book_id: 2,
          title: "검색된 만화",
          category: "C",
          file_path: "C/search.zip",
          file_type: "zip",
        },
      ],
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

  describe("뷰 모드 토글", () => {
    beforeEach(() => {
      mockUseOutletContext.mockReturnValue({ role: "viewer" });
    });

    const resolveEmpty = (_url, resolve, _reject, final) => {
      resolve({ status: "success", result: [], total: 0 });
      final();
    };

    it("기본은 커버 뷰다", async () => {
      mockRawJsonGetReq.mockImplementation(resolveEmpty);
      render(<LatestBooks />);
      await waitFor(() => expect(mockRawJsonGetReq).toHaveBeenCalled());
      const list = screen.getByTestId("search-result");
      expect(list.dataset.viewMode).toBe("cover");
      expect(
        screen.getByLabelText("커버 보기", { selector: "input" }).checked,
      ).toBe(true);
    });

    it("목록 보기를 누르면 목록 뷰로 바뀌고 탭별로 저장한다", async () => {
      mockRawJsonGetReq.mockImplementation(resolveEmpty);
      render(<LatestBooks />);
      await waitFor(() => expect(mockRawJsonGetReq).toHaveBeenCalled());

      fireEvent.click(
        screen.getByLabelText("목록 보기", { selector: "input" }),
      );

      expect(screen.getByTestId("search-result").dataset.viewMode).toBe(
        "list",
      );
      expect(localStorage.getItem("tm_latest_view_mode_book")).toBe("list");
      expect(localStorage.getItem("tm_latest_view_mode_comic")).toBeNull();
    });

    it("저장된 목록 뷰를 복원한다", async () => {
      localStorage.setItem("tm_latest_view_mode_comic", "list");
      mockRawJsonGetReq.mockImplementation(resolveEmpty);
      render(<LatestBooks contentType="comic" />);
      await waitFor(() => expect(mockRawJsonGetReq).toHaveBeenCalled());
      expect(screen.getByTestId("search-result").dataset.viewMode).toBe("list");
    });

    it("탭이 바뀌면 그 탭의 저장값을 다시 읽는다", async () => {
      localStorage.setItem("tm_latest_view_mode_comic", "list");
      mockRawJsonGetReq.mockImplementation(resolveEmpty);
      const { rerender } = render(<LatestBooks />);
      await waitFor(() => expect(mockRawJsonGetReq).toHaveBeenCalled());
      expect(screen.getByTestId("search-result").dataset.viewMode).toBe(
        "cover",
      );

      rerender(<LatestBooks contentType="comic" />);
      await waitFor(() =>
        expect(screen.getByTestId("search-result").dataset.viewMode).toBe(
          "list",
        ),
      );
    });

    it("localStorage를 쓸 수 없어도 커버 뷰로 동작하고 전환된다", async () => {
      const getSpy = vi
        .spyOn(Storage.prototype, "getItem")
        .mockImplementation(() => {
          throw new Error("blocked");
        });
      const setSpy = vi
        .spyOn(Storage.prototype, "setItem")
        .mockImplementation(() => {
          throw new Error("blocked");
        });
      try {
        mockRawJsonGetReq.mockImplementation(resolveEmpty);
        render(<LatestBooks />);
        await waitFor(() => expect(mockRawJsonGetReq).toHaveBeenCalled());
        expect(screen.getByTestId("search-result").dataset.viewMode).toBe(
          "cover",
        );

        fireEvent.click(
          screen.getByLabelText("목록 보기", { selector: "input" }),
        );
        expect(screen.getByTestId("search-result").dataset.viewMode).toBe(
          "list",
        );
      } finally {
        getSpy.mockRestore();
        setSpy.mockRestore();
      }
    });
  });
});

describe("LatestBooks 만화 대표 선정 경계", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  afterEach(cleanup);

  const renderComics = async (result) => {
    mockRawJsonGetReq.mockImplementation((_url, resolve, _reject, final) => {
      resolve({ status: "success", result });
      final();
    });
    render(<LatestBooks contentType="comic" />);
    await waitFor(() => expect(mockRawJsonGetReq).toHaveBeenCalled());
    return screen.getByTestId("search-result");
  };

  it.each([
    ["경로가 더 앞서면 교체한다", ["1_a/작품/02.pdf", "1_a/작품/01.pdf"], "2"],
    ["경로가 더 뒤면 유지한다", ["1_a/작품/01.pdf", "1_a/작품/02.pdf"], "1"],
  ])("둘 다 1권이 아닐 때 %s", async (_name, paths, expectedId) => {
    const list = await renderComics(
      paths.map((file_path, index) => ({
        book_id: index + 1,
        title: `작품 ${index + 5}권`,
        category: "1_a/작품",
        file_path,
      })),
    );
    await waitFor(() => expect(list.dataset.bookIds).toBe(expectedId));
  });

  it("둘 다 1권이면 경로 순으로 고른다", async () => {
    const list = await renderComics([
      { book_id: 1, title: "작품 1권", category: "1_a/작품", file_path: "1_a/작품/b.pdf" },
      { book_id: 2, title: "작품 1화", category: "1_a/작품", file_path: "1_a/작품/a.pdf" },
    ]);
    await waitFor(() => expect(list.dataset.bookIds).toBe("2"));
  });

  it("title과 file_path가 없는 항목도 안전하게 묶는다", async () => {
    const list = await renderComics([
      { book_id: 1, category: "1_a/작품" },
      { book_id: 2, category: "1_a/작품" },
      { book_id: 3, title: "작품 1권", category: "1_a/작품" },
      { book_id: 4, category: null },
    ]);
    await waitFor(() => expect(list.dataset.bookIds).toContain("3"));
  });

  it("만화 결과가 배열이 아니면 빈 목록으로 처리한다", async () => {
    const list = await renderComics({ unexpected: true });
    await waitFor(() => expect(list.textContent).toBe("최신 만화가 없습니다."));
  });
});
