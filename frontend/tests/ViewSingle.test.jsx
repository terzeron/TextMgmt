// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  fireEvent,
  cleanup,
} from "@testing-library/react";
import * as Common from "../src/Common";

afterEach(cleanup);

const { mockUseParams, mockUseSearchParams } = vi.hoisted(() => ({
  mockUseParams: vi.fn(() => ({})),
  mockUseSearchParams: vi.fn(() => [new URLSearchParams()]),
}));

vi.mock("react-router-dom", () => ({
  useParams: mockUseParams,
  useSearchParams: mockUseSearchParams,
}));

vi.mock("../src/Common", async () => {
  const actual = await vi.importActual("../src/Common");
  return {
    ...actual,
    jsonGetReq: vi.fn(),
    getApiUrlPrefix: vi.fn(() => "/api"),
  };
});

vi.mock("../src/ViewPDF", () => ({
  default: ({ bookId, pageCount, apiPrefix }) => (
    <div data-testid="view-pdf">
      PDF:{bookId}:pc={pageCount}:ap={apiPrefix || ""}
    </div>
  ),
}));
vi.mock("../src/ViewEPUB", () => ({
  default: ({ bookId, preview, apiPrefix }) => (
    <div data-testid="view-epub">
      EPUB:{bookId}:preview={String(!!preview)}:ap={apiPrefix || ""}
    </div>
  ),
}));
vi.mock("../src/ViewDOC", () => ({
  default: ({ bookId, fileType, lineCount, apiPrefix }) => (
    <div data-testid="view-doc">
      DOC:{bookId}:ft={fileType}:lc={lineCount}:ap={apiPrefix || ""}
    </div>
  ),
}));
vi.mock("../src/ViewTXT", () => ({
  default: ({ bookId, lineCount, apiPrefix }) => (
    <div data-testid="view-txt">
      TXT:{bookId}:lc={lineCount}:ap={apiPrefix || ""}
    </div>
  ),
}));
vi.mock("../src/ViewHTML", () => ({
  default: ({ bookId, apiPrefix }) => (
    <div data-testid="view-html">
      HTML:{bookId}:ap={apiPrefix || ""}
    </div>
  ),
}));
vi.mock("../src/ViewRTF", () => ({
  default: ({ bookId, apiPrefix }) => (
    <div data-testid="view-rtf">
      RTF:{bookId}:ap={apiPrefix || ""}
    </div>
  ),
}));
vi.mock("../src/ViewImage", () => ({
  default: ({ bookId, apiPrefix }) => (
    <div data-testid="view-image">
      IMG:{bookId}:ap={apiPrefix || ""}
    </div>
  ),
}));

import ViewSingle from "../src/ViewSingle";

