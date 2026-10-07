// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useState } from "react";
import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
  cleanup,
  within,
} from "@testing-library/react";

afterEach(cleanup);

// 서점 한 줄의 서점명 요소: 바로 뒤에 버튼 묶음(d-flex)이 오는 <strong>
const storeLabelElements = () =>
  [...document.querySelectorAll("strong")].filter((el) =>
    el.nextElementSibling?.classList.contains("d-flex"),
  );
const storeLabels = () => storeLabelElements().map((el) => el.textContent);
const storeRow = (label) =>
  storeLabelElements().find((el) => el.textContent === label).parentElement;

vi.mock("../src/Common", () => ({
  rawJsonGetReq: vi.fn(),
  getApiUrlPrefix: () => "http://localhost:8000",
  handleFetchErrors: (r) => r,
  getRandomLightColor: () => "#ccc",
  ROOT_DIRECTORY: "$$rootdir$$",
}));

import { rawJsonGetReq } from "../src/Common";
import Bookstore, {
  stripComicDirectoryMeta,
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

  it("자동 검색 시 yes24, aladin, kyobo, naver 네 서점의 카테고리를 수집한다", async () => {
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
      "/search/bookstore/kyobo": {
        status: "success",
        result: [
          {
            title: "T",
            author: "A",
            category: "소설 > 한국소설 > 한국소설일반",
            book_url: "u3",
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
            book_url: "u4",
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
    // kyobo 카테고리
    expect(Object.values(lastCall)).toContain("한국소설 한국소설일반");
    // naver 카테고리
    expect(Object.values(lastCall)).toContain("소설 추리/미스터리");
    // 교보 결과가 자기 키로 실린다. 자동 검색 목록에서 빠지면 조용히 사라진다.
    expect(Object.keys(lastCall).some((k) => k.startsWith("kyobo_"))).toBe(true);
  });

  it.each(["joara", "naverwebtoon", "kakaowebtoon"])("%s는 자동 검색과 카테고리 판정에 참여하지 않는다", async (store) => {
    const onCategoriesFound = vi.fn();

    mockSearchResponses({
      [`/search/bookstore/${store}`]: {
        status: "success",
        result: [
          { title: "T", author: "A", category: "판타지", book_url: "u" },
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
      expect(onCategoriesFound).toHaveBeenCalled();
    });

    // 조아라·웹툰 장르("판타지", "학원/판타지")는 서점 분류 체계와 어휘가 달라 유사도 판정을 흐린다.
    const storeCalls = rawJsonGetReq.mock.calls.filter(([url]) =>
      url.includes(`/search/bookstore/${store}`),
    );
    expect(storeCalls).toHaveLength(0);
    const lastCall =
      onCategoriesFound.mock.calls[onCategoriesFound.mock.calls.length - 1][0];
    expect(Object.keys(lastCall).some((k) => k.startsWith(`${store}_`))).toBe(
      false,
    );
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

  it("서점별 한 줄이 렌더링된다", async () => {
    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "" }} />,
      );
    });
    expect(storeLabels()).toEqual([
      "Yes24",
      "알라딘",
      "교보문고",
      "네이버쇼핑",
      "RIDI",
      "문피아",
      "시리즈",
      "조아라",
      "네이버웹툰",
      "카카오웹툰",
    ]);
  });

  it("만화 모드는 네이버쇼핑·문피아·시리즈·조아라를 보여주지 않는다", async () => {
    await act(async () => {
      render(
        <Bookstore
          comic
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
        />,
      );
    });
    expect(storeLabels()).toEqual([
      "Yes24",
      "알라딘",
      "교보문고",
      "RIDI",
      "네이버웹툰",
      "카카오웹툰",
    ]);
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
      name: "도서명",
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
      name: "도서명",
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

  it("ISBN 미지원 서점 줄에서 ISBN 버튼이 비활성화된다", async () => {
    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />,
      );
    });

    // 네이버쇼핑은 ISBN 미지원이라 ISBN 버튼이 disabled여야 한다
    expect(
      within(storeRow("네이버쇼핑")).getByRole("button", { name: "ISBN" })
        .disabled,
    ).toBe(true);
  });

  it("서점 링크가 search_url이 있을 때 표시된다", async () => {
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
        screen.getAllByText("서점").length,
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

  it.each([
    ["문피아", "munpia", "https://novel.munpia.com/page/hd.platinum/view/search/keyword/%EC%A0%80%EC%9E%90%20%EC%A0%9C%EB%AA%A9/order/search_result"],
    ["시리즈", "naverseries", "https://series.naver.com/search/search.series?t=all&q=%EC%A0%9C%EB%AA%A9%20%EC%A0%80%EC%9E%90"],
    ["조아라", "joara", "https://www.joara.com/search?target=subject&word=%EC%A0%9C%EB%AA%A9&search="],
    ["네이버웹툰", "naverwebtoon", "https://comic.naver.com/search?keyword=%EC%A0%9C%EB%AA%A9"],
    ["카카오웹툰", "kakaowebtoon", "https://webtoon.kakao.com/search?keyword=%EC%A0%9C%EB%AA%A9"],
  ])("%s는 응답 URL이 없어도 서점 검색 링크를 표시한다", async (label, store, expectedUrl) => {
    rawJsonGetReq.mockImplementation((_url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
    });

    await act(async () => {
      render(<Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "" }} />);
    });

    await act(async () => {
      fireEvent.click(
        within(storeRow(label)).getByRole("button", { name: "도서명" }),
      );
    });

    await waitFor(() => {
      const links = screen.getAllByText("서점");
      expect(
        links.some((link) => link.closest("a").getAttribute("href") === expectedUrl),
      ).toBe(true);
    });
  });
});

