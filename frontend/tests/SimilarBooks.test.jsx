// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";

afterEach(cleanup);

const { mockRawJsonGetReq, mockJsonDeleteReq } = vi.hoisted(() => ({
  mockRawJsonGetReq: vi.fn(),
  mockJsonDeleteReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  rawJsonGetReq: mockRawJsonGetReq,
  jsonDeleteReq: mockJsonDeleteReq,
  getApiUrlPrefix: () => "http://localhost:8000",
}));

import SimilarBooks from "../src/SimilarBooks";

const makeBook = (id, score = 0) => ({
  book_id: id,
  category: "test_category",
  title: `Book ${id}`,
  author: `Author ${id}`,
  file_path: `test_category/Book ${id}.pdf`,
  file_type: "pdf",
  file_size: 1000,
  score,
  updated_time: "2025-01-01T00:00:00.000000",
});

const mockBooks = (books) => {
  mockRawJsonGetReq.mockImplementation((url, resolve) => {
    resolve({ status: "success", result: books, total: books.length });
  });
};

describe("SimilarBooks", () => {
  beforeEach(() => {
    mockRawJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
  });

  // ── 점수 배지 표시 ──

  it("score > 0일 때 점수 배지를 표시한다", async () => {
    mockBooks([makeBook(1, 87.4)]);

    render(<SimilarBooks bookId={1} />);
    fireEvent.click(screen.getByText("유사한 책 목록"));

    await waitFor(() => {
      expect(screen.getByText("87")).toBeTruthy();
    });
  });

  it("file_size가 있으면 바이트 숫자를 점수 왼쪽에 가장 작은 배지로 표시한다", async () => {
    mockBooks([makeBook(1, 87.4)]);

    render(<SimilarBooks bookId={1} />);
    fireEvent.click(screen.getByText("유사한 책 목록"));

    await waitFor(() => {
      expect(screen.getByText("1,000")).toBeTruthy();
      expect(screen.getByText("87")).toBeTruthy();
    });

    expect(screen.queryByText("1,000 B")).toBeNull();

    const fileSizeBadge = screen.getByText("1,000");
    const scoreBadge = screen.getByText("87");
    const badges = Array.from(
      fileSizeBadge.parentElement.querySelectorAll("span"),
    );

    expect(badges.indexOf(fileSizeBadge)).toBeLessThan(
      badges.indexOf(scoreBadge),
    );
    expect(fileSizeBadge.style.padding).toBe("2px 6px");
    expect(fileSizeBadge.style.fontSize).toBe("0.45rem");
    expect(fileSizeBadge.style.lineHeight).toBe("1");
    expect(fileSizeBadge.style.transform).toBe("scale(0.75)");
  });

  it("file_size가 1024 이상이어도 KB나 MB로 변환하지 않는다", async () => {
    mockBooks([
      {
        ...makeBook(1, 87.4),
        file_size: 1048576,
      },
    ]);

    render(<SimilarBooks bookId={1} />);
    fireEvent.click(screen.getByText("유사한 책 목록"));

    await waitFor(() => {
      expect(screen.getByText("1,048,576")).toBeTruthy();
    });

    expect(screen.queryByText(/MB|KB/)).toBeNull();
  });

  it.each([
    ["null", null],
    ["undefined", undefined],
    ["빈 문자열", ""],
    ["음수", -1],
    ["숫자가 아닌 문자열", "abc"],
  ])("file_size가 %s이면 파일 크기 배지를 표시하지 않는다", async (_label, fileSize) => {
    mockBooks([
      {
        ...makeBook(1, 87.4),
        file_size: fileSize,
      },
    ]);

    render(<SimilarBooks bookId={1} />);
    fireEvent.click(screen.getByText("유사한 책 목록"));

    await waitFor(() => {
      expect(screen.getByText("87")).toBeTruthy();
    });

    const scoreBadge = screen.getByText("87");
    const badges = Array.from(
      scoreBadge.parentElement.querySelectorAll("span"),
    );

    expect(badges).toHaveLength(1);
    expect(badges[0]).toBe(scoreBadge);
  });

  it("score가 소수일 때 반올림하여 표시한다", async () => {
    mockBooks([makeBook(1, 92.6)]);

    render(<SimilarBooks bookId={1} />);

    // score >= 90이므로 자동 펼침
    await waitFor(() => {
      expect(screen.getByText("93")).toBeTruthy();
    });
  });

  it("score === 0이면 점수 배지를 표시하지 않는다", async () => {
    mockBooks([makeBook(1, 0)]);

    render(<SimilarBooks bookId={1} />);
    fireEvent.click(screen.getByText("유사한 책 목록"));

    await waitFor(() => {
      // 책 항목은 렌더링됨
      expect(screen.getByRole("button", { name: /\s편집$/ })).toBeTruthy();
    });

    // 점수 0은 배지로 표시되지 않아야 함
    expect(screen.queryByText("0")).toBeNull();
  });

  it("여러 책의 점수 배지가 각각 표시된다", async () => {
    mockBooks([makeBook(1, 95), makeBook(2, 72), makeBook(3, 0)]);

    render(<SimilarBooks bookId={1} />);

    // score >= 90인 책이 있으므로 자동 펼침
    await waitFor(() => {
      expect(screen.getByText("95")).toBeTruthy();
      expect(screen.getByText("72")).toBeTruthy();
    });
  });

  // ── 자동 펼침 ──

  it("90점 이상인 책이 있으면 자동으로 펼쳐진다", async () => {
    mockBooks([makeBook(1, 91), makeBook(2, 60)]);

    render(<SimilarBooks bookId={1} />);

    // 클릭 없이도 자동으로 펼쳐져 책 목록이 보여야 함
    await waitFor(() => {
      expect(screen.getByText("91")).toBeTruthy();
      expect(screen.getByText("60")).toBeTruthy();
    });
  });

  it("90점 미만이면 접힌 상태를 유지한다", async () => {
    mockBooks([makeBook(1, 89), makeBook(2, 50)]);

    render(<SimilarBooks bookId={1} />);

    // 접힌 상태이므로 책 목록이 보이지 않아야 함
    expect(screen.queryByRole("button", { name: /\s편집$/ })).toBeNull();
  });

  it("정확히 90점이면 자동으로 펼쳐진다 (경계값)", async () => {
    mockBooks([makeBook(1, 90)]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("90")).toBeTruthy();
    });
  });

  it("89.9점이면 자동으로 펼쳐지지 않는다 (경계값)", () => {
    mockBooks([makeBook(1, 89.9)]);

    render(<SimilarBooks bookId={1} />);

    expect(screen.queryByRole("button", { name: /\s편집$/ })).toBeNull();
  });

  it("bookId 변경 시 자동 펼침 상태가 초기화된다", async () => {
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        // 첫 번째 책: 90점 이상 → 자동 펼침
        resolve({ status: "success", result: [makeBook(1, 95)], total: 1 });
      } else {
        // 두 번째 책: 90점 미만 → 접힘
        resolve({ status: "success", result: [makeBook(2, 50)], total: 1 });
      }
    });

    const { rerender } = render(<SimilarBooks bookId={1} />);

    // 첫 번째 책: 자동 펼침
    await waitFor(() => {
      expect(screen.getByText("95")).toBeTruthy();
    });

    // bookId 변경 → 접힌 상태로 초기화
    rerender(<SimilarBooks bookId={2} />);

    await waitFor(() => {
      expect(screen.queryByText("95")).toBeNull();
    });
    // 90점 미만이므로 접힌 상태
    expect(screen.queryByText("50")).toBeNull();
    expect(screen.queryByRole("button", { name: /\s편집$/ })).toBeNull();
  });

  it("자동 펼침 후 헤더 클릭으로 닫을 수 있다", async () => {
    mockBooks([makeBook(1, 95)]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("95")).toBeTruthy();
    });

    // 헤더 클릭으로 닫기
    fireEvent.click(screen.getByText("유사한 책 목록"));
    expect(screen.queryByText("95")).toBeNull();
  });

  // ── 90점 이상 하이라이트 ──

  it("90점 이상인 책 행에 highlight-secondary 클래스가 적용된다", async () => {
    mockBooks([makeBook(1, 95), makeBook(2, 70)]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("95")).toBeTruthy();
    });

    const row95 = screen.getByText("95").closest("div[class]");
    const row70 = screen.getByText("70").closest("div[class]");
    expect(row95.classList.contains("highlight-secondary")).toBe(true);
    expect(row70.classList.contains("highlight-secondary")).toBe(false);
  });

  it("정확히 90점이면 highlight-secondary가 적용된다", async () => {
    mockBooks([makeBook(1, 90)]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("90")).toBeTruthy();
    });

    const row = screen.getByText("90").closest("div[class]");
    expect(row.classList.contains("highlight-secondary")).toBe(true);
  });

  it("89점이면 highlight-secondary가 적용되지 않는다", async () => {
    mockBooks([makeBook(1, 89)]);

    render(<SimilarBooks bookId={1} />);
    fireEvent.click(screen.getByText("유사한 책 목록"));

    await waitFor(() => {
      expect(screen.getByText("89")).toBeTruthy();
    });

    const row = screen.getByText("89").closest("div[class]");
    expect(row.classList.contains("highlight-secondary")).toBe(false);
  });

  // ── 기본 동작 ──

  it("bookId가 없으면 API를 호출하지 않는다", () => {
    render(<SimilarBooks />);
    expect(mockRawJsonGetReq).not.toHaveBeenCalled();
  });

  it("헤더 클릭으로 목록을 열고 닫을 수 있다", async () => {
    mockBooks([makeBook(1, 50)]);

    render(<SimilarBooks bookId={1} />);

    // 닫힌 상태에서는 책 목록이 보이지 않음
    expect(screen.queryByRole("button", { name: /\s편집$/ })).toBeNull();

    // 열기
    fireEvent.click(screen.getByText("유사한 책 목록"));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /\s편집$/ })).toBeTruthy();
    });

    // 닫기
    fireEvent.click(screen.getByText("유사한 책 목록"));
    expect(screen.queryByRole("button", { name: /\s편집$/ })).toBeNull();
  });

  it("유사한 책이 없으면 안내 메시지를 표시한다", async () => {
    mockBooks([]);

    render(<SimilarBooks bookId={1} />);
    fireEvent.click(screen.getByText("유사한 책 목록"));

    await waitFor(() => {
      expect(screen.getByText("유사한 책이 없습니다.")).toBeTruthy();
    });
  });

  // ── onSelect 콜백 ──

  it("책 항목 클릭 시 onSelect을 category/book_id로 호출한다", async () => {
    mockBooks([makeBook(42, 95)]);
    const onSelect = vi.fn();

    render(<SimilarBooks bookId={1} onSelect={onSelect} />);

    await waitFor(() => {
      expect(screen.getByText(/Book 42\.pdf/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/Book 42\.pdf/));
    expect(onSelect).toHaveBeenCalledWith("test_category/42");
  });

  it("onSelect이 없어도 책 항목 클릭 시 에러가 발생하지 않는다", async () => {
    mockBooks([makeBook(1, 95)]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText(/Book 1\.pdf/)).toBeTruthy();
    });

    expect(() => {
      fireEvent.click(screen.getByText(/Book 1\.pdf/));
    }).not.toThrow();
  });

  // ── 더 보기 ──

  it('total > 표시된 수일 때 "더 보기" 버튼을 표시한다', async () => {
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      resolve({ status: "success", result: [makeBook(1, 95)], total: 5 });
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });
  });

  it('total === 표시된 수이면 "더 보기" 버튼을 표시하지 않는다', async () => {
    mockBooks([makeBook(1, 95)]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("95")).toBeTruthy();
    });

    expect(screen.queryByText("더 보기")).toBeNull();
  });

  it('"더 보기" 클릭 시 추가 데이터를 로드한다', async () => {
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        resolve({ status: "success", result: [makeBook(2, 80)], total: 2 });
      }
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("더 보기"));

    await waitFor(() => {
      expect(screen.getByText("80")).toBeTruthy();
    });

    // 추가 로드 후 total과 같아지면 "더 보기" 사라짐
    expect(screen.queryByText("더 보기")).toBeNull();
  });

  // ── API 호출 ──

  it("올바른 URL로 API를 호출한다", () => {
    mockBooks([]);

    render(<SimilarBooks bookId={42} />);

    expect(mockRawJsonGetReq).toHaveBeenCalledWith(
      "/similar/42?offset=0&limit=10",
      expect.any(Function),
      expect.any(Function),
    );
  });

  it("bookId 변경 시 새 API를 호출한다", () => {
    mockBooks([]);

    const { rerender } = render(<SimilarBooks bookId={1} />);
    expect(mockRawJsonGetReq).toHaveBeenCalledWith(
      "/similar/1?offset=0&limit=10",
      expect.any(Function),
      expect.any(Function),
    );

    rerender(<SimilarBooks bookId={99} />);
    expect(mockRawJsonGetReq).toHaveBeenCalledWith(
      "/similar/99?offset=0&limit=10",
      expect.any(Function),
      expect.any(Function),
    );
  });

  it("API 에러 시 console.error를 호출한다", () => {
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});
    mockRawJsonGetReq.mockImplementation((url, resolve, reject) => {
      reject(new Error("Network error"));
    });

    render(<SimilarBooks bookId={1} />);

    expect(consoleError).toHaveBeenCalled();
    consoleError.mockRestore();
  });

  // ── 파일명 표시 ──

  it("file_path에서 파일명만 추출하여 표시한다", async () => {
    mockBooks([
      {
        ...makeBook(1, 95),
        file_path: "deep/nested/category/MyBook.epub",
      },
    ]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText(/MyBook\.epub/)).toBeTruthy();
    });

    // 전체 경로가 아닌 파일명만 표시
    expect(screen.queryByText(/deep\/nested/)).toBeNull();
  });

  it("_root 카테고리인 경우 _root 접두어를 제외하고 파일명만 표시한다", async () => {
    mockBooks([
      {
        ...makeBook(1, 95),
        category: "_root",
        file_path: "RootBook.epub",
      },
    ]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("RootBook.epub")).toBeTruthy();
    });

    expect(screen.queryByText("_root/RootBook.epub")).toBeNull();
  });

  it("file_path가 없으면 title로 파일명을 대체한다", async () => {
    mockBooks([
      {
        ...makeBook(1, 95),
        file_path: "",
        title: "대체 제목",
      },
    ]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText(/대체 제목/)).toBeTruthy();
    });
  });

  it("file_path와 title이 모두 없으면 Unknown으로 표시한다", async () => {
    mockBooks([
      {
        ...makeBook(1, 95),
        file_path: "",
        title: "",
      },
    ]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText(/Unknown/)).toBeTruthy();
    });
  });

  it("category가 없으면 _root로 대체하여 카테고리 접두어 없이 표시한다", async () => {
    mockBooks([
      {
        ...makeBook(1, 95),
        category: "",
        file_path: "NoCategoryBook.epub",
      },
    ]);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("NoCategoryBook.epub")).toBeTruthy();
    });
  });

  it("basePath가 빈 문자열이면 기본 경로(/book-edit)를 사용한다", async () => {
    mockBooks([makeBook(42, 95)]);
    const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);

    render(<SimilarBooks bookId={1} basePath="" />);

    await waitFor(() => {
      expect(screen.getByText(/Book 42\.pdf/)).toBeTruthy();
    });

    const editBtns = screen.getAllByRole("button", { name: /\s편집$/ });
    fireEvent.click(editBtns[0]);
    expect(openSpy).toHaveBeenCalledWith(
      expect.stringContaining("/book-edit/42"),
      "_blank",
      "noopener",
    );
    openSpy.mockRestore();
  });

  it('"더 보기" 로드 중 에러 발생 시 loadingMore가 해제된다', async () => {
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve, reject) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        reject(new Error("Network error"));
      }
    });

    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("더 보기"));

    // 에러 후 loadingMore가 false로 돌아와 "더 보기"가 다시 표시됨
    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
      expect(consoleSpy).toHaveBeenCalled();
    });
    consoleSpy.mockRestore();
  });

  it('"더 보기" Enter 키 입력 시 추가 데이터를 로드한다', async () => {
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        resolve({ status: "success", result: [makeBook(2, 80)], total: 2 });
      }
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    const loadMoreEl = screen.getByText("더 보기").closest('[role="button"]');
    fireEvent.keyDown(loadMoreEl, { key: "Enter" });

    await waitFor(() => {
      expect(screen.getByText("80")).toBeTruthy();
    });
  });

  it('"더 보기" Space 키 입력 시 추가 데이터를 로드한다', async () => {
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        resolve({ status: "success", result: [makeBook(2, 70)], total: 2 });
      }
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    const loadMoreEl = screen.getByText("더 보기").closest('[role="button"]');
    fireEvent.keyDown(loadMoreEl, { key: " " });

    await waitFor(() => {
      expect(screen.getByText("70")).toBeTruthy();
    });
  });

  it("편집/조회 버튼 클릭 시 window.open을 호출한다", async () => {
    mockBooks([makeBook(42, 95)]);
    const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText(/Book 42\.pdf/)).toBeTruthy();
    });

    const editBtns = screen.getAllByRole("button", { name: /\s편집$/ });
    fireEvent.click(editBtns[0]);
    expect(openSpy).toHaveBeenCalledWith(
      expect.stringContaining("/book-edit/42"),
      "_blank",
      "noopener",
    );

    const viewBtns = screen.getAllByRole("button", { name: /\s조회$/ });
    fireEvent.click(viewBtns[0]);
    expect(openSpy).toHaveBeenCalledWith(
      expect.stringContaining("/book-view/42"),
      "_blank",
      "noopener",
    );
    openSpy.mockRestore();
  });

  // ── 만화 컨텍스트 (apiPrefix, basePath) ──

  it('apiPrefix="/comics"일 때 /comics/similar/ URL로 API를 호출한다', () => {
    mockBooks([]);

    render(<SimilarBooks bookId={42} apiPrefix="/comics" />);

    expect(mockRawJsonGetReq).toHaveBeenCalledWith(
      "/comics/similar/42?offset=0&limit=10",
      expect.any(Function),
      expect.any(Function),
    );
  });

  it('apiPrefix="/comics"일 때 "더 보기" 클릭 시 /comics prefix로 API를 호출한다', async () => {
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        resolve({ status: "success", result: [makeBook(2, 80)], total: 2 });
      }
    });

    render(<SimilarBooks bookId={1} apiPrefix="/comics" />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("더 보기"));

    await waitFor(() => {
      const secondCall = mockRawJsonGetReq.mock.calls[1];
      expect(secondCall[0]).toMatch(/^\/comics\/similar\//);
    });
  });

  it('basePath="/comics-edit"일 때 편집 버튼이 comics-edit URL로 열린다', async () => {
    mockBooks([makeBook(42, 95)]);
    const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);

    render(<SimilarBooks bookId={1} basePath="/comics-edit" />);

    await waitFor(() => {
      expect(screen.getByText(/Book 42\.pdf/)).toBeTruthy();
    });

    const editBtns = screen.getAllByRole("button", { name: /\s편집$/ });
    fireEvent.click(editBtns[0]);
    expect(openSpy).toHaveBeenCalledWith(
      expect.stringContaining("/comics-edit/42"),
      "_blank",
      "noopener",
    );
    openSpy.mockRestore();
  });

  it('basePath="/comics-edit"일 때 조회 버튼이 comics-view URL로 열린다', async () => {
    mockBooks([makeBook(42, 95)]);
    const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);

    render(<SimilarBooks bookId={1} basePath="/comics-edit" />);

    await waitFor(() => {
      expect(screen.getByText(/Book 42\.pdf/)).toBeTruthy();
    });

    const viewBtns = screen.getAllByRole("button", { name: /\s조회$/ });
    fireEvent.click(viewBtns[0]);
    expect(openSpy).toHaveBeenCalledWith(
      expect.stringContaining("/comics-view/42"),
      "_blank",
      "noopener",
    );
    openSpy.mockRestore();
  });

  // ── 응답 falsy / 비정상 분기 ──

  it("초기 응답 status가 success가 아니면 목록을 갱신하지 않는다", async () => {
    // 32번 라인: data.status === "success" false arm
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      resolve({ status: "error", result: [makeBook(1, 95)], total: 1 });
    });

    render(<SimilarBooks bookId={1} />);

    // success가 아니므로 자동 펼침/목록 갱신이 없어 헤더만 존재
    await waitFor(() => {
      expect(screen.getByText("유사한 책 목록")).toBeTruthy();
    });
    // 수동으로 펼쳐도 목록은 비어있어 안내 메시지가 표시됨
    fireEvent.click(screen.getByText("유사한 책 목록"));
    await waitFor(() => {
      expect(screen.getByText("유사한 책이 없습니다.")).toBeTruthy();
    });
  });

  it("초기 응답 result가 없으면 빈 배열로 처리한다", async () => {
    // 33번 라인: data.result || [] 의 || [] arm
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      resolve({ status: "success", total: 0 });
    });

    render(<SimilarBooks bookId={1} />);
    fireEvent.click(screen.getByText("유사한 책 목록"));

    await waitFor(() => {
      expect(screen.getByText("유사한 책이 없습니다.")).toBeTruthy();
    });
  });

  it('초기 응답 total이 없으면 0으로 처리하여 "더 보기"를 표시하지 않는다', async () => {
    // 35번 라인: data.total || 0 의 || 0 arm
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      resolve({ status: "success", result: [makeBook(1, 95)] });
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("95")).toBeTruthy();
    });
    // total=0 < length=1 이므로 hasMore=false → "더 보기" 없음
    expect(screen.queryByText("더 보기")).toBeNull();
  });

  it('"더 보기" 응답 status가 success가 아니면 목록을 추가하지 않지만 loadingMore는 해제된다', async () => {
    // 60번 라인: data.status === "success" && data.result 의 false arm
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        // success가 아닌 응답 → 추가 안 됨, loadingMore만 해제
        resolve({ status: "error", result: [makeBook(2, 80)], total: 2 });
      }
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("더 보기"));

    // 두 번째 책은 추가되지 않음
    await waitFor(() => {
      expect(screen.queryByText("80")).toBeNull();
    });
    // loadingMore 해제 후 hasMore는 여전히 true(1<2)이므로 "더 보기" 재표시
    expect(screen.getByText("더 보기")).toBeTruthy();
  });

  it('"더 보기" 응답 result가 없으면 목록을 추가하지 않는다', async () => {
    // 60번 라인: ... && data.result 의 result falsy arm
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        resolve({ status: "success", total: 2 });
      }
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("더 보기"));

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });
    // 추가된 항목 없음 → 여전히 1개만
    expect(screen.getAllByRole("button", { name: /\s편집$/ })).toHaveLength(1);
  });

  it('"더 보기" 응답 total이 없으면 0으로 처리한다', async () => {
    // 62번 라인: data.total || 0 의 || 0 arm (load-more 경로)
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        // total 누락 → 0으로 처리, 항목은 추가됨
        resolve({ status: "success", result: [makeBook(2, 80)] });
      }
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("더 보기"));

    await waitFor(() => {
      expect(screen.getByText("80")).toBeTruthy();
    });
    // total=0 < length=2 → hasMore false → "더 보기" 사라짐
    expect(screen.queryByText("더 보기")).toBeNull();
  });

  // ── loadingMore 진행 중 분기 (로딩 표시 / 중복 호출 가드) ──

  it('"더 보기" 로드 중에는 "로딩 중..."을 표시하고 disabled 상태가 된다', async () => {
    // 163-164, 172번 라인: loadingMore true arm (className disabled, onClick undefined, "로딩 중..." 텍스트)
    let secondResolve = null;
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 2 });
      } else {
        // 두 번째 호출은 보류하여 loadingMore=true 상태 유지
        secondResolve = resolve;
      }
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    const loadMoreEl = screen.getByText("더 보기").closest('[role="button"]');
    fireEvent.click(screen.getByText("더 보기"));

    // 로딩 중 텍스트 및 disabled 클래스 표시
    await waitFor(() => {
      expect(screen.getByText("로딩 중...")).toBeTruthy();
    });
    expect(loadMoreEl.classList.contains("disabled")).toBe(true);

    // 보류했던 응답을 해제하여 후속 상태 정리
    secondResolve({ status: "success", result: [makeBook(2, 80)], total: 2 });
    await waitFor(() => {
      expect(screen.getByText("80")).toBeTruthy();
    });
  });

  it('로드 중 "더 보기"를 다시 클릭해도 중복 요청을 보내지 않는다', async () => {
    // 54번 라인: if (loadingMore) return; 의 early-return arm
    // 164번 라인: onClick={loadingMore ? undefined : handleLoadMore} 의 undefined arm
    let callCount = 0;
    const resolvers = [];
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount++;
      if (callCount === 1) {
        resolve({ status: "success", result: [makeBook(1, 95)], total: 3 });
      } else {
        // 후속 호출은 보류
        resolvers.push(resolve);
      }
    });

    render(<SimilarBooks bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("더 보기")).toBeTruthy();
    });

    // 첫 클릭 → loadingMore=true, 두 번째 호출 발생(보류)
    fireEvent.click(screen.getByText("더 보기"));
    await waitFor(() => {
      expect(screen.getByText("로딩 중...")).toBeTruthy();
    });

    expect(callCount).toBe(2);

    // 로딩 중 엘리먼트를 Enter/Space로 다시 시도 → 가드로 추가 호출 없음
    const loadMoreEl = screen
      .getByText("로딩 중...")
      .closest('[role="button"]');
    fireEvent.keyDown(loadMoreEl, { key: "Enter" });
    fireEvent.keyDown(loadMoreEl, { key: " " });

    // 여전히 2회 (초기 1 + load-more 1)
    expect(callCount).toBe(2);

    // 보류 응답 해제하여 정리
    resolvers[0]({ status: "success", result: [makeBook(2, 80)], total: 3 });
    await waitFor(() => {
      expect(screen.getByText("80")).toBeTruthy();
    });
  });
  // ── 삭제 버튼 ──

  describe("삭제 버튼", () => {
    let confirmSpy;
    let alertSpy;
    let errorSpy;

    beforeEach(() => {
      confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
      alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
      errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    });

    afterEach(() => {
      confirmSpy.mockRestore();
      alertSpy.mockRestore();
      errorSpy.mockRestore();
    });

    const deleteButtons = () =>
      screen.getAllByRole("button", { name: /\s삭제$/ });

    const openList = async () => {
      render(<SimilarBooks bookId={1} apiPrefix="/comics" />);
      fireEvent.click(screen.getByText("유사한 책 목록"));
      await waitFor(() => {
        expect(deleteButtons().length).toBeGreaterThan(0);
      });
    };

    it("confirm을 취소하면 삭제 API를 호출하지 않는다", async () => {
      mockBooks([makeBook(1, 50)]);
      confirmSpy.mockReturnValue(false);

      await openList();
      fireEvent.click(deleteButtons()[0]);

      expect(confirmSpy).toHaveBeenCalledWith(
        '"test_category/Book 1.pdf"을(를) 삭제하시겠습니까?',
      );
      expect(mockJsonDeleteReq).not.toHaveBeenCalled();
    });

    it("확인하면 apiPrefix가 붙은 삭제 API를 호출하고 목록에서 제거한다", async () => {
      mockBooks([makeBook(1, 50), makeBook(2, 40)]);
      mockJsonDeleteReq.mockImplementation((url, payload, resolve) => {
        resolve(null);
      });

      await openList();
      fireEvent.click(deleteButtons()[0]);

      expect(mockJsonDeleteReq.mock.calls[0][0]).toBe("/comics/books/1");
      await waitFor(() => {
        expect(screen.queryByText("test_category/Book 1.pdf")).toBeNull();
      });
      expect(screen.getByText("test_category/Book 2.pdf")).toBeTruthy();
    });

    it("삭제에 실패하면 알림을 띄우고 목록을 유지한다", async () => {
      mockBooks([makeBook(1, 50)]);
      mockJsonDeleteReq.mockImplementation((url, payload, resolve, reject) => {
        reject("permission denied");
      });

      await openList();
      fireEvent.click(deleteButtons()[0]);

      await waitFor(() => {
        expect(alertSpy).toHaveBeenCalledWith(
          "책 삭제에 실패했습니다. permission denied",
        );
      });
      expect(screen.getByText("test_category/Book 1.pdf")).toBeTruthy();
    });

    it("삭제 진행 중에는 중복 요청을 막고 해당 버튼만 스피너를 돌린다", async () => {
      mockBooks([makeBook(1, 50), makeBook(2, 40)]);
      const resolvers = [];
      mockJsonDeleteReq.mockImplementation((url, payload, resolve) => {
        resolvers.push(resolve);
      });

      await openList();
      fireEvent.click(deleteButtons()[0]);
      fireEvent.click(deleteButtons()[1]);

      expect(mockJsonDeleteReq).toHaveBeenCalledTimes(1);

      await waitFor(() => {
        expect(deleteButtons()[0].getAttribute("aria-busy")).toBe("true");
      });
      expect(deleteButtons()[0].querySelector(".fa-spin")).toBeTruthy();
      expect(deleteButtons()[1].getAttribute("aria-busy")).toBe("false");
      expect(deleteButtons()[1].querySelector(".fa-spin")).toBeNull();
      expect(deleteButtons()[1].disabled).toBe(true);

      resolvers[0](null);
      await waitFor(() => {
        expect(screen.queryByText("test_category/Book 1.pdf")).toBeNull();
      });
      expect(deleteButtons()[0].getAttribute("aria-busy")).toBe("false");
      expect(deleteButtons()[0].disabled).toBe(false);
    });
  });
  // ── 새로고침 버튼 ──

  describe("새로고침 버튼", () => {
    const refreshButton = () =>
      screen.getByLabelText("유사한 책 목록 새로고침");

    it("클릭하면 첫 페이지를 다시 요청하고 목록을 교체한다", async () => {
      let callCount = 0;
      mockRawJsonGetReq.mockImplementation((url, resolve) => {
        callCount++;
        resolve(
          callCount === 1
            ? { status: "success", result: [makeBook(1, 50)], total: 1 }
            : { status: "success", result: [makeBook(2, 60)], total: 1 },
        );
      });

      render(<SimilarBooks bookId={1} apiPrefix="/comics" />);
      fireEvent.click(screen.getByText("유사한 책 목록"));
      await waitFor(() => {
        expect(screen.getByText("test_category/Book 1.pdf")).toBeTruthy();
      });

      fireEvent.click(refreshButton());

      expect(mockRawJsonGetReq.mock.calls[1][0]).toBe(
        "/comics/similar/1?offset=0&limit=10",
      );
      await waitFor(() => {
        expect(screen.getByText("test_category/Book 2.pdf")).toBeTruthy();
      });
      expect(screen.queryByText("test_category/Book 1.pdf")).toBeNull();
    });

    it("접힌 상태에서 클릭하면 목록을 펼치고 헤더 토글은 일어나지 않는다", async () => {
      mockBooks([makeBook(1, 50)]);

      render(<SimilarBooks bookId={1} />);
      expect(screen.queryByText("test_category/Book 1.pdf")).toBeNull();

      fireEvent.click(refreshButton());

      await waitFor(() => {
        expect(screen.getByText("test_category/Book 1.pdf")).toBeTruthy();
      });
    });

    it("요청 중에는 스피너를 돌리고 중복 요청을 막는다", async () => {
      const resolvers = [];
      let callCount = 0;
      mockRawJsonGetReq.mockImplementation((url, resolve) => {
        callCount++;
        if (callCount === 1) {
          resolve({ status: "success", result: [makeBook(1, 50)], total: 1 });
        } else {
          resolvers.push(resolve);
        }
      });

      render(<SimilarBooks bookId={1} />);
      fireEvent.click(screen.getByText("유사한 책 목록"));
      await waitFor(() => {
        expect(screen.getByText("test_category/Book 1.pdf")).toBeTruthy();
      });

      fireEvent.click(refreshButton());
      await waitFor(() => {
        expect(refreshButton().getAttribute("aria-busy")).toBe("true");
      });
      expect(refreshButton().querySelector(".fa-spin")).toBeTruthy();
      expect(refreshButton().disabled).toBe(true);

      fireEvent.click(refreshButton());
      expect(callCount).toBe(2);

      resolvers[0]({ status: "success", result: [makeBook(2, 60)], total: 1 });
      await waitFor(() => {
        expect(refreshButton().getAttribute("aria-busy")).toBe("false");
      });
      expect(refreshButton().querySelector(".fa-spin")).toBeNull();
    });

    it("bookId가 없으면 새로고침 요청을 보내지 않는다", () => {
      mockBooks([]);

      render(<SimilarBooks />);
      fireEvent.click(refreshButton());

      expect(mockRawJsonGetReq).not.toHaveBeenCalled();
    });

    it("새로고침 요청이 실패하면 스피너를 멈추고 목록을 유지한다", async () => {
      const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
      let callCount = 0;
      mockRawJsonGetReq.mockImplementation((url, resolve, reject) => {
        callCount++;
        if (callCount === 1) {
          resolve({ status: "success", result: [makeBook(1, 50)], total: 1 });
        } else {
          reject("network error");
        }
      });

      render(<SimilarBooks bookId={1} />);
      fireEvent.click(screen.getByText("유사한 책 목록"));
      await waitFor(() => {
        expect(screen.getByText("test_category/Book 1.pdf")).toBeTruthy();
      });

      fireEvent.click(refreshButton());

      await waitFor(() => {
        expect(refreshButton().getAttribute("aria-busy")).toBe("false");
      });
      expect(screen.getByText("test_category/Book 1.pdf")).toBeTruthy();
      expect(errorSpy).toHaveBeenCalledWith("network error");
      errorSpy.mockRestore();
    });
  });
});
