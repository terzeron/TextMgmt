// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, cleanup } from "@testing-library/react";

afterEach(cleanup);

const { mockRawJsonGetReq, mockGetDocument } = vi.hoisted(() => ({
  mockRawJsonGetReq: vi.fn(),
  mockGetDocument: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  getApiUrlPrefix: () => "http://localhost:8000",
  rawJsonGetReq: mockRawJsonGetReq,
}));

vi.mock("pdfjs-dist/legacy/build/pdf.mjs", () => ({
  getDocument: mockGetDocument,
  GlobalWorkerOptions: { workerSrc: "" },
}));

vi.mock("../src/DirectoryPagePreview.css", () => ({}));

import DirectoryPagePreview from "../src/DirectoryPagePreview";
import {
  PREVIEW_HEIGHT,
  PREVIEW_PAGE_COUNT,
  previewScale,
} from "../src/directoryPreviewLayout";

const PDF_FILE = {
  book_id: 7,
  title: "첫 권",
  file_path: "5_완결/[작가] 제목/제목 01권.pdf",
  file_type: "pdf",
};

// 쪽마다 { width, height }(pt)를 가진 PDF 문서 mock
function createDoc(sizes) {
  const rendered = [];
  const getViewport = vi.fn((size) => ({ scale }) => ({
    width: size.width * scale,
    height: size.height * scale,
  }));
  const doc = {
    numPages: sizes.length,
    rendered,
    getPage: vi.fn((number) => {
      const size = sizes[number - 1];
      return Promise.resolve({
        getViewport: getViewport(size),
        render: vi.fn(({ viewport }) => {
          rendered.push({ number, viewport });
          return { promise: Promise.resolve() };
        }),
      });
    }),
    loadingTask: { destroy: vi.fn() },
  };
  return doc;
}

function respondWithFirstFile(file) {
  mockRawJsonGetReq.mockImplementation((url, resolve) => {
    resolve({ status: "success", result: file ? [file] : [], total: file ? 5 : 0 });
  });
}

function mockPdfFetch(total = "30") {
  return vi.fn(() =>
    Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers({ "X-Total-Pages": total }),
      arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)),
    }),
  );
}

describe("previewScale", () => {
  it("높이가 PREVIEW_HEIGHT 가 되는 배율을 준다", () => {
    expect(previewScale(612, 792, 1)).toBeCloseTo(PREVIEW_HEIGHT / 792);
    // 612x792 쪽도 dpr 2 에서 높이 PREVIEW_HEIGHT CSS px 의 2배 픽셀이 된다(상한 안쪽)
    expect(previewScale(612, 792, 2)).toBeCloseTo((PREVIEW_HEIGHT * 2) / 792);
  });

  it("면적 상한을 넘으면 배율을 낮춘다", () => {
    // 가로 펼침면(1224x792)을 dpr 3 으로 그리면 상한(1.5M 픽셀)을 넘으므로 배율이 byHeight 보다 낮아진다
    const scale = previewScale(1224, 792, 3);
    expect(scale).toBeLessThan((PREVIEW_HEIGHT * 3) / 792);
    expect(1224 * scale * (792 * scale)).toBeLessThanOrEqual(1_500_000 + 1);
  });

  it("쪽 크기가 비정상적으로 작아도(0.09pt) 고정 높이의 canvas 가 된다", () => {
    const scale = previewScale(0.0934, 0.0675, 1);
    expect(0.0675 * scale).toBeCloseTo(PREVIEW_HEIGHT, 0);
  });
});