describe("Bookstore 부분 검색어 / 폴백 렌더링", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("구분자만 있는 카테고리는 빈 문자열을 반환한다", () => {
    // ">" → split → ["", ""] → filter(Boolean) → [] → parts[0]가 undefined
    expect(getTwoLevelCategory(">")).toBe("");
    expect(getTwoLevelCategory(" > > ")).toBe("");
  });

  it("onCategoriesFound 없이도 자동 검색이 동작한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [{ title: "T", category: "소설 > 한국소설", book_url: "u" }],
          }),
        0,
      );
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });

    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
  });

  it("자동 검색은 ISBN과 저자+제목 결과가 없으면 제목 검색으로 폴백한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      const result = url.includes("title=") && !url.includes("author=")
        ? [{ title: "제목 결과", category: "소설 > SF", book_url: "title-only" }]
        : [];
      setTimeout(() => onSuccess({ status: "success", result }), 0);
    });

    render(
      <Bookstore
        bookInfo={{ title: "제목", author: "저자", isbn: "978" }}
        searchTrigger={1}
      />,
    );

    expect(await screen.findAllByText("제목 결과")).toHaveLength(4);
    const urls = rawJsonGetReq.mock.calls.map(([url]) => url);
    expect(urls.some((url) => url.includes("isbn=978"))).toBe(true);
    expect(urls.some((url) => url.includes("author=%EC%A0%80%EC%9E%90"))).toBe(true);
    expect(urls.some((url) => url.includes("title=%EC%A0%9C%EB%AA%A9") && !url.includes("author="))).toBe(true);
  });

  it("자동 검색에서 저자+제목 결과를 반환하고 카테고리를 전달한다", async () => {
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      const result = url.includes("author=")
        ? [{
            title: "검색 책",
            author: "저자",
            category: "소설 > 한국소설",
            book_url: url,
          }]
        : [];
      setTimeout(() => onSuccess({ status: "success", result }), 0);
    });

    render(
      <Bookstore
        bookInfo={{ title: "제목", author: "저자", isbn: "978" }}
        searchTrigger={1}
        onCategoriesFound={onCategoriesFound}
      />,
    );

    await waitFor(() => {
      expect(onCategoriesFound.mock.calls.at(-1)[0]).toEqual(
        expect.objectContaining({ yes24_0_0: "소설 한국소설" }),
      );
    });
  });

  it("저자+제목 버튼의 병렬 검색 결과로 카테고리를 전달한다", async () => {
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((_url, onSuccess) => {
      setTimeout(() => onSuccess({
        status: "success",
        result: [{
          title: "책",
          category: "소설 > 한국소설",
          book_url: "book",
        }],
      }), 0);
    });

    render(
      <Bookstore
        bookInfo={{ title: "제목", author: "저자", isbn: "" }}
        onCategoriesFound={onCategoriesFound}
      />,
    );
    fireEvent.click(screen.getAllByRole("button", { name: "도서명" })[0]);

    await waitFor(() => {
      expect(onCategoriesFound.mock.calls.at(-1)[0]).toEqual(
        expect.objectContaining({ yes24_0_0: "소설 한국소설" }),
      );
    });
  });

  it("자동 검색 요청 오류는 로그를 남기고 빈 결과로 폴백한다", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    rawJsonGetReq.mockImplementation((_url, _onSuccess, onError) => {
      setTimeout(() => onError("network error"), 0);
    });

    render(
      <Bookstore
        bookInfo={{ title: "제목", author: "저자", isbn: "" }}
        searchTrigger={1}
      />,
    );
    await waitFor(() => expect(rawJsonGetReq).toHaveBeenCalled());
    await act(async () => {
      for (const [, , onError] of rawJsonGetReq.mock.calls) {
        onError("network error");
      }
    });
    await waitFor(() => expect(errorSpy).toHaveBeenCalledWith("network error"));
    errorSpy.mockRestore();
  });

  it("ISBN 기반 자동 검색 요청 실패는 제목 검색으로 계속 진행한다", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    rawJsonGetReq.mockImplementation((url, onSuccess, onError) => {
      if (url.includes("isbn=")) {
        setTimeout(() => onError("ISBN network error"), 0);
      } else {
        setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
      }
    });

    render(
      <Bookstore
        bookInfo={{ title: "제목", author: "저자", isbn: "978" }}
        searchTrigger={1}
      />,
    );

    await waitFor(() => expect(errorSpy).toHaveBeenCalledWith("ISBN network error"));
    expect(rawJsonGetReq.mock.calls.some(([url]) => url.includes("title="))).toBe(true);
    errorSpy.mockRestore();
  });

  it("ISBN이 있어도 저자+제목 버튼은 제목과 저자만 요청한다", async () => {
    rawJsonGetReq.mockImplementation((_url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
    });
    render(
      <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />,
    );
    fireEvent.click(screen.getAllByRole("button", { name: "도서명" })[0]);

    await waitFor(() => expect(rawJsonGetReq).toHaveBeenCalled());
    expect(rawJsonGetReq.mock.calls[0][0]).toContain("title=");
    expect(rawJsonGetReq.mock.calls[0][0]).toContain("author=");
    expect(rawJsonGetReq.mock.calls[0][0]).not.toContain("isbn=");
  });

  it("ISBN이 없으면 제목과 저자+제목 검색에서 각각 2건씩 합쳐 최대 4건을 표시한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      const titleOnly = !url.includes("author=");
      const prefix = titleOnly ? "제목" : "저자제목";
      setTimeout(() => onSuccess({
        status: "success",
        result: Array.from({ length: 5 }, (_, i) => ({
          title: `${prefix}${i + 1}`,
          author: "저자",
          category: "소설 > 한국소설",
          book_url: `${prefix}${i + 1}`,
        })),
      }), 0);
    });

    await act(async () => {
      render(<Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "" }} />);
    });

    await act(async () => {
      fireEvent.click(screen.getAllByRole("button", { name: "도서명" })[0]);
    });

    await waitFor(() => {
      expect(screen.getByText("제목1")).toBeTruthy();
      expect(screen.getByText("제목2")).toBeTruthy();
      expect(screen.getByText("저자제목1")).toBeTruthy();
      expect(screen.getByText("저자제목2")).toBeTruthy();
      expect(screen.queryByText("제목3")).toBeNull();
      expect(screen.queryByText("저자제목3")).toBeNull();
    });
    expect(rawJsonGetReq).toHaveBeenCalledTimes(2);
    const urls = rawJsonGetReq.mock.calls.map(([url]) => url);
    expect(urls.some((url) => url.includes("title=%EC%A0%9C%EB%AA%A9") && !url.includes("author="))).toBe(true);
    expect(urls.some((url) => url.includes("title=%EC%A0%9C%EB%AA%A9") && url.includes("author="))).toBe(true);
  });

  it("저자 없이 제목만 있으면 author 파라미터 없이 자동 검색한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "not_found", result: [] }), 0);
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목만", author: "", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });

    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    const urls = rawJsonGetReq.mock.calls.map((c) => c[0]);
    expect(urls.some((u) => u.includes("title="))).toBe(true);
    expect(urls.every((u) => !u.includes("author="))).toBe(true);
  });

  it("제목 없이 저자만 있으면 author 파라미터만으로 버튼 검색한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
    });

    await act(async () => {
      render(<Bookstore bookInfo={{ title: "", author: "저자만", isbn: "" }} />);
    });

    const buttons = screen.getAllByRole("button", { name: "도서명" });
    await act(async () => {
      fireEvent.click(buttons[0]);
    });

    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    const lastUrl = rawJsonGetReq.mock.calls.at(-1)[0];
    expect(lastUrl).toContain("author=");
    expect(lastUrl).not.toContain("title=");
  });

  it("저자 없이 제목만 있으면 title 파라미터만으로 버튼 검색한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
    });

    await act(async () => {
      render(<Bookstore bookInfo={{ title: "제목만", author: "", isbn: "" }} />);
    });

    const buttons = screen.getAllByRole("button", { name: "도서명" });
    await act(async () => {
      fireEvent.click(buttons[0]);
    });

    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    const lastUrl = rawJsonGetReq.mock.calls.at(-1)[0];
    expect(lastUrl).toContain("title=");
    expect(lastUrl).not.toContain("author=");
  });

  it("message 없는 에러 응답에는 기본 에러 문구를 표시한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(() => onSuccess({ error: true }), 0);
    });

    await act(async () => {
      render(<Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "" }} />);
    });

    const buttons = screen.getAllByRole("button", { name: "도서명" });
    await act(async () => {
      fireEvent.click(buttons[0]);
    });

    await waitFor(() => {
      expect(
        screen.getAllByText("검색 중 오류가 발생했습니다.").length,
      ).toBeGreaterThan(0);
    });
  });

  it("book_url 이 없는 결과 항목도 인덱스를 key 로 렌더링한다", async () => {
    rawJsonGetReq.mockImplementation((url, onSuccess) => {
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [
              { title: "URL없는책1", author: "A" },
              { title: "URL없는책2", author: "B" },
            ],
          }),
        0,
      );
    });

    await act(async () => {
      render(<Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "" }} />);
    });

    const buttons = screen.getAllByRole("button", { name: "도서명" });
    await act(async () => {
      fireEvent.click(buttons[0]);
    });

    await waitFor(() => {
      expect(screen.getAllByText("URL없는책1").length).toBeGreaterThan(0);
      expect(screen.getAllByText("URL없는책2").length).toBeGreaterThan(0);
    });
  });
});

