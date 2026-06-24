// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";

afterEach(cleanup);

const { mockIsCacheInit, mockFetchMappings } = vi.hoisted(() => ({
  mockIsCacheInit: vi.fn(() => true),
  mockFetchMappings: vi.fn(() => Promise.resolve()),
}));
vi.mock("../src/categoryMappingCache", () => ({
  loadCategoryMappings: () => ({}),
  fetchCategoryMappings: (...a) => mockFetchMappings(...a),
  isCacheInitialized: (...a) => mockIsCacheInit(...a),
}));

// getSimilarityDebugInfo를 제어 가능하게 mock
const mockGetSimilarityDebugInfo = vi.fn();
vi.mock("../src/Actions", () => ({
  getSimilarityDebugInfo: (...args) => mockGetSimilarityDebugInfo(...args),
}));

import SimilarityDebug from "../src/SimilarityDebug";

// ── 헬퍼: 디버그 데이터 생성 ──

const makeDebugInfo = (bookstoreKeywords, categoryDetails = []) => ({
  bookstoreKeywords,
  categoryDetails,
});

// ── 기본 렌더링 ──

describe("SimilarityDebug 기본 렌더링", () => {
  it("debugInfo가 null이면 아무것도 렌더링하지 않는다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue(null);
    const { container } = render(
      <SimilarityDebug suggestedCategories={{}} categoryList={["소설"]} />,
    );
    expect(container.innerHTML).toBe("");
  });

  it("카드 헤더를 클릭하면 본문이 열린다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({ yes24_0_0: { original: "소설", keywords: ["소설"] } }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ yes24_0_0: "소설" }}
        categoryList={["소설"]}
      />,
    );
    // 헤더 존재 확인
    expect(screen.getByText("유사도 계산 디버그")).toBeTruthy();
    // 본문은 아직 안 보임
    expect(screen.queryByText("서점 카테고리에서 추출된 키워드:")).toBeNull();
    // 헤더 클릭
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    expect(screen.getByText("서점 카테고리에서 추출된 키워드:")).toBeTruthy();
  });
});

// ── 서점 뱃지 표시 (핵심 수정 부분) ──

describe("서점 뱃지 올바른 이름 표시", () => {
  it('yes24 키는 "yes24"로 표시한다', () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({
        yes24_0_0: { original: "소설 한국소설", keywords: ["한국소설"] },
      }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ yes24_0_0: "소설 한국소설" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    const badges = screen.getAllByText("yes24");
    expect(badges.length).toBeGreaterThanOrEqual(1);
  });

  it('aladin 키는 "aladin"으로 표시한다', () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({
        aladin_0_0: { original: "문학 한국문학", keywords: ["한국문학"] },
      }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ aladin_0_0: "문학 한국문학" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    const badges = screen.getAllByText("aladin");
    expect(badges.length).toBeGreaterThanOrEqual(1);
  });

  it('ridi 키는 "ridi"로 표시한다 (이전에는 aladin으로 잘못 표시됨)', () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({
        ridi_0_0: { original: "경영 경영일반", keywords: ["경영일반"] },
      }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ ridi_0_0: "경영 경영일반" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    const ridiBadges = screen.getAllByText("ridi");
    expect(ridiBadges.length).toBeGreaterThanOrEqual(1);
    // "aladin"이 표시되지 않아야 함
    expect(screen.queryByText("aladin")).toBeNull();
  });

  it("세 서점이 혼합되면 각각 올바른 이름으로 표시된다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({
        yes24_0_0: { original: "예술 대중문화론", keywords: ["대중문화론"] },
        aladin_0_0: {
          original: "예술/대중문화 예술/대중문화의 이해",
          keywords: ["대중문화의이해"],
        },
        ridi_0_0: { original: "경영/경제 경영일반", keywords: ["경영일반"] },
      }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{
          yes24_0_0: "예술 대중문화론",
          aladin_0_0: "예술/대중문화 예술/대중문화의 이해",
          ridi_0_0: "경영/경제 경영일반",
        }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    expect(screen.getAllByText("yes24").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("aladin").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("ridi").length).toBeGreaterThanOrEqual(1);
  });
});

