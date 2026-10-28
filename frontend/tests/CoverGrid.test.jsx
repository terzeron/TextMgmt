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
      /^http:\/\/api\/cover\/1\?v=\d{10}&path=/,
    );
    expect(imgs[1].getAttribute("src")).toMatch(
      /^http:\/\/api\/cover\/7\?v=\d{10}&path=/,
    );
    expect(imgs[0].getAttribute("srcset")).toMatch(
      /&w=100 1x, http:\/\/api\/cover\/1\?v=\d{10}.*&w=200 2x$/,
    );
  });

  it("만화 탭은 /comics 커버 API를 쓴다", () => {
    render(<CoverGrid results={makeBooks(1, "pdf")} basePath="/comics-view" />);
    expect(document.querySelector("img").getAttribute("src")).toMatch(
      /^http:\/\/api\/comics\/cover\/1\?v=\d{10}&path=/,
    );
  });

  it("동시에 커버 10개만 요청하고 완료된 슬롯으로 다음 커버를 요청한다", async () => {
    const { container } = render(
      <CoverGrid results={makeBooks(12, "epub")} basePath="/comics-view" />,
    );
    observers.at(-1).fire(true);

    const images = Array.from(container.querySelectorAll("img"));
    expect(images).toHaveLength(12);
    expect(images.slice(0, 10).every((image) => image.hasAttribute("src"))).toBe(true);
    expect(images.slice(10).every((image) => !image.hasAttribute("src"))).toBe(true);

    fireEvent.load(images[0]);
    await vi.waitFor(() =>
      expect(images[10].getAttribute("src")).toMatch(/\/comics\/cover\/11\?/),
    );
  });

  it("updated_time이 바뀌면 커버 URL도 바뀌어 브라우저 캐시를 우회한다", () => {
    const [book] = makeBooks(1, "pdf");
    const src = (updatedTime) => {
      const { container, unmount } = render(
        <CoverGrid
          results={[{ ...book, updated_time: updatedTime }]}
          basePath="/comics-view"
        />,
      );
      const value = container.querySelector("img").getAttribute("src");
      unmount();
      return value;
    };
    const before = src("2026-10-16T09:00:00.123456");
    const after = src("2026-10-16T10:30:00.654321");
    expect(before).toContain("&u=2026-10-16T09%3A00%3A00.123456");
    expect(after).not.toBe(before);
  });

  it("file_path 가 없으면 커버 URL 에 path 파라미터를 붙이지 않는다", () => {
    const [book] = makeBooks(1, "epub");
    const { container } = render(
      <CoverGrid results={[{ ...book, file_path: undefined }]} basePath="/book-view" />,
    );
    const src = container.querySelector("img").getAttribute("src");
    expect(src).toContain(`/cover/${book.book_id}?v=`);
    expect(src).not.toContain("path=");
  });

  it("ID와 updated_time이 같아도 원본 경로가 다르면 커버 URL을 구분한다", () => {
    const [book] = makeBooks(1, "pdf");
    const firstPath = "8_여성향/[문나영, 이범배] 신혼부부 생활 백서 (1-111화 완)/[0001] 0001화.pdf";
    const secondPath = "8_여성향/[이루이] 시한부의 아이까지 뺏으려 합니다 (1-82화 완)/[0001] 0001화.pdf";
    const { container, rerender } = render(
      <CoverGrid results={[{ ...book, file_path: firstPath }]} basePath="/comics-view" />,
    );
    const before = new URL(container.querySelector("img").src);

    rerender(
      <CoverGrid results={[{ ...book, file_path: secondPath }]} basePath="/comics-view" />,
    );
    const after = new URL(container.querySelector("img").src);

    expect(before.searchParams.get("path")).toBe(firstPath);
    expect(after.searchParams.get("path")).toBe(secondPath);
    expect(after.href).not.toBe(before.href);
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

  it("커버 제목의 [저자] prefix를 책과 만화 모두에서 숨긴다", () => {
    render(
      <CoverGrid
        results={[
          { ...makeBooks(1, "epub")[0], display_title: "[책 저자] 책 제목" },
          { ...makeBooks(1, "pdf")[0], book_id: 2, display_title: "[만화 작가] 만화 제목" },
          { ...makeBooks(1, "epub")[0], book_id: 3, display_title: "01 [번호 저자] 번호 책 제목" },
        ]}
        basePath="/comics-view"
      />,
    );

    expect(screen.getByText("책 제목")).toBeTruthy();
    expect(screen.getByText("만화 제목")).toBeTruthy();
    expect(screen.getByText("번호 책 제목")).toBeTruthy();
    expect(screen.queryByText("[책 저자] 책 제목")).toBeNull();
    expect(screen.queryByText("[만화 작가] 만화 제목")).toBeNull();
    expect(screen.queryByText("01 [번호 저자] 번호 책 제목")).toBeNull();
  });

  it(`커버 요청을 ${COVER_BATCH_SIZE}개씩 나누어 화면 끝에서 이어 그린다`, () => {
    render(<CoverGrid results={makeBooks(45, "txt")} basePath="/book-view" />);
    expect(screen.getAllByRole("link")).toHaveLength(10);

    observers.at(-1).fire(false);
    expect(screen.getAllByRole("link")).toHaveLength(10);

    observers.at(-1).fire(true);
    expect(screen.getAllByRole("link")).toHaveLength(20);

    observers.at(-1).fire(true);
    expect(screen.getAllByRole("link")).toHaveLength(30);
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
    expect(screen.getAllByRole("link")).toHaveLength(20);

    rerender(
      <CoverGrid results={makeBooks(30, "txt")} basePath="/book-view" />,
    );
    expect(screen.getAllByRole("link")).toHaveLength(10);
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

    expect(
      container.querySelector('a[href="/book-view/1?category=_root"]'),
    ).toBeTruthy();
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
      expect(container.querySelector("img").getAttribute("src")).toContain(
        "v=",
      );
    } finally {
      if (saved !== undefined) globalThis.__COVER_VERSION__ = saved;
    }
  });

  describe("화면 안 선택과 서버 페이지 이어받기", () => {
    it("서버 페이지를 이어받는 목록의 카드도 새 탭에서 연다", () => {
      render(
        <CoverGrid
          results={makeBooks(1, "epub")}
          basePath="/book-view"
          hasMore
          onLoadMore={vi.fn()}
        />,
      );

      const link = screen.getByRole("link");
      expect(link.getAttribute("target")).toBe("_blank");
      expect(link.getAttribute("rel")).toBe("noopener noreferrer");
      // 클릭을 가로채지 않으므로 링크 기본 동작(새 탭)이 그대로 일어난다.
      expect(fireEvent.click(link)).toBe(true);
    });

    it("그린 것을 다 보여 준 뒤 끝에 닿으면 서버의 다음 페이지를 부른다", () => {
      const onLoadMore = vi.fn();
      render(
        <CoverGrid
          results={makeBooks(20, "txt")}
          basePath="/book-view"
          hasMore
          onLoadMore={onLoadMore}
        />,
      );
      expect(screen.getAllByRole("link")).toHaveLength(10);

      observers.at(-1).fire(false);
      expect(onLoadMore).not.toHaveBeenCalled();

      observers.at(-1).fire(true);
      expect(screen.getAllByRole("link")).toHaveLength(20);
      observers.at(-1).fire(true);
      expect(onLoadMore).toHaveBeenCalledTimes(1);
    });

    it("받아 둔 항목이 남아 있으면 서버 페이지보다 먼저 그린다", () => {
      const onLoadMore = vi.fn();
      render(
        <CoverGrid
          results={makeBooks(45, "txt")}
          basePath="/book-view"
          hasMore
          onLoadMore={onLoadMore}
        />,
      );

      observers.at(-1).fire(true);

      expect(screen.getAllByRole("link")).toHaveLength(20);
      expect(onLoadMore).not.toHaveBeenCalled();
    });

    it("서버 페이지를 받는 중이면 다음 페이지를 중복해서 부르지 않는다", () => {
      const onLoadMore = vi.fn();
      render(
        <CoverGrid
          results={makeBooks(10, "txt")}
          basePath="/book-view"
          hasMore
          loading
          onLoadMore={onLoadMore}
        />,
      );

      observers.at(-1).fire(true);

      expect(onLoadMore).not.toHaveBeenCalled();
    });

    it("서버에 더 없으면 끝 감지 요소를 두지 않는다", () => {
      const { container } = render(
        <CoverGrid
          results={makeBooks(10, "txt")}
          basePath="/book-view"
          hasMore={false}
          onLoadMore={vi.fn()}
        />,
      );
      expect(container.querySelector(".cover-grid-sentinel")).toBeNull();
    });

    it("resetKey가 그대로면 서버 페이지가 이어붙어도 그린 개수를 되돌리지 않는다", () => {
      const props = {
        basePath: "/book-view",
        hasMore: true,
        onLoadMore: vi.fn(),
      };
      const { rerender } = render(
        <CoverGrid results={makeBooks(40, "txt")} resetKey="A" {...props} />,
      );
      observers.at(-1).fire(true);
      expect(screen.getAllByRole("link")).toHaveLength(20);

      rerender(
        <CoverGrid results={makeBooks(100, "txt")} resetKey="A" {...props} />,
      );

      expect(screen.getAllByRole("link")).toHaveLength(20);
    });

    it("resetKey가 바뀌면 첫 묶음부터 다시 그린다", () => {
      const props = {
        basePath: "/book-view",
        hasMore: true,
        onLoadMore: vi.fn(),
      };
      const { rerender } = render(
        <CoverGrid results={makeBooks(40, "txt")} resetKey="A" {...props} />,
      );
      observers.at(-1).fire(true);
      expect(screen.getAllByRole("link")).toHaveLength(20);

      rerender(
        <CoverGrid results={makeBooks(40, "txt")} resetKey="B" {...props} />,
      );

      expect(screen.getAllByRole("link")).toHaveLength(10);
    });

    it("IntersectionObserver가 없어도 서버 페이지를 이어받는다", () => {
      delete globalThis.IntersectionObserver;
      const onLoadMore = vi.fn();
      render(
        <CoverGrid
          results={makeBooks(25, "txt")}
          basePath="/book-view"
          hasMore
          onLoadMore={onLoadMore}
        />,
      );
      expect(screen.getAllByRole("link")).toHaveLength(25);
      // 중복 호출은 loading 상태와 useCoverBrowse의 loadingRef가 막는다.
      expect(onLoadMore).toHaveBeenCalled();
    });
  });
});