describe("Bookstore 만화 모드", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    rawJsonGetReq.mockImplementation((_url, onSuccess) => {
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [
              { title: "T", author: "A", category: "드라마", book_url: "u" },
            ],
          }),
        0,
      );
    });
  });

  it("ISBN 버튼은 ISBN이 없어 비활성이고 웹툰 줄에는 아예 없다", async () => {
    await act(async () => {
      render(
        <Bookstore
          comic
          bookInfo={{ title: "제목", author: "저자", isbn: "978" }}
        />,
      );
    });
    for (const label of ["Yes24", "알라딘", "교보문고", "RIDI"]) {
      expect(
        within(storeRow(label)).getByRole("button", { name: "ISBN" }).disabled,
      ).toBe(true);
    }
    for (const label of ["네이버웹툰", "카카오웹툰"]) {
      expect(
        within(storeRow(label)).queryByRole("button", { name: "ISBN" }),
      ).toBeNull();
    }
    expect(
      screen.getAllByRole("button", { name: "도서명" }).length,
    ).toBeGreaterThan(0);
  });

  it("bookInfo 에 ISBN 이 있어도 isbn 파라미터를 보내지 않는다", async () => {
    await act(async () => {
      render(
        <Bookstore
          comic
          bookInfo={{ title: "제목", author: "저자", isbn: "9781234567890" }}
          searchTrigger={1}
        />,
      );
    });
    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    for (const [url] of rawJsonGetReq.mock.calls) {
      expect(url).not.toContain("isbn=");
    }
  });

  it("만화 모드는 bookInfo에 저자가 있어도 author 파라미터 없이 제목만 검색한다", async () => {
    await act(async () => {
      render(
        <Bookstore
          comic
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });
    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    const urls = rawJsonGetReq.mock.calls.map(([url]) => url);
    expect(urls.some((url) => url.includes("title=%EC%A0%9C%EB%AA%A9"))).toBe(true);
    expect(urls.some((url) => url.includes("author="))).toBe(false);
  });

  it.each([
    ["[저자] 작품명 (1-34화)", "작품명"],
    ["(저자) 작품명 1~47화[완결]", "작품명"],
    ["작품명 [저자]", "작품명"],
    ["작품명 @ 저자", "작품명"],
    ["작품명 1-34화", "작품명"],
    ["1_장르", "1_장르"],
    ["[저자]", "[저자]"],
  ])("만화 디렉토리 이름 %s 은 %s 로 검색한다", async (name, expected) => {
    await act(async () => {
      render(
        <Bookstore
          comic
          bookInfo={{ title: name, author: "", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });
    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    const expectedParam = `title=${encodeURIComponent(expected)}`;
    const urls = rawJsonGetReq.mock.calls.map(([url]) => url);
    expect(urls.every((url) => url.includes(expectedParam))).toBe(true);
  });

  it("일반 서점 분류나 제목이 다른 웹툰으로 카테고리를 결정하지 않는다", async () => {
    const onCategoriesFound = vi.fn();
    await act(async () => {
      render(
        <Bookstore
          comic
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
          onCategoriesFound={onCategoriesFound}
        />,
      );
    });
    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    await act(async () => {
      fireEvent.click(screen.getAllByRole("button", { name: "도서명" })[0]);
    });
    expect(onCategoriesFound).toHaveBeenCalled();
    expect(onCategoriesFound.mock.calls.every(([categories]) => Object.keys(categories).length === 0)).toBe(true);
  });

  it("만화 모드가 아니면 ISBN 버튼이 그대로 있다", async () => {
    await act(async () => {
      render(
        <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />,
      );
    });
    expect(
      screen.getAllByRole("button", { name: "ISBN" }).length,
    ).toBeGreaterThan(0);
  });
});

describe("Bookstore 웹툰 저자 표시", () => {
  beforeEach(() => vi.clearAllMocks());

  it.each([
    [true, "naverwebtoon", "네이버웹툰"],
    [true, "kakaowebtoon", "카카오웹툰"],
    [false, "naverwebtoon", "네이버웹툰"],
    [false, "kakaowebtoon", "카카오웹툰"],
  ])("만화 모드 %s에서 %s의 저자 구분자만 쉼표로 표시한다", async (comic, store, label) => {
    const item = Object.freeze({ title: "작품 / 부제", author: "봉이 / 갈피 / 오윤", category: "로맨스 / 판타지" });
    const bookInfo = Object.freeze({ title: "작품 / 부제", author: "검색 저자 / 다른 저자" });
    rawJsonGetReq.mockImplementation((_url, success) => success({ status: "success", result: [item] }));
    render(<Bookstore comic={comic} bookInfo={bookInfo} />);
    const row = within(storeRow(label));
    const search = () => fireEvent.click(row.getByRole("button", { name: "도서명" }));
    search();

    await waitFor(() => expect(row.getByText("봉이, 갈피, 오윤")).toBeTruthy());
    expect(row.getByText(item.title)).toBeTruthy();
    expect(row.getByText(`| ${item.category}`)).toBeTruthy();
    expect(item.author).toBe("봉이 / 갈피 / 오윤");
    const requests = rawJsonGetReq.mock.calls.length;
    for (const [url] of rawJsonGetReq.mock.calls) {
      const parsed = new URL(url, "http://localhost");
      expect(parsed.pathname).toBe(`/search/bookstore/${store}`);
      expect(parsed.searchParams.get("title")).toBe(bookInfo.title);
      if (parsed.searchParams.has("author")) expect(parsed.searchParams.get("author")).toBe(bookInfo.author);
    }
    search();
    expect(rawJsonGetReq).toHaveBeenCalledTimes(requests);
    expect(row.getByText("봉이, 갈피, 오윤")).toBeTruthy();
  });

  it.each(["Yes24", "알라딘"])("%s 저자명의 슬래시는 변경하지 않는다", async (label) => {
    rawJsonGetReq.mockImplementation((_url, success) => success({ status: "success", result: [{ author: "봉이 / 갈피 / 오윤" }] }));
    render(<Bookstore comic bookInfo={{ title: "작품명" }} />);
    const row = within(storeRow(label));
    fireEvent.click(row.getByRole("button", { name: "도서명" }));
    await waitFor(() => expect(row.getByText("봉이 / 갈피 / 오윤")).toBeTruthy());
  });

  it.each(["봉이/갈피", "봉이, 갈피", "봉이", "", null, undefined])("웹툰 저자 %s는 지정된 구분자가 없으면 유지한다", async (author) => {
    rawJsonGetReq.mockImplementation((_url, success) => success({ status: "success", result: [{ title: "작품명", author }] }));
    render(<Bookstore comic bookInfo={{ title: "작품명" }} searchTrigger={1} />);
    for (const label of ["네이버웹툰", "카카오웹툰"]) {
      const row = within(storeRow(label));
      await waitFor(() => expect(row.getByText("작품명")).toBeTruthy());
      if (author) expect(row.getByText(author)).toBeTruthy();
      else expect(storeRow(label).querySelector("small").textContent).toBe("");
    }
  });
});

describe("Bookstore 웹툰 자동 검색", () => {
  const WEBTOON_STORES = ["naverwebtoon", "kakaowebtoon"];

  beforeEach(() => {
    vi.clearAllMocks();
    rawJsonGetReq.mockImplementation((_url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
    });
  });

  const calledStores = () =>
    new Set(
      rawJsonGetReq.mock.calls.map(([url]) => url.split("?")[0].split("/").pop()),
    );

  it("편집 컴포넌트가 분할한 제목은 다시 자르지 않고 검색한다", async () => {
    render(<Bookstore comic titleParsed bookInfo={{ title: "작품명 (특별한 부제)", author: "저자" }} searchTrigger={1} />);

    await waitFor(() => expect(rawJsonGetReq).toHaveBeenCalledTimes(4));
    for (const [url] of rawJsonGetReq.mock.calls) {
      expect(new URL(url, "http://localhost").searchParams.get("title")).toBe("작품명 (특별한 부제)");
    }
  });

  it.each([
    ["naverwebtoon", "작품명", "작품명"],
    ["kakaowebtoon", "작품명", "작품명 시즌 2"],
    ["naverwebtoon", "[저자] 작품명 (1~10화 완)", "작품명 외전"],
  ])("%s 작품명에 제목 %s이 포함되면 웹툰을 추천한다", async (store, title, resultTitle) => {
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((url, success) => success({
      status: "success",
      result: url.includes(`/${store}?`) ? [{ title: resultTitle, category: "드라마" }] : [],
    }));
    render(<Bookstore comic bookInfo={{ title }} searchTrigger={1} onCategoriesFound={onCategoriesFound} />);

    await waitFor(() => expect(onCategoriesFound).toHaveBeenLastCalledWith({ [`${store}_0_0`]: "웹툰" }));
  });

  it.each([
    ["작품명", "다른 작품", "success"],
    ["작품명 시즌 2", "작품명", "success"],
    ["", "작품명", "success"],
    ["작품명", undefined, "success"],
    ["작품명", "작품명", "failure"],
  ])("제목 %s과 작품명 %s이 조건에 맞지 않으면 웹툰을 추천하지 않는다", async (title, resultTitle, status) => {
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((_url, success) => success({
      status, result: status === "success" ? [{ title: resultTitle, category: "웹툰" }] : [],
    }));
    render(<Bookstore comic bookInfo={{ title }} searchTrigger={1} onCategoriesFound={onCategoriesFound} />);

    await waitFor(() => expect(onCategoriesFound).toHaveBeenLastCalledWith({}));
  });

  it.each([
    ["naverwebtoon", "로맨스"],
    ["kakaowebtoon", "로맨스"],
    ["naverwebtoon", "로맨스/드라마"],
    ["kakaowebtoon", "로맨스판타지"],
  ])("%s의 일치 작품 장르가 %s이면 웹툰 대신 여성향을 추천한다", async (store, category) => {
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((url, success) => success({
      status: "success", result: url.includes(`/${store}?`) ? [{ title: "작품명 외전", category }] : [],
    }));
    render(<Bookstore comic bookInfo={{ title: "작품명" }} searchTrigger={1} onCategoriesFound={onCategoriesFound} />);

    await waitFor(() => expect(onCategoriesFound).toHaveBeenLastCalledWith({ [`${store}_0_0`]: "여성향" }));
  });

  it("로맨스 장르여도 작품명이 불일치하면 여성향을 추천하지 않는다", async () => {
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((_url, success) => success({ status: "success", result: [{ title: "다른 작품", category: "로맨스" }] }));
    render(<Bookstore comic bookInfo={{ title: "작품명" }} searchTrigger={1} onCategoriesFound={onCategoriesFound} />);

    await waitFor(() => expect(onCategoriesFound).toHaveBeenLastCalledWith({}));
  });

  it("일치 작품에 로맨스가 있으면 일반 웹툰 추천보다 여성향을 우선한다", async () => {
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((url, success) => success({
      status: "success", result: [{ title: "작품명", category: url.includes("/naverwebtoon?") ? "로맨스" : "드라마" }],
    }));
    render(<Bookstore comic bookInfo={{ title: "작품명" }} searchTrigger={1} onCategoriesFound={onCategoriesFound} />);

    await waitFor(() => expect(onCategoriesFound).toHaveBeenLastCalledWith({ naverwebtoon_0_0: "여성향" }));
  });

  it.each([[true, "네이버웹툰", "naverwebtoon"], [false, "카카오웹툰", "kakaowebtoon"]])(
    "만화 모드 %s에서 %s 수동 검색도 웹툰을 추천한다",
    async (comic, label, store) => {
      const onCategoriesFound = vi.fn();
      rawJsonGetReq.mockImplementation((_url, success) => success({
        status: "success", result: [{ title: "작품명 외전", category: "드라마" }],
      }));
      render(<Bookstore comic={comic} bookInfo={{ title: "작품명" }} onCategoriesFound={onCategoriesFound} />);
      fireEvent.click(within(storeRow(label)).getByRole("button", { name: "도서명" }));

      await waitFor(() => expect(onCategoriesFound).toHaveBeenLastCalledWith({ [`${store}_0_0`]: "웹툰" }));
    },
  );

  it("만화 모드는 네이버웹툰·카카오웹툰도 자동 검색한다", async () => {
    await act(async () => {
      render(
        <Bookstore
          comic
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });
    await waitFor(() => {
      for (const store of WEBTOON_STORES) {
        expect(calledStores().has(store)).toBe(true);
      }
    });
    expect(calledStores().has("yes24")).toBe(true);
    expect(calledStores().has("aladin")).toBe(true);
    // 그 밖의 서점은 버튼을 눌러야 검색한다.
    for (const store of ["kyobo", "naver", "ridi"]) {
      expect(calledStores().has(store)).toBe(false);
    }
  });

  it("다음 검색을 시작하면 이전 검색의 웹툰 추천을 무시한다", async () => {
    const pending = [];
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((url, success) => pending.push({ url, success }));
    const { rerender } = render(<Bookstore comic bookInfo={{ title: "이전 작품" }} searchTrigger={1} onCategoriesFound={onCategoriesFound} />);
    rerender(<Bookstore comic bookInfo={{ title: "현재 작품" }} searchTrigger={2} onCategoriesFound={onCategoriesFound} />);
    await act(async () => {
      pending.filter(({ url }) => url.includes(encodeURIComponent("현재 작품"))).forEach(({ success }) => success({ status: "success", result: [] }));
    });
    await act(async () => {
      pending.filter(({ url }) => url.includes(encodeURIComponent("이전 작품"))).forEach(({ success }) => success({ status: "success", result: [{ title: "이전 작품" }] }));
    });
    expect(onCategoriesFound).toHaveBeenLastCalledWith({});
  });

  it("캐시를 재사용해도 웹툰을 추천하고 불일치하는 새 검색은 추천을 지운다", async () => {
    const onCategoriesFound = vi.fn();
    rawJsonGetReq.mockImplementation((_url, success) => success({ status: "success", result: [{ title: "작품명" }] }));
    const { rerender } = render(<Bookstore comic bookInfo={{ title: "작품명" }} onCategoriesFound={onCategoriesFound} />);
    const search = () => fireEvent.click(within(storeRow("네이버웹툰")).getByRole("button", { name: "도서명" }));
    search();
    await waitFor(() => expect(onCategoriesFound).toHaveBeenLastCalledWith({ naverwebtoon_0_0: "웹툰" }));
    const requests = rawJsonGetReq.mock.calls.length;
    onCategoriesFound.mockClear();
    search();
    expect(onCategoriesFound).toHaveBeenLastCalledWith({ naverwebtoon_0_0: "웹툰" });
    expect(rawJsonGetReq).toHaveBeenCalledTimes(requests);
    rerender(<Bookstore comic bookInfo={{ title: "다른 작품" }} onCategoriesFound={onCategoriesFound} />);
    search();
    await waitFor(() => expect(onCategoriesFound).toHaveBeenLastCalledWith({}));
  });

  it("수동 검색 추천으로 부모 상태를 갱신해도 렌더링 오류가 없다", async () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    rawJsonGetReq.mockImplementation((_url, success) => success({ status: "success", result: [{ title: "작품명" }] }));
    function Parent() {
      const [categories, setCategories] = useState({});
      return <><Bookstore comic bookInfo={{ title: "작품명" }} onCategoriesFound={setCategories} /><output data-testid="recommendations">{JSON.stringify(categories)}</output></>;
    }
    try {
      render(<Parent />);
      for (const label of ["네이버웹툰", "카카오웹툰"]) {
        fireEvent.click(within(storeRow(label)).getByRole("button", { name: "도서명" }));
        await waitFor(() => expect(screen.getByTestId("recommendations").textContent).toContain("웹툰"));
      }
      await waitFor(() => expect(JSON.parse(screen.getByTestId("recommendations").textContent)).toEqual({ naverwebtoon_0_0: "웹툰", kakaowebtoon_0_0: "웹툰" }));
      expect(errors).not.toHaveBeenCalled();
    } finally {
      errors.mockRestore();
    }
  });

  it("책 모드는 Yes24·알라딘·교보·네이버쇼핑만 자동 검색한다", async () => {
    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });
    await waitFor(() => {
      for (const store of ["yes24", "aladin", "kyobo", "naver"]) {
        expect(calledStores().has(store)).toBe(true);
      }
    });
    for (const store of ["ridi", "munpia", "naverseries", "joara"]) {
      expect(calledStores().has(store)).toBe(false);
    }
  });

  it("자동 검색한 방법이 토글 버튼의 선택 상태가 된다", async () => {
    rawJsonGetReq.mockImplementation((_url, onSuccess) => {
      setTimeout(
        () =>
          onSuccess({
            status: "success",
            result: [{ title: "T", book_url: "u" }],
          }),
        0,
      );
    });
    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "978" }}
          searchTrigger={1}
        />,
      );
    });
    const pressed = (label, name) =>
      within(storeRow(label))
        .getByRole("button", { name })
        .getAttribute("aria-pressed");
    await waitFor(() => {
      expect(pressed("Yes24", "ISBN")).toBe("true");
    });
    expect(pressed("Yes24", "도서명")).toBe("false");
    // 자동 검색하지 않는 서점은 어느 쪽도 선택되지 않는다.
    expect(pressed("RIDI", "ISBN")).toBe("false");
    expect(pressed("RIDI", "도서명")).toBe("false");

    await act(async () => {
      fireEvent.click(
        within(storeRow("RIDI")).getByRole("button", { name: "도서명" }),
      );
    });
    await waitFor(() => {
      expect(pressed("RIDI", "도서명")).toBe("true");
    });
  });

  it("책 모드는 웹툰을 자동 검색하지 않는다", async () => {
    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "저자", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });
    await waitFor(() => {
      expect(calledStores().has("yes24")).toBe(true);
    });
    for (const store of WEBTOON_STORES) {
      expect(calledStores().has(store)).toBe(false);
    }
  });
});

