// 일회성 생성기: 기존 JS 서점 카테고리 대응 로직의 입출력을 Rust 이전 검증용 fixture로 쓴다.
// Rust 구현(libs/book_manager/src/store_category_matcher.rs)이 이 값과 같아야 한다.
// JS 로직을 지우는 작업(Task 6)에서 이 파일도 함께 지운다.
import { describe, it, expect, vi } from "vitest";
import fs from "node:fs";
import path from "node:path";

const MAPPINGS = {
  "1_서양고전": ["클래식", "고전"],
  "5_음악": ["클래식", "음악"],
  "3_SF": ["과학소설", "SF"],
  "7_영어교육": ["영어", "토익"],
  "4_경영마케팅": ["마케팅", "경영일반"],
};

vi.mock("../src/categoryMappingCache", () => ({
  loadCategoryMappings: () => MAPPINGS,
  fetchCategoryMappings: () => Promise.resolve(MAPPINGS),
  isCacheInitialized: () => true,
  updateCachedMappings: () => {},
}));

const {
  calculateSimilarity,
  calculateCategoryScores,
  findTopSimilarCategories,
  getSimilarityDebugInfo,
} = await import("../src/Actions");
const { getTwoLevelCategory, extractMultiPathCategories } =
  await import("../src/Bookstore");

const CATEGORIES = [
  "3_SF",
  "2_소설한국",
  "4_경영마케팅",
  "4_경제",
  "5_음악",
  "1_서양고전",
  "7_영어교육",
  "3_판타지",
  "2_시",
  "4_철학윤리",
  "6_재테크",
  "3_무협",
  "4_역사인물",
  "5_수학과학일반",
  "2_소설일본",
  "4_심리학뇌과학",
  "2_수필서간일기",
  "nf_decode_work",
];

const STORE_CATEGORIES = [
  "SF 한국SF",
  "소설/시/희곡 한국소설",
  "경제경영 마케팅/세일즈",
  "경제경영 경제일반",
  "예술/대중문화 클래식",
  "외국어 토익",
  "인문학 서양철학",
  "소설/시/희곡 시",
  "판타지 무협",
  "역사 인물",
  "과학 수학",
  "국내도서 소설",
  "소설 일본소설",
  "a",
  "x y z w v u t s r q p o",
  "재테크/투자 부동산",
  "일반",
  "자기계발 처세술",
  "만화 학원만화",
  "English Fiction",
  "Business Management",
  "SF",
  "고전 서양고전문학",
  "중국소설",
  "심리학 뇌과학",
  "에세이 한국에세이",
  "판타지/환상문학",
  "경영일반",
  "소",
  "철학 윤리학",
];

const SIMILARITY_PAIRS = [
  ["SF", "SF"],
  ["소", "소설"],
  ["시", "러시아소설"],
  ["소설", "세계각국소설"],
  ["마케팅", "경영마케팅"],
  ["경제", "경제경영"],
  ["Fiction", "fiction"],
  ["ab", "abc"],
  ["a", "b"],
  ["무협", "판타지무협"],
  ["심리학", "심리학뇌과학"],
  ["철학", "철학윤리"],
  ["", "x"],
  ["English", "Englsh"],
  ["클래식", "고전음악"],
];

const RAW_PATHS = [
  "소설/시/희곡 > SF > 한국SF",
  "소설/시/희곡 > 중국소설",
  "한국SF",
  "",
  "소설 > 한국소설 || 소설 > 추리/미스터리/스릴러",
  "국내도서 > 경제경영 > 마케팅/세일즈 > 마케팅전략",
  "  a  >  b  ",
  "a>>b",
  "x || ",
  "외국도서 > ELT/사전 || 국내도서 > 외국어 > 토익",
];

const SUGGESTED = [
  [
    ["yes24_0_0", "SF 한국SF"],
    ["aladin_0_0", "SF 한국SF"],
    ["kyobo_0_0", "소설/시/희곡 한국소설"],
  ],
  [
    ["yes24_0_0", "예술/대중문화 클래식"],
    ["aladin_0_0", "고전 서양고전문학"],
  ],
  [
    ["yes24_0_0", "경제경영 마케팅/세일즈"],
    ["yes24_1_0", "경제경영 경제일반"],
    ["kyobo_0_0", "경영일반"],
  ],
  [["aladin_0_0", "x y z w v u t s r q p o"]],
  [
    ["yes24_0_0", "판타지 무협"],
    ["aladin_0_0", "판타지/환상문학"],
    ["kyobo_0_0", "만화 학원만화"],
  ],
];

describe("store category fixture generator", () => {
  it("writes the JS behavior as a JSON fixture", () => {
    const fixture = {
      categories: CATEGORIES,
      mappings: MAPPINGS,
      similarity: SIMILARITY_PAIRS.map(([a, b]) => ({
        a,
        b,
        score: calculateSimilarity(a, b),
      })),
      two_level: RAW_PATHS.map((input) => ({
        input,
        output: getTwoLevelCategory(input),
      })),
      multi_path: RAW_PATHS.map((input) => ({
        input,
        output: extractMultiPathCategories(input),
      })),
      scores: STORE_CATEGORIES.map((store_category) => ({
        store_category,
        scores: Object.entries(
          calculateCategoryScores(store_category, CATEGORIES),
        ),
      })),
      top: SUGGESTED.map((suggested) => ({
        suggested,
        top_n: 3,
        result: findTopSimilarCategories(
          Object.fromEntries(suggested),
          CATEGORIES,
          3,
        ),
      })),
      debug: SUGGESTED.map((suggested) => ({
        suggested,
        top_n: 5,
        result: getSimilarityDebugInfo(
          Object.fromEntries(suggested),
          CATEGORIES,
          5,
        ),
      })),
    };
    const out = path.resolve(
      __dirname,
      "../../tests/rust/book_manager/fixtures/store_category_scores.json",
    );
    fs.writeFileSync(out, JSON.stringify(fixture, null, 1) + "\n");
    expect(fixture.scores.length).toBe(STORE_CATEGORIES.length);
  });
});
