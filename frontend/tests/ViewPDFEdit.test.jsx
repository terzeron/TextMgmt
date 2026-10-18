// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  cleanup,
  fireEvent,
  within,
} from "@testing-library/react";

afterEach(cleanup);

// jsdom 에는 PointerEvent 가 없어 clientX/Y 가 전달되지 않는다.
if (typeof globalThis.PointerEvent === "undefined") {
  globalThis.PointerEvent = class PointerEvent extends MouseEvent {
    constructor(type, init = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 1;
    }
  };
}

globalThis.IntersectionObserver = class MockIntersectionObserver {
  constructor(callback) {
    this.callback = callback;
  }
  observe(element) {
    this.callback([{ isIntersecting: true, target: element }], this);
  }
  unobserve() {}
  disconnect() {}
};

const { mockGetDocument } = vi.hoisted(() => ({ mockGetDocument: vi.fn() }));

vi.mock("pdfjs-dist/legacy/build/pdf.mjs", () => ({
  getDocument: mockGetDocument,
  GlobalWorkerOptions: { workerSrc: "" },
}));

vi.mock("../src/Common", () => ({
  getApiUrlPrefix: () => "http://localhost:8000",
  getAuthToken: () => "test-token",
}));

import ViewPDF from "../src/ViewPDF";

function setupGetDocument() {
  mockGetDocument.mockImplementation(() => {
    const pdfDoc = {
      numPages: 1,
      getPage: vi.fn(() =>
        Promise.resolve({
          getViewport: () => ({ width: 800, height: 600 }),
          render: () => ({ promise: Promise.resolve() }),
        }),
      ),
    };
    pdfDoc.loadingTask = { promise: Promise.resolve(pdfDoc), destroy: vi.fn() };
    return pdfDoc.loadingTask;
  });
}

// GET /pdf-pages 는 총 3쪽 PDF 를, POST .../delete 는 deleteResponse 를 돌려준다.
// POST .../insert-preview 는 previewResponse 를 돌려준다(기본: 성공하는 JPEG blob).
let previewResponse;

function createFetch(deleteResponse) {
  return vi.fn((url, init) => {
    if (init?.method === "POST" && String(url).includes("/insert-preview")) {
      return Promise.resolve(previewResponse);
    }
    if (init?.method === "POST") return Promise.resolve(deleteResponse);
    return Promise.resolve({
      ok: true,
      status: 200,
      headers: new Headers({ "X-Total-Pages": "3" }),
      arrayBuffer: () => Promise.resolve(new ArrayBuffer(10)),
    });
  });
}

const okResponse = {
  ok: true,
  status: 200,
  json: () => Promise.resolve({ status: "success", result: {} }),
};

async function renderEditable(props = {}) {
  render(<ViewPDF bookId={1} pageCount={10} preview editable {...props} />);
  await waitFor(() => expect(screen.getByText(/총 3쪽 표시/)).toBeTruthy());
}