describe("stripComicDirectoryMeta", () => {
  it.each([
    ["개구리 인간 1~47화[완결]", "개구리 인간"],
    ["개구리 인간 1-5권", "개구리 인간"],
    ["나 혼자만 레벨업 (완결)", "나 혼자만 레벨업"],
    ["개구리 인간 (1-34화)", "개구리 인간"],
    ["개구리 인간(1-34화)", "개구리 인간"],
    ["개구리 인간 （1-34화）", "개구리 인간"],
    ["개구리 인간 (시즌2)", "개구리 인간"],
    ["개구리 인간 (Season 2) 1-3화", "개구리 인간"],
    ["개구리 인간 (장혁준) 1~47화[완결]", "개구리 인간"],
    ["나 혼자만 레벨업[완결]", "나 혼자만 레벨업"],
    ["이태원 클라쓰 연재중", "이태원 클라쓰"],
    ["유미의 세포들 12화", "유미의 세포들"],
    ["개구리 인간", "개구리 인간"],
    ["  개구리 인간  ", "개구리 인간"],
  ])("%s → %s", (input, expected) => {
    expect(stripComicDirectoryMeta(input)).toBe(expected);
  });

  it("제목 안의 숫자와 단어는 자르지 않는다", () => {
    expect(stripComicDirectoryMeta("7번 국도")).toBe("7번 국도");
    expect(stripComicDirectoryMeta("완결자")).toBe("완결자");
    expect(stripComicDirectoryMeta("연재의 기술")).toBe("연재의 기술");
  });

  it("이름이 표기로 시작하면 원본을 돌려준다", () => {
    expect(stripComicDirectoryMeta("완결 개구리 인간")).toBe("완결 개구리 인간");
    expect(stripComicDirectoryMeta("(장혁준) 개구리 인간")).toBe("(장혁준) 개구리 인간");
    expect(stripComicDirectoryMeta("")).toBe("");
    expect(stripComicDirectoryMeta(undefined)).toBe("");
  });
});

