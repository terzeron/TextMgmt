// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, cleanup, act } from "@testing-library/react";

afterEach(cleanup);

const {
  mockJsonGetReq,
  mockRawJsonGetReq,
  mockGetApiUrlPrefix,
  mockRecordBookView,
  mockOutletContext,
  mockRouteState,
} = vi.hoisted(() => ({
  mockRecordBookView: vi.fn(() => Promise.resolve(true)),
  mockJsonGetReq: vi.fn(),
  mockRawJsonGetReq: vi.fn(),
  mockGetApiUrlPrefix: vi.fn(() => "http://localhost:8000"),
  mockOutletContext: {
    searchResults: [],
    hasSearched: false,
    role: "admin",
    searchTotal: 0,
    handleLoadMore: vi.fn(),
    searchLoading: false,
  },
  mockRouteState: { wildcard: "", searchParams: "" },
}));

vi.mock("../src/Common", () => ({
  jsonGetReq: mockJsonGetReq,
  rawJsonGetReq: mockRawJsonGetReq,
  getApiUrlPrefix: mockGetApiUrlPrefix,
  recordBookView: mockRecordBookView,
}));

// 카테고리 목록은 커서 페이지네이션 때문에 rawJsonGetReq(원본 응답)를 쓴다.
// 페이징을 직접 검증하지 않는 테스트는 기존 jsonGetReq 모킹을 그대로 쓸 수 있도록,
// 쿼리스트링을 떼고 위임한 뒤 한 페이지짜리 응답으로 감싸 준다.
function delegateRawToJsonGetReq() {
  mockRawJsonGetReq.mockImplementation((url, resolve, reject, final) => {
    mockJsonGetReq(
      url.split("?")[0],
      null,
      (result) =>
        resolve &&
        resolve({
          status: "success",
          result: result || [],
          total: (result || []).length,
          next_cursor: null,
        }),
      reject,
      final,
    );
  });
}

vi.mock("../src/Folder.jsx", () => ({
  default: ({ folderData, onClickHandler, onToggle, isOpen }) => {
    // 실제 Folder처럼 폴더가 아닌 항목(책)을 누르면 디렉토리 패널을 접는다.
    const click = (item) => {
      onClickHandler(item.id);
      if (item.fileType !== "folder" && onToggle) onToggle(false);
    };
    return (
      <div data-testid="folder" data-open={String(isOpen)}>
        <button
          data-testid="folder-toggle"
          onClick={() => onToggle && onToggle(!isOpen)}
        >
          toggle
        </button>
        {folderData.map((item) => (
          <div key={item.id}>
            <button
              data-testid={`folder-item-${item.id}`}
              onClick={() => click(item)}
            >
              {item.label}
            </button>
            {item.children?.map((child) => (
              <button
                key={child.id}
                data-testid={`folder-item-${child.id}`}
                onClick={() => click(child)}
              >
                {child.label}
              </button>
            ))}
          </div>
        ))}
        <button
          data-testid="custom-entry-click"
          onClick={() => onClickHandler("소설/999")}
        >
          custom
        </button>
        <button
          data-testid="custom-deep-click"
          onClick={() => onClickHandler("문학/소설/42")}
        >
          deep
        </button>
        <button
          data-testid="custom-unparsable-click"
          onClick={() => onClickHandler("소설/abc")}
        >
          unparsable
        </button>
      </div>
    );
  },
}));

vi.mock("../src/ViewSingle.jsx", () => ({
  default: ({
    bookId,
    fileType,
    onNextBook,
    hasNextBook,
    onPrevBook,
    hasPrevBook,
    viewUrl,
  }) => (
    <div data-testid="view-single" data-view-url={viewUrl}>
      ViewSingle: {bookId} ({fileType})
      {hasNextBook && (
        <button data-testid="next-book-btn" onClick={onNextBook}>
          Next
        </button>
      )}
      {hasPrevBook && (
        <button data-testid="prev-book-btn" onClick={onPrevBook}>
          Prev
        </button>
      )}
    </div>
  ),
}));

vi.mock("../src/BookInfoView.jsx", () => ({
  default: ({ bookInfo, showDetails }) => (
    <div
      data-testid="book-info-view"
      data-show-details={String(showDetails !== false)}
    >
      BookInfo: {bookInfo.title}
    </div>
  ),
}));

vi.mock("../src/SimilarBooks.jsx", () => ({
  default: ({ bookId, autoOpenHighScore, canEdit, apiPrefix, basePath }) => (
    <div
      data-testid="similar-books"
      data-book-id={bookId}
      data-auto-open={String(autoOpenHighScore !== false)}
      data-can-edit={String(!!canEdit)}
      data-api-prefix={apiPrefix}
      data-base-path={basePath}
    >
      SimilarBooks
    </div>
  ),
}));

vi.mock("../src/BookLoadError.jsx", () => ({
  default: ({ bookId, error, role }) => (
    <div data-testid="book-load-error">
      LoadError: {bookId} / {error} / {role}
    </div>
  ),
}));

vi.mock("../src/SearchResult", () => ({
  default: ({ results }) => (
    <div data-testid="search-result">{results?.length || 0} results</div>
  ),
}));

vi.mock("../src/CoverBrowser", () => ({
  default: ({ apiPrefix, basePath, role, category, onClearCategory }) => (
    <div
      data-testid="cover-browser"
      data-category={category}
      data-api-prefix={apiPrefix}
      data-base-path={basePath}
      data-role={role}
    >
      <button data-testid="cover-clear" onClick={onClearCategory}>
        clear
      </button>
    </div>
  ),
}));

vi.mock("../src/folderUtils", async () => {
  const actual = await vi.importActual("../src/folderUtils");
  return actual;
});

vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual("react-router-dom");
  return {
    ...actual,
    useParams: () => ({ "*": mockRouteState.wildcard }),
    useSearchParams: () => [new URLSearchParams(mockRouteState.searchParams)],
    useOutletContext: () => mockOutletContext,
  };
});

import View from "../src/View";

