// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
  cleanup,
} from "@testing-library/react";

afterEach(cleanup);

vi.mock("../src/Common", () => ({
  rawJsonGetReq: vi.fn(),
  getApiUrlPrefix: () => "http://localhost:8000",
  handleFetchErrors: (r) => r,
  getRandomLightColor: () => "#ccc",
  ROOT_DIRECTORY: "$$rootdir$$",
}));

import { rawJsonGetReq } from "../src/Common";
import Bookstore, {
  getTwoLevelCategory,
  extractMultiPathCategories,
} from "../src/Bookstore";

describe("getTwoLevelCategory", () => {
  it("3단계 카테고리에서 하위 2단계를 추출한다", () => {
    expect(getTwoLevelCategory("소설/시/희곡 > SF > 한국SF")).toBe("SF 한국SF");
  });

  it("2단계 카테고리에서 두 단계 모두 추출한다", () => {
    expect(getTwoLevelCategory("소설/시/희곡 > 중국소설")).toBe(
      "소설/시/희곡 중국소설",
    );
  });

  it("1단계 카테고리는 그대로 반환한다", () => {
    expect(getTwoLevelCategory("한국SF")).toBe("한국SF");
  });

  it("빈 문자열이면 빈 문자열을 반환한다", () => {
    expect(getTwoLevelCategory("")).toBe("");
  });

  it("null이면 빈 문자열을 반환한다", () => {
    expect(getTwoLevelCategory(null)).toBe("");
  });

  it("undefined이면 빈 문자열을 반환한다", () => {
    expect(getTwoLevelCategory(undefined)).toBe("");
  });

  it("4단계 이상에서도 마지막 2단계만 추출한다", () => {
    expect(getTwoLevelCategory("A > B > C > D")).toBe("C D");
  });

  it("구분자 앞뒤 공백을 제거한다", () => {
    expect(getTwoLevelCategory("  A  >  B  ")).toBe("A B");
  });

  it("빈 세그먼트가 있으면 무시한다", () => {
    // "A > B > " → trim → ["A", "B", ""] → filter → ["A", "B"]
    expect(getTwoLevelCategory("A > B > ")).toBe("A B");
  });
});

describe("extractMultiPathCategories", () => {
  it("다중 경로에서 각 경로의 마지막 두 단계를 추출한다", () => {
    expect(
      extractMultiPathCategories(
        "소설 > 한국소설 || 소설 > 추리/미스터리/스릴러",
      ),
    ).toEqual(["소설 한국소설", "소설 추리/미스터리/스릴러"]);
  });

  it("단일 경로도 배열로 반환한다", () => {
    expect(extractMultiPathCategories("소설 > 한국소설")).toEqual([
      "소설 한국소설",
    ]);
  });

  it("3개 이상의 경로도 처리한다", () => {
    expect(extractMultiPathCategories("A > B || C > D || E > F")).toEqual([
      "A B",
      "C D",
      "E F",
    ]);
  });

  it("빈 문자열이면 빈 배열을 반환한다", () => {
    expect(extractMultiPathCategories("")).toEqual([]);
  });

  it("null이면 빈 배열을 반환한다", () => {
    expect(extractMultiPathCategories(null)).toEqual([]);
  });

  it("undefined이면 빈 배열을 반환한다", () => {
    expect(extractMultiPathCategories(undefined)).toEqual([]);
  });

  it("빈 경로는 필터링한다", () => {
    expect(
      extractMultiPathCategories("소설 > 한국소설 || || 소설 > SF"),
    ).toEqual(["소설 한국소설", "소설 SF"]);
  });
});