describe("Bookstore 만화 모드 검색어 정리", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    rawJsonGetReq.mockImplementation((_url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "success", result: [] }), 0);
    });
  });

  it("자동 검색이 권수·완결 표기를 뺀 제목을 보낸다", async () => {
    await act(async () => {
      render(
        <Bookstore
          comic
          bookInfo={{ title: "개구리 인간 1~47화[완결]", author: "", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });
    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    for (const [url] of rawJsonGetReq.mock.calls) {
      const title = new URLSearchParams(url.split("?")[1]).get("title");
      expect(title).toBe("개구리 인간");
    }
  });

  it("책 모드는 제목을 그대로 보낸다", async () => {
    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "개구리 인간 1~47화[완결]", author: "", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });
    await waitFor(() => {
      expect(rawJsonGetReq).toHaveBeenCalled();
    });
    for (const [url] of rawJsonGetReq.mock.calls) {
      const title = new URLSearchParams(url.split("?")[1]).get("title");
      expect(title).toBe("개구리 인간 1~47화[완결]");
    }
  });
});

describe("Bookstore 검색어 조합 경계", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    rawJsonGetReq.mockImplementation((_url, onSuccess) => {
      setTimeout(() => onSuccess({ status: "not_found", result: [] }), 0);
    });
  });

  it.each(["(1-5권) 작품", "[완결] 작품"])(
    "이름이 괄호 표기로 시작하면(%s) 원본을 돌려준다",
    (name) => {
      expect(stripComicDirectoryMeta(name)).toBe(name);
    },
  );

  it("저자 없이 제목만 검색하다 오류가 나면 오류 결과를 그대로 표시한다", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    rawJsonGetReq.mockImplementation((_url, _onSuccess, onError) => {
      setTimeout(() => onError(new Error("서버 오류")), 0);
    });

    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목만", author: "", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });

    expect((await screen.findAllByText("검색 중 오류가 발생했습니다.")).length).toBeGreaterThan(0);
    errorSpy.mockRestore();
  });

  it("제목 없이 저자만 있으면 ISBN 단계 없이 저자로 자동 검색한다", async () => {
    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "", author: "저자만", isbn: "" }}
          searchTrigger={1}
        />,
      );
    });

    await waitFor(() => expect(rawJsonGetReq).toHaveBeenCalled());
    const urls = rawJsonGetReq.mock.calls.map((c) => c[0]);
    expect(urls.every((u) => u.includes("author=") && !u.includes("title=") && !u.includes("isbn="))).toBe(true);
  });

  it("ISBN과 제목만 있으면 ISBN 실패 뒤 저자 없이 제목만으로 검색한다", async () => {
    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "제목", author: "", isbn: "978" }}
          searchTrigger={1}
        />,
      );
    });

    await waitFor(() =>
      expect(rawJsonGetReq.mock.calls.some((c) => c[0].includes("title=") && !c[0].includes("isbn="))).toBe(true),
    );
    const titleCalls = rawJsonGetReq.mock.calls.filter((c) => c[0].includes("title="));
    expect(titleCalls.every((c) => !c[0].includes("author="))).toBe(true);
  });

  it("ISBN만 있으면 ISBN 검색이 실패해도 제목·저자 검색은 하지 않는다", async () => {
    await act(async () => {
      render(
        <Bookstore
          bookInfo={{ title: "", author: "", isbn: "978" }}
          searchTrigger={1}
        />,
      );
    });

    await waitFor(() => expect(rawJsonGetReq).toHaveBeenCalled());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(rawJsonGetReq.mock.calls.every((c) => c[0].includes("isbn=978"))).toBe(true);
  });

  it("ISBN이 있어도 저자가 비어 있으면 저자+제목 버튼은 제목만 보낸다", async () => {
    render(
      <Bookstore bookInfo={{ title: "제목", author: "", isbn: "978" }} />,
    );
    fireEvent.click(screen.getAllByRole("button", { name: "도서명" })[0]);

    await waitFor(() => expect(rawJsonGetReq).toHaveBeenCalled());
    expect(rawJsonGetReq.mock.calls[0][0]).toContain("title=");
    expect(rawJsonGetReq.mock.calls[0][0]).not.toContain("author=");
  });
});

