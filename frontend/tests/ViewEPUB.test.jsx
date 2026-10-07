// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  act,
  cleanup,
  waitFor,
  fireEvent,
} from "@testing-library/react";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

// ─── epubjs mock ────────────────────────────────────────────────
// ViewEPUB는 epubjs 만 사용한다. ePub() 가 book을 반환하고,
// book.renderTo()가 rendition을 반환한다.
let renditionHandlers;
let lastRendition = null;
let lastBook = null;
let containerMetrics;
let mockViews = [];

let failNextDisplay = false;
let hangDisplay = false;
let mockCoverUrl = null;
let rejectReady = false;
let rejectCoverUrl = false;
let failEveryDisplay = false;
let failDisplayError = null;
let omitCoverUrl = false;

function setFailNextDisplay() {
  failNextDisplay = true;
}

function setHangDisplay() {
  hangDisplay = true;
}

function createMockRendition() {
  renditionHandlers = {};
  const container = document.createElement("div");
  containerMetrics = { scrollTop: 0, scrollHeight: 2000, clientHeight: 500 };
  Object.defineProperty(container, "scrollHeight", {
    get: () => containerMetrics.scrollHeight,
    configurable: true,
  });
  Object.defineProperty(container, "clientHeight", {
    get: () => containerMetrics.clientHeight,
    configurable: true,
  });
  Object.defineProperty(container, "scrollTop", {
    get: () => containerMetrics.scrollTop,
    set: (v) => {
      containerMetrics.scrollTop = v;
    },
    configurable: true,
  });
  container.scrollBy = vi.fn(({ top }) => {
    containerMetrics.scrollTop += top;
  });
  container.scrollTo = vi.fn();

  const rendition = {
    on: vi.fn((name, handler) => {
      renditionHandlers[name] = renditionHandlers[name] || [];
      renditionHandlers[name].push(handler);
    }),
    off: vi.fn((name, handler) => {
      renditionHandlers[name] = (renditionHandlers[name] || []).filter(
        (h) => h !== handler,
      );
    }),
    _emit(name, ...args) {
      for (const h of renditionHandlers[name] || []) h(...args);
    },
    display: vi.fn((target) => {
      if (failEveryDisplay) return Promise.reject(failDisplayError ?? new Error("표시 실패"));
      if (failNextDisplay && target !== undefined) {
        failNextDisplay = false;
        return Promise.reject(new Error("invalid location"));
      }
      if (hangDisplay) {
        return new Promise(() => {});
      }
      return Promise.resolve();
    }),
    next: vi.fn(() => Promise.resolve()),
    prev: vi.fn(() => Promise.resolve()),
    spread: vi.fn(),
    destroy: vi.fn(),
    themes: {
      default: vi.fn(),
      fontSize: vi.fn(),
      font: vi.fn(),
    },
    hooks: { content: { register: vi.fn() } },
    manager: { stage: { container }, views: { all: () => mockViews } },
  };
  return rendition;
}

function createMockBook() {
  const book = {
    ready: rejectReady ? Promise.reject(new Error("준비 실패")) : Promise.resolve(),
    coverUrl: vi.fn(() =>
      rejectCoverUrl
        ? Promise.reject(new Error("표지 실패"))
        : Promise.resolve(mockCoverUrl),
    ),
    loaded: { metadata: Promise.resolve({ title: "테스트 책 제목" }) },
    spine: Array.from({ length: 12 }, (_, i) => ({
      href: `OEBPS/Text/section${i}.html`,
      index: i,
    })),
    locations: {
      total: 10,
      generate: vi.fn(() => Promise.resolve([])),
      load: vi.fn(),
      save: vi.fn(() => '["cfi0","cfi1"]'),
      percentageFromCfi: vi.fn(() => 0.37),
      cfiFromPercentage: vi.fn((ratio) => `epubcfi(ratio-${ratio})`),
    },
    renderTo: vi.fn(() => {
      lastRendition = createMockRendition();
      lastRendition.book = lastBook;
      return lastRendition;
    }),
    destroy: vi.fn(),
  };
  if (omitCoverUrl) delete book.coverUrl;
  return book;
}

vi.mock("epubjs", () => ({
  __esModule: true,
  default: () => {
    lastBook = createMockBook();
    return lastBook;
  },
}));

vi.mock("../src/Common", () => ({
  getApiUrlPrefix: () => "http://localhost:8000",
}));
vi.mock("../src/ViewEPUB.css", () => ({}));

// fetch mock
const mockArrayBuffer = new ArrayBuffer(8);

function createFetchResponse(buf = mockArrayBuffer) {
  return {
    ok: true,
    headers: { get: () => null },
    arrayBuffer: () => Promise.resolve(buf),
    text: () => Promise.resolve(""),
  };
}

// localStorage mock
const localStorageMock = (() => {
  let store = {};
  return {
    getItem: vi.fn((key) => store[key] ?? null),
    setItem: vi.fn((key, value) => {
      store[key] = String(value);
    }),
    removeItem: vi.fn((key) => {
      delete store[key];
    }),
    clear: () => {
      store = {};
    },
    _get: (key) => store[key] ?? null,
    _set: (key, value) => {
      store[key] = value;
    },
  };
})();

