// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup, fireEvent } from "@testing-library/react";

vi.mock("../src/Common.js", () => ({
  getApiUrlPrefix: () => "http://api",
}));

import CoverGrid from "../src/CoverGrid";
import { readFileSync } from "node:fs";
import { COVER_THUMB_WIDTHS, coverThumbProps, markTallCover } from "../src/coverImageFit.js";

const loadWithSize = (img, width, height) => {
  Object.defineProperty(img, "naturalWidth", {
    value: width,
    configurable: true,
  });
  Object.defineProperty(img, "naturalHeight", {
    value: height,
    configurable: true,
  });
  fireEvent.load(img);
};

describe("markTallCover", () => {
  const mark = (width, height) => {
    const img = document.createElement("img");
    Object.defineProperty(img, "naturalWidth", { value: width });
    Object.defineProperty(img, "naturalHeight", { value: height });
    markTallCover({ currentTarget: img });
    return img.dataset.tall;
  };

  it("세로가 가로의 2배 미만이면 tall이 아니다", () => {
    expect(mark(690, 1000)).toBe("false");
    expect(mark(690, 1379)).toBe("false");
  });

  it("세로가 가로의 2배 이상이면 tall이다", () => {
    expect(mark(690, 1380)).toBe("true");
    expect(mark(690, 1600)).toBe("true");
  });

  it("가로가 더 긴 이미지와 크기를 모르는 이미지는 tall이 아니다", () => {
    expect(mark(1000, 500)).toBe("false");
    expect(mark(0, 0)).toBe("false");
  });
});

describe("coverThumbProps", () => {
  it("1x는 100, 2x는 200 썸네일을 srcSet으로 펼친다", () => {
    expect(coverThumbProps("/cover/7?v=1")).toEqual({
      src: "/cover/7?v=1&w=200",
      srcSet: "/cover/7?v=1&w=100 1x, /cover/7?v=1&w=200 2x",
    });
  });

  it("쿼리가 없는 URL에는 ?로 붙인다", () => {
    expect(coverThumbProps("/cover/7").srcSet).toBe(
      "/cover/7?w=100 1x, /cover/7?w=200 2x",
    );
  });
});

describe("썸네일 폭 계약", () => {
  it("CSS 카드 폭 상한은 서버 1x 썸네일 폭과 같고 2x는 그 두 배다", () => {
    const css = readFileSync("src/CoverGrid.css", "utf-8");
    const imageWidth = Number(css.match(/--cover-image-width:\s*(\d+)px/)[1]);
    const frameWidth = Number(css.match(/--cover-card-width:\s*calc\(var\(--cover-image-width\) \+ (\d+)px\)/)[1]);
    expect(COVER_THUMB_WIDTHS).toEqual([imageWidth, imageWidth * 2]);
    expect(frameWidth).toBe(2);
    const rust = readFileSync("../backend/src/book_manager/cover_list.rs", "utf-8");
    expect(rust).toMatch(
      new RegExp(`COVER_THUMBNAIL_WIDTHS: \\[u32; 2\\] = \\[${imageWidth}, ${imageWidth * 2}\\]`),
    );
  });
});

describe("CoverGrid 이미지 맞춤", () => {
  afterEach(cleanup);

  it("이미지가 로드되면 비율에 따라 data-tall을 붙인다", () => {
    class IO {
      observe() {}
      disconnect() {}
    }
    globalThis.IntersectionObserver = IO;
    render(
      <CoverGrid
        results={[
          { book_id: 1, category: "만화", file_path: "만화/a.pdf", file_type: "pdf" },
        ]}
        basePath="/comics-view"
      />,
    );
    const img = document.querySelector(".cover-grid-image");
    loadWithSize(img, 690, 1600);
    expect(img.dataset.tall).toBe("true");
    loadWithSize(img, 690, 1000);
    expect(img.dataset.tall).toBe("false");
    delete globalThis.IntersectionObserver;
  });
});
