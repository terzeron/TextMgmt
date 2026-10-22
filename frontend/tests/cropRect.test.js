import { describe, it, expect } from "vitest";
import {
  DEFAULT_CROP_RECT,
  DEFAULT_SPLIT_RECTS,
  MIN_CROP_FRACTION,
  boxCorners,
  dimClipPath,
  insidePage,
  normalizeAngle,
  resizeBox,
  resizeCursor,
  resizeRect,
  rotateBox,
  snapAngle,
  unionDimRects,
} from "../src/cropRect";

const rect = { x0: 0.2, y0: 0.2, x1: 0.8, y1: 0.8 };

describe("resizeRect", () => {
  it("변 핸들은 그 변만 움직인다", () => {
    expect(resizeRect(rect, "w", 0.1, 0.5)).toEqual({ ...rect, x0: 0.30000000000000004 });
    expect(resizeRect(rect, "n", 0.5, -0.1)).toEqual({ ...rect, y0: 0.1 });
    expect(resizeRect(rect, "e", 0.1, 0)).toEqual({ ...rect, x1: 0.9 });
    expect(resizeRect(rect, "s", 0, -0.1)).toEqual({ ...rect, y1: 0.7000000000000001 });
  });

  it("모서리 핸들은 두 변을 함께 움직인다", () => {
    const next = resizeRect(rect, "se", 0.1, -0.1);
    expect(next.x1).toBeCloseTo(0.9);
    expect(next.y1).toBeCloseTo(0.7);
    expect(next.x0).toBe(0.2);
    expect(next.y0).toBe(0.2);
  });

  it("쪽 밖으로 나가지 않는다", () => {
    expect(resizeRect(rect, "nw", -1, -1)).toEqual({ ...rect, x0: 0, y0: 0 });
    expect(resizeRect(rect, "se", 1, 1)).toEqual({ ...rect, x1: 1, y1: 1 });
  });

  it("반대편 변을 넘거나 최소 크기보다 작아지지 않는다", () => {
    const next = resizeRect(rect, "nw", 5, 5);
    expect(next.x0).toBeCloseTo(rect.x1 - MIN_CROP_FRACTION);
    expect(next.y0).toBeCloseTo(rect.y1 - MIN_CROP_FRACTION);
    const flipped = resizeRect(rect, "e", -5, 0);
    expect(flipped.x1).toBeCloseTo(rect.x0 + MIN_CROP_FRACTION);
  });

  it("기본 영역은 쪽 안쪽 10% 여백이다", () => {
    expect(DEFAULT_CROP_RECT).toEqual({
      x0: 0.1,
      y0: 0.1,
      x1: 0.9,
      y1: 0.9,
      angle: 0,
    });
  });
});

const area = (pieces) => pieces.reduce((sum, p) => sum + p.width * p.height, 0);
const rectArea = (r) => (r.x1 - r.x0) * (r.y1 - r.y0);