describe("ViewSingle", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUseParams.mockReturnValue({});
    mockUseSearchParams.mockReturnValue([new URLSearchParams()]);
    vi.stubGlobal("location", { href: "" });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  // ── 기본 렌더링 ──

  it("bookId가 없으면 안내 메시지를 표시한다", () => {
    render(<ViewSingle />);
    expect(screen.getByText("책이 선택되지 않았습니다.")).toBeTruthy();
  });

  it("props로 전달된 bookId에 해당하는 뷰어를 렌더링한다", async () => {
    render(<ViewSingle bookId={42} fileType="pdf" filePath="/test.pdf" />);
    await waitFor(() => {
      expect(screen.getByTestId("view-pdf")).toBeTruthy();
    });
  });

  // ── fileType별 뷰어 매핑 ──

  it.each([
    ["pdf", "view-pdf"],
    ["epub", "view-epub"],
    ["doc", "view-doc"],
    ["docx", "view-doc"],
    ["hwp", "view-doc"],
    ["txt", "view-txt"],
    ["html", "view-html"],
    ["rtf", "view-rtf"],
    ["jpg", "view-image"],
    ["gif", "view-image"],
    ["png", "view-image"],
  ])('fileType="%s"이면 %s 뷰어를 렌더링한다', async (fileType, testId) => {
    render(
      <ViewSingle
        bookId={1}
        fileType={fileType}
        filePath={`/test.${fileType}`}
      />,
    );
    await waitFor(() => {
      expect(screen.getByTestId(testId)).toBeTruthy();
    });
  });

  // ── standalone 모드 ──

  it("standalone 모드에서 body 스타일을 잠근다", async () => {
    mockUseParams.mockReturnValue({ entryId: "10", fileType: "pdf" });
    mockUseSearchParams.mockReturnValue([
      new URLSearchParams("path=/test.pdf"),
    ]);

    const { unmount } = render(<ViewSingle />);
    expect(document.documentElement.style.overflow).toBe("hidden");
    expect(document.body.style.position).toBe("fixed");

    unmount();
    expect(document.documentElement.style.overflow).toBe("");
  });

  it("standalone 모드에서 카테고리 정보를 로드하고 네비게이션을 처리한다", async () => {
    mockUseParams.mockReturnValue({ entryId: "101", fileType: "epub" });
    mockUseSearchParams.mockReturnValue([
      new URLSearchParams("path=1_fiction/a.epub&category=1_fiction"),
    ]);

    Common.jsonGetReq.mockImplementation((url, payload, resolve) => {
      resolve([
        {
          book_id: 100,
          title: "Book 0",
          file_type: "epub",
          file_path: "1_fiction/0.epub",
        },
        {
          book_id: 101,
          title: "Book 1",
          file_type: "epub",
          file_path: "1_fiction/a.epub",
        },
        {
          book_id: 102,
          title: "Book 2",
          file_type: "epub",
          file_path: "1_fiction/b.epub",
        },
      ]);
    });

    render(<ViewSingle />);

    await waitFor(() => {
      expect(Common.jsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("/categories/1_fiction"),
        null,
        expect.any(Function),
      );
    });

    await waitFor(() => {
      expect(screen.getByText(/이전 책으로/)).toBeTruthy();
      expect(screen.getByText(/다음 책으로/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/다음 책으로/));
    expect(window.location.href).toContain("102");
  });

  // ── standalone 네비게이션 경계 ──

  const BOOKS = [
    {
      book_id: 100,
      title: "Book 0",
      file_type: "epub",
      file_path: "1_fiction/0.epub",
    },
    {
      book_id: 101,
      title: "Book 1",
      file_type: "epub",
      file_path: "1_fiction/a.epub",
    },
    {
      book_id: 102,
      title: "Book 2",
      file_type: "epub",
      file_path: "1_fiction/b.epub",
    },
  ];

  it("첫 번째 책에서는 이전 책 버튼이 비활성화된다", async () => {
    mockUseParams.mockReturnValue({ entryId: "100", fileType: "epub" });
    mockUseSearchParams.mockReturnValue([
      new URLSearchParams("path=1_fiction/0.epub&category=1_fiction"),
    ]);
    Common.jsonGetReq.mockImplementation((url, payload, resolve) =>
      resolve(BOOKS),
    );

    render(<ViewSingle />);

    await waitFor(() => {
      expect(screen.getByText(/다음 책으로/)).toBeTruthy();
    });
    expect(screen.getByText(/이전 책으로/).closest("button").disabled).toBe(
      true,
    );
  });

  it("마지막 책에서는 다음 책 버튼이 비활성화된다", async () => {
    mockUseParams.mockReturnValue({ entryId: "102", fileType: "epub" });
    mockUseSearchParams.mockReturnValue([
      new URLSearchParams("path=1_fiction/b.epub&category=1_fiction"),
    ]);
    Common.jsonGetReq.mockImplementation((url, payload, resolve) =>
      resolve(BOOKS),
    );

    render(<ViewSingle />);

    await waitFor(() => {
      expect(screen.getByText(/이전 책으로/)).toBeTruthy();
    });
    expect(screen.getByText(/다음 책으로/).closest("button").disabled).toBe(
      true,
    );
  });

  it("api 파라미터가 있으면 이전 책 이동 URL 에 api 를 포함한다", async () => {
    mockUseParams.mockReturnValue({ entryId: "101", fileType: "epub" });
    mockUseSearchParams.mockReturnValue([
      new URLSearchParams(
        "path=1_fiction/a.epub&category=1_fiction&api=/comics",
      ),
    ]);
    Common.jsonGetReq.mockImplementation((url, payload, resolve) =>
      resolve(BOOKS),
    );

    render(<ViewSingle />);

    await waitFor(() => {
      expect(screen.getByText(/이전 책으로/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/이전 책으로/));
    expect(window.location.href).toContain("100");
    expect(window.location.href).toContain("api=");
  });

  // ── 카테고리 응답 엣지 케이스 ──

  it("카테고리 응답이 배열이 아니면 이전/다음 책 정보를 설정하지 않는다", async () => {
    mockUseParams.mockReturnValue({ entryId: "101", fileType: "epub" });
    mockUseSearchParams.mockReturnValue([
      new URLSearchParams("path=1_fiction/a.epub&category=1_fiction"),
    ]);
    Common.jsonGetReq.mockImplementation((url, payload, resolve) =>
      resolve({ not: "an array" }),
    );

    render(<ViewSingle />);

    await waitFor(() => {
      expect(Common.jsonGetReq).toHaveBeenCalled();
    });

    expect(screen.queryByText(/이전 책으로/)).toBeNull();
    expect(screen.queryByText(/다음 책으로/)).toBeNull();
  });

  it("책 목록에 title이 없는 항목이 있어도 정렬 시 오류 없이 처리한다", async () => {
    mockUseParams.mockReturnValue({ entryId: "101", fileType: "epub" });
    mockUseSearchParams.mockReturnValue([
      new URLSearchParams("path=1_fiction/a.epub&category=1_fiction"),
    ]);
    const BOOKS_NO_TITLE = [
      {
        book_id: 100,
        title: "Book 0",
        file_type: "epub",
        file_path: "1_fiction/0.epub",
      },
      { book_id: 101, file_type: "epub", file_path: "1_fiction/a.epub" }, // title 없음
      {
        book_id: 102,
        title: "Book 2",
        file_type: "epub",
        file_path: "1_fiction/b.epub",
      },
    ];
    Common.jsonGetReq.mockImplementation((url, payload, resolve) =>
      resolve(BOOKS_NO_TITLE),
    );

    render(<ViewSingle />);

    await waitFor(() => {
      expect(screen.getByText(/다음 책으로/)).toBeTruthy();
    });
    // title이 없는 책(101)이 정렬상 맨 앞으로 오므로 이전 책 버튼은 비활성화된다
    expect(screen.getByText(/이전 책으로/).closest("button").disabled).toBe(
      true,
    );
  });

  it("이동할 책에 file_type/file_path가 없으면 빈 값으로 URL을 구성한다", async () => {
    mockUseParams.mockReturnValue({ entryId: "300", fileType: "epub" });
    mockUseSearchParams.mockReturnValue([
      new URLSearchParams("path=1_fiction/a.epub&category=1_fiction"),
    ]);
    const BOOKS_MISSING_FIELDS = [
      {
        book_id: 300,
        title: "A",
        file_type: "epub",
        file_path: "1_fiction/a.epub",
      },
      { book_id: 301, title: "B" }, // file_type, file_path 없음
    ];
    Common.jsonGetReq.mockImplementation((url, payload, resolve) =>
      resolve(BOOKS_MISSING_FIELDS),
    );

    render(<ViewSingle />);

    await waitFor(() => {
      expect(screen.getByText(/다음 책으로/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/다음 책으로/));
    expect(window.location.href).toContain("/viewer//301");
    expect(window.location.href).toContain("path=&");
  });

  // ── 기타 버튼 및 엣지 케이스 ──

  it("viewUrl과 downloadUrl 버튼을 올바르게 렌더링한다", async () => {
    render(
      <ViewSingle
        bookId={1}
        fileType="pdf"
        filePath="/test.pdf"
        viewUrl="http://view"
        downloadUrl="http://dl"
      />,
    );
    await waitFor(() => {
      expect(screen.getByText("전체보기")).toBeTruthy();
      expect(screen.getByText("다운로드")).toBeTruthy();
    });
  });

  it("admin 역할일 때 편집 버튼을 표시한다", async () => {
    render(
      <ViewSingle
        bookId={1}
        fileType="pdf"
        filePath="/test.pdf"
        role="admin"
        editUrl="/edit/1"
      />,
    );
    await waitFor(() => {
      expect(screen.getByText("편집")).toBeTruthy();
    });
  });
});
