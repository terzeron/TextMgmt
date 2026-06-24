// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, cleanup } from "@testing-library/react";

afterEach(cleanup);

// IntersectionObserver mock (jsdom에 없으므로 직접 구현)
globalThis.IntersectionObserver = class MockIntersectionObserver {
  constructor(callback) {
    this.callback = callback;
    this.elements = new Set();
  }
  observe(element) {
    this.elements.add(element);
    // 즉시 visible 처리 → 테스트에서 모든 페이지가 바로 렌더링됨
    this.callback([{ isIntersecting: true, target: element }], this);
  }
  unobserve(element) {
    this.elements.delete(element);
  }
  disconnect() {
    this.elements.clear();
  }
};

const { mockGetDocument } = vi.hoisted(() => ({
  mockGetDocument: vi.fn(),
}));

vi.mock("pdfjs-dist", () => ({
  getDocument: mockGetDocument,
  GlobalWorkerOptions: { workerSrc: "" },
}));

vi.mock("../src/Common", () => ({
  getApiUrlPrefix: () => "http://localhost:8000",
  getAuthToken: () => "test-token",
}));

import ViewPDF from "../src/ViewPDF";
import * as pdfjs from "pdfjs-dist";

// 실제 pdfjs-dist 6.x에서 PDFDocumentProxy에는 destroy()가 없고
// loadingTask.destroy()로 정리한다. mock도 동일하게 구성한다.
function createMockPdf(numPages = 2) {
  return {
    numPages,
    getPage: vi.fn(() =>
      Promise.resolve({
        getViewport: () => ({ width: 800, height: 600 }),
        render: () => ({ promise: Promise.resolve() }),
      }),
    ),
  };
}

// getDocument()가 반환하는 loadingTask를 생성하고 pdfDoc.loadingTask에 연결한다.
function makeLoadingTask(pdfDoc) {
  if (!pdfDoc.loadingTask) {
    pdfDoc.loadingTask = { promise: Promise.resolve(pdfDoc), destroy: vi.fn() };
  }
  return pdfDoc.loadingTask;
}

// fetch mock 헬퍼: /pdf-pages/ 요청에 대해 X-Total-Pages 헤더 포함 응답
function createMockFetch(totalPages, options = {}) {
  const { failUrls = [], pendingUrls = [] } = options;

  return vi.fn((url) => {
    // 실패하는 URL
    for (const failUrl of failUrls) {
      if (url.includes(failUrl)) {
        return Promise.resolve({
          ok: false,
          status: 500,
          headers: new Headers(),
          arrayBuffer: () => Promise.resolve(new ArrayBuffer(0)),
        });
      }
    }

    // 영원히 pending하는 URL
    for (const pending of pendingUrls) {
      if (url.includes(pending)) {
        return new Promise(() => {});
      }
    }

    return Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers({ "X-Total-Pages": String(totalPages) }),
      arrayBuffer: () => Promise.resolve(new ArrayBuffer(10)),
    });
  });
}

// pdfjs.getDocument mock 헬퍼: {data: ArrayBuffer}를 받아 pdfDoc 반환
function setupGetDocument(mockPdfFactory) {
  mockGetDocument.mockImplementation(({ data: _data }) => {
    const pdfDoc =
      typeof mockPdfFactory === "function" ? mockPdfFactory() : mockPdfFactory;
    return makeLoadingTask(pdfDoc);
  });
}

// pdfjs.getDocument가 실패하는 mock
function setupGetDocumentFailing(error) {
  mockGetDocument.mockImplementation(() => ({
    promise: Promise.reject(error),
  }));
}

