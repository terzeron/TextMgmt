// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup, fireEvent } from "@testing-library/react";

const { mockUseCoverBrowse, searchResultProps } = vi.hoisted(() => ({
  mockUseCoverBrowse: vi.fn(),
  searchResultProps: { current: null },
}));

vi.mock("../src/useCoverBrowse", () => ({
  useCoverBrowse: mockUseCoverBrowse,
}));

// 카드·그리드는 SearchResult가 그린다. 여기서는 CoverBrowser가 넘기는 값만 본다.
vi.mock("../src/SearchResult", () => ({
  default: (props) => {
    searchResultProps.current = props;
    return (
      <div data-testid="search-result">
        <div data-testid="title">{props.title}</div>
        <div data-testid="empty">{props.emptyMessage}</div>
        <div data-testid="actions">{props.headerActions}</div>
        <button data-testid="load-more" onClick={props.onLoadMore}>
          more
        </button>
      </div>
    );
  },
}));

import CoverBrowser from "../src/CoverBrowser";

const state = (overrides = {}) => ({
  items: [{ book_id: 1 }, { book_id: 2 }],
  total: 330570,
  hasMore: true,
  loading: false,
  error: "",
  loadMore: vi.fn(),
  reload: vi.fn(),
  ...overrides,
});

const renderBrowser = (props = {}) =>
  render(
    <CoverBrowser
      apiPrefix=""
      basePath="/book-view"
      role="admin"
      category=""
      onClearCategory={vi.fn()}
      {...props}
    />,
  );

describe("CoverBrowser", () => {
  beforeEach(() => {
    mockUseCoverBrowse.mockReset();
    mockUseCoverBrowse.mockReturnValue(state());
    searchResultProps.current = null;
    localStorage.clear();
  });
  afterEach(cleanup);

  it("카테고리와 apiPrefix로 훅을 부른다", () => {
    renderBrowser({ apiPrefix: "/comics", category: "소설" });
    expect(mockUseCoverBrowse).toHaveBeenCalledWith({
      apiPrefix: "/comics",
      category: "소설",
    });
  });

  it("최신 탭과 같은 카드(SearchResult)에 목록과 서버 페이지 상태를 넘긴다", () => {
    mockUseCoverBrowse.mockReturnValue(state({ loading: true }));
    renderBrowser({ category: "소설" });

    const props = searchResultProps.current;
    expect(props.results).toEqual([{ book_id: 1 }, { book_id: 2 }]);
    expect(props.basePath).toBe("/book-view");
    expect(props.hasMore).toBe(true);
    expect(props.loading).toBe(true);
    // 커버 보기에서 디렉토리가 바뀌면 그린 개수를 처음으로 되돌리는 기준이다.
    expect(props.coverResetKey).toBe("소설");
    expect(props.role).toBe("admin");
    expect(props.showEditButton).toBe(true);
  });

  it("관리자가 아니면 편집 버튼을 숨긴다", () => {
    renderBrowser({ role: "viewer" });
    expect(searchResultProps.current.showEditButton).toBe(false);
  });

  it("다음 페이지는 카드가 부르는 loadMore로 이어받는다", () => {
    const loadMore = vi.fn();
    mockUseCoverBrowse.mockReturnValue(state({ loadMore }));
    renderBrowser();

    fireEvent.click(screen.getByTestId("load-more"));

    expect(loadMore).toHaveBeenCalledTimes(1);
  });

  it("루트일 때 제목·전체 건수·정렬 기준을 보여 준다", () => {
    renderBrowser();
    expect(screen.getByTestId("title").textContent).toBe(
      "전체 책330,570건(최신순)",
    );
    expect(screen.queryByRole("button", { name: /보기$/ })).toBeNull();
  });

  it("만화 탭은 전체 만화로 표시한다", () => {
    renderBrowser({ apiPrefix: "/comics", basePath: "/comics-view" });
    expect(screen.getByTestId("title").textContent).toContain("전체 만화");
  });

  it("디렉토리를 골랐으면 이름을 보여 주고 루트로 되돌리는 버튼을 둔다", () => {
    const onClearCategory = vi.fn();
    renderBrowser({ category: "5_수학과학일반", onClearCategory });

    expect(screen.getByTestId("title").textContent).toContain("5_수학과학일반");
    fireEvent.click(screen.getByRole("button", { name: "전체 책 보기" }));
    expect(onClearCategory).toHaveBeenCalledTimes(1);
  });

  it("목록/커버 전환 버튼을 최신 탭과 같은 모양으로 둔다", () => {
    renderBrowser();

    expect(screen.getByTitle("목록 보기")).toBeTruthy();
    expect(screen.getByTitle("커버 보기")).toBeTruthy();
    // 저장된 값이 없으면 커버 보기다.
    expect(searchResultProps.current.viewMode).toBe("cover");
  });

  it("보기 모드를 바꾸면 카드에 반영하고, 같은 탭의 다음 방문에도 유지한다", () => {
    const { unmount } = renderBrowser();

    fireEvent.click(screen.getByTitle("목록 보기"));
    expect(searchResultProps.current.viewMode).toBe("list");
    unmount();

    renderBrowser();
    expect(searchResultProps.current.viewMode).toBe("list");
  });

  it("보기 모드는 최신 탭과 같은 저장 값을 공유하고 책·만화가 따로 기억한다", () => {
    localStorage.setItem("tm_latest_view_mode_comic", "list");

    renderBrowser({ apiPrefix: "/comics", basePath: "/comics-view" });
    expect(searchResultProps.current.viewMode).toBe("list");
    cleanup();

    renderBrowser();
    expect(searchResultProps.current.viewMode).toBe("cover");
  });

  it("첫 페이지를 받는 중이면 로딩 문구를, 책이 없으면 안내 문구를 보인다", () => {
    mockUseCoverBrowse.mockReturnValue(
      state({ items: [], total: null, loading: true }),
    );
    const { unmount } = renderBrowser();
    expect(screen.getByTestId("empty").textContent).toBe("로딩 중...");
    expect(screen.getByTestId("title").textContent).toBe("전체 책(최신순)");
    unmount();

    mockUseCoverBrowse.mockReturnValue(state({ items: [], total: 0 }));
    renderBrowser();
    expect(screen.getByTestId("empty").textContent).toBe("책이 없습니다.");
  });

  it("오류가 나면 알림과 다시 시도 버튼을 보이고, 없다는 문구는 숨긴다", () => {
    const reload = vi.fn();
    mockUseCoverBrowse.mockReturnValue(
      state({ items: [], error: "책 목록을 불러오지 못했습니다.", reload }),
    );
    renderBrowser();

    expect(screen.getByText("책 목록을 불러오지 못했습니다.")).toBeTruthy();
    expect(screen.getByTestId("empty").textContent).toBe("");
    fireEvent.click(screen.getByRole("button", { name: "다시 시도" }));
    expect(reload).toHaveBeenCalledTimes(1);
  });
});
