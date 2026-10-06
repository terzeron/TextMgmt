// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  cleanup,
  fireEvent,
  act,
} from "@testing-library/react";

vi.mock("../src/Common.js", () => ({
  getApiUrlPrefix: () => "http://api",
}));

import CoverGrid, { COVER_BATCH_SIZE } from "../src/CoverGrid";

let observers = [];
class MockIntersectionObserver {
  constructor(callback) {
    this.callback = callback;
    observers.push(this);
  }
  observe() {}
  disconnect() {}
  fire(isIntersecting) {
    act(() => this.callback([{ isIntersecting }]));
  }
}

const makeBooks = (count, fileType = "epub") =>
  Array.from({ length: count }, (_, i) => ({
    book_id: i + 1,
    category: "소설",
    file_path: `소설/book${i + 1}.${fileType}`,
    file_type: fileType,
  }));

describe("CoverGrid", () => {
  beforeEach(() => {
    observers = [];
    globalThis.IntersectionObserver = MockIntersectionObserver;
  });

  afterEach(() => {
    cleanup();
    delete globalThis.IntersectionObserver;
  });

  it("epub·pdf는 커버 API 이미지를 요청한다", () => {
    render(
      <CoverGrid
        results={[
          ...makeBooks(1, "epub"),
          { ...makeBooks(1, "pdf")[0], book_id: 7 },
        ]}
        basePath="/book-view"
      />,
    );
    const imgs = Array.from(document.querySelectorAll("img"));
    expect(imgs[0].getAttribute("src")).toMatch(
      /^http:\/\/api\/cover\/1\?v=\d{10}$/,
    );
    expect(imgs[1].getAttribute("src")).toMatch(
      /^http:\/\/api\/cover\/7\?v=\d{10}$/,
    );
    expect(imgs[0].getAttribute("loading")).toBe("lazy");
  });

  it("만화 탭은 /comics 커버 API를 쓴다", () => {
    render(<CoverGrid results={makeBooks(1, "pdf")} basePath="/comics-view" />);
    expect(document.querySelector("img").getAttribute("src")).toMatch(
      /^http:\/\/api\/comics\/cover\/1\?v=\d{10}$/,
    );
  });

  it("텍스트 포맷은 이미지를 요청하지 않고 대문자 포맷명을 보여준다", () => {
    render(<CoverGrid results={makeBooks(1, "txt")} basePath="/book-view" />);
    expect(document.querySelector("img")).toBeNull();
    expect(screen.getByText("TXT")).toBeTruthy();
  });

  it("커버 로드에 실패하면 포맷명 박스로 바꾼다", () => {
    render(<CoverGrid results={makeBooks(1, "epub")} basePath="/book-view" />);
    fireEvent.error(document.querySelector("img"));
    expect(document.querySelector("img")).toBeNull();
    expect(screen.getByText("EPUB")).toBeTruthy();
  });

  it("카드는 새 탭에서 조회 화면을 연다", () => {
    render(<CoverGrid results={makeBooks(1, "epub")} basePath="/book-view" />);
    const link = screen.getByRole("link");
    expect(link.getAttribute("href")).toBe(
      "/book-view/1?category=%EC%86%8C%EC%84%A4",
    );
    expect(link.getAttribute("target")).toBe("_blank");
    expect(screen.getByText("book1.epub")).toBeTruthy();
  });

  it("커버 아래에는 파일명 대신 제목을 표시하고 저자명은 숨긴다", () => {
    const { container } = render(
      <CoverGrid
        results={[
          {
            book_id: 1,
            category: "소설",
            title: "표시할 책 제목",
            author: "숨길 저자",
            file_path: "소설/[숨길 저자] 원본 파일.epub",
            file_type: "epub",
          },
        ]}
        basePath="/book-view"
      />,
    );

    expect(screen.getByText("표시할 책 제목")).toBeTruthy();
    expect(container.textContent).not.toContain("숨길 저자");
    expect(screen.getByRole("link").getAttribute("title")).toBe(
      "표시할 책 제목",
    );
  });

  it("display_title이 있으면 커버 아래에 디렉토리 이름을 표시한다", () => {
    render(
      <CoverGrid
        results={[
          {
            ...makeBooks(1, "pdf")[0],
            title: "첫 파일 제목",
            display_title: "작품 디렉토리",
          },
        ]}
        basePath="/comics-view"
      />,
    );

    expect(screen.getByText("작품 디렉토리")).toBeTruthy();
    expect(screen.queryByText("첫 파일 제목")).toBeNull();
    expect(screen.getByRole("link").getAttribute("title")).toBe(
      "작품 디렉토리",
    );
  });

  it(`처음에는 ${COVER_BATCH_SIZE}개만 그리고 끝에 닿으면 더 그린다`, () => {
    render(<CoverGrid results={makeBooks(45, "txt")} basePath="/book-view" />);
    expect(screen.getAllByRole("link")).toHaveLength(20);

    observers.at(-1).fire(false);
    expect(screen.getAllByRole("link")).toHaveLength(20);

    observers.at(-1).fire(true);
    expect(screen.getAllByRole("link")).toHaveLength(40);

    observers.at(-1).fire(true);
    expect(screen.getAllByRole("link")).toHaveLength(45);
  });

  it("IntersectionObserver가 없으면 전부 그린다", () => {
    delete globalThis.IntersectionObserver;
    render(<CoverGrid results={makeBooks(25, "txt")} basePath="/book-view" />);
    expect(screen.getAllByRole("link")).toHaveLength(25);
  });

  it("결과가 바뀌면 다시 첫 묶음부터 그린다", () => {
    const { rerender } = render(
      <CoverGrid results={makeBooks(45, "txt")} basePath="/book-view" />,
    );
    observers.at(-1).fire(true);
    expect(screen.getAllByRole("link")).toHaveLength(40);

    rerender(
      <CoverGrid results={makeBooks(30, "txt")} basePath="/book-view" />,
    );
    expect(screen.getAllByRole("link")).toHaveLength(20);
  });

  it("file_type·category·제목 정보가 비어 있어도 기본값으로 그린다", () => {
    const { container } = render(
      <CoverGrid
        results={[
          { book_id: 1 },
          { book_id: 2, title: "제목만 있음", file_type: "txt" },
          { book_id: 3, display_title: "표시 제목", file_path: "a/b.txt" },
        ]}
        basePath="/book-edit"
      />,
    );

    expect(container.querySelector('a[href="/book-view/1?category=_root"]')).toBeTruthy();
    expect(screen.getByText("Unknown")).toBeTruthy();
    expect(screen.getAllByText("?").length).toBe(2);
    expect(screen.getByText("제목만 있음")).toBeTruthy();
    expect(screen.getByText("표시 제목")).toBeTruthy();
  });

  it("__COVER_VERSION__ 정의가 없으면 dev 버전으로 커버 URL을 만든다", async () => {
    const saved = globalThis.__COVER_VERSION__;
    delete globalThis.__COVER_VERSION__;
    vi.resetModules();
    try {
      const { default: FreshCoverGrid } = await import("../src/CoverGrid");
      const { container } = render(
        <FreshCoverGrid results={makeBooks(1)} basePath="/book-edit" />,
      );
      expect(container.querySelector("img").getAttribute("src")).toContain("v=");
    } finally {
      if (saved !== undefined) globalThis.__COVER_VERSION__ = saved;
    }
  });
});