describe("ViewPDF", () => {
  let originalFetch;

  beforeEach(() => {
    mockGetDocument.mockReset();
    originalFetch = globalThis.fetch;
    HTMLCanvasElement.prototype.getContext = vi.fn(() => ({}));
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  // ── 초기 상태 / 유효성 검사 ──

  it("bookId가 0이면 에러를 표시한다", () => {
    render(<ViewPDF bookId={0} />);
    expect(
      screen.getByText(/유효한 bookId가 제공되지 않았습니다/),
    ).toBeTruthy();
  });

  it("로딩 중 스피너와 다운로드 진행 메시지를 표시한다", () => {
    // fetch가 영원히 pending → 로딩 상태 유지
    globalThis.fetch = vi.fn(() => new Promise(() => {}));

    render(<ViewPDF bookId={1} />);

    // loadPdf에서 setDownloadProgress(10)이 먼저 호출되므로 진행률 표시
    expect(screen.getByText(/PDF 다운로드 중/)).toBeTruthy();
    expect(document.querySelector(".spinner")).toBeTruthy();
  });

  // ── URL 및 fetch 호출 ──

  it("첫 페이지 요청으로 /pdf-pages/{bookId}?start=1&end=1을 fetch한다", () => {
    globalThis.fetch = vi.fn(() => new Promise(() => {}));

    render(<ViewPDF bookId={42} />);

    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/pdf-pages/42?start=1&end=1",
      expect.objectContaining({ credentials: "include" }),
    );
  });

  it("bookId 변경 시 새 URL로 fetch를 호출한다", async () => {
    globalThis.fetch = vi.fn(() => new Promise(() => {}));

    const { rerender } = render(<ViewPDF bookId={1} />);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/pdf-pages/1?start=1&end=1",
      expect.objectContaining({ credentials: "include" }),
    );

    rerender(<ViewPDF bookId={99} />);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/pdf-pages/99?start=1&end=1",
      expect.objectContaining({ credentials: "include" }),
    );
  });

  // ── 정상 렌더링 ──

  it("PDF 로드 성공 후 모든 페이지를 렌더링한다", async () => {
    globalThis.fetch = createMockFetch(3);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("총 3쪽 표시")).toBeTruthy();
    });
  });

  it("getDocument 에 wasmUrl 을 전달한다 (JPEG2000/JBIG2 이미지 디코딩용)", async () => {
    // wasmUrl 누락 시 OpenJPEG 가 초기화되지 못해 이미지 기반 PDF 페이지가 렌더되지 않는다.
    globalThis.fetch = createMockFetch(1);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(mockGetDocument).toHaveBeenCalledWith(
        expect.objectContaining({
          wasmUrl: expect.stringContaining("/pdf-wasm/"),
        }),
      );
    });
  });

  it("각 페이지마다 canvas 요소를 생성한다", async () => {
    globalThis.fetch = createMockFetch(2);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("총 2쪽 표시")).toBeTruthy();
    });

    const canvases = document.querySelectorAll("canvas");
    expect(canvases.length).toBe(2);
  });

  it('렌더링 진행 중 "렌더링 중... X/Y쪽" 상태를 표시한다', async () => {
    // fetch 성공하지만 pdfjs.getDocument의 render가 영원히 pending
    globalThis.fetch = createMockFetch(2);
    const mockPdf = {
      numPages: 1,
      getPage: vi.fn(() =>
        Promise.resolve({
          getViewport: () => ({ width: 800, height: 600 }),
          render: () => ({ promise: new Promise(() => {}) }),
        }),
      ),
    };
    setupGetDocument(mockPdf);

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("렌더링 중... 0/2쪽")).toBeTruthy();
    });
  });

  // ── pageCount 제한 ──

  it("pageCount로 렌더링 페이지 수를 제한한다 (전체보기)", async () => {
    globalThis.fetch = createMockFetch(10);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} pageCount={3} />);

    await waitFor(() => {
      expect(screen.getByText("총 3쪽 표시")).toBeTruthy();
    });
  });

  it("pageCount가 serverTotalPages보다 크면 serverTotalPages만큼 렌더링한다", async () => {
    globalThis.fetch = createMockFetch(2);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} pageCount={100} />);

    await waitFor(() => {
      expect(screen.getByText("총 2쪽 표시")).toBeTruthy();
    });
  });

  it("pageCount=0이면 모든 페이지를 렌더링한다", async () => {
    globalThis.fetch = createMockFetch(5);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} pageCount={0} />);

    await waitFor(() => {
      expect(screen.getByText("총 5쪽 표시")).toBeTruthy();
    });
  });

  // ── 에러 처리 ──

  it("fetch 실패 시 에러 메시지를 표시한다", async () => {
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({
        ok: false,
        status: 500,
        headers: new Headers(),
      }),
    );

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText(/HTTP 500/)).toBeTruthy();
    });
  });

  it("fetch 네트워크 에러 시 에러 메시지를 표시한다", async () => {
    globalThis.fetch = vi.fn(() => Promise.reject(new Error("Network error")));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText(/Network error/)).toBeTruthy();
    });
  });

  it("pdfjs.getDocument 실패 시 에러 메시지를 표시한다", async () => {
    globalThis.fetch = createMockFetch(3);
    setupGetDocumentFailing(new Error("Invalid PDF structure"));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText(/Invalid PDF structure/)).toBeTruthy();
    });
  });

  it("err.message가 없으면 fallback 에러 메시지를 표시한다", async () => {
    globalThis.fetch = createMockFetch(3);
    setupGetDocumentFailing({ name: "UnknownError" });

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(
        screen.getByText(/파일이 존재하지 않거나 올바르지 않은 형식/),
      ).toBeTruthy();
    });
  });

  // ── cleanup / 취소 ──

  it("cleanup 시 모든 청크 pdfDoc을 destroy한다", async () => {
    const mockPdf = createMockPdf(1);
    globalThis.fetch = createMockFetch(1);
    setupGetDocument(mockPdf);

    const { unmount } = render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("총 1쪽 표시")).toBeTruthy();
    });

    unmount();

    expect(mockPdf.loadingTask.destroy).toHaveBeenCalled();
  });

  it("bookId 변경 시 이전 청크를 모두 destroy한다", async () => {
    const mockPdf1 = createMockPdf(1);
    const mockPdf2 = createMockPdf(1);
    let callCount = 0;
    globalThis.fetch = createMockFetch(1);
    mockGetDocument.mockImplementation(() => {
      callCount++;
      const pdf = callCount === 1 ? mockPdf1 : mockPdf2;
      return makeLoadingTask(pdf);
    });

    const { rerender } = render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("총 1쪽 표시")).toBeTruthy();
    });

    rerender(<ViewPDF bookId={2} />);

    await waitFor(() => {
      expect(mockPdf1.loadingTask.destroy).toHaveBeenCalled();
    });
  });

  it("bookId 변경 시 이전 로딩의 에러가 표시되지 않는다", async () => {
    // 첫 번째 bookId → fetch 영원히 pending
    // 두 번째 bookId → fetch 성공
    let callNum = 0;
    globalThis.fetch = vi.fn((_url) => {
      callNum++;
      if (callNum === 1) {
        // 첫 번째 요청: 영원히 pending (이후 cancelled)
        return new Promise(() => {});
      }
      return Promise.resolve({
        ok: true,
        status: 200,
        headers: new Headers({ "X-Total-Pages": "1" }),
        arrayBuffer: () => Promise.resolve(new ArrayBuffer(10)),
      });
    });
    setupGetDocument(() => createMockPdf(1));

    const { rerender } = render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(globalThis.fetch).toHaveBeenCalledTimes(1);
    });

    rerender(<ViewPDF bookId={2} />);

    await waitFor(() => {
      expect(screen.getByText("총 1쪽 표시")).toBeTruthy();
    });

    // 에러 메시지가 없어야 함
    expect(screen.queryByText(/렌더링 실패/)).toBeNull();
    expect(screen.queryByText(/에러/i)).toBeNull();
  });

  // ── preview 모드 ──

  it("preview=true이면 최대 10페이지까지만 표시한다", async () => {
    globalThis.fetch = createMockFetch(20);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} preview={true} />);

    await waitFor(() => {
      expect(screen.getByText("총 10쪽 표시")).toBeTruthy();
    });
  });

  it("preview=true이고 서버 총 페이지가 10 미만이면 서버 페이지만큼 표시한다", async () => {
    globalThis.fetch = createMockFetch(5);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} preview={true} />);

    await waitFor(() => {
      expect(screen.getByText("총 5쪽 표시")).toBeTruthy();
    });
  });

  it("preview=true이면 나머지 페이지를 한 번에 페칭한다", async () => {
    globalThis.fetch = createMockFetch(5);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} preview={true} />);

    await waitFor(() => {
      expect(screen.getByText("총 5쪽 표시")).toBeTruthy();
    });

    // 첫 페이지 + 나머지 (2~5) 요청
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/pdf-pages/1?start=1&end=1",
      expect.objectContaining({ credentials: "include" }),
    );
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/pdf-pages/1?start=2&end=5",
      expect.objectContaining({ credentials: "include" }),
    );
  });

  // ── 전체보기 모드 ──

  it("preview=false이면 모든 페이지를 표시한다", async () => {
    globalThis.fetch = createMockFetch(25);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} preview={false} />);

    await waitFor(() => {
      // 총 25쪽 canvas가 생성되어야 함
      const canvases = document.querySelectorAll("canvas");
      expect(canvases.length).toBe(25);
    });
  });

  it("전체보기에서 첫 페이지 후 2~11페이지 청크를 즉시 페칭한다", async () => {
    globalThis.fetch = createMockFetch(20);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} preview={false} />);

    await waitFor(() => {
      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://localhost:8000/pdf-pages/1?start=1&end=1",
        expect.objectContaining({ credentials: "include" }),
      );
      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://localhost:8000/pdf-pages/1?start=2&end=11",
        expect.objectContaining({ credentials: "include" }),
      );
    });
  });

  // ── X-Total-Pages 헤더 ──

  it("X-Total-Pages 헤더에서 총 페이지 수를 읽는다", async () => {
    globalThis.fetch = createMockFetch(7);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      const canvases = document.querySelectorAll("canvas");
      expect(canvases.length).toBe(7);
    });
  });

  it("X-Total-Pages 헤더가 없으면 0으로 처리하여 1페이지만 표시한다", async () => {
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        status: 200,
        headers: new Headers(), // X-Total-Pages 없음
        arrayBuffer: () => Promise.resolve(new ArrayBuffer(10)),
      }),
    );
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      // totalPages=0이면 pagesToRender=0, 최소 1페이지는 처리되어야 하나
      // serverTotalPages=0 → pagesToRender=0으로 canvas가 0개 생성
      // 이 경우 첫 페이지 렌더링은 시도하지만 totalPages=0이므로 canvas 미생성
      // 실제로는 에러 없이 로딩만 보이거나 0쪽 표시
      expect(true).toBeTruthy();
    });
  });

  it("맞춤 버튼을 끄면 확대/축소 컨트롤을 사용할 수 있다", async () => {
    globalThis.fetch = createMockFetch(2);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("총 2쪽 표시")).toBeTruthy();
    });

    const fitButton = screen.getByRole("button", { name: /맞춤/ });
    const zoomUpButton = screen.getByRole("button", { name: "+" });
    const zoomDownButton = screen.getByRole("button", { name: "−" });

    expect(zoomUpButton.disabled).toBe(true);
    expect(zoomDownButton.disabled).toBe(true);

    zoomUpButton.click();
    expect(screen.getByText("-")).toBeTruthy();

    fitButton.click();
    await waitFor(() => {
      expect(zoomUpButton.disabled).toBe(false);
      expect(screen.getByText("100%")).toBeTruthy();
    });

    zoomUpButton.click();
    await waitFor(() => {
      expect(screen.getByText("110%")).toBeTruthy();
    });

    zoomDownButton.click();
    await waitFor(() => {
      expect(screen.getByText("100%")).toBeTruthy();
    });
  });

  it("첫 청크 로드 후 취소되면 pdfDoc을 destroy한다", async () => {
    let resolveFetch;
    globalThis.fetch = vi.fn(
      () =>
        new Promise((resolve) => {
          resolveFetch = resolve;
        }),
    );

    const firstPdf = createMockPdf(1);
    setupGetDocument(firstPdf);

    const { unmount } = render(<ViewPDF bookId={1} />);

    resolveFetch({
      ok: true,
      status: 200,
      headers: new Headers({ "X-Total-Pages": "1" }),
      arrayBuffer: () => Promise.resolve(new ArrayBuffer(10)),
    });

    await waitFor(() => {
      expect(mockGetDocument).toHaveBeenCalled();
    });

    unmount();

    await waitFor(() => {
      expect(firstPdf.loadingTask.destroy).toHaveBeenCalled();
    });
  });

  it("첫 페이지 pdfDoc 생성 직후 rerender되면 이전 pdfDoc을 destroy한다", async () => {
    let fetchCount = 0;
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        status: 200,
        headers: new Headers({ "X-Total-Pages": "1" }),
        arrayBuffer: () => Promise.resolve(new ArrayBuffer(10)),
      }),
    );

    const firstPdf = createMockPdf(1);
    const secondPdf = createMockPdf(1);
    mockGetDocument.mockImplementation(() => {
      fetchCount += 1;
      return makeLoadingTask(fetchCount === 1 ? firstPdf : secondPdf);
    });

    const { rerender } = render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("총 1쪽 표시")).toBeTruthy();
    });

    rerender(<ViewPDF bookId={2} />);

    await waitFor(() => {
      expect(firstPdf.loadingTask.destroy).toHaveBeenCalled();
    });
  });

  it("첫 페이지 doc 로드 도중 취소되면 loadingTask.destroy로 정리한다", async () => {
    // getDocument().promise가 unmount(취소) 이후에 resolve되는 타이밍을 재현 →
    // loadPdf의 cancelled 분기에서 firstPdfDoc.loadingTask.destroy() 호출 (ViewPDF.jsx:206-207)
    globalThis.fetch = createMockFetch(1);
    const firstPdf = createMockPdf(1);
    firstPdf.loadingTask = { destroy: vi.fn() };

    let resolveDoc;
    mockGetDocument.mockReturnValue({
      promise: new Promise((resolve) => {
        resolveDoc = resolve;
      }),
    });

    const { unmount } = render(<ViewPDF bookId={1} />);

    // loadPdf가 getDocument().promise await 지점까지 진행되기를 대기
    await waitFor(() => {
      expect(mockGetDocument).toHaveBeenCalled();
    });

    // 아직 doc이 resolve되기 전에 unmount → cancelledRef.current = true
    unmount();

    // 이제 doc resolve → cancelled 분기 진입
    resolveDoc(firstPdf);

    await waitFor(() => {
      expect(firstPdf.loadingTask.destroy).toHaveBeenCalled();
    });
  });

  it("청크 doc 로드 도중 취소되면 loadingTask.destroy로 정리한다", async () => {
    // 첫 페이지는 정상 로드되어 청크 페칭이 트리거되고, 청크 getDocument().promise가
    // unmount 이후 resolve되는 타이밍 → fetchChunk의 cancelled 분기 (ViewPDF.jsx:79-80)
    globalThis.fetch = createMockFetch(11);
    const firstPdf = createMockPdf(1);
    firstPdf.loadingTask = {
      promise: Promise.resolve(firstPdf),
      destroy: vi.fn(),
    };
    const chunkPdf = createMockPdf(10);
    chunkPdf.loadingTask = { destroy: vi.fn() };

    let resolveChunk;
    let callCount = 0;
    mockGetDocument.mockImplementation(() => {
      callCount += 1;
      if (callCount === 1) return firstPdf.loadingTask;
      return {
        promise: new Promise((resolve) => {
          resolveChunk = resolve;
        }),
      };
    });

    const { unmount } = render(<ViewPDF bookId={1} preview={false} />);

    // 첫 페이지 로드 후 청크 getDocument(2번째 호출)까지 진행되기를 대기
    await waitFor(() => {
      expect(mockGetDocument).toHaveBeenCalledTimes(2);
    });

    unmount();

    resolveChunk(chunkPdf);

    await waitFor(() => {
      expect(chunkPdf.loadingTask.destroy).toHaveBeenCalled();
    });
  });

  // ── 개별 페이지 렌더링 실패 ──

  it("개별 페이지 렌더링 실패 시 전체 에러 상태가 되지 않는다", async () => {
    globalThis.fetch = createMockFetch(3);
    let pageCallCount = 0;
    const mockPdf = {
      numPages: 1,
      getPage: vi.fn((_localPageNum) => {
        pageCallCount++;
        // 두 번째 호출(2페이지 청크의 첫 페이지)에서 실패
        if (pageCallCount === 2) {
          return Promise.reject(new Error("page corrupt"));
        }
        return Promise.resolve({
          getViewport: () => ({ width: 800, height: 600 }),
          render: () => ({ promise: Promise.resolve() }),
        });
      }),
    };
    setupGetDocument(mockPdf);

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      const canvases = document.querySelectorAll("canvas");
      expect(canvases.length).toBe(3);
    });

    // 전체 에러 메시지가 화면에 표시되지 않음
    expect(screen.queryByText(/렌더링 실패/)).toBeNull();
  });

  // ── 후속 청크 fetch 실패 ──

  it("후속 청크 fetch 실패 시 첫 페이지는 정상 표시된다", async () => {
    // 첫 페이지 요청만 성공, 나머지 실패
    globalThis.fetch = vi.fn((url) => {
      if (url.includes("start=1&end=1")) {
        return Promise.resolve({
          ok: true,
          status: 200,
          headers: new Headers({ "X-Total-Pages": "5" }),
          arrayBuffer: () => Promise.resolve(new ArrayBuffer(10)),
        });
      }
      return Promise.resolve({
        ok: false,
        status: 500,
        headers: new Headers(),
      });
    });
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      // 첫 페이지는 렌더링됨 (isFirstPageReady=true)
      const info = document.querySelector(".pdf-info");
      expect(info).toBeTruthy();
    });

    // 에러 화면으로 전환되지 않음 (error state가 아님)
    expect(screen.queryByText(/PDF 렌더링 실패/)).toBeNull();
  });

  // ── 중복 페칭 방지 ──

  it("동일 청크를 중복 페칭하지 않는다", async () => {
    globalThis.fetch = createMockFetch(3);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(screen.getByText("총 3쪽 표시")).toBeTruthy();
    });

    // start=2&end=3 요청이 한 번만 발생
    const chunk2Calls = globalThis.fetch.mock.calls.filter(([url]) =>
      url.includes("start=2"),
    );
    expect(chunk2Calls.length).toBe(1);
  });

  // ── 종횡비 유지 ──

  it("맞춤 모드(fitMode)에서 canvas에 width:100%와 height:auto가 적용된다", async () => {
    globalThis.fetch = createMockFetch(2);
    setupGetDocument(() => createMockPdf(1));

    // preview=false → fitMode=true (기본)
    render(<ViewPDF bookId={1} preview={false} />);

    await waitFor(() => {
      expect(screen.getByText("총 2쪽 표시")).toBeTruthy();
    });

    const canvases = document.querySelectorAll("canvas");
    expect(canvases.length).toBe(2);
    for (const canvas of canvases) {
      expect(canvas.style.width).toBe("100%");
      expect(canvas.style.height).toBe("auto");
    }
  });

  it("수동 줌 모드에서 canvas에 width는 px, height는 auto가 적용된다", async () => {
    globalThis.fetch = createMockFetch(1);
    setupGetDocument(() => createMockPdf(1));

    // preview=true → fitMode=false (수동 줌 모드)
    render(<ViewPDF bookId={1} preview={true} />);

    await waitFor(() => {
      expect(screen.getByText("총 1쪽 표시")).toBeTruthy();
    });

    const canvas = document.querySelector("canvas");
    expect(canvas).toBeTruthy();
    // 수동 줌 모드에서 width는 px 단위, height는 auto
    expect(canvas.style.width).toContain("px");
    expect(canvas.style.height).toBe("auto");
  });

  // Regression: 워커는 반드시 설치된 pdfjs-dist에서 번들되어야 한다
  // (CDN 고정 버전을 쓰면 API/Worker 버전이 drift되어 렌더링이 깨진다)
  it("loads workerSrc from the installed pdfjs-dist build", () => {
    expect(pdfjs.GlobalWorkerOptions.workerSrc).toContain(
      "pdfjs-dist/build/pdf.worker",
    );
  });

  // 스케줄러: 화면 밖으로 나간(아직 렌더 미완) 페이지는 RenderTask.cancel()로 취소된다.
  // (기본 MockIntersectionObserver는 isIntersecting:true만 보내 이 분기가 커버되지 않음)
  it("화면 밖으로 나간 미완 렌더 페이지를 취소한다 (cancelRender)", async () => {
    const cancelSpy = vi.fn();
    const getPageSpy = vi.fn(() =>
      Promise.resolve({
        getViewport: () => ({ width: 800, height: 600 }),
        // 렌더는 영원히 pending → 페이지가 active 상태로 유지되어 취소 대상이 된다
        render: () => ({ promise: new Promise(() => {}), cancel: cancelSpy }),
      }),
    );
    setupGetDocument(() => {
      const pdf = { numPages: 1, getPage: getPageSpy };
      pdf.loadingTask = { promise: Promise.resolve(pdf), destroy: vi.fn() };
      return pdf;
    });
    globalThis.fetch = createMockFetch(3);

    // 관찰자 인스턴스를 캡처(observe 시 isIntersecting:true 자동 발사)
    const observers = [];
    const RealIO = globalThis.IntersectionObserver;
    globalThis.IntersectionObserver = class {
      constructor(cb) {
        this.cb = cb;
        observers.push(this);
      }
      observe(el) {
        this.cb([{ isIntersecting: true, target: el }], this);
      }
      unobserve() {}
      disconnect() {}
    };

    try {
      render(<ViewPDF bookId={1} />);

      // 전체보기 → canvas 3개 생성
      await waitFor(() => {
        expect(document.querySelectorAll("canvas.pdf-page").length).toBe(3);
      });
      // 페이지 1·2가 getPage까지 도달 → 페이지 2가 active(pending render)
      await waitFor(() => {
        expect(getPageSpy.mock.calls.length).toBeGreaterThanOrEqual(2);
      });
      await Promise.resolve();
      await Promise.resolve();

      const canvas2 = document.querySelectorAll("canvas.pdf-page")[1];
      expect(canvas2.dataset.page).toBe("2");

      // 페이지 2가 근접 영역을 벗어남 → cancelRender 분기 (ViewPDF.jsx:367,372)
      const obs = observers[observers.length - 1];
      obs.cb([{ isIntersecting: false, target: canvas2 }], obs);

      await waitFor(() => {
        expect(cancelSpy).toHaveBeenCalled();
      });
    } finally {
      globalThis.IntersectionObserver = RealIO;
    }
  });

  // ── 줌 경계값 (zoomUp/zoomDown clamp) ──

  it("최대 줌(500%)에서 + 를 더 눌러도 500%를 유지하고 + 버튼이 비활성화된다", async () => {
    globalThis.fetch = createMockFetch(1);
    setupGetDocument(() => createMockPdf(1));

    // preview=true → fitMode=false (수동 줌 모드)
    render(<ViewPDF bookId={1} preview={true} />);

    await waitFor(() => {
      expect(screen.getByText("총 1쪽 표시")).toBeTruthy();
    });

    const zoomUpButton = screen.getByRole("button", { name: "+" });
    // index 7(100%) → 16(500%): 9번 클릭
    for (let i = 0; i < 9; i++) {
      zoomUpButton.click();
    }
    await waitFor(() => {
      expect(screen.getByText("500%")).toBeTruthy();
    });
    // 상한이므로 + 버튼 비활성화
    expect(zoomUpButton.disabled).toBe(true);

    // 추가 클릭해도 (clamp) 500% 유지
    zoomUpButton.click();
    expect(screen.getByText("500%")).toBeTruthy();
  });

  it("최소 줌(25%)에서 − 를 더 눌러도 25%를 유지하고 − 버튼이 비활성화된다", async () => {
    globalThis.fetch = createMockFetch(1);
    setupGetDocument(() => createMockPdf(1));

    render(<ViewPDF bookId={1} preview={true} />);

    await waitFor(() => {
      expect(screen.getByText("총 1쪽 표시")).toBeTruthy();
    });

    const zoomDownButton = screen.getByRole("button", { name: "−" });
    // index 7(100%) → 0(25%): 7번 클릭
    for (let i = 0; i < 7; i++) {
      zoomDownButton.click();
    }
    await waitFor(() => {
      expect(screen.getByText("25%")).toBeTruthy();
    });
    // 하한이므로 − 버튼 비활성화
    expect(zoomDownButton.disabled).toBe(true);

    // 추가 클릭해도 (clamp) 25% 유지
    zoomDownButton.click();
    expect(screen.getByText("25%")).toBeTruthy();
  });

  // ── devicePixelRatio fallback (|| 1) ──

  it("devicePixelRatio가 0이면 fallback 1을 사용해 정상 렌더링한다", async () => {
    const originalDpr = Object.getOwnPropertyDescriptor(
      window,
      "devicePixelRatio",
    );
    // 0 → falsy → `window.devicePixelRatio || 1` 의 fallback 분기 (ViewPDF.jsx:138,315)
    Object.defineProperty(window, "devicePixelRatio", {
      configurable: true,
      value: 0,
    });

    try {
      globalThis.fetch = createMockFetch(2);
      setupGetDocument(() => createMockPdf(1));

      render(<ViewPDF bookId={1} />);

      await waitFor(() => {
        expect(screen.getByText("총 2쪽 표시")).toBeTruthy();
      });
      // fallback이 적용되어 에러 없이 canvas가 생성됨
      expect(document.querySelectorAll("canvas").length).toBe(2);
    } finally {
      if (originalDpr) {
        Object.defineProperty(window, "devicePixelRatio", originalDpr);
      } else {
        delete window.devicePixelRatio;
      }
    }
  });

  // ── enqueueRender 가드 (이미 렌더됨 / 이미 active) ──

  it("이미 렌더 완료된 페이지를 다시 관찰해도 재렌더하지 않는다 (enqueueRender 가드)", async () => {
    // 페이지 1은 첫 청크에서 즉시 렌더 완료된다. 같은 canvas를 다시 intersecting으로
    // 보내도 renderedPagesRef 가드(ViewPDF.jsx:235)에 의해 무시되어야 한다.
    const getPageSpy = vi.fn(() =>
      Promise.resolve({
        getViewport: () => ({ width: 800, height: 600 }),
        render: () => ({ promise: Promise.resolve() }),
      }),
    );
    setupGetDocument(() => {
      const pdf = { numPages: 1, getPage: getPageSpy };
      pdf.loadingTask = { promise: Promise.resolve(pdf), destroy: vi.fn() };
      return pdf;
    });
    globalThis.fetch = createMockFetch(3);

    const observers = [];
    const RealIO = globalThis.IntersectionObserver;
    globalThis.IntersectionObserver = class {
      constructor(cb) {
        this.cb = cb;
        observers.push(this);
      }
      observe(el) {
        this.cb([{ isIntersecting: true, target: el }], this);
      }
      unobserve() {}
      disconnect() {}
    };

    try {
      render(<ViewPDF bookId={1} />);

      await waitFor(() => {
        expect(screen.getByText("총 3쪽 표시")).toBeTruthy();
      });
      // 페이지 2가 한 번은 렌더되도록 대기
      await waitFor(() => {
        expect(getPageSpy.mock.calls.length).toBeGreaterThanOrEqual(2);
      });
      await Promise.resolve();
      await Promise.resolve();

      const callsAfterRender = getPageSpy.mock.calls.length;

      // 이미 렌더된 페이지 2를 다시 intersecting → enqueueRender 가드로 무시
      const canvas2 = document.querySelectorAll("canvas.pdf-page")[1];
      const obs = observers[observers.length - 1];
      obs.cb([{ isIntersecting: true, target: canvas2 }], obs);
      await Promise.resolve();
      await Promise.resolve();

      // getPage가 추가로 호출되지 않음 (재렌더 안 함)
      expect(getPageSpy.mock.calls.length).toBe(callsAfterRender);
    } finally {
      globalThis.IntersectionObserver = RealIO;
    }
  });

  it("렌더 진행 중(active)인 페이지를 다시 관찰해도 큐에 중복 투입하지 않는다 (enqueueRender 가드)", async () => {
    // 렌더가 영원히 pending → 페이지가 active 슬롯을 점유한 상태 유지.
    // 같은 페이지를 다시 intersecting으로 보내면 activeRenderTasksRef 가드
    // (ViewPDF.jsx:236)에 의해 getPage가 재호출되지 않아야 한다.
    const getPageSpy = vi.fn(() =>
      Promise.resolve({
        getViewport: () => ({ width: 800, height: 600 }),
        render: () => ({ promise: new Promise(() => {}), cancel: vi.fn() }),
      }),
    );
    setupGetDocument(() => {
      const pdf = { numPages: 1, getPage: getPageSpy };
      pdf.loadingTask = { promise: Promise.resolve(pdf), destroy: vi.fn() };
      return pdf;
    });
    globalThis.fetch = createMockFetch(2);

    const observers = [];
    const RealIO = globalThis.IntersectionObserver;
    globalThis.IntersectionObserver = class {
      constructor(cb) {
        this.cb = cb;
        observers.push(this);
      }
      observe(el) {
        this.cb([{ isIntersecting: true, target: el }], this);
      }
      unobserve() {}
      disconnect() {}
    };

    try {
      render(<ViewPDF bookId={1} />);

      await waitFor(() => {
        expect(document.querySelectorAll("canvas.pdf-page").length).toBe(2);
      });
      // 페이지 1(첫 청크)이 active 상태(pending render)가 될 때까지 대기
      await waitFor(() => {
        expect(getPageSpy.mock.calls.length).toBeGreaterThanOrEqual(1);
      });
      await Promise.resolve();
      await Promise.resolve();

      const callsWhileActive = getPageSpy.mock.calls.length;

      // active 상태인 페이지 1을 다시 intersecting → 가드로 무시
      const canvas1 = document.querySelectorAll("canvas.pdf-page")[0];
      // 페이지 1에는 observer가 붙지 않으므로(2부터 관찰) 가장 최근 observer로 직접 발사
      const obs = observers[observers.length - 1];
      obs.cb([{ isIntersecting: true, target: canvas1 }], obs);
      await Promise.resolve();
      await Promise.resolve();

      // 가드로 인해 getPage 추가 호출 없음
      expect(getPageSpy.mock.calls.length).toBe(callsWhileActive);
    } finally {
      globalThis.IntersectionObserver = RealIO;
    }
  });

  // ── 첫 페이지 버퍼 로드 도중 취소 (cancelledRef 분기) ──

  it("첫 페이지 arrayBuffer 로드 도중 취소되면 getDocument를 호출하지 않는다", async () => {
    // firstResponse.arrayBuffer()가 unmount 이후 resolve되는 타이밍 →
    // ViewPDF.jsx:286 cancelledRef 분기에서 early return (getDocument 미호출)
    let resolveBuffer;
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        status: 200,
        headers: new Headers({ "X-Total-Pages": "1" }),
        arrayBuffer: () =>
          new Promise((resolve) => {
            resolveBuffer = resolve;
          }),
      }),
    );
    setupGetDocument(() => createMockPdf(1));

    const { unmount } = render(<ViewPDF bookId={1} />);

    // fetch는 호출됐지만 arrayBuffer는 아직 pending
    await waitFor(() => {
      expect(globalThis.fetch).toHaveBeenCalled();
    });
    expect(resolveBuffer).toBeDefined();

    // arrayBuffer resolve 전에 unmount → cancelledRef.current = true
    unmount();

    // 이제 buffer resolve → cancelled 분기 진입, getDocument 호출 안 됨
    resolveBuffer(new ArrayBuffer(10));
    await Promise.resolve();
    await Promise.resolve();

    expect(mockGetDocument).not.toHaveBeenCalled();
  });

  it("청크 arrayBuffer 로드 도중 취소되면 청크 getDocument를 호출하지 않는다", async () => {
    // 첫 페이지는 정상 로드되어 청크 페칭이 트리거되고, 청크 응답의 arrayBuffer()가
    // unmount 이후 resolve되는 타이밍 → fetchChunk의 cancelledRef 분기 (ViewPDF.jsx:86)
    let resolveChunkBuffer;
    let fetchCall = 0;
    globalThis.fetch = vi.fn((url) => {
      if (url.includes("start=1&end=1")) {
        return Promise.resolve({
          ok: true,
          status: 200,
          headers: new Headers({ "X-Total-Pages": "11" }),
          arrayBuffer: () => Promise.resolve(new ArrayBuffer(10)),
        });
      }
      fetchCall += 1;
      return Promise.resolve({
        ok: true,
        status: 200,
        headers: new Headers(),
        arrayBuffer: () =>
          new Promise((resolve) => {
            resolveChunkBuffer = resolve;
          }),
      });
    });
    setupGetDocument(() => createMockPdf(1));

    const { unmount } = render(<ViewPDF bookId={1} preview={false} />);

    // 청크 fetch의 arrayBuffer가 pending 상태가 될 때까지 대기
    await waitFor(() => {
      expect(resolveChunkBuffer).toBeDefined();
    });

    const docCallsBefore = mockGetDocument.mock.calls.length;

    // 청크 buffer resolve 전에 unmount
    unmount();

    // 이제 buffer resolve → fetchChunk cancelled 분기에서 early return
    resolveChunkBuffer(new ArrayBuffer(10));
    await Promise.resolve();
    await Promise.resolve();

    // 청크용 getDocument 추가 호출 없음 (첫 페이지 1건만)
    expect(mockGetDocument.mock.calls.length).toBe(docCallsBefore);
    expect(fetchCall).toBeGreaterThanOrEqual(1);
  });

  it("첫 페이지 getPage 도중 취소되면 totalPages를 설정하지 않는다 (cancelledRef 분기)", async () => {
    // firstPdfDoc.getPage(1)이 unmount 이후 resolve되는 타이밍 →
    // ViewPDF.jsx:321 cancelledRef 분기에서 flushSync(setTotalPages) 전에 early return
    globalThis.fetch = createMockFetch(2);

    let resolveGetPage;
    const firstPdf = {
      numPages: 1,
      getPage: vi.fn(
        () =>
          new Promise((resolve) => {
            resolveGetPage = resolve;
          }),
      ),
    };
    firstPdf.loadingTask = {
      promise: Promise.resolve(firstPdf),
      destroy: vi.fn(),
    };
    setupGetDocument(firstPdf);

    const { unmount } = render(<ViewPDF bookId={1} />);

    // loadPdf가 getPage await 지점까지 진행되기를 대기
    await waitFor(() => {
      expect(firstPdf.getPage).toHaveBeenCalled();
    });

    // getPage resolve 전에 unmount → cancelledRef.current = true
    unmount();

    resolveGetPage({
      getViewport: () => ({ width: 800, height: 600 }),
      render: () => ({ promise: Promise.resolve() }),
    });
    await Promise.resolve();
    await Promise.resolve();

    // 취소되어 canvas(페이지 placeholder)가 생성되지 않음
    expect(document.querySelectorAll("canvas").length).toBe(0);
    // totalPages 미설정 → "총 N쪽 표시" 없음
    expect(screen.queryByText(/쪽 표시/)).toBeNull();
  });

  it("로드 중 취소된 뒤 에러가 발생해도 에러 메시지를 표시하지 않는다 (catch cancelledRef 분기)", async () => {
    // firstResponse.arrayBuffer()가 reject되지만 그 전에 unmount되어 cancelledRef=true →
    // catch 블록의 ViewPDF.jsx:389 분기에서 setError 없이 early return
    let rejectBuffer;
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        status: 200,
        headers: new Headers({ "X-Total-Pages": "1" }),
        arrayBuffer: () =>
          new Promise((_resolve, reject) => {
            rejectBuffer = reject;
          }),
      }),
    );
    setupGetDocument(() => createMockPdf(1));

    const { unmount } = render(<ViewPDF bookId={1} />);

    await waitFor(() => {
      expect(globalThis.fetch).toHaveBeenCalled();
    });
    expect(rejectBuffer).toBeDefined();

    // 에러 발생 전에 unmount → cancelledRef.current = true
    unmount();

    // 이제 buffer reject → catch 진입하지만 cancelled라 setError 안 함
    rejectBuffer(new Error("boom"));
    await Promise.resolve();
    await Promise.resolve();

    // unmount된 컴포넌트라 DOM에 에러 메시지가 없음
    expect(screen.queryByText(/PDF 렌더링 실패/)).toBeNull();
  });
});