describe("DirectoryPagePreview", () => {
  let originalFetch;

  beforeEach(() => {
    mockRawJsonGetReq.mockReset();
    mockGetDocument.mockReset();
    originalFetch = globalThis.fetch;
    HTMLCanvasElement.prototype.getContext = vi.fn(() => ({}));
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it("카테고리의 첫 파일을 limit=1 로 조회한다(경로는 구간별로 인코딩)", () => {
    respondWithFirstFile(null);

    render(<DirectoryPagePreview category="5_완결/[작가] 제목 (01-04권 완)" apiPrefix="/comics" />);

    expect(mockRawJsonGetReq.mock.calls[0][0]).toBe(
      "/comics/categories/5_%EC%99%84%EA%B2%B0/%5B%EC%9E%91%EA%B0%80%5D%20%EC%A0%9C%EB%AA%A9%20(01-04%EA%B6%8C%20%EC%99%84)?limit=1",
    );
  });

  it("첫 파일의 앞 10쪽을 요청하고 쪽마다 canvas 를 고정 높이로 놓는다", async () => {
    respondWithFirstFile(PDF_FILE);
    const fetchMock = mockPdfFetch("30");
    globalThis.fetch = fetchMock;
    const doc = createDoc(Array.from({ length: 10 }, () => ({ width: 612, height: 792 })));
    mockGetDocument.mockReturnValue({ promise: Promise.resolve(doc) });

    render(<DirectoryPagePreview category="5_완결/작품" apiPrefix="/comics" />);

    await waitFor(() => expect(screen.getAllByLabelText(/^\d+쪽$/)).toHaveLength(10));
    expect(fetchMock).toHaveBeenCalledWith(
      `http://localhost:8000/comics/pdf-pages/7?start=1&end=${PREVIEW_PAGE_COUNT}`,
      { credentials: "include" },
    );
    expect(mockGetDocument.mock.calls[0][0]).toEqual(expect.objectContaining({
      cMapUrl: expect.stringContaining("/pdf-cmaps/"),
      cMapPacked: true,
      standardFontDataUrl: expect.stringContaining("/pdf-standard-fonts/"),
      wasmUrl: expect.stringContaining("/pdf-wasm/"),
    }));
    for (const canvas of screen.getAllByLabelText(/^\d+쪽$/)) {
      expect(canvas.style.height).toBe(`${PREVIEW_HEIGHT}px`);
    }
    expect(screen.getByText(/제목 01권\.pdf · 1~10쪽 \/ 총 30쪽/)).toBeTruthy();
  });

  it("쪽이 10장보다 적으면 있는 만큼만 보여 준다", async () => {
    respondWithFirstFile(PDF_FILE);
    globalThis.fetch = mockPdfFetch("3");
    mockGetDocument.mockReturnValue({
      promise: Promise.resolve(createDoc(Array.from({ length: 3 }, () => ({ width: 612, height: 792 })))),
    });

    render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);

    await waitFor(() => expect(screen.getAllByLabelText(/^\d+쪽$/)).toHaveLength(3));
    expect(screen.getByText(/1~3쪽 \/ 총 3쪽/)).toBeTruthy();
  });

  it("비율이 다른 쪽도 높이 기준 배율로 그려 가로세로 비율이 유지된다", async () => {
    respondWithFirstFile(PDF_FILE);
    globalThis.fetch = mockPdfFetch("2");
    const doc = createDoc([{ width: 612, height: 792 }, { width: 1224, height: 792 }]);
    mockGetDocument.mockReturnValue({ promise: Promise.resolve(doc) });

    render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);

    await waitFor(() => expect(doc.rendered).toHaveLength(2));
    const [portrait, spread] = screen.getAllByLabelText(/^\d+쪽$/);
    expect(portrait.height).toBe(PREVIEW_HEIGHT);
    expect(spread.height).toBe(PREVIEW_HEIGHT);
    expect(portrait.width / portrait.height).toBeCloseTo(612 / 792, 2);
    expect(spread.width / spread.height).toBeCloseTo(1224 / 792, 2);
  });

  it("쪽 크기가 0.09pt 인 PDF 도 고정 높이로 보인다", async () => {
    respondWithFirstFile(PDF_FILE);
    globalThis.fetch = mockPdfFetch("1");
    const doc = createDoc([{ width: 0.0934, height: 0.0675 }]);
    mockGetDocument.mockReturnValue({ promise: Promise.resolve(doc) });

    render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);

    await waitFor(() => expect(doc.rendered).toHaveLength(1));
    const canvas = screen.getByLabelText("1쪽");
    expect(canvas.height).toBe(PREVIEW_HEIGHT);
    expect(canvas.width).toBeCloseTo(PREVIEW_HEIGHT * (0.0934 / 0.0675), -1);
  });

  it("PDF 가 아닌 첫 파일은 안내만 보이고 쪽을 요청하지 않는다", async () => {
    respondWithFirstFile({ ...PDF_FILE, file_type: "epub", file_path: "c/책.epub" });
    const fetchMock = mockPdfFetch();
    globalThis.fetch = fetchMock;

    render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);

    expect(await screen.findByText(/책\.epub\)이 PDF가 아니라/)).toBeTruthy();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("파일이 없는 디렉토리는 아무것도 표시하지 않는다", async () => {
    respondWithFirstFile(null);
    const { container } = render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);
    await waitFor(() => expect(container.querySelector("section")).toBeNull());
  });

  it("불러오는 동안 상태 문구를 보인다", () => {
    mockRawJsonGetReq.mockImplementation(() => {});
    render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);
    expect(screen.getByRole("status").textContent).toContain("불러오는 중");
  });

  it("목록 조회 실패와 HTTP 오류를 알린다", async () => {
    mockRawJsonGetReq.mockImplementation((url, resolve) => resolve({ status: "failure", error: "목록 오류" }));
    const first = render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);
    expect((await screen.findByRole("alert")).textContent).toContain("목록 오류");
    first.unmount();

    respondWithFirstFile(PDF_FILE);
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({ ok: false, status: 404, text: () => Promise.resolve("File not found") }),
    );
    render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);
    expect((await screen.findByRole("alert")).textContent).toContain("HTTP 404 - File not found");
  });

  it("목록 요청 자체가 실패해도 알린다", async () => {
    mockRawJsonGetReq.mockImplementation((url, resolve, reject) => reject("x"));
    render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);
    expect((await screen.findByRole("alert")).textContent).toContain("첫 파일을 불러오지 못했습니다");
  });

  it("언마운트하면 열어 둔 PDF 문서를 정리한다", async () => {
    respondWithFirstFile(PDF_FILE);
    globalThis.fetch = mockPdfFetch("1");
    const doc = createDoc([{ width: 612, height: 792 }]);
    mockGetDocument.mockReturnValue({ promise: Promise.resolve(doc) });
    const { unmount } = render(<DirectoryPagePreview category="c" apiPrefix="/comics" />);
    await waitFor(() => expect(screen.getByLabelText("1쪽")).toBeTruthy());

    unmount();

    expect(doc.loadingTask.destroy).toHaveBeenCalled();
  });

  it("카테고리가 바뀌면 새 디렉토리의 첫 파일을 다시 불러온다", async () => {
    respondWithFirstFile(null);
    const { rerender } = render(<DirectoryPagePreview category="a" apiPrefix="/comics" />);
    rerender(<DirectoryPagePreview category="b" apiPrefix="/comics" />);
    await waitFor(() => expect(mockRawJsonGetReq).toHaveBeenCalledTimes(2));
    expect(mockRawJsonGetReq.mock.calls[1][0]).toBe("/comics/categories/b?limit=1");
  });
});