describe("unionDimRects", () => {
  it("박스 하나면 박스 바깥 면적을 덮는다", () => {
    expect(area(unionDimRects([DEFAULT_CROP_RECT]))).toBeCloseTo(
      1 - rectArea(DEFAULT_CROP_RECT),
    );
  });

  it("기본 분할 박스는 쪽 네 모서리에 맞는 서로소 왼쪽·오른쪽 절반이다", () => {
    const { first, second } = DEFAULT_SPLIT_RECTS;
    // 왼쪽(파랑)은 왼쪽 절반 전체, 오른쪽(빨강)은 오른쪽 절반 전체다.
    expect(first).toEqual({ x0: 0, y0: 0, x1: 0.5, y1: 1, angle: 0 });
    expect(second).toEqual({ x0: 0.5, y0: 0, x1: 1, y1: 1, angle: 0 });
    // 가운데에서 딱 맞닿고 겹치지 않으며, 둘을 합치면 쪽 전체다.
    expect(first.x1).toBe(second.x0);
    expect(rectArea(first) + rectArea(second)).toBeCloseTo(1);
    // 쪽 전체가 박스 안이므로 어둡게 덮을 곳이 없다.
    expect(area(unionDimRects([first, second]))).toBeCloseTo(0);
  });

  it("떨어진 두 박스는 각 박스 안을 덮지 않는다", () => {
    const { first, second } = DEFAULT_SPLIT_RECTS;
    expect(area(unionDimRects([first, second]))).toBeCloseTo(
      1 - rectArea(first) - rectArea(second),
    );
  });

  it("겹친 영역은 한 번만 센다(다시 어두워지지 않는다)", () => {
    const a = { x0: 0.1, y0: 0.1, x1: 0.6, y1: 0.9 };
    const b = { x0: 0.4, y0: 0.2, x1: 0.9, y1: 0.8 };
    const overlap = (0.6 - 0.4) * (0.8 - 0.2);
    expect(area(unionDimRects([a, b]))).toBeCloseTo(
      1 - rectArea(a) - rectArea(b) + overlap,
    );
  });

  it("덮개 조각은 어떤 박스 안쪽 점도 덮지 않는다", () => {
    const boxes = [
      { x0: 0.1, y0: 0.1, x1: 0.6, y1: 0.9 },
      { x0: 0.4, y0: 0.2, x1: 0.9, y1: 0.8 },
    ];
    const pieces = unionDimRects(boxes);
    for (const [x, y] of [[0.3, 0.5], [0.5, 0.5], [0.8, 0.5], [0.5, 0.85]]) {
      const inBox = boxes.some((b) => x > b.x0 && x < b.x1 && y > b.y0 && y < b.y1);
      const covered = pieces.some(
        (p) => x > p.left && x < p.left + p.width && y > p.top && y < p.top + p.height,
      );
      expect(covered).toBe(!inBox);
    }
  });

  it("맞닿은 두 박스 사이에 틈이 생기지 않는다", () => {
    const pieces = unionDimRects([
      { x0: 0.05, y0: 0.05, x1: 0.5, y1: 0.95 },
      { x0: 0.5, y0: 0.05, x1: 0.95, y1: 0.95 },
    ]);
    expect(pieces.some((p) => 0.5 > p.left + 1e-9 && 0.5 < p.left + p.width - 1e-9 && p.top < 0.5 && p.top + p.height > 0.5)).toBe(false);
  });
});

describe("angle helpers", () => {
  it("normalizeAngle 은 (-180, 180] 로 맞춘다", () => {
    expect(normalizeAngle(190)).toBe(-170);
    expect(normalizeAngle(-190)).toBe(170);
    expect(normalizeAngle(180)).toBe(180);
    expect(normalizeAngle(-180)).toBe(180);
    expect(normalizeAngle(720)).toBe(0);
  });

  it("snapAngle 은 15도 단위로 반올림한다", () => {
    expect(snapAngle(22)).toBe(15);
    expect(snapAngle(23)).toBe(30);
    expect(snapAngle(-38)).toBe(-45);
  });

  it("resizeCursor 는 박스 방향에 맞춰 바뀐다", () => {
    expect(resizeCursor("n", 0)).toBe("ns-resize");
    expect(resizeCursor("n", 90)).toBe("ew-resize");
    expect(resizeCursor("e", 90)).toBe("ns-resize");
    expect(resizeCursor("ne", 45)).toBe("ew-resize");
    expect(resizeCursor("nw", 0)).toBe("nwse-resize");
  });
});

describe("boxCorners / insidePage", () => {
  const box = { x0: 0.3, y0: 0.4, x1: 0.7, y1: 0.6, angle: 0 };

  it("각도 0 이면 축에 나란한 꼭짓점이다", () => {
    const corners = boxCorners(box, 100, 100);
    expect(corners[0][0]).toBeCloseTo(30);
    expect(corners[0][1]).toBeCloseTo(40);
    expect(corners[2][0]).toBeCloseTo(70);
    expect(corners[2][1]).toBeCloseTo(60);
  });

  it("90도 회전하면 가로세로가 바뀌어 보인다(중심 고정, 시계 방향)", () => {
    const [topLeft, topRight] = boxCorners({ ...box, angle: 90 }, 100, 100);
    // 위쪽 왼쪽 꼭짓점이 오른쪽 위로 간다: (-20,-10) → (10,-20)
    expect(topLeft[0]).toBeCloseTo(60);
    expect(topLeft[1]).toBeCloseTo(30);
    expect(topRight[0]).toBeCloseTo(60);
    expect(topRight[1]).toBeCloseTo(70);
  });

  it("꼭짓점이 쪽 밖으로 나가면 insidePage 가 거짓이다", () => {
    expect(insidePage(box, 100, 100)).toBe(true);
    const full = { x0: 0, y0: 0, x1: 1, y1: 1 };
    expect(insidePage({ ...full, angle: 10 }, 100, 100)).toBe(false);
    expect(insidePage({ ...full, angle: 0 }, 100, 100)).toBe(true);
  });
});

