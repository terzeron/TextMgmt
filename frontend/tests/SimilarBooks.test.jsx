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

const { mockRawJsonGetReq } = vi.hoisted(() => ({
  mockRawJsonGetReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  rawJsonGetReq: mockRawJsonGetReq,
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
      expect(screen.getByText("편집")).toBeTruthy();
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
    expect(screen.queryByText("편집")).toBeNull();
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

    expect(screen.queryByText("편집")).toBeNull();
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
    expect(screen.queryByText("편집")).toBeNull();
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
    expect(screen.queryByText("편집")).toBeNull();

    // 열기
    fireEvent.click(screen.getByText("유사한 책 목록"));
    await waitFor(() => {
      expect(screen.getByText("편집")).toBeTruthy();
    });

    // 닫기
    fireEvent.click(screen.getByText("유사한 책 목록"));
    expect(screen.queryByText("편집")).toBeNull();
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

    const editBtns = screen.getAllByText("편집");
    fireEvent.click(editBtns[0]);
    expect(openSpy).toHaveBeenCalledWith(
      expect.stringContaining("/book-edit/42"),
      "_blank",
      "noopener",
    );

    const viewBtns = screen.getAllByText("조회");
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

    const editBtns = screen.getAllByText("편집");
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

    const viewBtns = screen.getAllByText("조회");
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
    expect(screen.getAllByText("편집")).toHaveLength(1);
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
});