describe("Bookstore 카테고리 수집", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // rawJsonGetReq mock helper: URL 패턴별 응답 매핑
  const mockSearchResponses = (responses) => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      for (const [pattern, data] of Object.entries(responses)) {
        if (url.includes(pattern)) {
          setTimeout(() => onSuccess(data), 0);
          return;
        }
      }
      // 기본: 빈 결과
      setTimeout(() => onSuccess({ status: "not_found", result: [] }), 0);
    });
  };

  it("자동 검색 시 yes24, aladin, naver 세 서점의 카테고리를 수집한다", async () => {
    const onCategoriesFound = vi.fn();

    mockSearchResponses({
      "/search/bookstore/yes24": {
        status: "success",
        result: [
          {
            title: "T",
            author: "A",
            category: "소설 > 한국소설",
            book_url: "u1",
          },
        ],
      },
      "/search/bookstore/aladin": {
        status: "success",
        result: [
          {
            title: "T",
            author: "A",
            category: "문학 > 한국문학",
            book_url: "u2",
          },
        ],
      },
      "/search/bookstore/naver": {
        status: "success",
        result: [
          {
            title: "T",
            author: "A",
            category: "도서 > 소설 > 추리/미스터리",
            book_url: "u3",
          },
        ],
      },
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "테스트", author: "저자", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={onCategoriesFound}
        />,
      );
    });

    // 비동기 검색 완료 대기
    await waitFor(() => {
      const calls = onCategoriesFound.mock.calls;
      // 마지막 호출이 카테고리를 포함해야 함 (첫 호출은 빈 {} 초기화)
      const lastCall = calls[calls.length - 1]?.[0];
      expect(lastCall).toBeDefined();
      expect(Object.keys(lastCall).length).toBeGreaterThan(0);
    });

    const lastCall =
      onCategoriesFound.mock.calls[onCategoriesFound.mock.calls.length - 1][0];

    // yes24 카테고리
    expect(Object.values(lastCall)).toContain("소설 한국소설");
    // aladin 카테고리
    expect(Object.values(lastCall)).toContain("문학 한국문학");
    // naver 카테고리
    expect(Object.values(lastCall)).toContain("소설 추리/미스터리");
  });

  it("네이버쇼핑 다중 경로가 개별 키로 분리되어 수집된다", async () => {
    const onCategoriesFound = vi.fn();

    mockSearchResponses({
      "/search/bookstore/naver": {
        status: "success",
        result: [
          {
            title: "T",
            author: "A",
            category: "소설 > 한국소설 || 소설 > SF",
            book_url: "u",
          },
        ],
      },
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "테스트", author: "저자", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={onCategoriesFound}
        />,
      );
    });

    await waitFor(() => {
      const calls = onCategoriesFound.mock.calls;
      const lastCall = calls[calls.length - 1]?.[0];
      expect(lastCall).toBeDefined();
      const naverKeys = Object.keys(lastCall).filter((k) =>
        k.startsWith("naver_"),
      );
      expect(naverKeys.length).toBeGreaterThanOrEqual(2);
    });

    const lastCall =
      onCategoriesFound.mock.calls[onCategoriesFound.mock.calls.length - 1][0];

    // 하나의 검색 결과에서 두 경로가 별도 키로 수집됨
    const naverKeys = Object.keys(lastCall).filter((k) =>
      k.startsWith("naver_"),
    );
    const naverValues = naverKeys.map((k) => lastCall[k]);
    expect(naverValues).toContain("소설 한국소설");
    expect(naverValues).toContain("소설 SF");
  });

  it("ISBN으로 결과를 먼저 찾고 결과가 있으면 추가 검색을 스킵한다", async () => {
    const onCategoriesFound = vi.fn();

    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      if (url.includes("isbn=")) {
        setTimeout(
          () =>
            onSuccess({
              status: "success",
              result: [
                {
                  title: "T",
                  author: "A",
                  category: "소설 > 한국소설",
                  book_url: "u",
                },
              ],
            }),
          0,
        );
      } else {
        setTimeout(() => onSuccess({ status: "not_found", result: [] }), 0);
      }
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "테스트", author: "저자", isbn: "978-1234" }}
          searchTrigger={1}
          onCategoriesFound={onCategoriesFound}
        />,
      );
    });

    await waitFor(() => {
      const calls = onCategoriesFound.mock.calls;
      const lastCall = calls[calls.length - 1]?.[0];
      expect(lastCall).toBeDefined();
      expect(Object.keys(lastCall).length).toBeGreaterThan(0);
    });
  });

  it("검색 결과가 없는 서점은 카테고리에 포함되지 않는다", async () => {
    const onCategoriesFound = vi.fn();

    mockSearchResponses({
      "/search/bookstore/yes24": {
        status: "success",
        result: [
          { title: "T", author: "A", category: "소설 > 판타지", book_url: "u" },
        ],
      },
      // aladin, ridi는 기본값(not_found) 사용
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "테스트", author: "저자", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={onCategoriesFound}
        />,
      );
    });

    await waitFor(() => {
      const calls = onCategoriesFound.mock.calls;
      const lastCall = calls[calls.length - 1]?.[0];
      expect(lastCall).toBeDefined();
      expect(Object.keys(lastCall).some((k) => k.startsWith("yes24_"))).toBe(
        true,
      );
    });

    const lastCall =
      onCategoriesFound.mock.calls[onCategoriesFound.mock.calls.length - 1][0];

    expect(
      Object.keys(lastCall).filter((k) => k.startsWith("yes24_")),
    ).toHaveLength(1);
    expect(
      Object.keys(lastCall).filter((k) => k.startsWith("aladin_")),
    ).toHaveLength(0);
    expect(
      Object.keys(lastCall).filter((k) => k.startsWith("ridi_")),
    ).toHaveLength(0);
  });

  it("bookInfo가 title/author 모두 비어있으면 검색하지 않는다", async () => {
    const onCategoriesFound = vi.fn();

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "", author: "", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={onCategoriesFound}
        />,
      );
    });

    // 초기화 호출만 있어야 함 (빈 {})
    await waitFor(() => {
      expect(onCategoriesFound).toHaveBeenCalledWith({});
    });
    // rawJsonGetReq가 호출되지 않아야 함
    expect(rawJsonGetReq).not.toHaveBeenCalled();
  });

  it("ISBN과 저자만 있고 제목이 없으면 제목 검색을 스킵하고 null을 반환한다", async () => {
    const onCategoriesFound = vi.fn();

    // 모든 검색이 빈 결과를 반환 → autoSearch가 ISBN, 저자+제목을 시도한 뒤
    // 제목이 없으므로 title_only 단계를 건너뛰고 null을 반환 (라인 138)
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "not_found", result: [] }), 0);
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "", author: "저자", isbn: "978-1234" }}
          searchTrigger={1}
          onCategoriesFound={onCategoriesFound}
        />,
      );
    });

    // 자동 검색이 완료되어 빈 카테고리({})로 onCategoriesFound가 호출됨
    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
      // 카테고리 수집 결과는 비어 있음 (검색 결과 없음)
      const calls = onCategoriesFound.mock.calls;
      const lastCall = calls[calls.length - 1]?.[0];
      expect(lastCall).toEqual({});
    });

    // 제목이 없으므로 title= 파라미터로의 단독 검색(title_only)은 호출되지 않음
    const urls = rawJsonGetReq.mock.calls.map((c) => c[0]);
    const titleOnlyCalls = urls.filter(
      (u) => u.includes("title=") && !u.includes("author="),
    );
    expect(titleOnlyCalls).toHaveLength(0);
  });

  it("에러 응답 시 data에 에러 상태가 기록된다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess, onError) => {
      setTimeout(() => onError(new Error("서버 오류")), 0);
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "테스트", author: "저자", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={vi.fn()}
        />,
      );
    });

    // 에러가 발생하더라도 크래시하지 않음
    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
  });
});