describe("resizeBox (회전한 박스)", () => {
  it("각도 0 이면 resizeRect 와 같다(픽셀 이동량을 비율로 바꾼다)", () => {
    const rect = { x0: 0.2, y0: 0.2, x1: 0.8, y1: 0.8, angle: 0 };
    expect(resizeBox(rect, "e", 10, 0, 100, 100)).toEqual(resizeRect(rect, "e", 0.1, 0));
  });

  it("기울어진 축을 따라 움직이고 반대편 변은 화면에서 고정된다", () => {
    const rect = { x0: 0.4, y0: 0.3, x1: 0.6, y1: 0.7, angle: 90 };
    // 90도: 박스의 가로축이 화면의 아래쪽을 향한다. 오른쪽 변(e)을 화면 아래로 10px 끌면 너비가 10px 는다.
    const next = resizeBox(rect, "e", 0, 10, 100, 100);
    expect(next.x1 - next.x0).toBeCloseTo(0.3);
    expect(next.y1 - next.y0).toBeCloseTo(0.4);
    expect(next.x0).toBeCloseTo(0.35);
    expect(next.y0).toBeCloseTo(0.35);
    // 반대편(w) 변의 중점은 (50, 40)에서 움직이지 않는다
    const [cx, cy] = [((next.x0 + next.x1) / 2) * 100, ((next.y0 + next.y1) / 2) * 100];
    const pw = (next.x1 - next.x0) * 100;
    expect(cx + Math.cos(Math.PI / 2) * (-pw / 2)).toBeCloseTo(50);
    expect(cy + Math.sin(Math.PI / 2) * (-pw / 2)).toBeCloseTo(40);
  });

  it("최소 크기보다 작아지지 않는다", () => {
    const rect = { x0: 0.4, y0: 0.3, x1: 0.6, y1: 0.7, angle: 30 };
    const next = resizeBox(rect, "w", 1000, 0, 100, 100);
    expect((next.x1 - next.x0) * 100).toBeGreaterThanOrEqual(MIN_CROP_FRACTION * 100 - 1e-6);
  });

  it("꼭짓점이 쪽 밖으로 나가기 전까지만 커진다", () => {
    const rect = { x0: 0.3, y0: 0.3, x1: 0.7, y1: 0.7, angle: 45 };
    const next = resizeBox(rect, "se", 500, 500, 100, 100);
    expect(insidePage(next, 100, 100)).toBe(true);
    expect(next.x1 - next.x0).toBeGreaterThan(0.4);
    expect(next.x1 - next.x0).toBeLessThan(1.0);
  });
});

describe("rotateBox", () => {
  it("여유가 있으면 목표 각도까지 돈다", () => {
    const square = { x0: 0.1, y0: 0.1, x1: 0.9, y1: 0.9, angle: 0 };
    expect(rotateBox(square, 0, 90, 100, 100)).toBeCloseTo(90);
  });

  it("꼭짓점이 쪽 밖으로 나가면 그 직전 각도에서 멈춘다", () => {
    const full = { x0: 0, y0: 0, x1: 1, y1: 1, angle: 0 };
    expect(Math.abs(rotateBox(full, 0, 45, 100, 100))).toBeLessThan(1);
    // 쪽 너비 전체인 막대: 조금만 기울여도 양끝이 쪽 밖으로 나간다
    const wide = { x0: 0, y0: 0.4, x1: 1, y1: 0.6, angle: 0 };
    const stopped = rotateBox(wide, 0, 6, 100, 100);
    expect(stopped).toBeGreaterThan(0);
    expect(stopped).toBeLessThan(6);
    expect(insidePage({ ...wide, angle: stopped }, 100, 100)).toBe(true);
  });

  it("가까운 방향으로 돈다(179 → -179 는 2도 차이)", () => {
    const square = { x0: 0.1, y0: 0.1, x1: 0.9, y1: 0.9, angle: 179 };
    expect(rotateBox(square, 179, -179, 100, 100)).toBeCloseTo(-179);
  });
});