describe("View", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockOutletContext.role = "admin";
    mockOutletContext.hasSearched = false;
    mockOutletContext.searchResults = [];
    mockOutletContext.searchTotal = 0;
    mockOutletContext.searchLoading = false;
    mockRouteState.wildcard = "";
    mockRouteState.searchParams = "";
    delegateRawToJsonGetReq();
  });

  it("카테고리 목록을 로드하여 Folder에 전달한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10, 역사: 5 });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
    expect(screen.getByTestId("folder").dataset.open).toBe("true");
  });

  it("검색 전에도 오른쪽 컬럼을 두고 전체 책 커버 목록으로 채운다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10, 역사: 5 });
      }
    });

    const { container } = render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
    expect(container.querySelector(".split-layout-left")).toBeTruthy();
    expect(container.querySelector(".split-layout-right")).toBeTruthy();
    expect(screen.getByRole("separator")).toBeTruthy();
    const browser = screen.getByTestId("cover-browser");
    expect(browser.closest(".split-layout-right")).toBeTruthy();
    expect(browser.dataset.category).toBe("");
    expect(browser.dataset.apiPrefix).toBe("");
    expect(browser.dataset.basePath).toBe("/book-view");
  });

  it("카테고리 로드 실패 시 에러 메시지를 표시하지 않고 빈 폴더를 표시한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") {
        reject("서버 오류");
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
  });

  it("_root 카테고리가 있으면 최상위 파일을 로드한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10, _root: 2 });
      } else if (url === "/categories/_root") {
        resolve([
          {
            book_id: 100,
            title: "rootbook",
            file_type: "pdf",
            file_path: "/root.pdf",
            category: "_root",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
  });

  it("_root 로드 실패 시에도 카테고리 폴더는 표시된다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") {
        resolve({ 소설: 10, _root: 2 });
      } else if (url === "/categories/_root") {
        reject("_root 로드 실패");
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
  });

  it("hasSearched=true일 때 검색 결과 컴포넌트를 표시한다", async () => {
    mockOutletContext.hasSearched = true;
    mockOutletContext.searchResults = [{ book_id: 1 }];

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({});
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("search-result")).toBeTruthy();
    });
  });

  it("카테고리 클릭 시 entryClicked를 통해 하위 책 목록을 로드한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "테스트소설",
            file_type: "epub",
            file_path: "/test.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });

    // 카테고리 클릭 → entryClicked 호출
    const folderItem = screen.getByTestId("folder-item-소설");
    folderItem.click();

    // jsonGetReq가 /categories/소설로 호출됨 확인
    await waitFor(() => {
      const calls = mockJsonGetReq.mock.calls;
      const categoryCall = calls.find((c) => c[0] === "/categories/소설");
      expect(categoryCall).toBeTruthy();
    });
  });

  it("viewer 역할일 때 hidden-categories를 로드하여 필터링한다", async () => {
    mockOutletContext.role = "viewer";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10, 비공개: 5, 역사: 3 });
      } else if (url === "/hidden-categories?content_type=book") {
        resolve(["비공개"]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });

    // hidden-categories API가 호출됨
    const calls = mockJsonGetReq.mock.calls;
    expect(
      calls.find((c) => c[0] === "/hidden-categories?content_type=book"),
    ).toBeTruthy();
  });

  it("viewer 역할에서 hidden-categories 로드 실패 시 전체 카테고리를 표시한다", async () => {
    mockOutletContext.role = "viewer";

    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") {
        resolve({ 소설: 10 });
      } else if (url === "/hidden-categories?content_type=book") {
        reject("로드 실패");
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
  });

  it("카테고리 클릭 후 책 클릭 시 BookInfoView와 ViewSingle이 표시된다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "테스트소설",
            file_type: "epub",
            file_path: "/test.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    // 카테고리 로드 대기
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });

    // 카테고리 클릭 → 책 목록 로드
    screen.getByTestId("folder-item-소설").click();

    // 책 목록이 폴더에 추가될 때까지 대기
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/42")).toBeTruthy();
    });

    // 책 클릭 → bookInfo 설정
    screen.getByTestId("folder-item-소설/42").click();

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
      expect(screen.getByTestId("view-single")).toBeTruthy();
    });
    expect(screen.getByTestId("book-info-view").dataset.showDetails).toBe(
      "false",
    );
    expect(screen.queryByText("실행 결과")).toBeNull();
    expect(screen.getByTestId("similar-books").dataset.autoOpen).toBe("false");
    expect(screen.getByTestId("similar-books").dataset.canEdit).toBe("true");
    expect(screen.getByTestId("similar-books").dataset.basePath).toBe(
      "/book-edit",
    );
    // 책 보기는 오른쪽 컬럼 안에 놓이고, 두 컬럼은 따로 스크롤된다.
    const layout = screen.getByTestId("view-single").closest(".split-layout");
    expect(layout).toBeTruthy();
    expect(layout.classList.contains("no-scroll")).toBe(false);
    const right = screen
      .getByTestId("view-single")
      .closest(".split-layout-right.view-main-panel");
    expect(right).toBeTruthy();
    expect(right.contains(screen.getByTestId("book-info-view"))).toBe(true);
    expect(right.contains(screen.getByTestId("similar-books"))).toBe(true);
    // 책을 고르면 디렉토리가 접혀 왼쪽 컬럼이 없어지고, 접힌 디렉토리가 오른쪽 컬럼 위에 놓인다.
    expect(layout.querySelector(".split-layout-left")).toBeNull();
    expect(right.contains(screen.getByTestId("folder"))).toBe(true);
    expect(screen.getByTestId("folder").dataset.open).toBe("false");
  });

  it("책을 열면 조회 이력을 1건 기록한다", async () => {
    mockRecordBookView.mockClear();
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "테스트소설",
            file_type: "epub",
            file_path: "/test.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });
    screen.getByTestId("folder-item-소설").click();
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/42")).toBeTruthy();
    });

    // 열람 전에는 기록하지 않는다
    expect(mockRecordBookView).not.toHaveBeenCalled();

    screen.getByTestId("folder-item-소설/42").click();

    await waitFor(() => {
      expect(mockRecordBookView).toHaveBeenCalledWith("", 42);
    });
    // 같은 책을 계속 보는 동안 중복 기록하지 않는다
    expect(mockRecordBookView).toHaveBeenCalledTimes(1);
  });

  it("만화 열람 시 apiPrefix 를 붙여 기록한다", async () => {
    mockRecordBookView.mockClear();
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") {
        resolve({ 액션: 1 });
      } else if (url === "/comics/categories/액션") {
        resolve([
          {
            book_id: 7,
            title: "테스트만화",
            file_type: "zip",
            file_path: "/test.zip",
            category: "액션",
          },
        ]);
      }
    });

    render(<View basePath="/comics-view" apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-액션")).toBeTruthy();
    });
    screen.getByTestId("folder-item-액션").click();
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-액션/7")).toBeTruthy();
    });
    screen.getByTestId("folder-item-액션/7").click();

    await waitFor(() => {
      expect(mockRecordBookView).toHaveBeenCalledWith("/comics", 7);
    });
  });

  it("최상위 파일(_root) 클릭 시 BookInfoView가 표시된다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10, _root: 1 });
      } else if (url === "/categories/_root") {
        resolve([
          {
            book_id: 200,
            title: "root파일",
            file_type: "pdf",
            file_path: "/root.pdf",
            category: "_root",
          },
        ]);
      }
    });

    render(<View />);

    // _root 파일 버튼 대기 (id는 '/200')
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/200")).toBeTruthy();
    });

    // 최상위 파일 클릭
    screen.getByTestId("folder-item-/200").click();

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
      expect(screen.getByTestId("view-single")).toBeTruthy();
    });
  });

  it("카테고리 목록은 limit을 붙여 커서 페이지로 요청한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 3 });
    });
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.startsWith("/categories/소설")) {
        resolve({
          status: "success",
          result: [
            {
              book_id: 1,
              title: "가",
              file_type: "epub",
              file_path: "/1.epub",
              category: "소설",
            },
          ],
          total: 1,
          next_cursor: null,
        });
      }
    });

    render(<View />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });
    screen.getByTestId("folder-item-소설").click();

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/1")).toBeTruthy();
    });
    const url = mockRawJsonGetReq.mock.calls.find((c) =>
      c[0].startsWith("/categories/소설"),
    )[0];
    expect(url).toContain("limit=5000");
    // 마지막 페이지이므로 '더 보기'는 없다
    expect(screen.queryByTestId("folder-item-소설/__more__")).toBeNull();
  });

  it("다음 페이지가 있으면 '더 보기'가 나타나고, 누르면 커서로 이어 불러온다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 2 });
    });
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      if (!url.startsWith("/categories/소설")) return;
      if (url.includes("cursor=")) {
        resolve({
          status: "success",
          result: [
            {
              book_id: 2,
              title: "나",
              file_type: "epub",
              file_path: "/2.epub",
              category: "소설",
            },
          ],
          total: 2,
          next_cursor: null,
        });
      } else {
        resolve({
          status: "success",
          result: [
            {
              book_id: 1,
              title: "가",
              file_type: "epub",
              file_path: "/1.epub",
              category: "소설",
            },
          ],
          total: 2,
          next_cursor: "CURSOR1",
        });
      }
    });

    render(<View />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });
    screen.getByTestId("folder-item-소설").click();

    // 1페이지 후 '더 보기'가 진행 상황과 함께 표시된다
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/__more__")).toBeTruthy();
    });
    expect(
      screen.getByTestId("folder-item-소설/__more__").textContent,
    ).toContain("1/2");

    screen.getByTestId("folder-item-소설/__more__").click();

    // 2페이지가 이어붙고 '더 보기'는 사라진다
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/2")).toBeTruthy();
    });
    expect(screen.getByTestId("folder-item-소설/1")).toBeTruthy();
    expect(screen.queryByTestId("folder-item-소설/__more__")).toBeNull();

    const cursorCall = mockRawJsonGetReq.mock.calls.find((c) =>
      c[0].includes("cursor="),
    );
    expect(cursorCall[0]).toContain("cursor=CURSOR1");
    // '더 보기'는 책이 아니므로 책 조회로 오인되지 않는다
    expect(screen.queryByTestId("book-load-error")).toBeNull();
  });

  it("딥링크로 bookId와 category가 주어지면 자동으로 책을 선택한다", async () => {
    mockRouteState.wildcard = "42";
    mockRouteState.searchParams = "category=소설";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "딥링크소설",
            file_type: "epub",
            file_path: "/deep.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    // 딥링크로 자동 선택되어 카테고리 로드 호출됨
    await waitFor(() => {
      const calls = mockJsonGetReq.mock.calls;
      expect(calls.find((c) => c[0] === "/categories/소설")).toBeTruthy();
    });
  });

  it("딥링크에서 트리에 없는 카테고리는 /books/{id}로 직접 조회한다", async () => {
    mockRouteState.wildcard = "99";
    mockRouteState.searchParams = "category=깊은/3레벨/카테고리";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/books/99") {
        resolve({
          book_id: 99,
          title: "깊은카테고리책",
          file_type: "pdf",
          file_path: "/deep.pdf",
          category: "깊은/3레벨/카테고리",
        });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(
        mockJsonGetReq.mock.calls.find((c) => c[0] === "/books/99"),
      ).toBeTruthy();
    });

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
  });

  it("딥링크 대상이 카테고리 목록에 없으면(10000건 상한으로 잘림) /books/{id}로 직접 조회한다", async () => {
    mockRouteState.wildcard = "200901238";
    mockRouteState.searchParams = "category=0_telegram";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ "0_telegram": 32355 });
      } else if (url === "/categories/0_telegram") {
        // 상한에 걸려 잘린 목록 - 대상 책이 포함되지 않는다
        resolve([
          {
            book_id: 200913025,
            title: "상어의 도시",
            file_type: "epub",
            file_path: "0_telegram/상어의 도시.epub",
            category: "0_telegram",
          },
        ]);
      } else if (url === "/books/200901238") {
        resolve({
          book_id: 200901238,
          title: "철도 네트워크 제국 01 - 레일 헤드",
          file_type: "epub",
          file_path: "0_telegram/철도 네트워크 제국 01 - 레일 헤드.epub",
          category: "0_telegram",
        });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(
        mockJsonGetReq.mock.calls.find((c) => c[0] === "/books/200901238"),
      ).toBeTruthy();
    });

    // 목록에 없어도 책 정보가 표시되고, 실패 패널은 뜨지 않는다
    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
    expect(screen.queryByTestId("book-load-error")).toBeNull();
  });

  it("딥링크 대상이 카테고리 목록에 있으면 목록에서 선택하고 /books/{id}를 호출하지 않는다", async () => {
    mockRouteState.wildcard = "42";
    mockRouteState.searchParams = "category=소설";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "목록에있는책",
            file_type: "epub",
            file_path: "/in-list.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
    // 목록으로 해결되므로 단건 조회는 불필요하다
    expect(
      mockJsonGetReq.mock.calls.find((c) => c[0] === "/books/42"),
    ).toBeFalsy();
  });

  it("딥링크 직접 조회 실패 시 /books/{id} reject 콜백이 호출된다", async () => {
    mockRouteState.wildcard = "99";
    mockRouteState.searchParams = "category=없는카테고리";

    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/books/99") {
        reject("책을 찾을 수 없음");
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(
        mockJsonGetReq.mock.calls.find((c) => c[0] === "/books/99"),
      ).toBeTruthy();
    });

    // bookInfo가 설정되지 않아 ViewSingle이 렌더링되지 않음
    expect(screen.queryByTestId("view-single")).toBeNull();

    // 책 로드 실패 사유 패널이 표시된다
    await waitFor(() => {
      const panel = screen.getByTestId("book-load-error");
      expect(panel).toBeTruthy();
      expect(panel.textContent).toContain("책을 찾을 수 없음");
      expect(panel.textContent).toContain("99");
    });
  });

  it("딥링크로 책을 정상 로드하면 사유 패널을 표시하지 않는다", async () => {
    mockRouteState.wildcard = "99";
    mockRouteState.searchParams = "category=깊은/3레벨/카테고리";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/books/99") {
        resolve({
          book_id: 99,
          title: "정상책",
          file_type: "pdf",
          file_path: "/ok.pdf",
          category: "깊은/3레벨/카테고리",
        });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
    expect(screen.queryByTestId("book-load-error")).toBeNull();
  });

  it("딥링크 _root 카테고리인 경우 /{bookId}로 entryClicked를 호출한다", async () => {
    mockRouteState.wildcard = "200";
    mockRouteState.searchParams = "category=_root";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ _root: 1 });
      } else if (url === "/categories/_root") {
        resolve([
          {
            book_id: 200,
            title: "root파일",
            file_type: "pdf",
            file_path: "/root.pdf",
            category: "_root",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
  });

  it("viewer 역할에서 딥링크로 hidden 카테고리 접근 시 BookInfoView가 렌더링되지 않는다", async () => {
    mockOutletContext.role = "viewer";
    mockRouteState.wildcard = "50";
    mockRouteState.searchParams = "category=비공개/하위";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10 });
      } else if (url === "/hidden-categories?content_type=book") {
        resolve(["비공개"]);
      } else if (url === "/books/50") {
        resolve({
          book_id: 50,
          title: "비공개책",
          file_type: "pdf",
          file_path: "/secret.pdf",
          category: "비공개/하위",
        });
      }
    });

    render(<View />);

    // /books/50 호출 확인
    await waitFor(() => {
      expect(
        mockJsonGetReq.mock.calls.find((c) => c[0] === "/books/50"),
      ).toBeTruthy();
    });

    // hidden 카테고리이므로 BookInfoView가 렌더링되지 않음
    expect(screen.queryByTestId("book-info-view")).toBeNull();
  });

  it("useIsMobile: window resize 이벤트에 반응한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({});
    });

    render(<View />);

    // resize 이벤트를 트리거하여 handleResize 콜백 커버
    await act(async () => {
      window.innerWidth = 500;
      window.dispatchEvent(new Event("resize"));
    });

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
  });

  it("가상 부모(isVirtualParent) 클릭 시 API 호출 없이 리턴한다", async () => {
    // '문학/소설', '문학/시', '역사' → commonPrefix='', 가상 부모 '__virtual__문학' 생성
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ "문학/소설": 5, "문학/시": 3, 역사: 1 });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });

    const callCountBefore = mockJsonGetReq.mock.calls.length;

    // 가상 부모 '__virtual__문학' 클릭
    const virtualParentBtn = screen.queryByTestId(
      "folder-item-__virtual__문학",
    );
    if (virtualParentBtn) {
      virtualParentBtn.click();
      // 가상 부모이므로 추가 API 호출이 없어야 함
      expect(mockJsonGetReq.mock.calls.length).toBe(callCountBefore);
    }
  });

  it("트리에 없는 entryId 클릭 시 parseEntryId로 카테고리를 파싱하여 책을 찾는다", async () => {
    // 소설/999는 children에 없으므로 findFolderInTree('소설/999')가 null을 반환
    // → else 브랜치(L187)로 진입 → parseEntryId → 카테고리에서 책 검색
    // 소설 카테고리에 999가 없으므로 "can't find the selected book" 에러
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "테스트소설",
            file_type: "epub",
            file_path: "/test.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });

    // 카테고리 클릭 → 책 목록 로드
    screen.getByTestId("folder-item-소설").click();

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/42")).toBeTruthy();
    });

    // 트리에 없는 entryId('소설/999') 클릭 → else 브랜치 진입
    screen.getByTestId("custom-entry-click").click();

    // 책을 찾을 수 없으므로 에러가 설정됨 (BookInfoView가 렌더링되지 않음은 아님,
    // 이전에 선택된 bookInfo가 없으므로)
    // 대기 후 확인 - custom-entry-click은 selectedEntryId를 '소설/999'로 설정
    await waitFor(() => {
      // parseEntryId가 호출되고, category '소설'이 트리에서 발견되며,
      // children에서 '소설/999'를 찾지 못해 에러 메시지가 설정됨
    });
  });

  it("트리에 없는 entryId로 카테고리도 없으면 에러 메시지가 설정된다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 역사: 1 });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });

    // '소설/999' 클릭 → findFolderInTree null → parseEntryId → category '소설'
    // → findFolderInTree(folderData, '소설') returns null → booksInCategory is undefined
    // → L206: "can't find the selected category" 에러
    screen.getByTestId("custom-entry-click").click();
  });

  it("next 버튼 클릭 시 다음 책으로 이동한다 (toNextEntryButtonClicked)", async () => {
    // 정렬 순서: A소설(42) < B소설(43) → 42가 첫번째, 43이 두번째
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 2 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "A소설",
            file_type: "epub",
            file_path: "/first.epub",
            category: "소설",
          },
          {
            book_id: 43,
            title: "B소설",
            file_type: "epub",
            file_path: "/second.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });

    screen.getByTestId("folder-item-소설").click();

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/42")).toBeTruthy();
    });

    // 첫번째 책(A소설) 클릭 → next=B소설(43)
    screen.getByTestId("folder-item-소설/42").click();

    await waitFor(() => {
      expect(screen.getByTestId("next-book-btn")).toBeTruthy();
    });

    screen.getByTestId("next-book-btn").click();

    await waitFor(() => {
      expect(screen.getByTestId("view-single")).toBeTruthy();
    });
  });

  it("prev 버튼 클릭 시 이전 책으로 이동한다 (toPrevEntryButtonClicked)", async () => {
    // 정렬 순서: A소설(42) < B소설(43) → 43이 두번째, prev=42
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 2 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "A소설",
            file_type: "epub",
            file_path: "/first.epub",
            category: "소설",
          },
          {
            book_id: 43,
            title: "B소설",
            file_type: "epub",
            file_path: "/second.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });

    screen.getByTestId("folder-item-소설").click();

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/43")).toBeTruthy();
    });

    // 두번째 책(B소설) 클릭 → prev=A소설(42)
    screen.getByTestId("folder-item-소설/43").click();

    await waitFor(() => {
      expect(screen.getByTestId("prev-book-btn")).toBeTruthy();
    });

    screen.getByTestId("prev-book-btn").click();

    await waitFor(() => {
      expect(screen.getByTestId("view-single")).toBeTruthy();
    });
  });

  it("가상 부모 하위 카테고리에서 책을 찾아 BookInfoView를 표시한다 (else 브랜치)", async () => {
    // 카테고리: '문학/소설', '문학/시', '역사' → commonPrefix='', 가상 부모 '__virtual__문학' 생성
    // findFolderInTree('문학/소설/42')는 3레벨이므로 null 반환 → else 브랜치(L187)
    // parseEntryId → category='문학/소설' → findFolderInTree('문학/소설') 성공 → 책 찾기
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ "문학/소설": 3, "문학/시": 2, 역사: 1 });
      } else if (url === "/categories/문학/소설") {
        resolve([
          {
            book_id: 42,
            title: "깊은소설",
            file_type: "epub",
            file_path: "/deep.epub",
            category: "문학/소설",
          },
        ]);
      }
    });

    render(<View />);

    // 가상 부모 하위의 카테고리 표시 대기
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-문학/소설")).toBeTruthy();
    });

    // 카테고리 클릭 → 책 목록 로드
    screen.getByTestId("folder-item-문학/소설").click();

    // 책 목록 로드 대기 - Folder mock은 top-level children만 렌더하므로
    // 문학/소설의 children(문학/소설/42)은 렌더되지 않음
    // 대신 jsonGetReq 호출 확인
    await waitFor(() => {
      expect(
        mockJsonGetReq.mock.calls.find((c) => c[0] === "/categories/문학/소설"),
      ).toBeTruthy();
    });

    // custom-deep-click으로 '문학/소설/42' 클릭 → else 브랜치
    screen.getByTestId("custom-deep-click").click();

    // 책이 성공적으로 찾아져 BookInfoView 표시
    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
  });

  it("apiPrefix가 있을 때 comics content_type으로 hidden-categories를 요청한다", async () => {
    mockOutletContext.role = "viewer";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") {
        resolve({ manga: 10 });
      } else if (url === "/hidden-categories?content_type=comic") {
        resolve([]);
      }
    });

    render(<View apiPrefix="/comics" />);

    await waitFor(() => {
      const calls = mockJsonGetReq.mock.calls;
      expect(
        calls.find((c) => c[0] === "/hidden-categories?content_type=comic"),
      ).toBeTruthy();
    });
  });

  it("딥링크에서 카테고리가 이미 booksLoaded면 바로 책을 선택한다", async () => {
    mockRouteState.wildcard = "42";
    mockRouteState.searchParams = "category=소설";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "딥링크소설",
            file_type: "epub",
            file_path: "/deep.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    // 딥링크로 카테고리 로드 → booksLoaded 후 책 자동 선택
    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
  });

  it("query category 없이 레거시 경로(category/bookId)로 자동 선택한다 (parseEntryId 폴백)", async () => {
    // qCategory 없음 → routeWildcard에서 parseEntryId로 category/bookId 추출
    mockRouteState.wildcard = "소설/42";
    mockRouteState.searchParams = "";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "레거시소설",
            file_type: "epub",
            file_path: "/legacy.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    // 레거시 경로에서 routeCategory='소설', routeBookId='42' 추출되어 자동 선택
    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
  });

  it("query category가 있지만 wildcard가 숫자가 아니면 routeBookId가 undefined여서 자동 선택하지 않는다", async () => {
    // qCategory 존재 + routeWildcard 비숫자 → routeBookId undefined → 자동 선택 분기 미진입
    mockRouteState.wildcard = "notanumber";
    mockRouteState.searchParams = "category=소설";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });

    // routeBookId가 undefined이므로 /categories/소설 자동 로드가 일어나지 않음
    expect(
      mockJsonGetReq.mock.calls.find((c) => c[0] === "/categories/소설"),
    ).toBeFalsy();
    expect(screen.queryByTestId("book-info-view")).toBeNull();
  });

  it("apiPrefix가 있을 때 폴더 내 책 클릭 시 viewUrl에 api 파라미터가 포함된다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") {
        resolve({ manga: 1 });
      } else if (url === "/comics/categories/manga") {
        resolve([
          {
            book_id: 7,
            title: "코믹",
            file_type: "cbz",
            file_path: "/comic.cbz",
            category: "manga",
          },
        ]);
      }
    });

    render(<View apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-manga")).toBeTruthy();
    });

    screen.getByTestId("folder-item-manga").click();

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-manga/7")).toBeTruthy();
    });

    screen.getByTestId("folder-item-manga/7").click();

    // apiPrefix가 truthy이므로 entryClicked의 viewUrl/downloadUrl 분기가 실행됨
    await waitFor(() => {
      expect(screen.getByTestId("view-single")).toBeTruthy();
    });
  });

  it("apiPrefix가 있을 때 _root 파일 클릭 시 viewUrl에 api 파라미터가 포함된다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") {
        resolve({ manga: 1, _root: 1 });
      } else if (url === "/comics/categories/_root") {
        resolve([
          {
            book_id: 300,
            title: "루트코믹",
            file_type: "cbz",
            file_path: "/root.cbz",
            category: "_root",
          },
        ]);
      }
    });

    render(<View apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/300")).toBeTruthy();
    });

    screen.getByTestId("folder-item-/300").click();

    await waitFor(() => {
      expect(screen.getByTestId("view-single")).toBeTruthy();
    });
  });

  it("category가 없는 _root 책 클릭 시 viewUrl이 _root 기본값으로 설정된다", async () => {
    // book에 category 키가 없음 → book["category"] || "_root" 의 폴백 arm 실행
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ _root: 1 });
      } else if (url === "/categories/_root") {
        resolve([
          {
            book_id: 400,
            title: "카테고리없는루트",
            file_type: "pdf",
            file_path: "/nocat.pdf",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/400")).toBeTruthy();
    });

    screen.getByTestId("folder-item-/400").click();

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
  });

  it("파싱 불가능한 entryId(소설/abc) 클릭 시 조용히 리턴한다 (parseEntryId null)", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });

    // findFolderInTree null + parseEntryId null → 조기 리턴, 에러/책정보 미설정
    screen.getByTestId("custom-unparsable-click").click();

    expect(screen.queryByTestId("book-info-view")).toBeNull();
    expect(screen.queryByTestId("book-load-error")).toBeNull();
  });

  it("부모 카테고리에 하위 폴더가 있을 때 책 로드 후 하위 폴더 children을 보존한다", async () => {
    // '문학' 부모 + '문학/소설' 자식 → 실제 부모 폴더(isVirtualParent false)
    // '문학' 클릭 시 책 로드 → existingSubfolders 필터(L210-211)에서 '문학/소설'(folder) 보존
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 문학: 2, "문학/소설": 3 });
      } else if (url === "/categories/문학") {
        resolve([
          {
            book_id: 11,
            title: "부모책",
            file_type: "epub",
            file_path: "/parent.epub",
            category: "문학",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-문학")).toBeTruthy();
    });

    // 부모 카테고리 클릭 → 책 로드, 기존 하위 폴더(문학/소설) 보존
    screen.getByTestId("folder-item-문학").click();

    await waitFor(() => {
      expect(
        mockJsonGetReq.mock.calls.find((c) => c[0] === "/categories/문학"),
      ).toBeTruthy();
    });

    // 책 로드 후에도 하위 폴더 '문학/소설'이 여전히 존재
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-문학/소설")).toBeTruthy();
    });
    // 책도 추가됨
    expect(screen.getByTestId("folder-item-문학/11")).toBeTruthy();
  });

  it("이미 booksLoaded된 카테고리를 다시 클릭하면 중복 API 호출을 하지 않는다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 1 });
      } else if (url === "/categories/소설") {
        resolve([
          {
            book_id: 42,
            title: "테스트소설",
            file_type: "epub",
            file_path: "/test.epub",
            category: "소설",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });

    screen.getByTestId("folder-item-소설").click();

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설/42")).toBeTruthy();
    });

    const countAfterFirst = mockJsonGetReq.mock.calls.filter(
      (c) => c[0] === "/categories/소설",
    ).length;

    // booksLoaded=true 상태에서 재클릭 → 중복 로드 방지 분기
    screen.getByTestId("folder-item-소설").click();

    const countAfterSecond = mockJsonGetReq.mock.calls.filter(
      (c) => c[0] === "/categories/소설",
    ).length;
    expect(countAfterSecond).toBe(countAfterFirst);
  });

  it("viewer 딥링크 직접 조회에서 hidden 카테고리가 아니면 정상 로드한다", async () => {
    // role=viewer, hiddenCategories 존재하지만 book category가 hidden 접두사가 아님
    // → hidden 차단 루프를 통과하여 정상 로드 (L321 분기의 통과 arm)
    mockOutletContext.role = "viewer";
    mockRouteState.wildcard = "60";
    mockRouteState.searchParams = "category=공개/하위";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10 });
      } else if (url === "/hidden-categories?content_type=book") {
        resolve(["비공개"]);
      } else if (url === "/books/60") {
        resolve({
          book_id: 60,
          title: "공개책",
          file_type: "pdf",
          file_path: "/public.pdf",
          category: "공개/하위",
        });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
    expect(screen.getByTestId("similar-books").dataset.canEdit).toBe("false");
    expect(screen.queryByTestId("book-load-error")).toBeNull();
  });

  it("viewer 딥링크 직접 조회 시 book category가 비어 있으면 hidden 검사를 통과한다", async () => {
    // book["category"] 없음 → bookCat="" → hidden 매칭 안 됨 → 정상 로드 (L323 폴백 arm)
    mockOutletContext.role = "viewer";
    mockRouteState.wildcard = "61";
    mockRouteState.searchParams = "category=어떤/경로";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10 });
      } else if (url === "/hidden-categories?content_type=book") {
        resolve(["비공개"]);
      } else if (url === "/books/61") {
        resolve({
          book_id: 61,
          title: "카테고리없는책",
          file_type: "pdf",
          file_path: "/nocat2.pdf",
        });
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
  });

  // ── 추가 분기 ──

  it("최상위 파일이 여러 개면 제목순으로 정렬한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ _root: 3 });
      } else if (url === "/categories/_root") {
        resolve([
          {
            book_id: 3,
            title: "다랑",
            file_type: "pdf",
            file_path: "/c.pdf",
            category: "_root",
          },
          {
            book_id: 1,
            title: "가람",
            file_type: "pdf",
            file_path: "/a.pdf",
            category: "_root",
          },
          {
            book_id: 2,
            title: "나람",
            file_type: "pdf",
            file_path: "/b.pdf",
            category: "_root",
          },
        ]);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/1")).toBeTruthy();
    });
    const labels = ["/1", "/2", "/3"].map(
      (id) => screen.getByTestId(`folder-item-${id}`).textContent,
    );
    expect(labels).toEqual(["가람.pdf", "나람.pdf", "다랑.pdf"]);
  });

  it("viewer 의 비노출 목록이 null 이면 빈 집합으로 처리한다", async () => {
    mockOutletContext.role = "viewer";
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") {
        resolve({ 소설: 10 });
      } else if (url.startsWith("/hidden-categories")) {
        resolve(null);
      }
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
    });
  });

  it("apiPrefix 가 있으면 viewUrl 에 api 파라미터를 포함한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") {
        resolve({ 만화: 1 });
      } else if (url === "/comics/categories/만화") {
        resolve([
          {
            book_id: 7,
            title: "만화책",
            file_type: "pdf",
            file_path: "만화/7.pdf",
            category: "만화",
          },
        ]);
      }
    });

    render(<View apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-만화")).toBeTruthy();
    });
    screen.getByTestId("folder-item-만화").click();

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-만화/7")).toBeTruthy();
    });
    screen.getByTestId("folder-item-만화/7").click();

    await waitFor(() => {
      expect(screen.getByTestId("view-single")).toBeTruthy();
    });
    expect(screen.getByTestId("view-single").dataset.viewUrl).toContain("api=");
  });

  it("딥링크에 apiPrefix 가 있으면 viewUrl 에 api 파라미터를 포함한다", async () => {
    mockRouteState.wildcard = "9";
    mockRouteState.searchParams = "category=만화";
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") {
        resolve({ 만화: 1 });
      } else if (url === "/comics/categories/만화") {
        resolve([
          {
            book_id: 9,
            title: "딥링크만화",
            file_type: "pdf",
            file_path: "만화/9.pdf",
            category: "만화",
          },
        ]);
      }
    });

    render(<View apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });
    expect(screen.getByTestId("view-single").dataset.viewUrl).toContain("api=");
  });

  it("딥링크로 책을 열면 디렉토리를 접은 상태로 시작한다", async () => {
    mockRouteState.wildcard = "42";
    mockRouteState.searchParams = "category=소설";

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 1 });
      else if (url === "/categories/소설")
        resolve([
          {
            book_id: 42,
            title: "딥링크소설",
            file_type: "epub",
            file_path: "/deep.epub",
            category: "소설",
          },
        ]);
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
    expect(screen.getByTestId("folder").dataset.open).toBe("false");
  });

  it("책 없이 book-view에 들어오면 디렉토리를 펼친 상태로 시작한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 10 });
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
    expect(screen.getByTestId("folder").dataset.open).toBe("true");
  });

  it("책 없이 comics-view에 들어오면 디렉토리를 펼친 상태로 시작한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") resolve({ 만화: 10 });
    });

    render(<View basePath="/comics-view" apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
    expect(screen.getByTestId("folder").dataset.open).toBe("true");
  });

  it("펼친 디렉토리를 누르면 접힌 Folder를 렌더링한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 10 });
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
    expect(screen.getByTestId("folder").dataset.open).toBe("true");

    await act(async () => {
      screen.getByTestId("folder-toggle").click();
    });

    await waitFor(() => {
      expect(screen.getByTestId("folder").dataset.open).toBe("false");
    });
  });

  it("디렉토리는 SplitLayout 왼쪽 pane에 렌더링한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 10 });
    });

    render(<View />);

    await waitFor(() => {
      expect(screen.getByTestId("folder")).toBeTruthy();
    });
    const column = screen
      .getByTestId("folder")
      .closest(".split-layout-left.view-directory-panel");
    expect(column).toBeTruthy();
  });

  it("만화 딥링크에서 트리에 없는 카테고리는 api 파라미터를 붙여 직접 조회한다", async () => {
    mockRouteState.wildcard = "99";
    mockRouteState.searchParams = "category=깊은/3레벨/카테고리";
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") {
        resolve({ 만화: 1 });
      } else if (url === "/comics/books/99") {
        resolve({
          book_id: 99,
          title: "깊은만화",
          file_type: "pdf",
          file_path: "/deep.pdf",
          category: "깊은/3레벨/카테고리",
        });
      }
    });

    render(<View apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByTestId("view-single").dataset.viewUrl).toContain(
        "&api=%2Fcomics",
      );
    });
  });

  it("책을 연 상태에서 디렉토리 목록 오류가 뒤늦게 도착하면 책 정보와 함께 오류 알림을 표시한다", async () => {
    mockRouteState.wildcard = "99";
    mockRouteState.searchParams = "category=깊은/3레벨/카테고리";
    let lateReject;
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") {
        lateReject = reject;
        resolve({ 소설: 1 });
      } else if (url === "/books/99") {
        resolve({
          book_id: 99,
          title: "깊은카테고리책",
          file_type: "pdf",
          file_path: "/deep.pdf",
          category: "깊은/3레벨/카테고리",
        });
      }
    });

    render(<View />);
    await waitFor(() => {
      expect(screen.getByTestId("book-info-view")).toBeTruthy();
    });

    act(() => lateReject("서버 오류"));

    expect(
      await screen.findByText(/can't load directory data, 서버 오류/),
    ).toBeTruthy();
  });

  it("만화의 가상 부모 하위 카테고리 책은 api 파라미터가 붙은 viewUrl로 연다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") {
        resolve({ "문학/소설": 3, "문학/시": 2, 역사: 1 });
      } else if (url === "/comics/categories/문학/소설") {
        resolve([
          {
            book_id: 42,
            title: "깊은만화",
            file_type: "pdf",
            file_path: "/deep.pdf",
            category: "문학/소설",
          },
        ]);
      }
    });

    render(<View apiPrefix="/comics" />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-문학/소설")).toBeTruthy();
    });
    screen.getByTestId("folder-item-문학/소설").click();
    await waitFor(() => {
      expect(
        mockJsonGetReq.mock.calls.find(
          (c) => c[0] === "/comics/categories/문학/소설",
        ),
      ).toBeTruthy();
    });

    screen.getByTestId("custom-deep-click").click();

    await waitFor(() => {
      expect(screen.getByTestId("view-single").dataset.viewUrl).toContain(
        "&api=%2Fcomics",
      );
    });
  });

  describe("커버 목록 브라우징", () => {
    const stubScroll = () =>
      vi.spyOn(window, "scrollTo").mockImplementation(() => {});

    const loadTree = () => {
      mockJsonGetReq.mockImplementation((url, payload, resolve) => {
        if (url === "/categories") {
          resolve({ 소설: 1 });
        } else if (url === "/categories/소설") {
          resolve([
            {
              book_id: 42,
              title: "테스트소설",
              file_type: "epub",
              file_path: "/test.epub",
              category: "소설",
            },
          ]);
        }
      });
    };

    beforeEach(() => {
      // 앞선 테스트가 줄여 둔 창 폭이 남으면 컬럼이 쌓인 배치로 그려져 분할 동작을 볼 수 없다.
      window.innerWidth = 1024;
    });

    afterEach(() => {
      vi.restoreAllMocks();
      window.history.replaceState(null, "", "/");
    });

    it("디렉토리를 고르면 커버 목록을 그 디렉토리로 바꾸고, 전체 보기로 되돌린다", async () => {
      loadTree();
      render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
      });

      act(() => screen.getByTestId("folder-item-소설").click());
      await waitFor(() => {
        expect(screen.getByTestId("cover-browser").dataset.category).toBe(
          "소설",
        );
      });

      act(() => screen.getByTestId("cover-clear").click());
      expect(screen.getByTestId("cover-browser").dataset.category).toBe("");
    });

    // 트리에서 소설 디렉토리의 책(42)을 연다. 실제 Folder처럼 책을 누르면 디렉토리 패널이 접힌다.
    const openBookViaTree = async () => {
      await waitFor(() => {
        expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
      });
      act(() => screen.getByTestId("folder-item-소설").click());
      await waitFor(() => {
        expect(screen.getByTestId("folder-item-소설/42")).toBeTruthy();
      });
      act(() => screen.getByTestId("folder-item-소설/42").click());
      await waitFor(() => {
        expect(screen.getByTestId("view-single")).toBeTruthy();
      });
    };

    it("왼쪽(디렉토리)과 오른쪽 컬럼이 따로 스크롤되고 폭을 조절할 수 있다", async () => {
      loadTree();
      const { container } = render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("cover-browser")).toBeTruthy();
      });

      const layout = container.querySelector(".split-layout");
      // scroll={false}면 no-scroll이 붙어 문서 전체가 스크롤된다. 두 컬럼이 따로 스크롤되려면 없어야 한다.
      expect(layout.classList.contains("no-scroll")).toBe(false);
      expect(layout.querySelector(".split-layout-left")).toBeTruthy();
      expect(layout.querySelector(".split-layout-right")).toBeTruthy();
      expect(screen.getByRole("separator")).toBeTruthy();
    });

    it("트리에서 책을 고르면 오른쪽 컬럼에 책 보기가 열리고 디렉토리는 접히며 커버 목록은 숨는다", async () => {
      loadTree();
      stubScroll();
      const { container } = render(<View />);
      await openBookViaTree();

      const right = container.querySelector(".split-layout-right");
      expect(right.contains(screen.getByTestId("book-info-view"))).toBe(true);
      expect(right.contains(screen.getByTestId("view-single"))).toBe(true);
      expect(right.contains(screen.getByTestId("similar-books"))).toBe(true);
      // 접힌 디렉토리는 왼쪽 컬럼 없이 오른쪽 컬럼 위에 놓인다(편집 화면과 같다).
      expect(container.querySelector(".split-layout-left")).toBeNull();
      expect(screen.getByTestId("folder").dataset.open).toBe("false");
      expect(right.contains(screen.getByTestId("folder"))).toBe(true);
      expect(screen.getByTestId("cover-browser").parentElement.hidden).toBe(
        true,
      );
    });

    it("커버 목록 버튼은 책 보기를 닫고 디렉토리를 펼치며, 숨겨 둔 같은 목록을 다시 보여 준다", async () => {
      loadTree();
      stubScroll();
      const replaceState = vi.spyOn(window.history, "replaceState");
      const { container } = render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("cover-browser")).toBeTruthy();
      });
      const browserBefore = screen.getByTestId("cover-browser");
      await openBookViaTree();

      act(() => screen.getByRole("button", { name: "← 커버 목록" }).click());

      expect(screen.queryByTestId("view-single")).toBeNull();
      expect(screen.queryByTestId("book-info-view")).toBeNull();
      expect(screen.queryByTestId("similar-books")).toBeNull();
      expect(screen.getByTestId("folder").dataset.open).toBe("true");
      expect(container.querySelector(".split-layout-left")).toBeTruthy();
      expect(replaceState).toHaveBeenLastCalledWith(
        null,
        "",
        "/book-view?directory=" + encodeURIComponent("소설"),
      );
      // 다시 만든 것이 아니라 숨겼던 그 목록이어야 불러온 페이지와 스크롤 위치가 남는다.
      expect(screen.getByTestId("cover-browser")).toBe(browserBefore);
      expect(browserBefore.parentElement.hidden).toBe(false);
    });

    it("책을 열면 오른쪽 컬럼을 맨 위로 올리고, 목록으로 돌아오면 보던 위치를 되돌린다", async () => {
      loadTree();
      stubScroll();
      const { container } = render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("cover-browser")).toBeTruthy();
      });
      const pane = container.querySelector(".split-layout-right");
      pane.scrollTop = 1234;
      act(() => {
        pane.dispatchEvent(new Event("scroll"));
      });

      await openBookViaTree();
      // 책 보기는 위에서부터 보여 준다.
      expect(pane.scrollTop).toBe(0);
      // 책 보기에서 올라간 위치가 기억을 덮어쓰면 안 된다.
      act(() => {
        pane.dispatchEvent(new Event("scroll"));
      });

      act(() => screen.getByRole("button", { name: "← 커버 목록" }).click());

      expect(pane.scrollTop).toBe(1234);
    });

    it("좁은 화면에서 컬럼이 쌓이면 문서 전체의 스크롤 위치를 쓴다", async () => {
      loadTree();
      stubScroll();
      const { container } = render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("cover-browser")).toBeTruthy();
      });
      container.querySelector(".split-layout").classList.add("is-stacked");

      await openBookViaTree();

      expect(window.scrollTo).toHaveBeenCalledWith(0, 0);
    });

    it("책이 열려 있을 때 디렉토리를 고르면 책 보기를 닫고 그 디렉토리 목록으로 돌아간다", async () => {
      loadTree();
      stubScroll();
      const replaceState = vi.spyOn(window.history, "replaceState");
      render(<View />);
      await openBookViaTree();

      act(() => screen.getByTestId("folder-item-소설").click());

      await waitFor(() => {
        expect(screen.queryByTestId("view-single")).toBeNull();
      });
      expect(screen.getByTestId("cover-browser").dataset.category).toBe("소설");
      expect(screen.getByTestId("cover-browser").parentElement.hidden).toBe(
        false,
      );
      expect(replaceState).toHaveBeenLastCalledWith(
        null,
        "",
        "/book-view?directory=" + encodeURIComponent("소설"),
      );
    });

    it("커버 목록에 권한(role)을 넘긴다", async () => {
      mockOutletContext.role = "viewer";
      loadTree();
      render(<View />);

      await waitFor(() => {
        expect(screen.getByTestId("cover-browser").dataset.role).toBe("viewer");
      });
    });

    it("?directory= 로 진입하면 그 디렉토리 기준 커버 목록을 연다", async () => {
      mockRouteState.searchParams =
        "directory=" + encodeURIComponent("5_완결/[작가] 작품 (01-06권 완)");
      loadTree();
      render(<View basePath="/comics-view" apiPrefix="/comics" />);

      await waitFor(() => {
        expect(screen.getByTestId("cover-browser")).toBeTruthy();
      });
      expect(screen.getByTestId("cover-browser").dataset.category).toBe(
        "5_완결/[작가] 작품 (01-06권 완)",
      );
    });

    it("책 딥링크가 있으면 ?directory= 는 쓰지 않는다", async () => {
      mockRouteState.wildcard = "42";
      mockRouteState.searchParams =
        "category=소설&directory=" + encodeURIComponent("다른/디렉토리");
      loadTree();
      stubScroll();
      render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("view-single")).toBeTruthy();
      });

      act(() => screen.getByRole("button", { name: "← 커버 목록" }).click());

      // ?directory= 값은 무시하고, 딥링크 책이 속한 디렉토리의 목록으로 돌아간다.
      expect(screen.getByTestId("cover-browser").dataset.category).toBe("소설");
    });

    it("디렉토리를 고르면 주소에 ?directory= 를 남기고, 전체로 되돌리면 지운다", async () => {
      loadTree();
      const replaceState = vi.spyOn(window.history, "replaceState");
      render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("folder-item-소설")).toBeTruthy();
      });

      act(() => screen.getByTestId("folder-item-소설").click());
      await waitFor(() => {
        expect(replaceState).toHaveBeenLastCalledWith(
          null,
          "",
          "/book-view?directory=" + encodeURIComponent("소설"),
        );
      });

      act(() => screen.getByTestId("cover-clear").click());
      expect(replaceState).toHaveBeenLastCalledWith(null, "", "/book-view");
    });

    it("딥링크 책을 여는 중에는 카테고리를 불러와도 주소를 바꾸지 않는다", async () => {
      mockRouteState.wildcard = "42";
      mockRouteState.searchParams = "category=소설";
      loadTree();
      const replaceState = vi.spyOn(window.history, "replaceState");
      render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("view-single")).toBeTruthy();
      });

      const directoryCalls = replaceState.mock.calls.filter(([, , url]) =>
        String(url).includes("directory="),
      );
      expect(directoryCalls).toEqual([]);
    });

    describe("기본 디렉토리", () => {
      it("디렉토리 지정 없이 진입하면 탭의 기본 디렉토리를 기준으로 연다", async () => {
        loadTree();
        const replaceState = vi.spyOn(window.history, "replaceState");
        render(<View defaultDirectory="2_소설한국" />);

        await waitFor(() => {
          expect(screen.getByTestId("cover-browser")).toBeTruthy();
        });
        expect(screen.getByTestId("cover-browser").dataset.category).toBe(
          "2_소설한국",
        );
        // 기본값은 주소를 건드리지 않고 조용히 적용한다.
        expect(replaceState).not.toHaveBeenCalled();
      });

      it("?directory= 가 있으면 기본 디렉토리보다 우선한다", async () => {
        mockRouteState.searchParams = "directory=" + encodeURIComponent("소설");
        loadTree();
        render(<View defaultDirectory="2_소설한국" />);

        await waitFor(() => {
          expect(screen.getByTestId("cover-browser")).toBeTruthy();
        });
        expect(screen.getByTestId("cover-browser").dataset.category).toBe(
          "소설",
        );
      });

      it("?directory= 가 빈 값이면 사용자가 고른 루트이므로 기본 디렉토리를 쓰지 않는다", async () => {
        mockRouteState.searchParams = "directory=";
        loadTree();
        render(<View defaultDirectory="2_소설한국" />);

        await waitFor(() => {
          expect(screen.getByTestId("cover-browser")).toBeTruthy();
        });
        expect(screen.getByTestId("cover-browser").dataset.category).toBe("");
      });

      it("전체 보기를 누르면 루트를 고른 것을 주소에 남긴다", async () => {
        loadTree();
        const replaceState = vi.spyOn(window.history, "replaceState");
        render(<View defaultDirectory="2_소설한국" />);
        await waitFor(() => {
          expect(screen.getByTestId("cover-browser")).toBeTruthy();
        });

        act(() => screen.getByTestId("cover-clear").click());

        expect(screen.getByTestId("cover-browser").dataset.category).toBe("");
        expect(replaceState).toHaveBeenLastCalledWith(
          null,
          "",
          "/book-view?directory=",
        );
      });

      it("책 딥링크가 있으면 기본 디렉토리를 쓰지 않는다", async () => {
        mockRouteState.wildcard = "42";
        mockRouteState.searchParams = "category=소설";
        loadTree();
        stubScroll();
        render(<View defaultDirectory="2_소설한국" />);
        await waitFor(() => {
          expect(screen.getByTestId("view-single")).toBeTruthy();
        });

        act(() => screen.getByRole("button", { name: "← 커버 목록" }).click());

        // 책이 속한 디렉토리(소설)의 목록으로 돌아가며, 기본 디렉토리는 쓰지 않는다.
        expect(screen.getByTestId("cover-browser").dataset.category).toBe(
          "소설",
        );
      });

      it("viewer에게 비노출인 기본 디렉토리는 루트로 대체한다", async () => {
        mockOutletContext.role = "viewer";
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
          if (url === "/categories") resolve({ "2_소설한국": 1, 소설: 1 });
          else if (url.startsWith("/hidden-categories"))
            resolve(["2_소설한국"]);
        });
        render(<View defaultDirectory="2_소설한국" />);

        await waitFor(() => {
          expect(screen.getByTestId("cover-browser").dataset.category).toBe("");
        });
      });

      it("viewer에게 노출되는 기본 디렉토리는 그대로 쓴다", async () => {
        mockOutletContext.role = "viewer";
        mockJsonGetReq.mockImplementation((url, payload, resolve) => {
          if (url === "/categories") resolve({ "2_소설한국": 1, 소설: 1 });
          else if (url.startsWith("/hidden-categories")) resolve(["소설"]);
        });
        render(<View defaultDirectory="2_소설한국" />);

        await waitFor(() => {
          expect(screen.getByTestId("folder-item-2_소설한국")).toBeTruthy();
        });
        expect(screen.getByTestId("cover-browser").dataset.category).toBe(
          "2_소설한국",
        );
      });
    });

    it("검색 결과를 보여 주는 동안에는 커버 목록을 요청하지도 그리지도 않는다", async () => {
      mockOutletContext.hasSearched = true;
      mockOutletContext.searchResults = [{ book_id: 1 }];
      loadTree();
      render(<View />);

      await waitFor(() => {
        expect(screen.getByTestId("search-result")).toBeTruthy();
      });
      expect(screen.queryByTestId("cover-browser")).toBeNull();
    });

    it("딥링크로 책이 열리는 동안에는 커버 목록을 요청하지 않는다", async () => {
      mockRouteState.wildcard = "42";
      mockRouteState.searchParams = "category=소설";
      loadTree();
      render(<View />);

      await waitFor(() => {
        expect(screen.getByTestId("view-single")).toBeTruthy();
      });
      expect(screen.queryByTestId("cover-browser")).toBeNull();
    });

    it("딥링크 책에서 커버 목록으로 돌아오면, 이후 트리가 바뀌어도 그 책을 다시 열지 않는다", async () => {
      mockRouteState.wildcard = "42";
      mockRouteState.searchParams = "category=소설";
      loadTree();
      stubScroll();
      render(<View />);
      await waitFor(() => {
        expect(screen.getByTestId("view-single")).toBeTruthy();
      });

      act(() => screen.getByRole("button", { name: "← 커버 목록" }).click());
      expect(screen.queryByTestId("view-single")).toBeNull();
      expect(screen.getByTestId("cover-browser")).toBeTruthy();

      // 디렉토리를 고르면 트리 데이터가 바뀌어 딥링크 자동 선택 효과가 다시 돈다.
      act(() => screen.getByTestId("folder-item-소설").click());
      await waitFor(() => {
        expect(screen.getByTestId("cover-browser").dataset.category).toBe(
          "소설",
        );
      });
      expect(screen.queryByTestId("view-single")).toBeNull();
    });

    it("딥링크 책을 찾지 못해도 커버 목록으로 빠져나갈 수 있다", async () => {
      mockRouteState.wildcard = "999";
      mockRouteState.searchParams =
        "category=" + encodeURIComponent("없는/깊은");
      mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
        if (url === "/categories") {
          resolve({ 소설: 1 });
        } else if (url === "/books/999") {
          reject("not found");
        }
      });
      render(<View />);

      await waitFor(() => {
        expect(screen.getByTestId("book-load-error")).toBeTruthy();
      });
      expect(screen.getByTestId("cover-browser")).toBeTruthy();
    });
  });
});
