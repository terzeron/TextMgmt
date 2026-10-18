// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup, fireEvent } from "@testing-library/react";

vi.mock("../src/Common.js", () => ({
  getApiUrlPrefix: () => "http://api",
}));

import CoverGrid from "../src/CoverGrid";
import { markTallCover } from "../src/coverImageFit.js";

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
