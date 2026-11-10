import { describe, it, expect } from "vitest";
import { computeTilePlan, DISPLAY_SCALE } from "../src/pdfRenderScale";

describe("computeTilePlan", () => {
  // 120화 1쪽과 같은 크기: 517.5 x 12287.25pt
  const tall = { getViewport: () => ({ width: 517.5, height: 12287.25 }) };

  it("면적 상한을 넘는 긴 쪽은 화면 폭 x dpr 해상도로 타일을 나눈다", () => {
    const plan = computeTilePlan(tall, 690, 2);
    expect(plan.width).toBe(1380);
    expect(plan.height).toBe(32766);
    expect(plan.scale).toBeCloseTo(1380 / 517.5);
    const tileH = Math.floor(3_000_000 / 1380);
    expect(plan.tiles).toHaveLength(Math.ceil(32766 / tileH));
    expect(plan.tiles[0]).toEqual({ top: 0, height: tileH });
    expect(plan.tiles[1].top).toBe(tileH);
    const last = plan.tiles.at(-1);
    expect(last.top + last.height).toBe(32766);
    for (const t of plan.tiles) expect(plan.width * t.height).toBeLessThanOrEqual(3_000_000);
  });

  it("화면 폭이 작으면 기본 배율(DISPLAY_SCALE x dpr)을 하한으로 쓴다", () => {
    const plan = computeTilePlan(tall, 10, 1);
    expect(plan.scale).toBeCloseTo(DISPLAY_SCALE);
  });

  it("일반 비율의 쪽은 면적 상한을 넘어도 타일로 나누지 않는다", () => {
    const normal = { getViewport: () => ({ width: 600, height: 900 }) };
    expect(computeTilePlan(normal, 690, 2)).toBeNull();
    expect(computeTilePlan(normal, 1200, 2)).toBeNull();
  });

  it("긴 쪽이라도 면적 상한 안에 들면 타일이 필요 없다", () => {
    const strip = { getViewport: () => ({ width: 100, height: 400 }) };
    expect(computeTilePlan(strip, 300, 1)).toBeNull();
  });

  it("화면 폭을 모르거나 쪽 크기가 0 이면 null 이다", () => {
    expect(computeTilePlan(tall, 0, 2)).toBeNull();
    const zero = { getViewport: () => ({ width: 0, height: 100 }) };
    expect(computeTilePlan(zero, 690, 2)).toBeNull();
  });
});