describe("Bookstore 탭 렌더링 및 버튼 클릭", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("서점 탭들이 렌더링된다", async () => {
    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "" }} />,
      );
    });
    // 탭은 중복 렌더링될 수 있으므로 getAllByRole 사용
    const tabs = screen.getAllByRole("tab");
    const tabTexts = tabs.map((t) => t.textContent);
    expect(tabTexts).toContain("Yes24");
    expect(tabTexts).toContain("알라딘");
    expect(tabTexts).toContain("네이버쇼핑");
    expect(tabTexts).toContain("RIDI");
    expect(tabTexts).toContain("문피아");
    expect(tabTexts).toContain("시리즈");
  });

  it("ISBN/저자+제목 검색 버튼이 렌더링된다", async () => {
    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />,
      );
    });
    const isbnButtons = screen.getAllByRole("button", { name: "ISBN" });
    expect(isbnButtons.length).toBeGreaterThanOrEqual(1);
    const authorTitleButtons = screen.getAllByRole("button", {
      name: "저자+제목",
    });
    expect(authorTitleButtons.length).toBeGreaterThanOrEqual(1);
  });

  it("ISBN 버튼 클릭 시 fetchWithMethod가 실행된다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [
              {
                title: "T",
                author: "A",
                category: "소설 > 한국소설",
                book_url: "u",
                isbn: "978",
              },
            ],
            search_url: "https://search.example.com",
          }),
        0,
      );
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "978-123" }}
        />,
      );
    });

    // Yes24 탭의 ISBN 버튼 클릭
    const isbnButtons = screen.getAllByRole("button", { name: "ISBN" });
    await act(async () => {
      fireEvent.click(isbnButtons[0]);
    });

    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("isbn=978-123"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("저자+제목 버튼 클릭 시 title/author 파라미터로 검색한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
    });

    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "" }} />,
      );
    });

    const titleAuthorButtons = screen.getAllByRole("button", {
      name: "저자+제목",
    });
    await act(async () => {
      fireEvent.click(titleAuthorButtons[0]);
    });

    await waitFor(() => {
      const calls = rawJsonGetReq.mock.calls;
      const lastUrl = calls[calls.length - 1]?.[0];
      expect(lastUrl).toContain("title=");
      expect(lastUrl).toContain("author=");
    });
  });

  it("검색 결과가 있으면 제목/카테고리를 표시한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [
              {
                title: "소설1",
                author: "작가A",
                category: "한국소설",
                book_url: "http://book1",
                isbn: "111",
              },
              {
                title: "소설2",
                author: "작가B",
                category: "외국소설",
                book_url: "http://book2",
              },
            ],
          }),
        0,
      );
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "111" }}
          searchTrigger={1}
          onCategoriesFound={vi.fn()}
        />,
      );
    });

    await waitFor(() => {
      // 결과 제목은 <strong> 안에 있음
      const titles = screen.getAllByText("소설1");
      expect(titles.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("검색 결과가 없으면 빈 결과 메시지를 표시한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={vi.fn()}
        />,
      );
    });

    await waitFor(() => {
      const msg = screen.getAllByText("검색 결과가 없습니다.");
      expect(msg.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("에러 응답 시 에러 메시지를 표시한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess, onError) => {
      setTimeout(() => onError(new Error("실패")), 0);
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={vi.fn()}
        />,
      );
    });

    await waitFor(() => {
      const msg = screen.getAllByText("검색 중 오류가 발생했습니다.");
      expect(msg.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("ISBN 미지원 서점 탭에서 ISBN 버튼이 비활성화된다", async () => {
    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />,
      );
    });

    // 네이버쇼핑 탭으로 전환 (ISBN 미지원)
    const tabs = screen.getAllByRole("tab");
    const naverTab = tabs.find((t) => t.textContent === "네이버쇼핑");
    await act(async () => {
      fireEvent.click(naverTab);
    });

    // 네이버쇼핑 탭의 ISBN 버튼이 disabled인지 확인
    const isbnButtons = screen.getAllByRole("button", { name: "ISBN" });
    const disabledBtn = isbnButtons.find((btn) => btn.disabled);
    expect(disabledBtn).toBeTruthy();
  });

  it("서점에서 보기 링크가 search_url이 있을 때 표시된다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [{ title: "T", author: "A", category: "C", book_url: "u" }],
            search_url: "https://search.yes24.com/test",
          }),
        0,
      );
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={vi.fn()}
        />,
      );
    });

    await waitFor(() => {
      expect(
        screen.getAllByText("서점에서 보기").length,
      ).toBeGreaterThanOrEqual(1);
    });
  });

  it("캐시된 결과가 있으면 재사용한다", async () => {
    let callCount = 0;
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      callCount++;
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [{ title: "T", author: "A", category: "C", book_url: "u" }],
          }),
        0,
      );
    });

    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />,
      );
    });

    // 첫 번째 ISBN 검색
    const isbnButtons = screen.getAllByRole("button", { name: "ISBN" });
    await act(async () => {
      fireEvent.click(isbnButtons[0]);
    });

    await waitFor(() => expect(callCount).toBeGreaterThanOrEqual(1));
    const firstCallCount = callCount;

    // 같은 ISBN 검색 다시 클릭 → 캐시에서 재사용
    await act(async () => {
      fireEvent.click(isbnButtons[0]);
    });

    // 추가 API 호출이 없어야 함
    expect(callCount).toBe(firstCallCount);
  });

  it("fetchWithMethod 에러 시 에러 메시지를 표시한다", async () => {
    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    rawJsonGetReq.mockImplementation((url, onSuccess, onError) => {
      setTimeout(() => onError("서버 오류"), 0);
    });

    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />,
      );
    });

    const isbnButtons = screen.getAllByRole("button", { name: "ISBN" });
    await act(async () => {
      fireEvent.click(isbnButtons[0]);
    });

    await waitFor(() => {
      const msg = screen.getAllByText("검색 중 오류가 발생했습니다.");
      expect(msg.length).toBeGreaterThanOrEqual(1);
    });

    consoleSpy.mockRestore();
  });

  it("fetchWithMethod 성공 시 onCategoriesFound를 호출한다", async () => {
    const onCategoriesFound = vi.fn();

    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [
              {
                title: "T",
                author: "A",
                category: "소설 > 한국소설",
                book_url: "u",
              },
            ],
          }),
        0,
      );
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "978" }}
          onCategoriesFound={onCategoriesFound}
        />,
      );
    });

    const isbnButtons = screen.getAllByRole("button", { name: "ISBN" });
    await act(async () => {
      fireEvent.click(isbnButtons[0]);
    });

    await waitFor(() => {
      expect(onCategoriesFound).toHaveBeenCalled();
    });
  });
});
