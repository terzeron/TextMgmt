import { describe, it, expect } from "vitest";
import { rankCoverImages, formatCoverBytes } from "../src/coverImageRanking";

describe("책별 표지 크기 순위", () => {
  it("확보한 3개 후보를 큰 순서로 초록색, 노란색, 빨간색으로 나눈다", () => {
    const candidates = ["small", "large", "middle"].map((candidate_id) => ({ candidate_id }));
    const images = {
      small: { width: 200, height: 300 },
      large: { width: 600, height: 900 },
      middle: { width: 400, height: 600 },
    };
    const ranks = rankCoverImages(candidates, images);
    expect(ranks.large).toEqual({ rank: 1, color: "#00ff00", count: 3 });
    expect(ranks.middle).toEqual({ rank: 2, color: "#ffff00", count: 3 });
    expect(ranks.small).toEqual({ rank: 3, color: "#ff0000", count: 3 });
  });

  it("확보 전 후보는 제외하고 같은 픽셀 수에는 같은 색을 쓴다", () => {
    const candidates = ["a", "b", "pending"].map((candidate_id) => ({ candidate_id }));
    const ranks = rankCoverImages(candidates, { a: { width: 300, height: 600 }, b: { width: 600, height: 300 } });
    expect(ranks.a.color).toBe("#00ff00");
    expect(ranks.b.color).toBe(ranks.a.color);
    expect(ranks.pending).toBeUndefined();
    expect(ranks.a.count).toBe(2);
  });

  it("후보 1개는 초록색이며 다른 책의 이미지는 순위에서 제외한다", () => {
    expect(rankCoverImages([{ candidate_id: "a" }], {
      a: { width: 100, height: 200 }, other: { width: 1000, height: 2000 },
    }).a).toEqual({ rank: 1, color: "#00ff00", count: 1 });
    expect(rankCoverImages([], {})).toEqual({});
  });

  it("파일 크기를 B와 KB로 표시한다", () => {
    expect(formatCoverBytes(500)).toBe("500 B");
    expect(formatCoverBytes(1536)).toBe("1.5 KB");
  });

  it("이전 방문에서 로컬 캐시한 표지도 이미지 요청 없이 순위에 포함한다", () => {
    const ranked = rankCoverImages([
      { candidate_id: "cached", width: 1000, height: 1500 },
      { candidate_id: "visible" },
    ], { visible: { width: 500, height: 750 } });
    expect(ranked.cached.color).toBe("#00ff00");
    expect(ranked.visible.color).toBe("#ff0000");
  });
});