// ── 뱃지 색상 ──

describe("서점 뱃지 색상", () => {
  it("yes24 뱃지는 보라색(#6A1B9A)이다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({ yes24_0_0: { original: "소설", keywords: ["소설"] } }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ yes24_0_0: "소설" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    const badge = screen.getByText("yes24");
    expect(badge.style.backgroundColor).toBe("rgb(106, 27, 154)");
  });

  it("aladin 뱃지는 파란색(#0D47A1)이다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({ aladin_0_0: { original: "소설", keywords: ["소설"] } }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ aladin_0_0: "소설" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    const badge = screen.getByText("aladin");
    expect(badge.style.backgroundColor).toBe("rgb(13, 71, 161)");
  });

  it("ridi 뱃지는 틸색(#00897B)이다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({ ridi_0_0: { original: "소설", keywords: ["소설"] } }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ ridi_0_0: "소설" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    const badge = screen.getByText("ridi");
    expect(badge.style.backgroundColor).toBe("rgb(0, 137, 123)");
  });

  it("알 수 없는 서점은 회색(#455A64) 폴백을 사용한다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({ unknown_0_0: { original: "소설", keywords: ["소설"] } }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ unknown_0_0: "소설" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    const badge = screen.getByText("unknown");
    expect(badge.style.backgroundColor).toBe("rgb(69, 90, 100)");
  });
});

// ── 매칭 상세 테이블의 서점 뱃지 ──

describe("매칭 상세 뱃지 표시", () => {
  it("categoryDetails의 matchDetails에서도 올바른 서점 이름을 표시한다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue({
      bookstoreKeywords: {
        ridi_0_0: { original: "경영 경영일반", keywords: ["경영일반"] },
      },
      categoryDetails: [
        {
          category: "4_경제",
          categoryName: "경제",
          dirKeywords: ["경제"],
          totalScore: 0.5,
          matchDetails: [
            {
              store: "ridi_0_0",
              bookstoreKeyword: "경영일반",
              dirKeyword: "경제",
              similarity: 0.35,
            },
          ],
        },
      ],
    });
    render(
      <SimilarityDebug
        suggestedCategories={{ ridi_0_0: "경영 경영일반" }}
        categoryList={["4_경제"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    // 매칭 상세에서도 "ridi"로 표시 (이전에는 "aladin"으로 잘못 표시)
    const ridiBadges = screen.getAllByText("ridi");
    // 키워드 테이블 + 매칭 상세 = 최소 2개
    expect(ridiBadges.length).toBe(2);
  });

  it("여러 서점의 매칭 상세가 각각 올바른 이름으로 표시된다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue({
      bookstoreKeywords: {
        yes24_0_0: { original: "예술 대중문화론", keywords: ["대중문화론"] },
        aladin_0_0: { original: "예술 대중문화", keywords: ["대중문화"] },
      },
      categoryDetails: [
        {
          category: "5_미술예술건축",
          categoryName: "미술예술건축",
          dirKeywords: ["미술", "예술", "건축"],
          totalScore: 0.43,
          matchDetails: [
            {
              store: "yes24_0_0",
              bookstoreKeyword: "대중문화론",
              dirKeyword: "미술예술건축",
              similarity: 0.36,
            },
            {
              store: "aladin_0_0",
              bookstoreKeyword: "대중문화",
              dirKeyword: "미술예술건축",
              similarity: 0.36,
            },
          ],
        },
      ],
    });
    render(
      <SimilarityDebug
        suggestedCategories={{
          yes24_0_0: "예술 대중문화론",
          aladin_0_0: "예술 대중문화",
        }}
        categoryList={["5_미술예술건축"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    // 매칭 상세에서 yes24, aladin 각각 표시
    expect(screen.getAllByText("yes24").length).toBe(2); // 키워드 + 매칭
    expect(screen.getAllByText("aladin").length).toBe(2); // 키워드 + 매칭
  });
});