globalThis.fetch = vi.fn(() => Promise.resolve(createFetchResponse()));

Object.defineProperty(window, "localStorage", {
  value: localStorageMock,
  writable: true,
});

// matchMedia mock: 방향(orientation) 테스트를 위해 matches를 바꿀 수 있게 둔다.
// 같은 쿼리는 실제 MediaQueryList처럼 동일 인스턴스를 반환한다.
let mqMock = null;
let mqListeners = [];

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: vi.fn((query) => {
    if (!mqMock) {
      mqMock = {
        matches: false,
        media: query,
        addEventListener: vi.fn((_, handler) => mqListeners.push(handler)),
        removeEventListener: vi.fn((_, handler) => {
          mqListeners = mqListeners.filter((h) => h !== handler);
        }),
        addListener: vi.fn((handler) => mqListeners.push(handler)),
        removeListener: vi.fn(),
      };
    }
    return mqMock;
  }),
});

import ViewEPUB from "../src/ViewEPUB";

// 책이 "표시된" 상태로 만든다. fetch→render→display 는 모두 microtask이므로
// 타이머 가중치와 무관하게 비우고 나서 displayed 이벤트를 발화한다.
async function openBook() {
  await waitFor(() => expect(lastRendition?.display).toHaveBeenCalled(), {
    timeout: 3000,
  });
  await act(async () => {
    lastRendition._emit("displayed", {
      href: "OEBPS/Text/section0.html",
      index: 0,
    });
  });
}

// fake-timer 테스트 전용: microtask 플러시 + displayed 발화
async function openBookWithoutTimers() {
  for (let i = 0; i < 8; i++) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
  }
  expect(lastRendition?.display).toHaveBeenCalled();
  await act(async () => {
    lastRendition._emit("displayed", {
      href: "OEBPS/Text/section0.html",
      index: 0,
    });
  });
}