describe("dimClipPath", () => {
  it("바깥 사각형과 박스 구멍을 even-odd 한 경로로 잇는다", () => {
    const path = dimClipPath([{ x0: 0.1, y0: 0.1, x1: 0.9, y1: 0.9, angle: 0 }], 100, 100);
    expect(path.startsWith("polygon(evenodd, 0% 0%, 100% 0%, 100% 100%")).toBe(true);
    expect(path).toContain("10% 10%");
    expect(path).toContain("90% 90%");
  });

  it("기울이면 꼭짓점이 회전한 위치로 들어간다", () => {
    const path = dimClipPath([{ x0: 0.3, y0: 0.4, x1: 0.7, y1: 0.6, angle: 90 }], 100, 100);
    expect(path).toContain("60% 30%");
    expect(path).toContain("60% 70%");
  });
});

describe("cropRect 보강 (경계 분기)", () => {
  it("unionDimRects 는 거의 같은 x 경계가 만든 폭 0 띠를 건너뛴다", () => {
    const a = { x0: 0.5, y0: 0, x1: 0.8, y1: 1 };
    const b = { x0: 0.5 + 1e-12, y0: 0, x1: 0.9, y1: 1 };
    const pieces = unionDimRects([a, b]);
    // 폭이 1e-9 이하인 조각은 없고, 덮는 면적은 쪽 전체에서 합집합(0.5~0.9)을 뺀 값이다.
    expect(pieces.every((piece) => piece.width > 1e-9)).toBe(true);
    expect(area(pieces)).toBeCloseTo(1 - 0.4);
  });

  it("angle 이 없는 박스는 각도 0 으로 보고 꼭짓점을 계산한다", () => {
    const corners = boxCorners({ x0: 0.25, y0: 0.25, x1: 0.75, y1: 0.75 }, 100, 100);
    expect(corners).toEqual([
      [25, 25],
      [75, 25],
      [75, 75],
      [25, 75],
    ]);
  });

  it("angle 이 없는 박스의 resizeBox 는 resizeRect 와 같다", () => {
    const plain = { x0: 0.2, y0: 0.2, x1: 0.8, y1: 0.8 };
    expect(resizeBox(plain, "s", 0, 10, 100, 100)).toEqual(resizeRect(plain, "s", 0, 0.1));
  });

  it("기울어진 박스의 위쪽(n) 변은 박스 축을 따라 움직이고 아래쪽 변은 고정된다", () => {
    const rect = { x0: 0.4, y0: 0.3, x1: 0.6, y1: 0.7, angle: 90 };
    // 90도: 박스의 세로축이 화면의 왼쪽을 향한다. 위쪽 변을 화면 왼쪽으로 10px 끌면 높이가 10px 줄어든다.
    const next = resizeBox(rect, "n", -10, 0, 100, 100);
    expect(next.y1 - next.y0).toBeCloseTo(0.3);
    expect(next.x1 - next.x0).toBeCloseTo(0.2);
  });

  it("기울어진 박스의 아래쪽(s) 변도 박스 축을 따라 움직인다", () => {
    const rect = { x0: 0.4, y0: 0.3, x1: 0.6, y1: 0.7, angle: 90 };
    // 90도: 아래쪽 변을 화면 왼쪽으로 10px 끌면 높이가 10px 는다.
    const next = resizeBox(rect, "s", -10, 0, 100, 100);
    expect(next.y1 - next.y0).toBeCloseTo(0.5);
  });
});