describe("SimilarityDebug 카테고리 매핑 초기화", () => {
  it("캐시가 초기화되지 않았으면 fetchCategoryMappings로 매핑을 로드한다", () => {
    mockIsCacheInit.mockReturnValueOnce(false);
    mockGetSimilarityDebugInfo.mockReturnValue(null);
    render(
      <SimilarityDebug suggestedCategories={{}} categoryList={["소설"]} />,
    );
    // isCacheInitialized()가 false → useEffect의 else 분기 실행
    expect(mockFetchMappings).toHaveBeenCalled();
  });
});

// ── debugInfo 계산 조건 분기 (line 40) ──

describe("debugInfo 계산 조건의 거짓 분기", () => {
  it("suggestedCategories가 null이면 디버그 정보를 계산하지 않는다", () => {
    mockGetSimilarityDebugInfo.mockClear();
    mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo({}));
    const { container } = render(
      <SimilarityDebug suggestedCategories={null} categoryList={["소설"]} />,
    );
    // suggestedCategories가 falsy → else 분기 → debugInfo=null → 렌더링 없음
    expect(container.innerHTML).toBe("");
    expect(mockGetSimilarityDebugInfo).not.toHaveBeenCalled();
  });

  it("categoryList가 비어 있으면 디버그 정보를 계산하지 않는다", () => {
    mockGetSimilarityDebugInfo.mockClear();
    mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo({}));
    const { container } = render(
      <SimilarityDebug
        suggestedCategories={{ yes24_0_0: "소설" }}
        categoryList={[]}
      />,
    );
    // categoryList?.length가 0(falsy) → else 분기 → debugInfo=null
    expect(container.innerHTML).toBe("");
    expect(mockGetSimilarityDebugInfo).not.toHaveBeenCalled();
  });

  it("categoryList가 undefined이면 옵셔널 체이닝으로 계산하지 않는다", () => {
    mockGetSimilarityDebugInfo.mockClear();
    mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo({}));
    const { container } = render(
      <SimilarityDebug suggestedCategories={{ yes24_0_0: "소설" }} />,
    );
    // categoryList가 undefined → categoryList?.length === undefined(falsy)
    expect(container.innerHTML).toBe("");
    expect(mockGetSimilarityDebugInfo).not.toHaveBeenCalled();
  });

  it("suggestedCategories가 빈 객체면 길이 0 검사에서 걸러진다", () => {
    mockGetSimilarityDebugInfo.mockClear();
    mockGetSimilarityDebugInfo.mockReturnValue(makeDebugInfo({}));
    const { container } = render(
      <SimilarityDebug suggestedCategories={{}} categoryList={["소설"]} />,
    );
    // Object.keys(suggestedCategories).length > 0 거짓 → else 분기
    expect(container.innerHTML).toBe("");
    expect(mockGetSimilarityDebugInfo).not.toHaveBeenCalled();
  });
});

// ── getStoreName의 else 분기 (line 15) ──

