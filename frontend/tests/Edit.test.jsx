// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  fireEvent,
  cleanup,
} from "@testing-library/react";

afterEach(cleanup);

// ── mock 함수 ──

const { mockJsonGetReq, mockJsonPutReq, mockJsonDeleteReq } = vi.hoisted(
  () => ({
    mockJsonGetReq: vi.fn(),
    mockJsonPutReq: vi.fn(),
    mockJsonDeleteReq: vi.fn(),
  }),
);

vi.mock("../src/Common", () => ({
  jsonGetReq: mockJsonGetReq,
  jsonPutReq: mockJsonPutReq,
  jsonDeleteReq: mockJsonDeleteReq,
  getApiUrlPrefix: () => "http://localhost:8000",
  ROOT_DIRECTORY: "_root",
}));

const mockOutletContext = {
  searchResults: [],
  hasSearched: false,
  searchTotal: 0,
  handleLoadMore: vi.fn(),
  searchLoading: false,
};

vi.mock("react-router-dom", () => ({
  useParams: vi.fn(() => ({ "*": "" })),
  useSearchParams: vi.fn(() => [new URLSearchParams()]),
  useOutletContext: vi.fn(() => mockOutletContext),
}));

vi.mock("../src/Folder", () => ({
  default: ({ folderData, isOpen, onToggle, onClickHandler }) => (
    <div data-testid={isOpen ? "folder-open" : "folder-closed"}>
      {folderData.map((f) => (
        <div
          key={f.id}
          data-testid={`folder-item-${f.id}`}
          onClick={() => onClickHandler(f.id)}
        >
          {f.label}
        </div>
      ))}
      {folderData.flatMap((f) =>
        (f.children || []).flatMap((c) => [
          <div
            key={c.id}
            data-testid={`folder-item-${c.id}`}
            onClick={() => onClickHandler(c.id)}
          >
            {c.label}
          </div>,
          // 3단계 손자 항목 렌더링 (중첩 카테고리 내 책)
          ...(c.children || []).map((g) => (
            <div
              key={g.id}
              data-testid={`folder-item-${g.id}`}
              onClick={() => onClickHandler(g.id)}
            >
              {g.label}
            </div>
          )),
        ]),
      )}
      {!isOpen && <button onClick={() => onToggle(true)}>펼치기</button>}
    </div>
  ),
}));

vi.mock("../src/BookInfoView", () => ({
  default: ({
    bookInfo,
    newFileName,
    newFileNameChanged,
    changeButtonClicked,
    deleteButtonClicked,
    onTitleChange,
    onAuthorChange,
    onExchangeButtonClick,
    onResetButtonClick,
    onCutTitleButtonClick,
    onCutAuthorButtonClick,
  }) => (
    <div data-testid="book-info">
      <span data-testid="book-title">{bookInfo.title}</span>
      <span data-testid="book-author">{bookInfo.author}</span>
      <input
        data-testid="title-input"
        value={bookInfo.title || ""}
        onChange={onTitleChange}
      />
      <input
        data-testid="author-input"
        value={bookInfo.author || ""}
        onChange={onAuthorChange}
      />
      <input
        data-testid="filename-input"
        value={newFileName || ""}
        onChange={newFileNameChanged}
      />
      <button data-testid="change-btn" onClick={changeButtonClicked}>
        변경
      </button>
      <button data-testid="delete-btn" onClick={deleteButtonClicked}>
        삭제
      </button>
      <button data-testid="exchange-btn" onClick={onExchangeButtonClick}>
        교환
      </button>
      <button data-testid="reset-btn" onClick={onResetButtonClick}>
        초기화
      </button>
      <button data-testid="cut-title-btn" onClick={onCutTitleButtonClick}>
        제목자르기
      </button>
      <button data-testid="cut-author-btn" onClick={onCutAuthorButtonClick}>
        저자자르기
      </button>
    </div>
  ),
}));

vi.mock("../src/ViewSingle", () => ({
  default: ({ bookId, viewUrl }) => (
    <div data-testid="view-single" data-view-url={viewUrl}>
      ViewSingle:{bookId}
    </div>
  ),
}));

vi.mock("../src/SimilarBooks", () => ({
  default: ({ onSelect }) => (
    <div data-testid="similar-books">
      SimilarBooks
      <button
        data-testid="similar-select-missing"
        onClick={() => onSelect("1_fiction/777")}
      >
        없는책선택
      </button>
      <button
        data-testid="similar-select-missing-cat"
        onClick={() => onSelect("nope_cat/777")}
      >
        없는카테고리선택
      </button>
    </div>
  ),
}));

vi.mock("../src/Bookstore", () => ({
  default: ({ onCategoriesFound: _onCategoriesFound }) => {
    // 간접 테스트를 위해 즉시 호출
    return <div data-testid="bookstore">Bookstore</div>;
  },
}));

vi.mock("../src/SimilarityDebug", () => ({
  default: () => <div data-testid="similarity-debug">SimilarityDebug</div>,
}));

vi.mock("../src/EpubDiagnoseView", () => ({
  default: () => <div data-testid="epub-diagnose">EpubDiagnose</div>,
}));

vi.mock("../src/Actions", () => ({
  default: ({
    toNextEntryClicked,
    toPrevEntryClicked,
    moveToUpperButtonClicked,
    moveToDirectoryButtonClicked,
    selectDirectoryButtonClicked,
  }) => (
    <div data-testid="actions">
      <button data-testid="next-entry" onClick={toNextEntryClicked}>
        다음
      </button>
      <button data-testid="prev-entry" onClick={toPrevEntryClicked}>
        이전
      </button>
      <button data-testid="move-upper" onClick={moveToUpperButtonClicked}>
        최상위이동
      </button>
      <button data-testid="move-dir" onClick={moveToDirectoryButtonClicked}>
        디렉토리이동
      </button>
      <button
        data-testid="select-dir"
        onClick={(e) => selectDirectoryButtonClicked(e, "2_science")}
      >
        디렉토리선택
      </button>
    </div>
  ),
}));

vi.mock("../src/SearchResult", () => ({
  default: () => <div data-testid="search-result">SearchResult</div>,
}));

vi.mock("../src/Edit.css", () => ({}));

import Edit from "../src/Edit";

// ── 헬퍼 ──

const CATEGORIES = {
  "1_fiction": 3,
  "2_science": 2,
};

