import { describe, it, expect } from "vitest";
import {
  swipeDirection,
  keyDirection,
  currentPageIndex,
} from "../src/pdfPageTurn";

const at = (top) => ({ getBoundingClientRect: () => ({ top }) });

describe("swipeDirection", () => {
  it("왼쪽으로 밀면 다음, 오른쪽으로 밀면 이전이다", () => {
    expect(swipeDirection(-60, 0)).toBe(1);
    expect(swipeDirection(60, 5)).toBe(-1);
  });

  it("40px 미만이거나 세로 이동이 크면 스와이프가 아니다", () => {
    expect(swipeDirection(-39, 0)).toBe(0);
    expect(swipeDirection(-60, 50)).toBe(0);
    expect(swipeDirection(-60, 40)).toBe(1);
  });
});

describe("keyDirection", () => {
  it("오른쪽 방향키는 다음, 왼쪽은 이전, 나머지는 0이다", () => {
    expect(keyDirection("ArrowRight")).toBe(1);
    expect(keyDirection("ArrowLeft")).toBe(-1);
    expect(keyDirection("ArrowDown")).toBe(0);
    expect(keyDirection(" ")).toBe(0);
  });
});

describe("currentPageIndex", () => {
  it("툴바 아래 기준선에 가장 가까운 쪽을 고른다", () => {
    const container = at(100);
    const pages = [at(-500), at(132), at(700)]; // 기준선 = 100 + 32
    expect(currentPageIndex(container, pages, 32)).toBe(1);
  });

  it("쪽이 없으면 -1이다", () => {
    expect(currentPageIndex(at(0), [], 32)).toBe(-1);
  });
});