describe("getStoreName 분기", () => {
  it("'_'가 없는 키는 키 전체를 서점명으로 사용한다", () => {
    // key.indexOf('_') === -1 → idx > 0 거짓 → key 그대로 반환
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({ yes24: { original: "소설", keywords: ["소설"] } }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ yes24: "소설" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    // '_'가 없으므로 키 전체("yes24")가 그대로 표시되고 알려진 색상 적용
    const badge = screen.getByText("yes24");
    expect(badge.style.backgroundColor).toBe("rgb(106, 27, 154)");
  });

  it("'_'가 맨 앞(index 0)인 키는 키 전체를 그대로 사용한다", () => {
    // key.indexOf('_') === 0 → idx > 0 거짓 → key 그대로 반환
    mockGetSimilarityDebugInfo.mockReturnValue(
      makeDebugInfo({ _hidden: { original: "소설", keywords: ["소설"] } }),
    );
    render(
      <SimilarityDebug
        suggestedCategories={{ _hidden: "소설" }}
        categoryList={["소설"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    // "_hidden"은 STORE_COLORS에 없으므로 회색 폴백
    const badge = screen.getByText("_hidden");
    expect(badge.style.backgroundColor).toBe("rgb(69, 90, 100)");
  });
});

// ── categoryDetails 행 스타일 및 총점 뱃지 분기 (line 120, 138, 154) ──

describe("categoryDetails 행 분기", () => {
  it("1위 행은 강조 스타일 없이 success 총점 뱃지, 2-5위는 노란 배경에 secondary 뱃지", () => {
    const makeDetail = (i) => ({
      category: `cat_${i}`,
      categoryName: `name_${i}`,
      dirKeywords: [`kw${i}`],
      totalScore: 0.5 - i * 0.05,
      matchDetails: [],
    });
    mockGetSimilarityDebugInfo.mockReturnValue({
      bookstoreKeywords: {
        yes24_0_0: { original: "소설", keywords: ["소설"] },
      },
      // 7개 행: idx 0(1위), idx 1-4(2-5위), idx 5-6(6위 이상)
      categoryDetails: [0, 1, 2, 3, 4, 5, 6].map(makeDetail),
    });
    const { container } = render(
      <SimilarityDebug
        suggestedCategories={{ yes24_0_0: "소설" }}
        categoryList={["cat_0"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));

    // 두 번째 테이블(상위 카테고리 유사도)의 행을 검사
    const rows = Array.from(container.querySelectorAll("tbody tr")).filter(
      (r) => /^cat_/.test(r.querySelector("strong")?.textContent || ""),
    );
    expect(rows.length).toBe(7);

    // idx 0(1위): 배경색 없음
    expect(rows[0].style.backgroundColor).toBe("");
    // idx 1(2위): 옅은 노란색 #FFF9C4
    expect(rows[1].style.backgroundColor).toBe("rgb(255, 249, 196)");
    // idx 4(5위): 여전히 노란색 (idx < 5)
    expect(rows[4].style.backgroundColor).toBe("rgb(255, 249, 196)");
    // idx 5(6위): idx >= 5 → 배경색 없음
    expect(rows[5].style.backgroundColor).toBe("");
    expect(rows[6].style.backgroundColor).toBe("");

    // 총점 뱃지: 1위는 success(녹색), 나머지는 secondary
    // 행별 마지막 td의 뱃지 색상으로 success/secondary 구분
    const firstTotalBadge = rows[0].querySelector("td:last-child .badge");
    const secondTotalBadge = rows[1].querySelector("td:last-child .badge");
    expect(firstTotalBadge.className).toMatch(/bg-success/);
    expect(secondTotalBadge.className).toMatch(/bg-secondary/);
  });

  it("matchDetails의 알 수 없는 서점은 회색(#455A64) 폴백을 사용한다", () => {
    mockGetSimilarityDebugInfo.mockReturnValue({
      bookstoreKeywords: {
        mystore_0_0: { original: "기타", keywords: ["기타"] },
      },
      categoryDetails: [
        {
          category: "cat_unknown",
          categoryName: "기타",
          dirKeywords: ["기타"],
          totalScore: 0.3,
          matchDetails: [
            {
              store: "mystore_0_0",
              bookstoreKeyword: "기타",
              dirKeyword: "기타",
              similarity: 0.3,
            },
          ],
        },
      ],
    });
    render(
      <SimilarityDebug
        suggestedCategories={{ mystore_0_0: "기타" }}
        categoryList={["cat_unknown"]}
      />,
    );
    fireEvent.click(screen.getByText("유사도 계산 디버그"));
    // 키워드 테이블 + 매칭 상세 모두 폴백 회색
    const badges = screen.getAllByText("mystore");
    expect(badges.length).toBe(2);
    badges.forEach((b) =>
      expect(b.style.backgroundColor).toBe("rgb(69, 90, 100)"),
    );
  });
});