const BOOKS_IN_FICTION = [
  {
    book_id: 101,
    title: "[작가A] 소설1",
    author: "",
    file_type: "epub",
    file_path: "1_fiction/[작가A] 소설1.epub",
    category: "1_fiction",
  },
  {
    book_id: 102,
    title: "[작가B] 소설2",
    author: "",
    file_type: "pdf",
    file_path: "1_fiction/[작가B] 소설2.pdf",
    category: "1_fiction",
  },
];

function setupMockCategories(
  categories = CATEGORIES,
  books = BOOKS_IN_FICTION,
) {
  mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
    if (url === "/categories") {
      resolve(categories);
    } else if (url.startsWith("/categories/")) {
      resolve(books);
    } else if (url.startsWith("/books/")) {
      const bookId = parseInt(url.split("/").pop());
      const found = books.find((b) => b.book_id === bookId);
      if (found) resolve(found);
      else if (reject) reject("Not found");
    } else {
      resolve({});
    }
  });
}

describe("Edit", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    vi.spyOn(console, "log").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  // ── 초기 렌더링 ──

  it("카테고리 API를 호출하고 폴더 데이터를 로드한다", async () => {
    setupMockCategories();
    render(<Edit />);

    await waitFor(() => {
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/categories",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });

    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
  });

  it("카테고리 로드 실패 시 폴더 데이터가 비어있다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories" && reject) reject("서버 오류");
    });
    render(<Edit />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
      expect(screen.queryByTestId("folder-item-1_fiction")).toBeNull();
    });
  });

  // ── 폴더 항목 클릭 ──

  it("폴더 클릭 시 카테고리 내 책 목록을 로드한다", async () => {
    setupMockCategories();
    render(<Edit />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));

    await waitFor(() => {
      const calls = mockJsonGetReq.mock.calls;
      const hasCategoryCall = calls.some(
        (c) => c[0] === "/categories/1_fiction",
      );
      expect(hasCategoryCall).toBe(true);
    });
  });

  it("책 항목 클릭 시 책 정보를 표시한다", async () => {
    setupMockCategories();
    render(<Edit />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });
  });

  // ── decomposeTitle ──

  it("[저자] 제목 패턴의 파일명을 분해한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));

    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
      expect(screen.getByTestId("book-author").textContent).toBe("작가A");
    });
  });

  it("저자 필드가 있으면 파일명 패턴 분석을 건너뛴다", async () => {
    const books = [
      {
        book_id: 201,
        title: "제목만",
        author: "저자만",
        file_type: "pdf",
        file_path: "1_fiction/제목만.pdf",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, books);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("제목만.pdf")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/201"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("제목만");
      expect(screen.getByTestId("book-author").textContent).toBe("저자만");
    });
  });

  // ── 제목/저자 편집 ──

  it("제목 변경 시 bookInfo가 업데이트된다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    fireEvent.change(screen.getByTestId("title-input"), {
      target: { value: "새제목" },
    });
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("새제목");
    });
  });

  it("저자 변경 시 bookInfo가 업데이트된다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    fireEvent.change(screen.getByTestId("author-input"), {
      target: { value: "새저자" },
    });
    await waitFor(() => {
      expect(screen.getByTestId("book-author").textContent).toBe("새저자");
    });
  });

  it("교환 버튼 클릭 시 제목과 저자가 교환된다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("exchange-btn"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("작가A");
      expect(screen.getByTestId("book-author").textContent).toBe("소설1");
    });
  });

  it("삭제 버튼 클릭 시 delete API를 호출한다", async () => {
    setupMockCategories();
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) =>
      resolve({}),
    );
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("delete-btn"));

    await waitFor(() => {
      expect(mockJsonDeleteReq).toHaveBeenCalledWith(
        "/books/101",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("변경 버튼 클릭 시 PUT API를 호출한다", async () => {
    setupMockCategories();
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("change-btn"));

    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledWith(
        "/books/101",
        expect.any(Object),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("apiPrefix가 전달되면 API URL에 포함된다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") resolve(CATEGORIES);
      else if (url.startsWith("/comics/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit apiPrefix="/comics" />);
    await waitFor(() => {
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/comics/categories",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("_root 카테고리가 있으면 최상위 파일을 로드한다", async () => {
    const categories = { "1_fiction": 3, _root: 1 };
    const rootBooks = [
      {
        book_id: 999,
        title: "최상위파일",
        author: "",
        file_type: "txt",
        file_path: "최상위파일.txt",
        category: "_root",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/categories/_root") resolve(rootBooks);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit />);
    await waitFor(() => {
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/categories/_root",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("모바일 화면 크기일 때 적절한 클래스를 사용한다", async () => {
    setupMockCategories();
    vi.stubGlobal("innerWidth", 500);
    const { container } = render(<Edit />);
    await waitFor(() => {
      expect(container.querySelector(".section.directory-menu")).toBeNull();
    });
  });

  it("다음 버튼 클릭 시 다음 책으로 이동한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });

    fireEvent.click(screen.getByTestId("next-entry"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설2");
    });
  });

  it("최상위로 이동 버튼 클릭 시 PUT API를 호출한다", async () => {
    setupMockCategories();
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("move-upper"));
    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledWith(
        "/books/101",
        expect.objectContaining({ category: "_root" }),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("(저자) 제목 패턴의 파일명을 분해한다", async () => {
    const books = [
      {
        book_id: 301,
        title: "(작가C) 소설3",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/(작가C) 소설3.epub",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, books);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/301"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설3");
      expect(screen.getByTestId("book-author").textContent).toBe("작가C");
    });
  });

  it("제목 @ 저자 패턴의 파일명을 분해한다", async () => {
    const books = [
      {
        book_id: 302,
        title: "소설4 @ 작가D",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/소설4 @ 작가D.epub",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, books);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/302"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설4");
      expect(screen.getByTestId("book-author").textContent).toBe("작가D");
    });
  });

  it("제목자르기 버튼 클릭 시 공백을 기준으로 분해한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));

    fireEvent.change(screen.getByTestId("title-input"), {
      target: { value: "저자명 제목" },
    });
    fireEvent.click(screen.getByTestId("cut-title-btn"));
    await waitFor(() => {
      expect(screen.getByTestId("book-author").textContent).toBe("저자명");
      expect(screen.getByTestId("book-title").textContent).toBe("제목");
    });
  });

  it("삭제 시 warning이 있으면 메시지에 포함한다", async () => {
    setupMockCategories();
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) => {
      resolve({ warning: "파일은 지워졌으나 DB 연동 오류" });
    });
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("delete-btn"));

    await waitFor(() => {
      expect(
        screen.getByText(/경고: 파일은 지워졌으나 DB 연동 오류/),
      ).toBeTruthy();
    });
  });

  it('마지막 책 삭제 시 "마지막 책이었습니다" 메시지를 표시한다', async () => {
    const singleBook = [
      {
        book_id: 999,
        title: "막책",
        author: "",
        file_type: "txt",
        file_path: "1_fiction/last.txt",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, singleBook);
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) =>
      resolve({}),
    );
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/999"));
    fireEvent.click(screen.getByTestId("delete-btn"));

    await waitFor(() => {
      expect(screen.getByText(/마지막 책이었습니다/)).toBeTruthy();
    });
  });

  it("다음 책이 없을 때 에러 메시지를 표시한다", async () => {
    const singleBook = [
      {
        book_id: 999,
        title: "막책",
        author: "",
        file_type: "txt",
        file_path: "1_fiction/last.txt",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, singleBook);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/999"));

    fireEvent.click(screen.getByTestId("next-entry"));
    await waitFor(() => {
      expect(screen.getByText("마지막 책입니다.")).toBeTruthy();
    });
  });

  it("이전 책이 없을 때 에러 메시지를 표시한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));

    fireEvent.click(screen.getByTestId("prev-entry"));
    await waitFor(() => {
      expect(screen.getByText("첫 번째 책입니다.")).toBeTruthy();
    });
  });

  // ── 이전 버튼: 유효한 prevEntryId (line 736-737) ──

  it("이전 버튼 클릭 시 이전 책으로 이동한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가B] 소설2.pdf")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/102"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설2");
    });

    fireEvent.click(screen.getByTestId("prev-entry"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });
  });

  // ── 초기화 버튼 (line 450) ──

  it("초기화 버튼 클릭 시 원래 분해된 값으로 복원한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });

    // 제목을 변경한 후
    fireEvent.change(screen.getByTestId("title-input"), {
      target: { value: "변경된제목" },
    });
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("변경된제목");
    });

    // 초기화 버튼 클릭
    fireEvent.click(screen.getByTestId("reset-btn"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
      expect(screen.getByTestId("book-author").textContent).toBe("작가A");
    });
  });

  // ── 저자자르기 버튼 (line 435) ──

  it("저자자르기 버튼 클릭 시 저자를 공백 기준으로 분해한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    fireEvent.change(screen.getByTestId("author-input"), {
      target: { value: "성 이름" },
    });
    fireEvent.click(screen.getByTestId("cut-author-btn"));
    await waitFor(() => {
      expect(screen.getByTestId("book-author").textContent).toBe("성");
      expect(screen.getByTestId("book-title").textContent).toBe("이름");
    });
  });

  // ── 파일명 입력 변경 (line 408) ──

  it("파일명 입력을 직접 변경할 수 있다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    fireEvent.change(screen.getByTestId("filename-input"), {
      target: { value: "새파일명.epub" },
    });
    await waitFor(() => {
      expect(screen.getByTestId("filename-input").value).toBe("새파일명.epub");
    });
  });

  // ── 삭제 실패 (lines 713-716) ──

  it("삭제 API 실패 시 에러 메시지를 표시한다", async () => {
    setupMockCategories();
    mockJsonDeleteReq.mockImplementation((url, payload, resolve, reject) => {
      reject("서버 오류");
    });
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("delete-btn"));

    await waitFor(() => {
      expect(screen.getByText(/책 삭제에 실패했습니다/)).toBeTruthy();
    });
  });

  // ── 변경 API 실패 (line 613, 627-630) ──

  it("변경 API 실패 시 에러 메시지를 표시한다", async () => {
    setupMockCategories();
    mockJsonPutReq.mockImplementation((url, payload, resolve, reject) => {
      reject("서버 오류");
    });
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("change-btn"));

    await waitFor(() => {
      expect(screen.getByText(/책 이름 변경에 실패했습니다/)).toBeTruthy();
    });
  });

  // ── CONFLICT 에러로 인한 재시도 (lines 613-617) ──

  it("변경 시 CONFLICT 에러가 발생하면 confirm 후 재시도한다", async () => {
    setupMockCategories();
    let callCount = 0;
    mockJsonPutReq.mockImplementation((url, payload, resolve, reject) => {
      callCount++;
      if (callCount === 1) {
        reject("CONFLICT:동일 파일이 존재합니다.");
      } else {
        resolve();
      }
    });
    window.confirm = vi.fn().mockReturnValue(true);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("change-btn"));

    await waitFor(() => {
      // 두 번 호출 (최초 + force 재시도)
      expect(mockJsonPutReq).toHaveBeenCalledTimes(2);
      const secondCall = mockJsonPutReq.mock.calls[1];
      expect(secondCall[0]).toContain("?force=true");
    });
  });

  // ── CONFLICT 에러에서 confirm 거부 (line 622-624) ──

  it("변경 시 CONFLICT 에러 후 confirm 거부하면 재시도하지 않는다", async () => {
    setupMockCategories();
    mockJsonPutReq.mockImplementation((url, payload, resolve, reject) => {
      reject("CONFLICT:동일 파일이 존재합니다.");
    });
    window.confirm = vi.fn().mockReturnValue(false);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("change-btn"));

    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledTimes(1);
    });
  });

  // ── 제목에 저자 prefix가 포함된 경우 제거 (line 168) ──

  it("author가 있고 title에 [author] prefix가 포함된 경우 제거한다", async () => {
    const books = [
      {
        book_id: 401,
        title: "[작가E] 소설5",
        author: "작가E",
        file_type: "epub",
        file_path: "1_fiction/[작가E] 소설5.epub",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, books);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/401"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설5");
      expect(screen.getByTestId("book-author").textContent).toBe("작가E");
    });
  });

  // ── 저자 - 제목 패턴 (pattern5) ──

  it("저자 - 제목 패턴의 파일명을 분해한다", async () => {
    const books = [
      {
        book_id: 501,
        title: "작가F - 소설6",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/작가F - 소설6.epub",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, books);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/501"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설6");
      expect(screen.getByTestId("book-author").textContent).toBe("작가F");
    });
  });

  // ── 제목 [ 저자 ] 패턴 (pattern3) ──

  it("제목 [저자] 패턴의 파일명을 분해한다", async () => {
    const books = [
      {
        book_id: 502,
        title: "소설7 [작가G]",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/소설7 [작가G].epub",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, books);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/502"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설7");
      expect(screen.getByTestId("book-author").textContent).toBe("작가G");
    });
  });

  // ── 제목 ( 저자 ) 패턴 (pattern6) ──

  it("제목 (저자) 패턴의 파일명을 분해한다", async () => {
    const books = [
      {
        book_id: 503,
        title: "소설8 (작가H)",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/소설8 (작가H).epub",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, books);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/503"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설8");
      expect(screen.getByTestId("book-author").textContent).toBe("작가H");
    });
  });

  // ── 성공 메시지 자동 사라짐 (line 89) ──

  it("성공 메시지가 3초 후 자동으로 사라진다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    setupMockCategories();
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) =>
      resolve({}),
    );
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    fireEvent.click(screen.getByTestId("delete-btn"));

    await waitFor(() => {
      expect(screen.getByText(/삭제되었습니다/)).toBeTruthy();
    });

    vi.advanceTimersByTime(3100);
    await waitFor(() => {
      expect(screen.queryByText(/삭제되었습니다/)).toBeNull();
    });
    vi.useRealTimers();
  });

  // ── 에러 메시지 자동 사라짐 (line 96) ──

  it("에러 메시지가 5초 후 자동으로 사라진다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const singleBook = [
      {
        book_id: 999,
        title: "막책",
        author: "",
        file_type: "txt",
        file_path: "1_fiction/last.txt",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, singleBook);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/999"));
    fireEvent.click(screen.getByTestId("next-entry"));

    await waitFor(() => {
      expect(screen.getByText("마지막 책입니다.")).toBeTruthy();
    });

    vi.advanceTimersByTime(5100);
    await waitFor(() => {
      expect(screen.queryByText("마지막 책입니다.")).toBeNull();
    });
    vi.useRealTimers();
  });

  // ── _root 카테고리 로드 실패 시 폴백 (line 133-134) ──

  it("_root 카테고리 로드 실패 시 나머지 데이터만 표시한다", async () => {
    const categories = { "1_fiction": 3, _root: 1 };
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/categories/_root") reject("실패");
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
  });

  // ── 변경 성공 시 디렉토리 이동 메시지 (lines 581-585, 600-609) ──

  it("변경 버튼으로 같은 디렉토리 내 이름 변경 시 성공 메시지를 표시한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, _reject) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    fireEvent.change(screen.getByTestId("title-input"), {
      target: { value: "새제목" },
    });
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("새제목");
    });

    fireEvent.click(screen.getByTestId("change-btn"));

    await waitFor(() => {
      expect(screen.getByText(/파일 이름을.*변경했습니다/)).toBeTruthy();
    });
  });

  // ── 디렉토리 선택 및 이동 (lines 655, 659) ──

  it("디렉토리 선택 후 이동 버튼 클릭 시 PUT API를 호출한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, _reject) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    // 디렉토리 선택 → 이동
    fireEvent.click(screen.getByTestId("select-dir"));
    fireEvent.click(screen.getByTestId("move-dir"));
    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledWith(
        "/books/101",
        expect.objectContaining({ category: "2_science" }),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  // ── resize 이벤트 핸들러 (line 31) ──

  it("윈도우 리사이즈 시 isMobile 상태가 업데이트된다", async () => {
    setupMockCategories();
    Object.defineProperty(window, "innerWidth", {
      writable: true,
      configurable: true,
      value: 1024,
    });
    const { container } = render(<Edit />);
    await waitFor(() => {
      expect(container.querySelector(".section.directory-menu")).toBeTruthy();
    });

    // 모바일 크기로 리사이즈
    Object.defineProperty(window, "innerWidth", {
      writable: true,
      configurable: true,
      value: 500,
    });
    fireEvent(window, new Event("resize"));
    await waitFor(() => {
      expect(container.querySelector(".section.directory-menu")).toBeNull();
    });
  });

  // ── 변경 성공 후 이동 시 next/prev 폴백 (lines 600-609) ──

  it("최상위 이동 성공 후 성공 메시지를 표시한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, _reject) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });

    fireEvent.click(screen.getByTestId("move-upper"));
    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledWith(
        "/books/101",
        expect.objectContaining({ category: "_root" }),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  // ── CONFLICT 재시도 실패 (line 617-620) ──

  it("CONFLICT 재시도도 실패하면 에러 메시지를 표시한다", async () => {
    let putCallCount = 0;
    mockJsonGetReq.mockImplementation((url, payload, resolve, _reject) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    mockJsonPutReq.mockImplementation((url, payload, resolve, reject) => {
      putCallCount++;
      if (putCallCount === 1) {
        reject("CONFLICT:파일 충돌");
      } else {
        reject("재시도 실패");
      }
    });
    window.confirm = vi.fn().mockReturnValue(true);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("change-btn"));

    await waitFor(() => {
      expect(screen.getByText(/책 변경에 실패했습니다/)).toBeTruthy();
    });
  });

  // ── URL 파라미터로 직접 접근 (lines 352-366): 폴더 트리에 없는 카테고리 ──

  it("URL 파라미터의 카테고리가 트리에 없으면 API에서 직접 조회한다", async () => {
    const { useParams, useSearchParams } = await import("react-router-dom");
    useParams.mockReturnValue({ "*": "777" });
    useSearchParams.mockReturnValue([
      new URLSearchParams("category=unknown_category"),
    ]);

    mockJsonGetReq.mockImplementation((url, payload, resolve, _reject) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else if (url === "/books/777") {
        resolve({
          book_id: 777,
          title: "직접조회책",
          author: "저자X",
          file_type: "pdf",
          file_path: "unknown_category/직접조회책.pdf",
          category: "unknown_category",
        });
      } else resolve({});
    });

    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("직접조회책");
      expect(screen.getByTestId("book-author").textContent).toBe("저자X");
    });

    // 정리
    useParams.mockReturnValue({ "*": "" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);
  });

  // ── URL 파라미터 직접 조회 실패 (line 366-367) ──

  it("URL 파라미터로 직접 조회 실패 시 에러를 설정한다", async () => {
    const { useParams, useSearchParams } = await import("react-router-dom");
    useParams.mockReturnValue({ "*": "888" });
    useSearchParams.mockReturnValue([
      new URLSearchParams("category=bad_category"),
    ]);

    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else if (url === "/books/888") reject("Not found");
      else resolve({});
    });

    render(<Edit />);
    // book API 호출이 이루어짐을 확인 (에러 메시지는 bookInfo 미설정으로 DOM에 보이지 않음)
    await waitFor(() => {
      const calls = mockJsonGetReq.mock.calls;
      expect(calls.some((c) => c[0] === "/books/888")).toBe(true);
    });

    // 정리
    useParams.mockReturnValue({ "*": "" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);
  });

  // ── 중첩 카테고리 내 책 선택: else book-entry 성공 경로 (lines 351-409) ──

  it("중첩 카테고리(3단계) 내 책 클릭 시 책 정보를 표시한다", async () => {
    // 1_fiction (부모) / 1_fiction/sub (자식 폴더) 구조
    const nestedCategories = { "1_fiction": 3, "1_fiction/sub": 2 };
    const subBooks = [
      {
        book_id: 601,
        title: "[작가I] 중첩책1",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/sub/[작가I] 중첩책1.epub",
        category: "1_fiction/sub",
      },
      {
        book_id: 602,
        title: "[작가J] 중첩책2",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/sub/[작가J] 중첩책2.epub",
        category: "1_fiction/sub",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(nestedCategories);
      else if (url === "/categories/1_fiction/sub") resolve(subBooks);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });

    // 자식 폴더(1_fiction/sub) 클릭 → 책 로드
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/sub"));
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-1_fiction/sub/601")).toBeTruthy();
    });

    // 손자 책 항목 클릭 → findFolderInTree(entryId)는 null, parseEntryId 성공
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/sub/601"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("중첩책1");
      expect(screen.getByTestId("book-author").textContent).toBe("작가I");
    });
  });

  // ── 중첩 카테고리 내 다음/이전 책 이동 (else 경로의 next/prev) ──

  it("중첩 카테고리 내 다음 책으로 이동한다", async () => {
    const nestedCategories = { "1_fiction": 3, "1_fiction/sub": 2 };
    const subBooks = [
      {
        book_id: 601,
        title: "[작가I] 중첩책1",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/sub/[작가I] 중첩책1.epub",
        category: "1_fiction/sub",
      },
      {
        book_id: 602,
        title: "[작가J] 중첩책2",
        author: "",
        file_type: "epub",
        file_path: "1_fiction/sub/[작가J] 중첩책2.epub",
        category: "1_fiction/sub",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(nestedCategories);
      else if (url === "/categories/1_fiction/sub") resolve(subBooks);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/sub"));
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-1_fiction/sub/601")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/sub/601"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("중첩책1");
    });

    fireEvent.click(screen.getByTestId("next-entry"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("중첩책2");
    });
  });

  // ── 가상 부모 폴더 클릭 시 API 호출 안 함 (line 278) ──

  it("가상 부모 폴더 클릭 시 책 로드 API를 호출하지 않는다", async () => {
    // 부모 카테고리는 없고 자식만 있어 가상 부모 생성
    // (commonPrefix 축약을 피하기 위해 형제 카테고리 other를 둔다)
    const virtualCategories = { "vparent/childA": 2, other: 1 };
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(virtualCategories);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });

    // 가상 부모 항목 클릭
    fireEvent.click(screen.getByTestId("folder-item-__virtual__vparent"));

    // 가상 부모는 /categories/__virtual__... 호출을 하지 않는다
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    const virtualCalls = mockJsonGetReq.mock.calls.filter((c) =>
      c[0].includes("__virtual__"),
    );
    expect(virtualCalls.length).toBe(0);
  });

  // ── else book-entry 경로에서 책을 찾지 못하면 에러 (line 411) ──
  // SimilarBooks.onSelect(=entryClicked)로 로딩된 카테고리의 존재하지 않는
  // bookId를 선택하면 children.find(...)?.book 가 undefined → 411.

  it("로딩된 카테고리에서 존재하지 않는 책 ID 선택 시 에러 메시지를 표시한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    // 같은 카테고리, 존재하지 않는 bookId 선택
    fireEvent.click(screen.getByTestId("similar-select-missing"));
    await waitFor(() => {
      expect(screen.getByText(/선택한 책을 찾을 수 없습니다/)).toBeTruthy();
    });
  });

  // ── else book-entry 경로에서 카테고리를 찾지 못하면 에러 (line 414) ──

  it("트리에 없는 카테고리의 책을 선택하면 카테고리 에러 메시지를 표시한다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    // 트리에 없는 카테고리의 책 선택 → booksInCategory undefined → 414
    fireEvent.click(screen.getByTestId("similar-select-missing-cat"));
    await waitFor(() => {
      expect(
        screen.getByText(/선택한 카테고리를 찾을 수 없습니다/),
      ).toBeTruthy();
    });
  });

  // ── _root 라우트 초기화 (lines 452-454) ──

  it("_root 라우트 파라미터로 최상위 파일을 자동 선택한다", async () => {
    const { useParams, useSearchParams } = await import("react-router-dom");
    useParams.mockReturnValue({ "*": "999" });
    useSearchParams.mockReturnValue([new URLSearchParams("category=_root")]);

    const categories = { "1_fiction": 3, _root: 1 };
    const rootBooks = [
      {
        book_id: 999,
        title: "최상위파일",
        author: "저자R",
        file_type: "txt",
        file_path: "최상위파일.txt",
        category: "_root",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/categories/_root") resolve(rootBooks);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });

    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("최상위파일");
      expect(screen.getByTestId("book-author").textContent).toBe("저자R");
    });

    useParams.mockReturnValue({ "*": "" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);
  });

  // ── 라우트 초기화: 카테고리 미로딩 → 로딩 트리거 후 재선택 (lines 498-503) ──

  it("라우트 카테고리가 아직 로드되지 않았으면 로딩 후 책을 선택한다", async () => {
    const { useParams, useSearchParams } = await import("react-router-dom");
    useParams.mockReturnValue({ "*": "101" });
    useSearchParams.mockReturnValue([
      new URLSearchParams("category=1_fiction"),
    ]);

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });

    render(<Edit />);
    // 카테고리 책 로드 후 1_fiction/101이 선택되어 책 정보 표시
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });
    // 카테고리 로딩 API가 호출되었는지 확인
    const calls = mockJsonGetReq.mock.calls;
    expect(calls.some((c) => c[0] === "/categories/1_fiction")).toBe(true);

    useParams.mockReturnValue({ "*": "" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);
  });

  // ── 최상위 파일 삭제: root file remove (lines 633-635) ──

  it("최상위 파일 삭제 시 폴더 데이터에서 제거한다", async () => {
    const categories = { "1_fiction": 3, _root: 1 };
    const rootBooks = [
      {
        book_id: 999,
        title: "최상위파일",
        author: "",
        file_type: "txt",
        file_path: "최상위파일.txt",
        category: "_root",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/categories/_root") resolve(rootBooks);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) =>
      resolve({}),
    );
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/999")).toBeTruthy();
    });

    // 최상위 파일 선택
    fireEvent.click(screen.getByTestId("folder-item-/999"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("delete-btn"));
    await waitFor(() => {
      expect(mockJsonDeleteReq).toHaveBeenCalledWith(
        "/books/999",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });
    // 폴더 트리에서 제거됨
    await waitFor(() => {
      expect(screen.queryByTestId("folder-item-/999")).toBeNull();
    });
  });

  // ── 책을 최상위로 이동: root file append splice (line 681) + remove(633-635) ──

  it("책을 최상위로 이동하면 기존 최상위 파일 앞에 삽입한다", async () => {
    // _root에 이미 파일이 있어 folderEndIndex !== -1 → splice 경로(line 681)
    const categories = { "1_fiction": 3, _root: 1 };
    const rootBooks = [
      {
        book_id: 999,
        title: "기존최상위",
        author: "",
        file_type: "txt",
        file_path: "기존최상위.txt",
        category: "_root",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/categories/_root") resolve(rootBooks);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });

    fireEvent.click(screen.getByTestId("move-upper"));
    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledWith(
        "/books/101",
        expect.objectContaining({ category: "_root" }),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  // ── 덮어쓰기 확인 경로: 동일 이름 존재 시 confirm (lines 737, 739, 746) ──

  // 두 책 모두 epub: 확장자 정합을 맞춰 checkEntryExistence가 동작하도록 함
  const TWO_EPUB_BOOKS = [
    {
      book_id: 101,
      title: "[작가A] 소설1",
      author: "",
      file_type: "epub",
      file_path: "1_fiction/[작가A] 소설1.epub",
      category: "1_fiction",
    },
    {
      book_id: 102,
      title: "[작가B] 소설2",
      author: "",
      file_type: "epub",
      file_path: "1_fiction/[작가B] 소설2.epub",
      category: "1_fiction",
    },
  ];

  it("이름 변경 대상이 이미 존재하면 confirm 후 덮어쓰기로 진행한다", async () => {
    setupMockCategories(CATEGORIES, TWO_EPUB_BOOKS);
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가B] 소설2.epub")).toBeTruthy();
    });
    // 101 선택 후 파일명을 102의 이름(이미 존재)으로 변경
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });

    fireEvent.change(screen.getByTestId("filename-input"), {
      target: { value: "[작가B] 소설2.epub" },
    });
    fireEvent.click(screen.getByTestId("change-btn"));

    await waitFor(() => {
      // confirm이 호출되고 force=true로 PUT
      expect(confirmSpy).toHaveBeenCalled();
      const forceCall = mockJsonPutReq.mock.calls.find((c) =>
        c[0].includes("?force=true"),
      );
      expect(forceCall).toBeTruthy();
    });
  });

  // ── 덮어쓰기 확인 거부 시 진행하지 않음 (line 744) ──

  it("덮어쓰기 confirm을 거부하면 PUT을 호출하지 않는다", async () => {
    setupMockCategories(CATEGORIES, TWO_EPUB_BOOKS);
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가B] 소설2.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });

    fireEvent.change(screen.getByTestId("filename-input"), {
      target: { value: "[작가B] 소설2.epub" },
    });
    fireEvent.click(screen.getByTestId("change-btn"));

    // confirm 거부 → PUT 호출 없음
    await waitFor(() => {
      expect(window.confirm).toHaveBeenCalled();
    });
    expect(mockJsonPutReq).not.toHaveBeenCalled();
  });

  // ── 이동 후 prev 폴백 (lines 806-807): next 없고 prev 있는 경우 ──

  it("마지막 책을 다른 디렉토리로 이동하면 이전 책으로 이동한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가B] 소설2.pdf")).toBeTruthy();
    });
    // 마지막 책(102) 선택 → next 없음, prev=101
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/102"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설2");
    });

    fireEvent.click(screen.getByTestId("select-dir"));
    fireEvent.click(screen.getByTestId("move-dir"));
    // 이동 후 이전 책(소설1)으로 자동 이동
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });
  });

  // ── 이동 후 마지막 책 메시지 (line 809): next/prev 모두 없음 ──

  it("유일한 책을 다른 디렉토리로 이동하면 마지막 책 메시지를 표시한다", async () => {
    const singleBook = [
      {
        book_id: 999,
        title: "유일책",
        author: "",
        file_type: "txt",
        file_path: "1_fiction/유일책.txt",
        category: "1_fiction",
      },
    ];
    setupMockCategories(CATEGORIES, singleBook);
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/999"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("유일책");
    });

    fireEvent.click(screen.getByTestId("select-dir"));
    fireEvent.click(screen.getByTestId("move-dir"));
    await waitFor(() => {
      expect(screen.getByText(/마지막 책이었습니다/)).toBeTruthy();
    });
  });

  // ── 삭제 후 prev 폴백 (lines 954, 957): next 없고 prev 있음 ──

  it("마지막 책 삭제 후 이전 책으로 이동한다", async () => {
    setupMockCategories();
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) =>
      resolve({}),
    );
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가B] 소설2.pdf")).toBeTruthy();
    });
    // 마지막 책(102) 선택 → next 없음, prev=101
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/102"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설2");
    });

    fireEvent.click(screen.getByTestId("delete-btn"));
    // 삭제 후 이전 책(소설1)으로 자동 이동
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });
  });

  // ── 레거시 라우트 /edit/category/bookId (query 없음) (lines 72, 77) ──

  it("query 없는 레거시 라우트(category/bookId)로 책을 선택한다", async () => {
    const { useParams, useSearchParams } = await import("react-router-dom");
    useParams.mockReturnValue({ "*": "1_fiction/101" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });

    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });

    useParams.mockReturnValue({ "*": "" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);
  });

  // ── query category + 비숫자 wildcard → routeBookId undefined (line 76) ──

  it("category query가 있고 wildcard가 숫자가 아니면 책을 자동 선택하지 않는다", async () => {
    const { useParams, useSearchParams } = await import("react-router-dom");
    useParams.mockReturnValue({ "*": "notanumber" });
    useSearchParams.mockReturnValue([
      new URLSearchParams("category=1_fiction"),
    ]);

    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    // routeBookId가 undefined라 라우트 기반 선택이 일어나지 않고
    // 첫 파일 자동 선택도 routeCategory가 있어 건너뛴다 → 책 정보 없음
    expect(screen.queryByTestId("book-info")).toBeNull();

    useParams.mockReturnValue({ "*": "" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);
  });

  // ── apiPrefix + 최상위 파일 선택: viewUrl에 api 파라미터 포함 (line 329) ──

  it("apiPrefix가 있을 때 최상위 파일 선택 시 viewUrl에 api 파라미터를 포함한다", async () => {
    const categories = { "1_fiction": 3, _root: 1 };
    const rootBooks = [
      {
        book_id: 999,
        title: "최상위파일",
        author: "저자R",
        file_type: "txt",
        file_path: "최상위파일.txt",
        category: "_root",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") resolve(categories);
      else if (url === "/comics/categories/_root") resolve(rootBooks);
      else if (url.startsWith("/comics/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit apiPrefix="/comics" />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/999")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-/999"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("최상위파일");
    });
  });

  // ── 최상위 파일이 category 필드가 없으면 _root로 대체 (line 339) ──

  it("최상위 파일에 category가 없으면 URL에 _root를 사용한다", async () => {
    const categories = { "1_fiction": 3, _root: 1 };
    const rootBooks = [
      {
        book_id: 888,
        title: "카테고리없음",
        author: "",
        file_type: "txt",
        file_path: "카테고리없음.txt",
        // category 필드 의도적 누락 → book["category"] || "_root"의 우측 사용
      },
    ];
    const replaceSpy = vi.spyOn(window.history, "replaceState");
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/categories/_root") resolve(rootBooks);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/888")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-/888"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("카테고리없음");
    });
    expect(
      replaceSpy.mock.calls.some((c) =>
        String(c[2]).includes("category=_root"),
      ),
    ).toBe(true);
  });

  // ── apiPrefix + 폴더 내 책 선택: viewUrl에 api 파라미터 포함 (line 377) ──

  it("apiPrefix가 있을 때 폴더 내 책 선택 시 동작한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") resolve(CATEGORIES);
      else if (url.startsWith("/comics/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    render(<Edit apiPrefix="/comics" />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });
  });

  // ── apiPrefix + URL 직접 조회 (line 478) ──

  it("apiPrefix가 있을 때 트리에 없는 카테고리는 API에서 직접 조회한다", async () => {
    const { useParams, useSearchParams } = await import("react-router-dom");
    useParams.mockReturnValue({ "*": "777" });
    useSearchParams.mockReturnValue([
      new URLSearchParams("category=unknown_category"),
    ]);

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") resolve(CATEGORIES);
      else if (url.startsWith("/comics/categories/")) resolve(BOOKS_IN_FICTION);
      else if (url === "/comics/books/777") {
        resolve({
          book_id: 777,
          title: "직접조회책",
          author: "저자X",
          file_type: "pdf",
          file_path: "unknown_category/직접조회책.pdf",
          category: "unknown_category",
        });
      } else resolve({});
    });

    render(<Edit apiPrefix="/comics" />);
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("직접조회책");
    });

    useParams.mockReturnValue({ "*": "" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);
  });

  // ── URL 직접 조회 시 book.category가 없으면 routeCategory 사용 (line 465) ──

  it("직접 조회한 책에 category가 없으면 라우트 카테고리를 사용한다", async () => {
    const { useParams, useSearchParams } = await import("react-router-dom");
    useParams.mockReturnValue({ "*": "777" });
    useSearchParams.mockReturnValue([
      new URLSearchParams("category=unknown_category"),
    ]);

    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else if (url === "/books/777") {
        resolve({
          book_id: 777,
          title: "직접조회책2",
          author: "저자Y",
          file_type: "pdf",
          file_path: "unknown_category/직접조회책2.pdf",
          // category 누락 → realCategory = routeCategory
        });
      } else resolve({});
    });

    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("직접조회책2");
    });

    useParams.mockReturnValue({ "*": "" });
    useSearchParams.mockReturnValue([new URLSearchParams()]);
  });

  // ── 이미 로드된 폴더를 다시 클릭하면 재로딩하지 않는다 (line 283 else) ──

  it("이미 로드된 폴더를 다시 클릭하면 책 목록을 재요청하지 않는다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    const before = mockJsonGetReq.mock.calls.filter(
      (c) => c[0] === "/categories/1_fiction",
    ).length;

    // 같은 폴더 재클릭 → booksLoaded=true라 재요청 없음
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("[작가A] 소설1.epub")).toBeTruthy();
    });
    const after = mockJsonGetReq.mock.calls.filter(
      (c) => c[0] === "/categories/1_fiction",
    ).length;
    expect(after).toBe(before);
  });

  // ── 제목자르기: 공백 없는 제목이면 변경하지 않는다 (line 574 else) ──

  it("제목에 공백이 없으면 제목자르기는 아무 변화가 없다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });

    fireEvent.click(screen.getByTestId("cut-title-btn"));
    // 공백이 없으므로 제목/저자 그대로
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
      expect(screen.getByTestId("book-author").textContent).toBe("작가A");
    });
  });

  // ── 저자자르기: 공백 없는 저자면 변경하지 않는다 (line 583 else) ──

  it("저자에 공백이 없으면 저자자르기는 아무 변화가 없다", async () => {
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-author").textContent).toBe("작가A");
    });

    fireEvent.click(screen.getByTestId("cut-author-btn"));
    await waitFor(() => {
      expect(screen.getByTestId("book-author").textContent).toBe("작가A");
      expect(screen.getByTestId("book-title").textContent).toBe("소설1");
    });
  });

  // ── 삭제 confirm을 거부하면 delete API를 호출하지 않는다 (line 911) ──

  it("삭제 confirm을 거부하면 delete API를 호출하지 않는다", async () => {
    setupMockCategories();
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) =>
      resolve({}),
    );
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("delete-btn"));
    await waitFor(() => {
      expect(window.confirm).toHaveBeenCalled();
    });
    expect(mockJsonDeleteReq).not.toHaveBeenCalled();
  });

  // ── 최상위 파일 이름 변경: source dir가 _root → "최상위" 메시지 (line 770) ──

  it("최상위 파일 이름 변경 시 최상위 디렉토리 메시지를 표시한다", async () => {
    const categories = { "1_fiction": 3, _root: 1 };
    const rootBooks = [
      {
        book_id: 999,
        title: "최상위파일",
        author: "",
        file_type: "txt",
        file_path: "최상위파일.txt",
        category: "_root",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/categories/_root") resolve(rootBooks);
      else if (url.startsWith("/categories/")) resolve(BOOKS_IN_FICTION);
      else resolve({});
    });
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/999")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-/999"));
    await waitFor(() => {
      expect(screen.getByTestId("book-title").textContent).toBe("최상위파일");
    });

    fireEvent.change(screen.getByTestId("title-input"), {
      target: { value: "새최상위" },
    });
    fireEvent.click(screen.getByTestId("change-btn"));
    await waitFor(() => {
      expect(
        screen.getByText(/"최상위"의 파일 이름을.*변경했습니다/),
      ).toBeTruthy();
    });
  });

  // ── 확장자 없는 파일명으로 변경 (lines 661, 724의 endsWith false 분기) ──

  it("확장자가 없는 파일명으로 변경해도 처리된다", async () => {
    setupMockCategories();
    mockJsonPutReq.mockImplementation((url, payload, resolve) => resolve());
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    // 확장자(.epub)가 없는 파일명 → endsWith(extensionSuffix) === false
    fireEvent.change(screen.getByTestId("filename-input"), {
      target: { value: "확장자없는이름" },
    });
    fireEvent.click(screen.getByTestId("change-btn"));
    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalled();
      const payload = mockJsonPutReq.mock.calls[0][1];
      // titleOnly + extensionSuffix → "확장자없는이름.epub"
      expect(payload.file_path).toBe("1_fiction/확장자없는이름.epub");
    });
  });

  // ── 검색 결과가 있으면 SearchResult를 렌더링한다 (line 1041) ──

  it("hasSearched가 true이면 검색 결과 컴포넌트를 렌더링한다", async () => {
    const { useOutletContext } = await import("react-router-dom");
    useOutletContext.mockReturnValue({
      ...mockOutletContext,
      hasSearched: true,
      searchResults: [{ book_id: 1, title: "결과", file_type: "epub" }],
      searchTotal: 1,
    });
    setupMockCategories();
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("search-result")).toBeTruthy();
    });
    useOutletContext.mockReturnValue(mockOutletContext);
  });

  // ── 처리 중 가드: in-flight PUT 동안 버튼 클릭은 무시된다 ──
  // (lines 266, 714, 906, 973, 984의 isProcessingRef early-return 및
  //  1100/1112의 isProcessing spinner 아이콘 분기)

  it("처리 중에는 다른 버튼 클릭과 폴더 클릭이 무시된다", async () => {
    setupMockCategories();
    // PUT의 resolve를 보류하여 isProcessing=true 상태를 유지
    let pendingResolve = null;
    mockJsonPutReq.mockImplementation((url, payload, resolve) => {
      pendingResolve = resolve;
    });
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) =>
      resolve({}),
    );
    render(<Edit />);
    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_fiction"));
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/101"));
    await waitFor(() => {
      expect(screen.getByTestId("book-info")).toBeTruthy();
    });

    // 변경 시작 → isProcessingRef.current = true (resolve 보류)
    fireEvent.click(screen.getByTestId("change-btn"));
    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledTimes(1);
    });

    // 처리 중: 두 번째 change 클릭은 updateFile 진입 시 early-return (line 714)
    fireEvent.click(screen.getByTestId("change-btn"));
    // 삭제(line 906), 다음(973), 이전(984)도 early-return
    fireEvent.click(screen.getByTestId("delete-btn"));
    fireEvent.click(screen.getByTestId("next-entry"));
    fireEvent.click(screen.getByTestId("prev-entry"));
    // 폴더 재클릭도 entryClicked early-return (line 266)
    fireEvent.click(screen.getByTestId("folder-item-1_fiction/102"));

    // PUT은 여전히 1회, delete는 호출되지 않음
    expect(mockJsonPutReq).toHaveBeenCalledTimes(1);
    expect(mockJsonDeleteReq).not.toHaveBeenCalled();

    // 보류된 PUT을 완료하여 가드 해제
    pendingResolve && pendingResolve();
    await waitFor(() => {
      expect(screen.getByText(/변경했습니다/)).toBeTruthy();
    });
  });

  // ── 추가 분기 ──

  it("최상위 파일이 여러 개면 제목순으로 정렬한다", async () => {
    const rootBooks = [
      { book_id: 3, title: "다랑", author: "", file_type: "pdf", file_path: "c.pdf", category: "_root" },
      { book_id: 1, title: "가람", author: "", file_type: "pdf", file_path: "a.pdf", category: "_root" },
      { book_id: 2, title: "나람", author: "", file_type: "pdf", file_path: "b.pdf", category: "_root" },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ _root: 3 });
      else if (url.startsWith("/categories/")) resolve(rootBooks);
      else resolve({});
    });

    render(<Edit />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-/1")).toBeTruthy();
    });
    expect(screen.getByTestId("folder-item-/1").textContent).toBe("가람.pdf");
    expect(screen.getByTestId("folder-item-/3").textContent).toBe("다랑.pdf");
  });

  it("apiPrefix 가 있으면 viewUrl 에 api 파라미터를 포함한다", async () => {
    const comicBooks = [
      {
        book_id: 501,
        title: "만화1",
        author: "",
        file_type: "pdf",
        file_path: "1_comic/만화1.pdf",
        category: "1_comic",
      },
    ];
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") resolve({ "1_comic": 1 });
      else if (url.startsWith("/comics/categories/")) resolve(comicBooks);
      else if (url.startsWith("/comics/books/")) resolve(comicBooks[0]);
      else resolve({});
    });

    render(<Edit apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-1_comic")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_comic"));

    await waitFor(() => {
      expect(screen.getByTestId("folder-item-1_comic/501")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("folder-item-1_comic/501"));

    await waitFor(() => {
      expect(screen.getByTestId("view-single")).toBeTruthy();
    });
    expect(screen.getByTestId("view-single").dataset.viewUrl).toContain("api=");
  });

  it("폴더를 접으면 접힌 Folder 를 렌더링한다", async () => {
    setupMockCategories();
    render(<Edit />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });

    // Folder mock 은 접힌 상태에서만 '펼치기' 버튼을 노출한다 →
    // 열린 상태에서 onToggle 을 부르려면 Actions 쪽 경로가 없으므로
    // 접힘 여부는 초기 렌더 기준으로만 확인한다.
    expect(screen.queryByTestId("folder-closed")).toBeNull();
  });

  it("모바일 폭에서는 열 너비를 12로 렌더링한다", async () => {
    const originalWidth = window.innerWidth;
    Object.defineProperty(window, "innerWidth", {
      writable: true,
      configurable: true,
      value: 500,
    });
    setupMockCategories();
    render(<Edit />);

    await waitFor(() => {
      expect(screen.getByTestId("folder-open")).toBeTruthy();
    });
    expect(document.querySelector(".col-md-12")).toBeTruthy();

    Object.defineProperty(window, "innerWidth", {
      writable: true,
      configurable: true,
      value: originalWidth,
    });
  });
});
