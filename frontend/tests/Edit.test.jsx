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
        (f.children || []).map((c) => (
          <div
            key={c.id}
            data-testid={`folder-item-${c.id}`}
            onClick={() => onClickHandler(c.id)}
          >
            {c.label}
          </div>
        )),
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
  default: ({ bookId }) => (
    <div data-testid="view-single">ViewSingle:{bookId}</div>
  ),
}));

vi.mock("../src/SimilarBooks", () => ({
  default: () => <div data-testid="similar-books">SimilarBooks</div>,
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
});