describe("ViewPDF 편집 모드: 쪽 삭제 표시", () => {
  let originalFetch;

  beforeEach(() => {
    mockGetDocument.mockReset();
    setupGetDocument();
    originalFetch = globalThis.fetch;
    HTMLCanvasElement.prototype.getContext = vi.fn(() => ({}));
    window.confirm = vi.fn(() => true);
    let blobCount = 0;
    URL.createObjectURL = vi.fn(() => `blob:preview-${(blobCount += 1)}`);
    URL.revokeObjectURL = vi.fn();
    previewResponse = {
      ok: true,
      status: 200,
      blob: () => Promise.resolve(new Blob(["jpeg"], { type: "image/jpeg" })),
    };
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it("editable 이 아니면 휴지통 버튼이 없다", async () => {
    globalThis.fetch = createFetch(okResponse);
    render(<ViewPDF bookId={1} pageCount={10} preview />);
    await waitFor(() => expect(screen.getByText("총 3쪽 표시")).toBeTruthy());
    expect(screen.queryByLabelText(/삭제 표시/)).toBeNull();
  });

  it("쪽마다 휴지통 버튼이 있고 저장 버튼은 처음에 없다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();
    expect(screen.getAllByLabelText(/쪽 삭제 표시/)).toHaveLength(3);
    expect(screen.queryByLabelText("저장")).toBeNull();
  });

  it("휴지통을 누르면 회색 레이어와 저장 버튼이 나타나고, 다시 누르면 사라진다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();

    fireEvent.click(screen.getByLabelText("2쪽 삭제 표시"));
    expect(screen.getByTestId("pdf-delete-overlay-2")).toBeTruthy();
    expect(screen.queryByTestId("pdf-delete-overlay-1")).toBeNull();
    expect(screen.getByLabelText("저장")).toBeTruthy();

    fireEvent.click(screen.getByLabelText("2쪽 삭제 표시"));
    expect(screen.queryByTestId("pdf-delete-overlay-2")).toBeNull();
    expect(screen.queryByLabelText("저장")).toBeNull();
  });

  it("저장하면 표시한 쪽 번호를 POST 하고 PDF 를 다시 불러온다", async () => {
    const fetchMock = createFetch(okResponse);
    globalThis.fetch = fetchMock;
    await renderEditable();

    fireEvent.click(screen.getByLabelText("3쪽 삭제 표시"));
    fireEvent.click(screen.getByLabelText("1쪽 삭제 표시"));
    const getsBefore = fetchMock.mock.calls.filter(
      ([, init]) => !init?.method,
    ).length;
    // 저장 버튼은 표시한 쪽마다 있고, 어느 것을 눌러도 표시한 쪽 전체를 저장한다.
    expect(screen.getAllByLabelText("저장")).toHaveLength(2);
    fireEvent.click(screen.getAllByLabelText("저장")[0]);

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(([, init]) => init?.method === "POST");
      expect(post[0]).toBe("http://localhost:8000/pdf-pages/1/delete");
      expect(JSON.parse(post[1].body)).toEqual({
        pages: [1, 3],
        crops: [],
        splits: [],
        inserts: [],
      });
    });
    await waitFor(() => {
      const getsAfter = fetchMock.mock.calls.filter(
        ([, init]) => !init?.method,
      ).length;
      expect(getsAfter).toBeGreaterThan(getsBefore);
    });
    await waitFor(() => expect(screen.queryByLabelText("저장")).toBeNull());
    expect(screen.queryByTestId("pdf-delete-overlay-1")).toBeNull();
  });

  it("확인 창에서 취소하면 서버를 호출하지 않는다", async () => {
    const fetchMock = createFetch(okResponse);
    globalThis.fetch = fetchMock;
    window.confirm = vi.fn(() => false);
    await renderEditable();

    fireEvent.click(screen.getByLabelText("1쪽 삭제 표시"));
    fireEvent.click(screen.getByLabelText("저장"));

    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === "POST"),
    ).toBe(false);
    expect(screen.getByTestId("pdf-delete-overlay-1")).toBeTruthy();
  });

  it("서버가 실패를 돌려주면 오류를 보이고 표시를 유지한다", async () => {
    globalThis.fetch = createFetch({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ status: "failure", error: "모든 페이지를 삭제할 수 없습니다" }),
    });
    await renderEditable();

    fireEvent.click(screen.getByLabelText("1쪽 삭제 표시"));
    fireEvent.click(screen.getByLabelText("저장"));

    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("모든 페이지를 삭제할 수 없습니다"),
    );
    expect(screen.getByTestId("pdf-delete-overlay-1")).toBeTruthy();
    expect(screen.getByLabelText("저장")).toBeTruthy();
  });

  it("HTTP 오류도 오류로 보인다", async () => {
    globalThis.fetch = createFetch({
      ok: false,
      status: 403,
      text: () => Promise.resolve("forbidden"),
    });
    await renderEditable();

    fireEvent.click(screen.getByLabelText("1쪽 삭제 표시"));
    fireEvent.click(screen.getByLabelText("저장"));

    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("HTTP 403"),
    );
  });

  it("쪽마다 크롭 버튼이 있고 좌상단 버튼은 없다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();
    expect(screen.getAllByLabelText(/쪽 크롭/)).toHaveLength(3);
    expect(screen.queryByLabelText(/왼쪽 절반/)).toBeNull();
  });

  it("크롭 버튼은 우상단 휴지통 바로 왼쪽에 있다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();
    const crop = screen.getByLabelText("1쪽 크롭");
    expect(crop.nextElementSibling).toBe(screen.getByLabelText("1쪽 삭제 표시"));
  });

  it("크롭을 누르면 크롭 박스와 저장 버튼이 나타나고, 다시 누르면 사라진다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();

    fireEvent.click(screen.getByLabelText("2쪽 크롭"));
    expect(screen.getAllByTestId("pdf-crop-box")).toHaveLength(1);
    expect(screen.getAllByLabelText("저장")).toHaveLength(1);
    expect(screen.getByLabelText("2쪽 크롭").getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(screen.getByLabelText("2쪽 크롭"));
    expect(screen.queryByTestId("pdf-crop-box")).toBeNull();
    expect(screen.queryByLabelText("저장")).toBeNull();
  });

  it("같은 쪽의 쪽 전체 삭제와 크롭은 서로 배타적이다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();

    fireEvent.click(screen.getByLabelText("1쪽 삭제 표시"));
    expect(screen.getByTestId("pdf-delete-overlay-1")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("1쪽 크롭"));
    expect(screen.queryByTestId("pdf-delete-overlay-1")).toBeNull();
    expect(screen.getAllByTestId("pdf-crop-box")).toHaveLength(1);
    fireEvent.click(screen.getByLabelText("1쪽 삭제 표시"));
    expect(screen.queryByTestId("pdf-crop-box")).toBeNull();
    expect(screen.getByTestId("pdf-delete-overlay-1")).toBeTruthy();
  });

  it("저장하면 쪽 전체 삭제와 크롭 영역을 함께 보낸다", async () => {
    const fetchMock = createFetch(okResponse);
    globalThis.fetch = fetchMock;
    await renderEditable();

    fireEvent.click(screen.getByLabelText("3쪽 삭제 표시"));
    fireEvent.click(screen.getByLabelText("1쪽 크롭"));
    fireEvent.click(screen.getAllByLabelText("저장")[0]);

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(([, init]) => init?.method === "POST");
      expect(JSON.parse(post[1].body)).toEqual({
        pages: [3],
        crops: [{ page: 1, x0: 0.1, y0: 0.1, x1: 0.9, y1: 0.9, angle: 0 }],
        splits: [],
        inserts: [],
      });
    });
    expect(window.confirm.mock.calls[0][0]).toContain("1쪽 크롭");
  });

  it("박스를 드래그해 조절한 영역이 저장 요청에 반영된다", async () => {
    const fetchMock = createFetch(okResponse);
    globalThis.fetch = fetchMock;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
      width: 200,
      height: 100,
      left: 0,
      top: 0,
      right: 200,
      bottom: 100,
    });
    await renderEditable();

    fireEvent.click(screen.getByLabelText("2쪽 크롭"));
    const handle = document.querySelector('[data-handle="nw"]');
    fireEvent.pointerDown(handle, { clientX: 20, clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerUp(handle, { pointerId: 1 });
    fireEvent.click(screen.getByLabelText("저장"));

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(([, init]) => init?.method === "POST");
      const [crop] = JSON.parse(post[1].body).crops;
      expect(crop.page).toBe(2);
      expect(crop.x0).toBeCloseTo(0, 5);
      expect(crop.y0).toBeCloseTo(0, 5);
      expect(crop.x1).toBeCloseTo(0.9, 5);
    });
  });

  it("1장씩 보기를 켜면 크롭 표시를 버리고 크롭 버튼을 끈다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();

    fireEvent.click(screen.getByLabelText("1쪽 크롭"));
    expect(screen.getByTestId("pdf-crop-box")).toBeTruthy();
    fireEvent.click(screen.getByText("1장씩"));

    expect(screen.queryByTestId("pdf-crop-box")).toBeNull();
    expect(screen.getByLabelText("1쪽 크롭").disabled).toBe(true);
  });

  it("분할 버튼은 크롭 버튼 바로 왼쪽에 있다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();
    expect(screen.getAllByLabelText(/쪽 분할/)).toHaveLength(3);
    expect(screen.getByLabelText("1쪽 분할").nextElementSibling).toBe(
      screen.getByLabelText("1쪽 크롭"),
    );
  });

  it("분할을 누르면 앞(파랑)·뒤(빨강) 박스 두 개가 나타나고, 다시 누르면 사라진다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();

    fireEvent.click(screen.getByLabelText("2쪽 분할"));
    const boxes = screen.getAllByTestId("pdf-crop-box");
    expect(boxes.map((box) => box.dataset.box)).toEqual(["first", "second"]);
    expect(boxes[0].style.getPropertyValue("--crop-color")).toBe("#4a90d9");
    expect(boxes[1].style.getPropertyValue("--crop-color")).toBe("#d9534f");
    expect(screen.getByText("앞")).toBeTruthy();
    expect(screen.getByText("뒤")).toBeTruthy();
    expect(screen.getAllByLabelText("저장")).toHaveLength(1);

    fireEvent.click(screen.getByLabelText("2쪽 분할"));
    expect(screen.queryByTestId("pdf-crop-box")).toBeNull();
    expect(screen.queryByLabelText("저장")).toBeNull();
  });

  it("같은 쪽의 분할은 삭제·크롭과 서로 배타적이다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();

    fireEvent.click(screen.getByLabelText("1쪽 분할"));
    expect(screen.getAllByTestId("pdf-crop-box")).toHaveLength(2);
    fireEvent.click(screen.getByLabelText("1쪽 크롭"));
    expect(screen.getAllByTestId("pdf-crop-box")).toHaveLength(1);
    fireEvent.click(screen.getByLabelText("1쪽 분할"));
    expect(screen.getAllByTestId("pdf-crop-box")).toHaveLength(2);
    fireEvent.click(screen.getByLabelText("1쪽 삭제 표시"));
    expect(screen.queryByTestId("pdf-crop-box")).toBeNull();
    expect(screen.getByTestId("pdf-delete-overlay-1")).toBeTruthy();
  });

  it("두 박스는 독립적으로 조절되고 저장 요청에 first/second 로 담긴다", async () => {
    const fetchMock = createFetch(okResponse);
    globalThis.fetch = fetchMock;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
      width: 200,
      height: 100,
      left: 0,
      top: 0,
      right: 200,
      bottom: 100,
    });
    await renderEditable();

    fireEvent.click(screen.getByLabelText("2쪽 분할"));
    // 뒤쪽(빨강) 박스의 왼쪽 변을 오른쪽으로 20px(0.1) 민다
    const handle = document.querySelector('[data-box="second"] [data-handle="w"]');
    fireEvent.pointerDown(handle, { clientX: 100, clientY: 50, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientX: 120, clientY: 50, pointerId: 1 });
    fireEvent.pointerUp(handle, { pointerId: 1 });
    fireEvent.click(screen.getByLabelText("저장"));

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(([, init]) => init?.method === "POST");
      const [split] = JSON.parse(post[1].body).splits;
      expect(split.page).toBe(2);
      expect(split.first).toEqual({ x0: 0, y0: 0, x1: 0.5, y1: 1, angle: 0 });
      expect(split.second.x0).toBeCloseTo(0.6, 5);
      expect(split.second.x1).toBeCloseTo(1, 5);
      expect(split.second.y0).toBe(0);
      expect(split.second.y1).toBe(1);
    });
    expect(window.confirm.mock.calls[0][0]).toContain("2쪽 분할");
  });

  it("1장씩 보기를 켜면 분할 표시도 버리고 분할 버튼을 끈다", async () => {
    globalThis.fetch = createFetch(okResponse);
    await renderEditable();

    fireEvent.click(screen.getByLabelText("1쪽 분할"));
    fireEvent.click(screen.getByText("1장씩"));

    expect(screen.queryByTestId("pdf-crop-box")).toBeNull();
    expect(screen.getByLabelText("1쪽 분할").disabled).toBe(true);
  });

  describe("크롭 박스 회전", () => {
    const page = (size) =>
      vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
        width: size,
        height: size,
        left: 0,
        top: 0,
        right: size,
        bottom: size,
      });
    const rotateHandle = () => document.querySelector('[data-testid="pdf-crop-rotate"]');

    it("박스마다 회전 손잡이가 박스 안에 있다", async () => {
      globalThis.fetch = createFetch(okResponse);
      await renderEditable();
      fireEvent.click(screen.getByLabelText("1쪽 분할"));
      expect(screen.getAllByTestId("pdf-crop-rotate")).toHaveLength(2);
      for (const handle of screen.getAllByTestId("pdf-crop-rotate")) {
        expect(handle.closest('[data-testid="pdf-crop-box"]')).toBeTruthy();
      }
    });

    it("회전 손잡이를 드래그하면 박스가 기울고 각도 라벨과 기울어진 덮개가 나타난다", async () => {
      globalThis.fetch = createFetch(okResponse);
      page(400);
      await renderEditable();
      fireEvent.click(screen.getByLabelText("1쪽 크롭"));
      expect(screen.queryByTestId("pdf-crop-dim-rotated")).toBeNull();

      // 중심 (200,200) 의 위쪽(-90°)에서 오른쪽(0°)으로: 시계 방향 +90°
      fireEvent.pointerDown(rotateHandle(), { clientX: 200, clientY: 40, pointerId: 1 });
      fireEvent.pointerMove(rotateHandle(), { clientX: 360, clientY: 200, pointerId: 1 });
      fireEvent.pointerUp(rotateHandle(), { pointerId: 1 });

      const box = screen.getByTestId("pdf-crop-box");
      expect(Number(box.dataset.angle)).toBeCloseTo(90, 3);
      expect(box.style.transform).toBe("rotate(90deg)");
      expect(screen.getByText("90.0°")).toBeTruthy();
      expect(screen.getByTestId("pdf-crop-dim-rotated").style.clipPath).toContain("evenodd");
    });

    it("Shift 를 누르고 돌리면 15도 단위로 맞춰진다", async () => {
      globalThis.fetch = createFetch(okResponse);
      page(400);
      await renderEditable();
      fireEvent.click(screen.getByLabelText("1쪽 크롭"));

      // 목표 약 20° → 15° (-70° 방향의 점)
      fireEvent.pointerDown(rotateHandle(), { clientX: 200, clientY: 40, pointerId: 1 });
      fireEvent.pointerMove(rotateHandle(), {
        clientX: 200 + 100 * Math.cos((-70 * Math.PI) / 180),
        clientY: 200 + 100 * Math.sin((-70 * Math.PI) / 180),
        shiftKey: true,
        pointerId: 1,
      });
      fireEvent.pointerUp(rotateHandle(), { pointerId: 1 });

      expect(Number(screen.getByTestId("pdf-crop-box").dataset.angle)).toBe(15);
    });

    it("쪽 밖으로 나가는 각도까지는 돌지 않는다", async () => {
      globalThis.fetch = createFetch(okResponse);
      page(400);
      await renderEditable();
      fireEvent.click(screen.getByLabelText("1쪽 분할"));
      // 세로로 긴 앞쪽 박스를 45° 돌리려 하면 꼭짓점이 쪽 밖으로 나간다
      const handle = document.querySelector('[data-box="first"] [data-testid="pdf-crop-rotate"]');
      fireEvent.pointerDown(handle, { clientX: 110, clientY: 40, pointerId: 1 });
      fireEvent.pointerMove(handle, { clientX: 110 + 100, clientY: 200 - 100, pointerId: 1 });
      fireEvent.pointerUp(handle, { pointerId: 1 });

      const angle = Number(document.querySelector('[data-box="first"]').dataset.angle);
      expect(Math.abs(angle)).toBeLessThan(45);
    });

    it("저장 요청에 각도가 담긴다", async () => {
      const fetchMock = createFetch(okResponse);
      globalThis.fetch = fetchMock;
      page(400);
      await renderEditable();
      fireEvent.click(screen.getByLabelText("1쪽 크롭"));
      fireEvent.pointerDown(rotateHandle(), { clientX: 200, clientY: 40, pointerId: 1 });
      fireEvent.pointerMove(rotateHandle(), { clientX: 360, clientY: 200, pointerId: 1 });
      fireEvent.pointerUp(rotateHandle(), { pointerId: 1 });
      fireEvent.click(screen.getByLabelText("저장"));

      await waitFor(() => {
        const post = fetchMock.mock.calls.find(([, init]) => init?.method === "POST");
        const [crop] = JSON.parse(post[1].body).crops;
        expect(crop.page).toBe(1);
        expect(crop.angle).toBeCloseTo(90, 3);
      });
    });
  });

  describe("이미지 URL 삽입", () => {
    const openDialog = (pageNum) =>
      fireEvent.click(screen.getByLabelText(`${pageNum}쪽 앞에 이미지 삽입`));
    const submitUrl = (url) => {
      fireEvent.change(screen.getByLabelText("이미지 URL"), {
        target: { value: url },
      });
      fireEvent.click(screen.getByText("삽입 표시"));
    };
    // 모달에서 URL 을 제출하고, 미리보기가 뷰어에 나타날 때까지 기다린다.
    const stage = async (pageNum, url) => {
      openDialog(pageNum);
      submitUrl(url);
      await waitFor(() =>
        expect(screen.getByTestId(`pdf-insert-preview-${pageNum}`)).toBeTruthy(),
      );
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    };

    it("이미지 삽입 버튼이 쪽마다 있고 분할 버튼 바로 왼쪽에 있다", async () => {
      globalThis.fetch = createFetch(okResponse);
      await renderEditable();
      expect(screen.getAllByLabelText(/쪽 앞에 이미지 삽입$/)).toHaveLength(3);
      expect(
        screen.getByLabelText("1쪽 앞에 이미지 삽입").nextElementSibling,
      ).toBe(screen.getByLabelText("1쪽 분할"));
    });

    it("버튼을 누르면 URL 입력 모달이 열리고, 취소하면 아무것도 표시하지 않는다", async () => {
      const fetchMock = createFetch(okResponse);
      globalThis.fetch = fetchMock;
      await renderEditable();

      openDialog(2);
      expect(screen.getByRole("dialog")).toBeTruthy();
      expect(
        screen.getByText("2쪽 앞에 이미지 삽입", { selector: ".modal-title" }),
      ).toBeTruthy();
      fireEvent.click(screen.getByText("취소"));

      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(screen.queryByTestId("pdf-insert-preview-2")).toBeNull();
      expect(screen.queryByLabelText("저장")).toBeNull();
      expect(
        fetchMock.mock.calls.some(([url]) => String(url).includes("insert-preview")),
      ).toBe(false);
    });

    it("제출하면 서버에서 미리보기를 받아 와 삽입될 쪽을 해당 쪽 바로 앞에 보여 준다", async () => {
      const fetchMock = createFetch(okResponse);
      globalThis.fetch = fetchMock;
      await renderEditable();

      await stage(2, "  https://img.example.com/a.png  ");

      const [url, init] = fetchMock.mock.calls.find(([u]) =>
        String(u).includes("/insert-preview"),
      );
      expect(url).toBe("http://localhost:8000/pdf-pages/1/insert-preview");
      expect(JSON.parse(init.body)).toEqual({ url: "https://img.example.com/a.png" });

      const preview = screen.getByTestId("pdf-insert-preview-2");
      const image = preview.querySelector("img");
      expect(image.getAttribute("src")).toBe("blob:preview-1");
      expect(image.alt).toBe("2쪽 앞에 삽입될 이미지");
      expect(preview.textContent).toContain("새 쪽 · 저장 전 (2쪽 앞에 삽입)");
      // 2쪽의 편집 래퍼 바로 앞에 놓인다 = 실제로 2쪽 앞에 새 쪽이 끼어 보인다
      expect(preview.nextElementSibling.classList.contains("pdf-page-wrap")).toBe(true);
      expect(preview.nextElementSibling.querySelector('[aria-label="2쪽 삭제 표시"]')).toBeTruthy();
      expect(preview.previousElementSibling.querySelector('[aria-label="1쪽 삭제 표시"]')).toBeTruthy();
      // 저장 버튼은 삽입 미리보기와 그 쪽(2쪽)에 하나씩 있다
      expect(screen.getAllByLabelText("저장")).toHaveLength(2);
      expect(within(preview).getByLabelText("저장")).toBeTruthy();
      expect(screen.getByLabelText("2쪽 앞에 이미지 삽입").getAttribute("aria-pressed")).toBe("true");
      expect(screen.getByText(/변경 표시 1쪽/)).toBeTruthy();
    });

    it("미리보기를 받아 오는 동안 모달은 '불러오는 중'을 보이고 닫히지 않는다", async () => {
      let release;
      const base = createFetch(okResponse);
      globalThis.fetch = vi.fn((url, init) =>
        String(url).includes("/insert-preview")
          ? new Promise((resolve) => {
              release = () => resolve(previewResponse);
            })
          : base(url, init),
      );
      await renderEditable();

      openDialog(1);
      submitUrl("https://img.example.com/a.png");

      await waitFor(() => expect(screen.getByText("불러오는 중…")).toBeTruthy());
      expect(screen.getByLabelText("이미지 URL").disabled).toBe(true);
      expect(screen.getByText("취소").disabled).toBe(true);
      expect(screen.queryByTestId("pdf-insert-preview-1")).toBeNull();

      release();
      await waitFor(() => expect(screen.getByTestId("pdf-insert-preview-1")).toBeTruthy());
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    });

    it("서버가 이미지를 가져오지 못하면 모달에 사유를 보이고 아무것도 표시하지 않는다", async () => {
      globalThis.fetch = createFetch(okResponse);
      previewResponse = {
        ok: false,
        status: 400,
        text: () => Promise.resolve("내부 주소의 이미지는 사용할 수 없습니다"),
      };
      await renderEditable();

      openDialog(1);
      submitUrl("http://127.0.0.1/a.png");

      await waitFor(() =>
        expect(screen.getByRole("alert").textContent).toContain("내부 주소의 이미지는 사용할 수 없습니다"),
      );
      expect(screen.getByRole("dialog")).toBeTruthy();
      expect(screen.queryByTestId("pdf-insert-preview-1")).toBeNull();
      expect(screen.queryByLabelText("저장")).toBeNull();
      // 오류가 나도 다시 시도할 수 있다
      expect(screen.getByText("삽입 표시").disabled).toBe(false);
    });

    it("400 이 아닌 HTTP 오류와 네트워크 오류도 모달에 보인다", async () => {
      globalThis.fetch = createFetch(okResponse);
      previewResponse = { ok: false, status: 502, text: () => Promise.resolve("bad gateway") };
      await renderEditable();
      openDialog(1);
      submitUrl("https://img.example.com/a.png");
      await waitFor(() =>
        expect(screen.getByRole("alert").textContent).toContain("HTTP 502"),
      );

      const base = globalThis.fetch;
      globalThis.fetch = vi.fn((url, init) =>
        String(url).includes("/insert-preview")
          ? Promise.reject(new Error("네트워크 끊김"))
          : base(url, init),
      );
      submitUrl("https://img.example.com/b.png");
      await waitFor(() =>
        expect(screen.getByRole("alert").textContent).toContain("네트워크 끊김"),
      );
    });

    it("다시 열면 기존 URL 이 채워져 있고, 수정하면 이전 미리보기를 해제한다", async () => {
      globalThis.fetch = createFetch(okResponse);
      await renderEditable();
      await stage(1, "https://img.example.com/a.png");

      openDialog(1);
      expect(screen.getByLabelText("이미지 URL").value).toBe("https://img.example.com/a.png");
      submitUrl("https://img.example.com/b.png");

      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      await waitFor(() =>
        expect(screen.getByTestId("pdf-insert-preview-1").querySelector("img").getAttribute("src")).toBe("blob:preview-2"),
      );
      expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:preview-1");
    });

    it("삽입 미리보기의 저장 버튼은 ×(삭제) 바로 왼쪽에 있고 같은 저장을 한다", async () => {
      const fetchMock = createFetch(okResponse);
      globalThis.fetch = fetchMock;
      await renderEditable();
      await stage(2, "https://img.example.com/a.png");
      const preview = screen.getByTestId("pdf-insert-preview-2");
      const save = within(preview).getByLabelText("저장");
      expect(save.nextElementSibling).toBe(
        within(preview).getByLabelText("2쪽 앞 이미지 삽입 취소"),
      );

      fireEvent.click(save);

      await waitFor(() => {
        const post = fetchMock.mock.calls.find(
          ([u, init]) => init?.method === "POST" && String(u).endsWith("/delete"),
        );
        expect(JSON.parse(post[1].body).inserts).toEqual([
          { page: 2, url: "https://img.example.com/a.png" },
        ]);
      });
      expect(window.confirm.mock.calls[0][0]).toContain("2쪽 앞에 이미지 삽입");
      await waitFor(() => expect(screen.queryByTestId("pdf-insert-preview-2")).toBeNull());
    });

    describe("이 기기의 이미지", () => {
      const photo = () => new File(["x"], "photo.jpg", { type: "image/jpeg" });
      const pickFile = (file) =>
        fireEvent.change(screen.getByLabelText("이 기기에서 이미지 선택"), {
          target: { files: [file] },
        });
      const stageFile = async (pageNum, file) => {
        openDialog(pageNum);
        pickFile(file);
        await waitFor(() =>
          expect(screen.getByTestId(`pdf-insert-preview-${pageNum}`)).toBeTruthy(),
        );
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      };

      it("파일을 고르면 insert-preview-upload 로 올려 미리보기를 보여 준다", async () => {
        const fetchMock = createFetch(okResponse);
        globalThis.fetch = fetchMock;
        await renderEditable();
        const file = photo();

        await stageFile(2, file);

        const [url, init] = fetchMock.mock.calls.find(([u]) =>
          String(u).includes("/insert-preview-upload"),
        );
        expect(url).toBe("http://localhost:8000/pdf-pages/1/insert-preview-upload");
        expect(init.body.get("file")).toBe(file);
        const preview = screen.getByTestId("pdf-insert-preview-2");
        expect(preview.querySelector("img").getAttribute("src")).toBe("blob:preview-1");
        // URL 이 없으므로 툴팁에는 파일 이름을 쓴다.
        expect(preview.querySelector(".viewer-edit-tag").title).toBe("photo.jpg");
      });

      it("저장하면 미리 본 JPEG 를 image_data(base64)로 보내고 url 은 보내지 않는다", async () => {
        const fetchMock = createFetch(okResponse);
        globalThis.fetch = fetchMock;
        await renderEditable();
        await stageFile(2, photo());

        fireEvent.click(within(screen.getByTestId("pdf-insert-preview-2")).getByLabelText("저장"));

        await waitFor(() => {
          const post = fetchMock.mock.calls.find(
            ([u, init]) => init?.method === "POST" && String(u).endsWith("/delete"),
          );
          // 미리보기 응답의 Blob 내용은 "jpeg" 다.
          expect(JSON.parse(post[1].body).inserts).toEqual([
            { page: 2, image_data: "anBlZw==" },
          ]);
        });
      });

      it("URL 삽입과 파일 삽입을 한 번에 저장한다", async () => {
        const fetchMock = createFetch(okResponse);
        globalThis.fetch = fetchMock;
        await renderEditable();
        await stage(1, "https://img.example.com/a.png");
        await stageFile(3, photo());

        fireEvent.click(within(screen.getByTestId("pdf-insert-preview-1")).getByLabelText("저장"));

        await waitFor(() => {
          const post = fetchMock.mock.calls.find(
            ([u, init]) => init?.method === "POST" && String(u).endsWith("/delete"),
          );
          expect(JSON.parse(post[1].body).inserts).toEqual([
            { page: 1, url: "https://img.example.com/a.png" },
            { page: 3, image_data: "anBlZw==" },
          ]);
        });
      });

      it("서버가 거절하면 사유를 모달에 보이고 삽입을 표시하지 않는다", async () => {
        globalThis.fetch = createFetch(okResponse);
        previewResponse = {
          ok: false,
          status: 400,
          text: () => Promise.resolve("이미지 형식을 읽을 수 없습니다"),
        };
        await renderEditable();

        openDialog(2);
        pickFile(photo());

        expect(await screen.findByText("이미지 형식을 읽을 수 없습니다")).toBeTruthy();
        expect(screen.queryByTestId("pdf-insert-preview-2")).toBeNull();
        expect(screen.getByRole("dialog")).toBeTruthy();
      });

      it("15MB 를 넘는 파일은 서버에 올리지 않고 거절한다", async () => {
        const fetchMock = createFetch(okResponse);
        globalThis.fetch = fetchMock;
        await renderEditable();
        const big = photo();
        Object.defineProperty(big, "size", { value: 15 * 1024 * 1024 + 1 });

        openDialog(2);
        pickFile(big);

        expect(await screen.findByText("이미지는 15MB 이하여야 합니다")).toBeTruthy();
        expect(
          fetchMock.mock.calls.some(([u]) => String(u).includes("/insert-preview")),
        ).toBe(false);
      });
    });

    it("저장 중에는 삽입 미리보기의 저장·취소 버튼이 비활성화된다", async () => {
      let release;
      const base = createFetch(okResponse);
      globalThis.fetch = vi.fn((url, init) =>
        init?.method === "POST" && String(url).endsWith("/delete")
          ? new Promise((resolve) => {
              release = () => resolve(okResponse);
            })
          : base(url, init),
      );
      await renderEditable();
      await stage(1, "https://img.example.com/a.png");
      const preview = screen.getByTestId("pdf-insert-preview-1");

      fireEvent.click(within(preview).getByLabelText("저장"));

      await waitFor(() => expect(within(preview).getByLabelText("저장").disabled).toBe(true));
      expect(within(preview).getByLabelText("1쪽 앞 이미지 삽입 취소").disabled).toBe(true);
      release();
      await waitFor(() => expect(screen.queryByTestId("pdf-insert-preview-1")).toBeNull());
    });

    it("× 로 삽입을 취소하면 미리보기가 사라지고 blob URL 을 해제한다", async () => {
      globalThis.fetch = createFetch(okResponse);
      await renderEditable();
      await stage(3, "https://img.example.com/a.png");

      fireEvent.click(screen.getByLabelText("3쪽 앞 이미지 삽입 취소"));

      expect(screen.queryByTestId("pdf-insert-preview-3")).toBeNull();
      expect(screen.queryByLabelText("저장")).toBeNull();
      expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:preview-1");
    });

    it("삽입 미리보기는 같은 쪽의 삭제·크롭·분할 표시와 함께 있을 수 있다", async () => {
      globalThis.fetch = createFetch(okResponse);
      await renderEditable();
      await stage(2, "https://img.example.com/a.png");

      fireEvent.click(screen.getByLabelText("2쪽 분할"));

      expect(screen.getAllByTestId("pdf-crop-box")).toHaveLength(2);
      expect(screen.getByTestId("pdf-insert-preview-2")).toBeTruthy();
    });

    it("저장하면 삽입 요청을 쪽 번호 순서로 보내고 미리보기를 비운다", async () => {
      const fetchMock = createFetch(okResponse);
      globalThis.fetch = fetchMock;
      await renderEditable();
      await stage(3, "https://img.example.com/c.png");
      await stage(1, "https://img.example.com/a.png");
      fireEvent.click(screen.getByLabelText("3쪽 삭제 표시"));

      fireEvent.click(screen.getAllByLabelText("저장")[0]);

      await waitFor(() => {
        const post = fetchMock.mock.calls.find(
          ([u, init]) => init?.method === "POST" && String(u).endsWith("/delete"),
        );
        expect(JSON.parse(post[1].body)).toEqual({
          pages: [3],
          crops: [],
          splits: [],
          inserts: [
            { page: 1, url: "https://img.example.com/a.png" },
            { page: 3, url: "https://img.example.com/c.png" },
          ],
        });
      });
      expect(window.confirm.mock.calls[0][0]).toContain("1, 3쪽 앞에 이미지 삽입");
      await waitFor(() => expect(screen.queryByTestId("pdf-insert-preview-1")).toBeNull());
      expect(screen.queryByTestId("pdf-insert-preview-3")).toBeNull();
      expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:preview-1");
      expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:preview-2");
    });

    it("저장이 실패하면 오류를 보이고 삽입 미리보기를 유지한다", async () => {
      globalThis.fetch = createFetch({
        ok: true,
        status: 200,
        json: () =>
          Promise.resolve({
            status: "failure",
            error: "1쪽 앞에 넣을 이미지를 가져올 수 없습니다: 시간 초과",
          }),
      });
      await renderEditable();
      await stage(1, "https://img.example.com/a.png");

      fireEvent.click(screen.getAllByLabelText("저장")[0]);

      await waitFor(() =>
        expect(screen.getByRole("alert").textContent).toContain("시간 초과"),
      );
      expect(screen.getByTestId("pdf-insert-preview-1")).toBeTruthy();
    });

    it("뷰어를 닫으면 남은 미리보기 blob URL 을 해제한다", async () => {
      globalThis.fetch = createFetch(okResponse);
      const { unmount } = render(
        <ViewPDF bookId={1} pageCount={10} preview editable />,
      );
      await waitFor(() => expect(screen.getByText(/총 3쪽 표시/)).toBeTruthy());
      await stage(2, "https://img.example.com/a.png");

      unmount();

      expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:preview-1");
    });
  });
});