describe("ViewEPUB(세로 스크롤 뷰어)", () => {
  beforeEach(() => {
    lastRendition = null;
    lastBook = null;
    mockCoverUrl = null;
    failNextDisplay = false;
    hangDisplay = false;
    rejectReady = false;
    rejectCoverUrl = false;
    failEveryDisplay = false;
    failDisplayError = null;
    omitCoverUrl = false;
    localStorageMock.clear();
    localStorageMock.getItem.mockReset();
    localStorageMock.getItem.mockImplementation((key) => localStorageMock._get(key));
    localStorageMock.setItem.mockClear();
    globalThis.fetch.mockClear();
    containerMetrics = { scrollTop: 0, scrollHeight: 2000, clientHeight: 500 };
    mockViews = [];
    mqMock = null;
    mqListeners = [];
  });

  it("bookId가 없으면 에러를 표시한다", () => {
    render(<ViewEPUB bookId={0} />);
    expect(screen.getByText(/유효한 bookId/)).toBeTruthy();
  });

  it("bookId가 없으면 fetch를 호출하지 않는다", () => {
    render(<ViewEPUB bookId={0} />);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it("preview=true이면 chapters=10으로 요청한다", async () => {
    render(<ViewEPUB bookId={7} preview={true} />);
    await waitFor(() => expect(globalThis.fetch).toHaveBeenCalledTimes(1));
    expect(globalThis.fetch.mock.calls[0][0]).toBe(
      "http://localhost:8000/preview/7?chapters=10",
    );
  });

  it("전체보기는 chapters=0으로 요청한다", async () => {
    render(<ViewEPUB bookId={7} />);
    await waitFor(() => expect(globalThis.fetch).toHaveBeenCalledTimes(1));
    expect(globalThis.fetch.mock.calls[0][0]).toBe(
      "http://localhost:8000/preview/7?chapters=0",
    );
  });

  it("EPUB 내부 스크립트 실행을 허용하지 않는다", async () => {
    render(<ViewEPUB bookId={7} />);
    await waitFor(() => expect(lastBook?.renderTo).toHaveBeenCalled());
    expect(lastBook.renderTo.mock.calls[0][1].allowScriptedContent).toBe(false);
  });

  it("초기 로딩 시 스피너를 표시한다", () => {
    render(<ViewEPUB bookId={1} />);
    expect(screen.getByText("로딩 중...")).toBeTruthy();
  });

  it("표지가 있으면 맨 앞에 표시하고 다음 이동 시 본문으로 간다", async () => {
    mockCoverUrl = "blob:epub-cover";
    render(<ViewEPUB bookId={1} />);
    await openBook();

    expect(screen.getByRole("img", { name: "책 표지" }).getAttribute("src")).toBe(
      "blob:epub-cover",
    );
    expect(lastRendition.next).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "다음 페이지" }));

    expect(screen.queryByRole("img", { name: "책 표지" })).toBeNull();
    expect(lastRendition.next).not.toHaveBeenCalled();
  });

  it("저장된 읽기 위치가 있으면 표지 대신 해당 위치를 복원한다", async () => {
    mockCoverUrl = "blob:epub-cover";
    localStorageMock._set("epub_location_1", "epubcfi(/6/6!/4/2)");
    render(<ViewEPUB bookId={1} />);
    await openBook();

    expect(lastBook.coverUrl).not.toHaveBeenCalled();
    expect(screen.queryByRole("img", { name: "책 표지" })).toBeNull();
  });

  it("displayed 이벤트가 오면 로딩이 풀린다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    await waitFor(() =>
      expect(screen.queryByText("로딩 중...")).toBeNull(),
    );
  });

  it("EPUB 렌더 및 파싱 이벤트를 오류 메시지로 표시한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());

    await act(async () => {
      lastRendition._emit("displayerror", new Error("화면 오류"));
    });
    expect(screen.getByText("EPUB 렌더링 오류: 화면 오류")).toBeTruthy();

    await act(async () => {
      lastRendition._emit("error", "문서 오류");
    });
    expect(screen.getByText("EPUB 파싱 오류: 문서 오류")).toBeTruthy();
  });

  it("책 준비 실패 후 표지가 있으면 표지를 거쳐 EPUB 표시를 복구한다", async () => {
    rejectReady = true;
    mockCoverUrl = "blob:fallback-cover";
    render(<ViewEPUB bookId={1} />);

    await waitFor(() => expect(lastRendition?.display).toHaveBeenCalled());
    await act(async () => {
      lastRendition._emit("displayed");
    });
    expect(screen.getByTestId("epub-cover-page")).toBeTruthy();
    expect(lastBook.coverUrl).toHaveBeenCalledOnce();
  });

  it("책 준비와 복구 표시가 모두 실패하면 복구 오류를 표시한다", async () => {
    rejectReady = true;
    rejectCoverUrl = true;
    failEveryDisplay = true;
    render(<ViewEPUB bookId={1} />);

    expect(await screen.findByText("EPUB 표시 실패: 표시 실패")).toBeTruthy();
    expect(localStorageMock.removeItem).toHaveBeenCalledWith("epub_location_1");
  });

  it("저장 위치를 읽을 수 없으면 오류를 삼키고 처음부터 표시한다", async () => {
    localStorageMock.getItem.mockImplementation((key) => {
      if (key.startsWith("epub_location_")) {
        throw new Error("storage disabled");
      }
      return null;
    });
    render(<ViewEPUB bookId={1} />);
    await openBook();
    expect(lastRendition.display).toHaveBeenCalledWith();
  });

  it("표지 함수를 지원하지 않는 책은 표지 없이 복구한다", async () => {
    rejectReady = true;
    omitCoverUrl = true;
    render(<ViewEPUB bookId={1} />);

    await waitFor(() => expect(lastRendition?.display).toHaveBeenCalled());
    expect(screen.queryByTestId("epub-cover-page")).toBeNull();
  });

  it("rendered 신호와 이미지·폰트·크기 변경마다 뷰를 다시 확장한다", async () => {
    const observers = [];
    class MockResizeObserver {
      constructor(callback) {
        this.callback = callback;
        this.observe = vi.fn();
        observers.push(this);
      }
    }
    vi.stubGlobal("ResizeObserver", MockResizeObserver);

    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());

    const contentDocument = document.implementation.createHTMLDocument();
    const image = contentDocument.createElement("img");
    const loadedImage = contentDocument.createElement("img");
    Object.defineProperty(image, "complete", { value: false });
    Object.defineProperty(loadedImage, "complete", { value: true });
    const loadListener = vi.spyOn(image, "addEventListener");
    Object.defineProperty(contentDocument, "images", {
      configurable: true,
      value: [image, loadedImage],
    });
    Object.defineProperty(contentDocument, "fonts", {
      configurable: true,
      value: { ready: Promise.resolve() },
    });
    const view = {
      contents: { document: contentDocument },
      expand: vi.fn(),
    };

    act(() => lastRendition._emit("rendered", {}, view));
    expect(view.expand).toHaveBeenCalled();
    expect(observers[0].observe).toHaveBeenCalledWith(contentDocument.body);
    expect(loadListener).toHaveBeenCalledWith("load", expect.any(Function), {
      once: true,
    });

    act(() => observers[0].callback());
    image.dispatchEvent(new Event("load"));
    await waitFor(() => {
      expect(view.expand.mock.calls.length).toBeGreaterThanOrEqual(6);
    }, { timeout: 3000 });
  });

  it("본문 문서가 없거나 head·body가 없으면 콘텐츠 후처리를 건너뛴다", async () => {
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    const [lockOverflow, applyFont] = lastRendition.hooks.content.register.mock.calls.map(
      ([callback]) => callback,
    );
    expect(() => lockOverflow({})).not.toThrow();
    expect(() => applyFont({ document: {} })).not.toThrow();
    expect(() => lastRendition._emit("rendered", {}, { contents: { document: {} } })).not.toThrow();
  });

  it("글꼴 변경 시 현재 열린 뷰의 본문 스타일도 갱신한다", async () => {
    const contentDocument = document.implementation.createHTMLDocument();
    const view = {
      contents: { document: contentDocument },
      expand: vi.fn(),
    };
    mockViews = [view];
    render(<ViewEPUB bookId={1} />);
    await openBook();

    fireEvent.change(screen.getByLabelText("글꼴 선택"), {
      target: { value: "'Noto Sans CJK KR', sans-serif" },
    });

    expect(contentDocument.getElementById("epub-font-face-override")).toBeTruthy();
    expect(contentDocument.getElementById("epub-font-family-override").textContent).toContain("sans-serif");
    expect(view.expand).toHaveBeenCalled();
  });

  it("글자 크기가 최대이면 확대를 막고 축소는 적용한다", async () => {
    localStorageMock._set("epub_fontSize", "160");
    render(<ViewEPUB bookId={1} />);
    await openBook();

    const increase = screen.getByRole("button", { name: "글자 크기 늘리기" });
    const decrease = screen.getByRole("button", { name: "글자 크기 줄이기" });
    expect(increase.disabled).toBe(true);
    fireEvent.click(decrease);
    expect(lastRendition.themes.fontSize).toHaveBeenLastCalledWith("140%");
  });

  it("본문 방향키 왼쪽으로 이전 페이지를 연다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();

    fireEvent.keyDown(document, { key: "ArrowLeft" });
    expect(lastRendition.prev).toHaveBeenCalledOnce();
  });

  it("연속 위치 변경은 이전 저장 타이머를 취소한다", async () => {
    vi.useFakeTimers();
    render(<ViewEPUB bookId={1} />);
    await openBookWithoutTimers();
    await act(async () => {
      lastRendition._emit("relocated", {
        start: { cfi: "epubcfi(/6/2)" },
      });
      lastRendition._emit("relocated", {
        start: { cfi: "epubcfi(/6/4)" },
      });
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(localStorage.getItem("epub_location_1")).toBe("epubcfi(/6/4)");
  });

  it("표지 페이지에서 왼쪽 방향키로 이전 본문으로 이동하지 않는다", async () => {
    mockCoverUrl = "blob:epub-cover";
    render(<ViewEPUB bookId={1} />);
    await openBook();

    fireEvent.keyDown(document, { key: "ArrowLeft" });
    expect(lastRendition.prev).not.toHaveBeenCalled();
  });

  it("30초 내 로딩 미완료 시 타임아웃 에러를 표시한다", async () => {
    vi.useFakeTimers();
    render(<ViewEPUB bookId={1} />);
    setHangDisplay();
    // fetch부터 render까지 microtask 체인을 타이머 단위로 비운다
    for (let i = 0; i < 8; i++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
    }
    expect(lastRendition).not.toBeNull();
    await act(async () => {
      vi.advanceTimersByTime(31_000);
    });
    expect(screen.getByText(/시간이 초과/)).toBeTruthy();
    vi.useRealTimers();
  }, 10_000);

  it("서버 에러(non-ok) 시 에러 메시지를 표시한다", async () => {
    globalThis.fetch.mockImplementationOnce(() =>
      Promise.resolve({
        ok: false,
        status: 500,
        text: () => Promise.resolve("boom"),
        arrayBuffer: () => Promise.resolve(mockArrayBuffer),
      }),
    );
    render(<ViewEPUB bookId={1} />);
    await waitFor(() =>
      expect(screen.getByText(/boom/)).toBeTruthy(),
    );
  });

  it("네트워크 에러를 표시한다", async () => {
    globalThis.fetch.mockImplementationOnce(() =>
      Promise.reject(new TypeError("Failed to fetch")),
    );
    render(<ViewEPUB bookId={1} />);
    await waitFor(() =>
      expect(screen.getByText(/EPUB 로딩 실패/)).toBeTruthy(),
    );
  });

  it("언마운트 시 fetch를 abort한다", () => {
    const { unmount } = render(<ViewEPUB bookId={1} />);
    unmount();
    const signal = globalThis.fetch.mock.calls[0][1].signal;
    expect(signal.aborted).toBe(true);
  });

  it("언마운트 시 rendition과 book을 destroy한다", async () => {
    const { unmount } = render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    const rendition = lastRendition;
    const book = lastBook;
    unmount();
    expect(rendition.destroy).toHaveBeenCalled();
    expect(book.destroy).toHaveBeenCalled();
  });

  it("저장된 CFI 위치로 복원한다", async () => {
    localStorageMock._set("epub_location_1", "epubcfi(/6/6!/4/2/2[c01]/1:0)");
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    expect(lastRendition.display).toHaveBeenCalledWith(
      "epubcfi(/6/6!/4/2/2[c01]/1:0)",
    );
  });

  it("preview 는 저장된 위치를 무시하고 처음부터 시작한다", async () => {
    localStorageMock._set("epub_location_1", "epubcfi(/6/x)");
    render(<ViewEPUB bookId={1} preview={true} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    expect(lastRendition.display).toHaveBeenCalledWith();
  });

  it("잘못된 저장 위치는 제거하고 처음부터 표시한다", async () => {
    localStorageMock._set("epub_location_1", "epubcfi(bad)");
    setFailNextDisplay();
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    await waitFor(() => {
      expect(lastRendition.display).toHaveBeenCalledWith();
      expect(localStorage.getItem("epub_location_1")).toBeNull();
    });
  });

  it("다음 페이지 버튼은 항상 rendition.next()를 호출한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    fireEvent.click(screen.getByRole("button", { name: "다음 페이지" }));
    expect(lastRendition.next).toHaveBeenCalled();
  });

  it("이전 페이지 버튼은 항상 rendition.prev()를 호출한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    fireEvent.click(screen.getByRole("button", { name: "이전 페이지" }));
    expect(lastRendition.prev).toHaveBeenCalled();
  });

  it("위치 계산 전에는 displayed.page/total(챕터 기준)로 표기한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    lastBook.locations.percentageFromCfi.mockReturnValue(null);
    await act(async () => {
      lastRendition._emit("relocated", {
        start: {
          cfi: "epubcfi(/6/4)",
          href: "OEBPS/Text/section1.html",
          displayed: { page: 3, total: 12 },
        },
      });
    });
    expect(screen.getByText("3 / 12")).toBeTruthy();
  });

  it("위치 계산이 끝나면 전체 진행률(%)과 슬라이더를 보여준다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    await act(async () => {
      lastRendition._emit("relocated", {
        start: { cfi: "epubcfi(/6/4)", displayed: { page: 3, total: 12 } },
      });
    });
    expect(screen.getByText("37%")).toBeTruthy();
    expect(lastBook.locations.generate).toHaveBeenCalledWith(1024);
    expect(screen.getByRole("slider", { name: "읽기 진행률" })).toBeTruthy();
  });

  it("끝 버튼은 책 전체 비율 1 위치로 이동한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    await act(async () => {
      lastRendition._emit("relocated", {
        start: { cfi: "epubcfi(/6/4)", displayed: { page: 3, total: 12 } },
      });
    });
    fireEvent.click(screen.getByRole("button", { name: "책의 끝으로" }));
    expect(lastBook.locations.cfiFromPercentage).toHaveBeenCalledWith(1);
    expect(lastRendition.display).toHaveBeenLastCalledWith("epubcfi(ratio-1)");
  });

  it("슬라이더를 움직이면 해당 비율 위치로 이동한다(debounce)", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    await act(async () => {
      lastRendition._emit("relocated", {
        start: { cfi: "epubcfi(/6/4)", displayed: { page: 3, total: 12 } },
      });
    });
    vi.useFakeTimers();
    fireEvent.change(screen.getByRole("slider", { name: "읽기 진행률" }), {
      target: { value: "500" },
    });
    await act(async () => {
      vi.advanceTimersByTime(200);
    });
    vi.useRealTimers();
    expect(lastRendition.display).toHaveBeenLastCalledWith("epubcfi(ratio-0.5)");
  });

  it("캐시된 위치가 있으면 다시 계산하지 않는다", async () => {
    localStorage.setItem("epub_locations_1", '["cfi0","cfi1"]');
    render(<ViewEPUB bookId={1} />);
    await openBook();
    expect(lastBook.locations.load).toHaveBeenCalledWith('["cfi0","cfi1"]');
    expect(lastBook.locations.generate).not.toHaveBeenCalled();
    localStorage.removeItem("epub_locations_1");
  });

  it("스페이스·아래·오른쪽 키는 다음 페이지로 간다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    for (const key of [" ", "ArrowDown", "ArrowRight"]) {
      fireEvent.keyDown(document, { key });
    }
    expect(lastRendition.next).toHaveBeenCalledTimes(3);
  });

  it("백스페이스·위·왼쪽 키는 이전 페이지로 간다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    for (const key of ["Backspace", "ArrowUp", "ArrowLeft"]) {
      fireEvent.keyDown(document, { key });
    }
    expect(lastRendition.prev).toHaveBeenCalledTimes(3);
  });

  it("PageDown/PageUp은 10페이지씩 이동한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    await act(async () => {
      fireEvent.keyDown(document, { key: "PageDown" });
    });
    expect(lastRendition.next).toHaveBeenCalledTimes(10);
    await act(async () => {
      fireEvent.keyDown(document, { key: "PageUp" });
    });
    expect(lastRendition.prev).toHaveBeenCalledTimes(10);
  });

  it("Home은 첫 페이지, End는 마지막 위치로 이동한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    fireEvent.keyDown(document, { key: "Home" });
    expect(lastRendition.display).toHaveBeenLastCalledWith(0);
    fireEvent.keyDown(document, { key: "End" });
    expect(lastRendition.display).toHaveBeenLastCalledWith("epubcfi(ratio-1)");
  });

  it("슬라이더에 포커스가 있으면 키를 가로채지 않는다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    await act(async () => {
      lastRendition._emit("relocated", {
        start: { cfi: "epubcfi(/6/4)", displayed: { page: 3, total: 12 } },
      });
    });
    fireEvent.keyDown(screen.getByRole("slider", { name: "읽기 진행률" }), {
      key: "ArrowRight",
    });
    expect(lastRendition.next).not.toHaveBeenCalled();
  });

  it("Ctrl 조합 키는 무시한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    fireEvent.keyDown(document, { key: "ArrowLeft", ctrlKey: true });
    expect(lastRendition.prev).not.toHaveBeenCalled();
  });

  it("preview에서는 위치 계산을 하지 않는다", async () => {
    render(<ViewEPUB bookId={1} preview />);
    await openBook();
    expect(lastBook.locations.generate).not.toHaveBeenCalled();
  });

  it("relocated 이후 읽기 위치(CFI)가 저장된다(throttle)", async () => {
    vi.useFakeTimers();
    render(<ViewEPUB bookId={1} />);
    await openBookWithoutTimers();
    await act(async () => {
      lastRendition._emit("relocated", {
        start: {
          cfi: "epubcfi(/6/4)",
          href: "OEBPS/Text/section1.html",
          displayed: { page: 3, total: 12 },
        },
      });
    });
    await act(async () => {
      vi.advanceTimersByTime(250);
    });
    expect(localStorageMock.setItem).toHaveBeenCalled();
    expect(localStorageMock.setItem.mock.calls.at(-1)[1]).toBe(
      "epubcfi(/6/4)",
    );
    vi.useRealTimers();
  });

  it("preview는 읽기 위치를 저장하지 않는다", async () => {
    vi.useFakeTimers();
    render(<ViewEPUB bookId={1} preview={true} />);
    await openBookWithoutTimers();
    await act(async () => {
      lastRendition._emit("relocated", {
        start: {
          cfi: "epubcfi(/6/4)",
          href: "OEBPS/Text/section1.html",
          displayed: { page: 3, total: 12 },
        },
      });
    });
    await act(async () => {
      vi.advanceTimersByTime(300);
    });
    expect(localStorageMock.setItem).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("본문에 touch-action: pan-y를 건다(터치 팬 차단, 그리기 유지)", async () => {
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    const hook = lastRendition.hooks.content.register.mock.calls[0][0];
    const contentDocument = document.implementation.createHTMLDocument();

    hook({ document: contentDocument });

    expect(contentDocument.documentElement.style.getPropertyValue("touch-action")).toBe(
      "pan-y",
    );
    expect(contentDocument.body.style.getPropertyValue("touch-action")).toBe(
      "pan-y",
    );
    // overflow는 잠그지 않는다 — 본문의 넘치는 컬럼이 그려져야 한다
    expect(contentDocument.body.style.overflow).toBe("");
  });

  it("rendered 이후에도 touch-action을 다시 건다", async () => {
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    const contentDocument = document.implementation.createHTMLDocument();
    const view = {
      contents: { document: contentDocument },
      expand: vi.fn(),
    };

    act(() => lastRendition._emit("rendered", null, view));

    expect(contentDocument.body.style.getPropertyValue("touch-action")).toBe(
      "pan-y",
    );
  });

  it("툴바와 페이지 정보는 preview에서 숨긴다", async () => {
    render(<ViewEPUB bookId={1} preview={true} />);
    await openBook();
    expect(screen.queryByLabelText("글자 크기 늘리기")).toBeNull();
    expect(screen.queryByTestId("epub-page-info")).toBeNull();
  });

  it("A+ 클릭 시 themes.fontSize가 반영되고 localStorage에 저장된다", async () => {
    render(<ViewEPUB bookId={1} standalone={true} />);
    await openBook();
    fireEvent.click(screen.getByRole("button", { name: "글자 크기 늘리기" }));
    expect(lastRendition.themes.fontSize).toHaveBeenCalledWith("120%");
    expect(localStorage.getItem("epub_fontSize")).toBe("120");
  });

  it("글자 크기가 최소(80%)이면 A- 버튼이 비활성화된다", async () => {
    localStorageMock.clear();
    localStorageMock._set("epub_fontSize", "80");
    render(<ViewEPUB bookId={1} />);
    await openBook();
    const decreaseButton = screen.getByRole("button", {
      name: "글자 크기 줄이기",
    });
    expect(decreaseButton.disabled).toBe(true);
  });

  it("글꼴을 바꾸면 본문 텍스트 요소에 !important 규칙을 주입하고 localStorage에 저장된다", async () => {
    render(<ViewEPUB bookId={1} standalone={true} />);
    await openBook();
    expect(screen.queryByRole("option", { name: "Serif" })).toBeNull();
    expect(screen.queryByRole("option", { name: "Sans-serif" })).toBeNull();
    expect(screen.getByRole("option", { name: "나눔바른고딕" })).toBeTruthy();
    fireEvent.change(screen.getByLabelText("글꼴 선택"), {
      target: { value: "'Noto Sans CJK KR', sans-serif" },
    });
    const hook = lastRendition.hooks.content.register.mock.calls.at(-1)[0];
    const contentDocument = document.implementation.createHTMLDocument();
    hook({ document: contentDocument });
    const styleEl = contentDocument.getElementById(
      "epub-font-family-override",
    );
    expect(styleEl?.textContent).toContain("sans-serif");
    expect(styleEl?.textContent).toContain("!important");
    expect(styleEl?.textContent).toContain("p");
    expect(localStorage.getItem("epub_fontFamily")).toBe(
      "'Noto Sans CJK KR', sans-serif",
    );
  });

  it("글꼴을 기본으로 바꾸면 주입한 규칙을 제거한다", async () => {
    localStorageMock._set("epub_fontFamily", "'Noto Sans CJK KR', sans-serif");
    render(<ViewEPUB bookId={1} standalone={true} />);
    await openBook();
    fireEvent.change(screen.getByLabelText("글꼴 선택"), {
      target: { value: "" },
    });
    const hook = lastRendition.hooks.content.register.mock.calls.at(-1)[0];
    const contentDocument = document.implementation.createHTMLDocument();
    const styleEl = contentDocument.createElement("style");
    styleEl.id = "epub-font-family-override";
    contentDocument.head.appendChild(styleEl);
    hook({ document: contentDocument });
    expect(
      contentDocument.getElementById("epub-font-family-override"),
    ).toBeNull();
  });

  it("콘텐츠 문서에 웹폰트 @font-face를 주입한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    const hook = lastRendition.hooks.content.register.mock.calls.at(-1)[0];
    const contentDocument = document.implementation.createHTMLDocument();
    hook({ document: contentDocument });
    const faceEl = contentDocument.getElementById("epub-font-face-override");
    expect(faceEl?.textContent).toContain("@font-face");
    expect(faceEl?.textContent).toContain("'Nanum Gothic'");
    expect(faceEl?.textContent).toContain("'Nanum Myeongjo'");
    expect(faceEl?.textContent).toContain("'Nanum Barun Gothic'");
    expect(faceEl?.textContent).toContain("NanumBarunGothicSubset.woff2");
    expect(faceEl?.textContent).toContain("NanumBarunGothicBoldSubset.woff2");
    expect(faceEl?.textContent).toContain("NanumBarunGothicLightSubset.woff2");
    expect(faceEl?.textContent).toContain("NanumBarunGothicUltraLightSubset.woff2");
    expect(faceEl?.textContent).toContain("'Noto Serif CJK KR'");
    expect(faceEl?.textContent).toContain("U+4E00-9FFF");
    expect(faceEl?.textContent).toContain("/fonts/noto-serif-kr-400.woff2");
    expect(faceEl?.textContent).toContain("/fonts/noto-sans-kr-400.woff2");
    expect(faceEl?.textContent).toContain("/fonts/nanum-gothic-korean-400.woff2");
    expect(faceEl?.textContent).toContain("unicode-range");
  });

  it("전체보기 뷰어는 툴바가 가리지 않게 여백 클래스를 넣는다", () => {
    const { container } = render(<ViewEPUB bookId={1} />);
    expect(container.firstChild.className).toContain("epub-viewer--framed");
  });

  it("preview 뷰어는 여백 클래스를 넣지 않는다", () => {
    const { container } = render(<ViewEPUB bookId={1} preview={true} />);
    expect(container.firstChild.className).not.toContain(
      "epub-viewer--framed",
    );
  });

  it("이전 나눔명조 설정을 한자 fallback을 포함한 값으로 이어받는다", async () => {
    localStorageMock._set("epub_fontFamily", "'Nanum Myeongjo', serif");
    render(<ViewEPUB bookId={1} standalone={true} />);
    await openBook();
    expect(screen.getByLabelText("글꼴 선택").value).toBe(
      "'Nanum Myeongjo', 'Noto Serif CJK KR', serif",
    );
  });

  it("독자 위치가 저장된 글꼴 크기가 첫 렌더에 적용된다", async () => {
    localStorageMock._set("epub_fontSize", "120");
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    expect(lastRendition.themes.fontSize).toHaveBeenCalledWith("120%");
  });

  it("키보드 방향키 우로도 다음 페이지로 간다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    containerMetrics = { scrollTop: 1950, scrollHeight: 2000, clientHeight: 500 };
    fireEvent.keyDown(document, { key: "ArrowRight" });
    expect(lastRendition.next).toHaveBeenCalled();
  });

  it("iframe 내부 keyup(ArrowRight)도 다음 페이지로 간다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    containerMetrics = { scrollTop: 1950, scrollHeight: 2000, clientHeight: 500 };
    act(() => lastRendition._emit("keyup", { key: "ArrowRight" }));
    expect(lastRendition.next).toHaveBeenCalled();
  });

  it("standalone이면 100% 높이다", () => {
    const { container } = render(<ViewEPUB bookId={1} standalone={true} />);
    expect(container.firstChild.style.height).toBe("100%");
  });

  it("preview면 60vh 높이다", () => {
    const { container } = render(<ViewEPUB bookId={1} preview={true} />);
    expect(container.firstChild.style.height).toBe("60vh");
  });

  it("내장 전체보기는 100dvh 높이다", () => {
    const { container } = render(<ViewEPUB bookId={1} />);
    expect(container.firstChild.style.height).toBe("100dvh");
  });

  it("세로모드(portrait)에서는 spread를 none으로 강제한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    expect(lastRendition.spread).toHaveBeenCalledWith("none", undefined);
  });

  it("가로모드(landscape)에서는 spread를 always(minSpreadWidth 1)로 강제한다", async () => {
    window.matchMedia("(orientation: landscape)").matches = true;
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    expect(lastRendition.spread).toHaveBeenCalledWith("always", 1);
  });

  it("방향 전환 시 spread를 다시 강제한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    lastRendition.spread.mockClear();
    mqMock.matches = true;
    await act(async () => {
      mqListeners.forEach((h) => h());
    });
    expect(lastRendition.spread).toHaveBeenCalledWith("always", 1);
  });

  it("matchMedia change 리스너는 언마운트 시 정리한다", async () => {
    const { unmount } = render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    expect(mqMock.removeEventListener).not.toHaveBeenCalled();
    unmount();
    expect(mqMock.removeEventListener).toHaveBeenCalled();
  });

  // ── 방어 분기 ──

  it("타이머 id가 비어 있는 환경에서도 표시·오류·위치 이벤트와 정리가 안전하다", async () => {
    const realSetTimeout = globalThis.setTimeout;
    vi.spyOn(globalThis, "setTimeout").mockImplementation((fn, ms, ...args) =>
      ms === 30_000 ? 0 : realSetTimeout(fn, ms, ...args),
    );
    const { unmount } = render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition?.display).toHaveBeenCalled());

    await act(async () => {
      lastRendition._emit("displayed", {});
      lastRendition._emit("relocated", {});
      lastRendition._emit("displayerror", {});
      lastRendition._emit("error", {});
    });
    expect(screen.getByText(/EPUB 파싱 오류/)).toBeTruthy();

    unmount();
    expect(lastRendition.destroy).toHaveBeenCalled();
  });

  it("복구 표시 실패의 오류 객체에 message가 없으면 문자열로 바꿔 표시한다", async () => {
    const realSetTimeout = globalThis.setTimeout;
    vi.spyOn(globalThis, "setTimeout").mockImplementation((fn, ms, ...args) =>
      ms === 30_000 ? 0 : realSetTimeout(fn, ms, ...args),
    );
    rejectReady = true;
    failEveryDisplay = true;
    failDisplayError = "문자열 오류";
    render(<ViewEPUB bookId={1} />);

    expect(await screen.findByText("EPUB 표시 실패: 문자열 오류")).toBeTruthy();
  });

  it("표지 함수가 없는 책도 정상 경로에서는 표지 없이 본문을 표시한다", async () => {
    omitCoverUrl = true;
    render(<ViewEPUB bookId={1} />);
    await openBook();

    expect(screen.queryByTestId("epub-cover-page")).toBeNull();
    expect(lastRendition.display).toHaveBeenCalledWith();
  });

  it("뷰 목록이 비어 있는 매니저에서도 글꼴 변경이 안전하다", async () => {
    mockViews = null;
    render(<ViewEPUB bookId={1} />);
    await openBook();

    fireEvent.change(screen.getByLabelText("글꼴 선택"), {
      target: { value: "'Noto Sans CJK KR', sans-serif" },
    });

    expect(lastRendition.themes.fontSize).toHaveBeenCalled();
  });

  it("이미 주입된 글꼴 스타일 요소는 새로 만들지 않고 갱신한다", async () => {
    const contentDocument = document.implementation.createHTMLDocument();
    mockViews = [{ contents: { document: contentDocument }, expand: vi.fn() }];
    render(<ViewEPUB bookId={1} />);
    await openBook();
    const select = screen.getByLabelText("글꼴 선택");

    fireEvent.change(select, { target: { value: "'Noto Sans CJK KR', sans-serif" } });
    fireEvent.change(select, { target: { value: "'Noto Serif CJK KR', serif" } });

    expect(contentDocument.querySelectorAll("#epub-font-face-override")).toHaveLength(1);
    expect(contentDocument.querySelectorAll("#epub-font-family-override")).toHaveLength(1);
    expect(contentDocument.getElementById("epub-font-family-override").textContent).toContain("serif");
  });

  it("images 목록이 없는 본문 문서도 다시 확장한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastRendition).not.toBeNull());
    const view = { contents: { document: { body: document.createElement("body") } }, expand: vi.fn() };

    expect(() => act(() => lastRendition._emit("rendered", {}, view))).not.toThrow();
    expect(view.expand).toHaveBeenCalled();
  });

  it("preview 표지에서 다음 키를 누르면 위치를 저장하지 않고 본문으로 넘어간다", async () => {
    mockCoverUrl = "blob:epub-cover";
    render(<ViewEPUB bookId={1} preview />);
    await openBook();
    expect(screen.getByTestId("epub-cover-page")).toBeTruthy();

    fireEvent.keyDown(document, { key: "ArrowRight" });

    await waitFor(() => expect(screen.queryByTestId("epub-cover-page")).toBeNull());
    expect(localStorageMock.setItem).not.toHaveBeenCalledWith(
      expect.stringMatching(/^epub_location_/),
      expect.anything(),
    );
  });

  it("방향키가 아닌 키는 무시한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();

    fireEvent.keyDown(document, { key: "x" });

    expect(lastRendition.next).not.toHaveBeenCalled();
    expect(lastRendition.prev).not.toHaveBeenCalled();
  });

  it("서버가 빈 오류 본문을 주면 상태 코드로 오류를 표시한다", async () => {
    globalThis.fetch.mockImplementationOnce(() =>
      Promise.resolve({
        ok: false,
        status: 502,
        text: () => Promise.resolve(""),
        arrayBuffer: () => Promise.resolve(mockArrayBuffer),
      }),
    );
    render(<ViewEPUB bookId={1} />);

    expect(await screen.findByText(/서버 응답 오류: 502/)).toBeTruthy();
  });

  it("요청 중단(AbortError)은 오류로 표시하지 않는다", async () => {
    globalThis.fetch.mockImplementationOnce(() =>
      Promise.reject(new DOMException("aborted", "AbortError")),
    );
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(globalThis.fetch).toHaveBeenCalled());
    await act(async () => {});

    expect(screen.queryByText(/EPUB 로딩 실패/)).toBeNull();
  });
});
