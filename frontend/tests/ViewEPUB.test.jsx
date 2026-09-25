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

let failNextDisplay = false;
let hangDisplay = false;

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
    manager: { stage: { container } },
  };
  return rendition;
}

function createMockBook() {
  return {
    ready: Promise.resolve(),
    loaded: { metadata: Promise.resolve({ title: "테스트 책 제목" }) },
    spine: Array.from({ length: 12 }, (_, i) => ({
      href: `OEBPS/Text/section${i}.html`,
      index: i,
    })),
    renderTo: vi.fn(() => {
      lastRendition = createMockRendition();
      lastRendition.book = lastBook;
      return lastRendition;
    }),
    destroy: vi.fn(),
  };
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
    failNextDisplay = false;
    hangDisplay = false;
    localStorageMock.clear();
    localStorageMock.getItem.mockClear();
    localStorageMock.setItem.mockClear();
    globalThis.fetch.mockClear();
    containerMetrics = { scrollTop: 0, scrollHeight: 2000, clientHeight: 500 };
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

  it("초기 로딩 시 스피너를 표시한다", () => {
    render(<ViewEPUB bookId={1} />);
    expect(screen.getByText("로딩 중...")).toBeTruthy();
  });

  it("displayed 이벤트가 오면 로딩이 풀린다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    await waitFor(() =>
      expect(screen.queryByText("로딩 중...")).toBeNull(),
    );
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

  it("relocated의 displayed.page/total로 페이지 정보를 갱신한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
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

  it("글꼴을 바꾸면 themes.font가 반영되고 localStorage에 저장된다", async () => {
    render(<ViewEPUB bookId={1} standalone={true} />);
    await openBook();
    fireEvent.change(screen.getByLabelText("글꼴 선택"), {
      target: { value: "sans-serif" },
    });
    expect(lastRendition.themes.font).toHaveBeenCalledWith("sans-serif");
    expect(localStorage.getItem("epub_fontFamily")).toBe("sans-serif");
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

});
