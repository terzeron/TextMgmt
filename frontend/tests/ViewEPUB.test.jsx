// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import React from "react";
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
let firstSpineIdref = "";
let deferGenerate = false;
let resolveGenerate = null;

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
      generate: vi.fn(() =>
        deferGenerate
          ? new Promise((resolve) => {
              resolveGenerate = () => resolve([]);
            })
          : Promise.resolve([]),
      ),
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
  book.spine.hooks = { content: { register: vi.fn() } };
  if (firstSpineIdref) book.spine.get = vi.fn(() => ({ idref: firstSpineIdref }));
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
vi.mock("../src/EpubCoverModal", () => ({
  default: function EpubCoverModalStub({ hasCover, initialUrl, onSubmit, onClose }) {
    const [message, setMessage] = React.useState("");
    return (
      <div
        data-testid="cover-modal"
        data-has-cover={String(hasCover)}
        data-initial-url={initialUrl || ""}
      >
        <button
          onClick={async () =>
            setMessage((await onSubmit("https://img.example.com/c.jpg")) || "")
          }
        >
          삽입 표시
        </button>
        <button
          onClick={async () =>
            setMessage(
              (await onSubmit(new File(["x"], "cover.jpg", { type: "image/jpeg" }))) || "",
            )
          }
        >
          파일 표시
        </button>
        <button onClick={onClose}>모달 닫기</button>
        {message && <span>{message}</span>}
      </div>
    );
  },
}));

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
    firstSpineIdref = "";
    deferGenerate = false;
    resolveGenerate = null;
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

  it("EPUB 은 HTTP 캐시를 재검증해서 받는다(저장 뒤 옛 파일이 보이지 않게)", async () => {
    render(<ViewEPUB bookId={7} />);
    await waitFor(() => expect(globalThis.fetch).toHaveBeenCalledTimes(1));
    // 표지 저장은 EPUB 의 mtime 을 되돌린다. 재검증하지 않으면 브라우저가 휴리스틱 캐시로 저장 전 응답을 재사용한다.
    expect(globalThis.fetch.mock.calls[0][1].cache).toBe("no-cache");
  });

  it("렌더링 전에 EPUB의 실행 코드를 제거하고 CSP를 첫 항목으로 넣는다", async () => {
    render(<ViewEPUB bookId={7} />);
    await waitFor(() => expect(lastBook?.renderTo).toHaveBeenCalled());
    const secure = lastBook.spine.hooks.content.register.mock.calls[0][0];
    const doc = new DOMParser().parseFromString(`<html xmlns="http://www.w3.org/1999/xhtml"><head>
      <script>parent.attacked = true;</script><meta http-equiv="refresh" content="0;url=evil.html" />
      <meta name="viewport" content="width=600,height=800" /><link rel="stylesheet" href="book.css" />
      </head><body onload="alert(1)"><p epub:type="pagebreak">본문</p><img src="image.jpg" onerror="alert(1)" />
      <iframe srcdoc="bad"></iframe><a href="javascript:alert(1)">링크</a></body></html>`, "text/html");
    await secure(doc);
    const policy = doc.head.firstElementChild;
    expect(policy.getAttribute("http-equiv")).toBe("Content-Security-Policy");
    expect(policy.content).toContain("script-src 'none'");
    expect(policy.content).toContain("frame-src 'none'");
    expect(doc.querySelector('script, iframe, [onload], [onerror], meta[http-equiv="refresh"]')).toBeNull();
    expect(doc.querySelector("a").hasAttribute("href")).toBe(false);
    expect(doc.querySelector("p").textContent).toBe("본문");
    expect(doc.querySelector("img").getAttribute("src")).toBe("image.jpg");
    expect(doc.querySelector("link").getAttribute("href")).toBe("book.css");
    expect(doc.querySelector('meta[name="viewport"]').content).toBe("width=600,height=800");
  });

  it("보안 정책을 넣을 수 없는 본문은 렌더링을 거부한다", async () => {
    render(<ViewEPUB bookId={7} />);
    await openBook();
    const secure = lastBook.spine.hooks.content.register.mock.calls[0][0];
    const doc = new DOMParser().parseFromString("<html><body>본문</body></html>", "application/xhtml+xml");
    await expect(secure(doc)).rejects.toThrow("head");
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

  it("spine 첫 항목이 도구가 넣은 표지 페이지면 표지 오버레이를 띄우지 않는다", async () => {
    mockCoverUrl = "blob:epub-cover";
    firstSpineIdref = "tm-cover-page-5a991dcf381ed4bf";
    render(<ViewEPUB bookId={1} />);
    await openBook();

    expect(lastBook.coverUrl).toHaveBeenCalledOnce();
    expect(screen.queryByTestId("epub-cover-page")).toBeNull();
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

  describe("가로 스와이프", () => {
    let swipeViews;
    beforeEach(() => { swipeViews = new Map(); });
    const touch = (x, y = 100, identifier = 1) => ({
      clientX: x, clientY: y, identifier,
    });
    const emit = (type, touches, target, changedTouches = touches) => {
      const doc = target.ownerDocument;
      if (doc !== document && !swipeViews.has(doc)) {
        const view = { contents: { document: doc } };
        swipeViews.set(doc, view);
        act(() => lastRendition._emit("rendered", {}, view));
      }
      const event = new Event(type, { bubbles: true, cancelable: true });
      Object.defineProperties(event, { touches: { value: touches }, changedTouches: { value: changedTouches } });
      act(() => target.dispatchEvent(event));
      return event;
    };

    it("가로 이동의 기본 스크롤을 막고 손을 뗄 때 한 번만 넘긴다", async () => {
      render(<ViewEPUB bookId={1} />);
      await openBook();
      const doc = document.implementation.createHTMLDocument();
      emit("touchstart", [touch(250)], doc.body);
      const move = emit("touchmove", [touch(180, 110)], doc.body);
      expect(move.defaultPrevented).toBe(true);
      expect(lastRendition.next).not.toHaveBeenCalled();
      const end = emit("touchend", [], doc.body, [touch(100, 110)]);
      expect(end.defaultPrevented).toBe(true);
      expect(lastRendition.next).toHaveBeenCalledTimes(1);
    });

    it("세로 이동은 기본 스크롤을 막지 않는다", async () => {
      render(<ViewEPUB bookId={1} />);
      await openBook();
      const doc = document.implementation.createHTMLDocument();
      emit("touchstart", [touch(250)], doc.body);
      expect(emit("touchmove", [touch(240, 200)], doc.body).defaultPrevented).toBe(false);
      emit("touchend", [], doc.body, [touch(240, 250)]);
      expect(lastRendition.next).not.toHaveBeenCalled();
    });

    it("섹션을 다시 표시해도 중복 처리하지 않고 제거한 문서는 해제한다", async () => {
      render(<ViewEPUB bookId={1} />);
      await openBook();
      const doc = document.implementation.createHTMLDocument();
      emit("touchstart", [touch(250)], doc.body);
      const view = swipeViews.get(doc);
      act(() => lastRendition._emit("rendered", {}, view));
      emit("touchend", [], doc.body, [touch(100)]);
      expect(lastRendition.next).toHaveBeenCalledTimes(1);
      act(() => lastRendition._emit("removed", {}, view));
      emit("touchstart", [touch(250)], doc.body);
      emit("touchend", [], doc.body, [touch(100)]);
      expect(lastRendition.next).toHaveBeenCalledTimes(1);
    });

    it.each([false, true])("iframe에서 좌우로 한 쪽씩 넘긴다 (preview=%s)", async (preview) => {
      render(<ViewEPUB bookId={1} preview={preview} />);
      await openBook();
      const doc = document.implementation.createHTMLDocument();
      emit("touchstart", [touch(250)], doc.body);
      emit("touchend", [], doc.body, [touch(100)]);
      expect(lastRendition.next).toHaveBeenCalledTimes(1);
      emit("touchstart", [touch(100)], doc.body);
      emit("touchend", [], doc.body, [touch(250)]);
      expect(lastRendition.prev).toHaveBeenCalledTimes(1);
    });

    it("표지 위 스와이프는 본문을 건너뛰지 않고 표지만 닫는다", async () => {
      mockCoverUrl = "blob:cover";
      render(<ViewEPUB bookId={1} />);
      await openBook();
      const host = screen.getByTestId("epub-host");
      fireEvent.touchStart(host, { touches: [touch(250)] });
      fireEvent.touchEnd(host, { touches: [], changedTouches: [touch(100)] });
      expect(screen.queryByTestId("epub-cover-page")).toBeNull();
      expect(lastRendition.next).not.toHaveBeenCalled();
    });

    it.each([
      ["짧은 터치", touch(230)],
      ["세로 이동", touch(240, 250)],
      ["대각선 이동", touch(150, 200)],
      ["다른 손가락", touch(100, 100, 2)],
    ])("%s는 넘기지 않는다", async (_name, end) => {
      render(<ViewEPUB bookId={1} />);
      await openBook();
      const host = screen.getByTestId("epub-host");
      emit("touchstart", [touch(250)], host);
      emit("touchend", [], host, [end]);
      expect(lastRendition.next).not.toHaveBeenCalled();
      expect(lastRendition.prev).not.toHaveBeenCalled();
    });

    it.each(["touchcancel", "multitouch", "vertical"])("%s 후에는 남은 손가락으로 넘기지 않는다", async (reason) => {
      render(<ViewEPUB bookId={1} />);
      await openBook();
      const doc = document.implementation.createHTMLDocument();
      emit("touchstart", [touch(250)], doc.body);
      if (reason === "touchcancel") doc.body.dispatchEvent(new Event("touchcancel", { bubbles: true }));
      else emit("touchmove", reason === "multitouch"
        ? [touch(240), touch(200, 100, 2)] : [touch(240, 200)], doc.body);
      emit("touchend", [], doc.body, [touch(100)]);
      expect(lastRendition.next).not.toHaveBeenCalled();
    });

    it.each(["a", "button", "input", "select", "textarea"])("%s 조작은 페이지를 넘기지 않는다", async (tag) => {
      render(<ViewEPUB bookId={1} />);
      await openBook();
      const target = document.createElement(tag);
      screen.getByTestId("epub-host").appendChild(target);
      emit("touchstart", [touch(250)], target);
      emit("touchend", [], target, [touch(100)]);
      expect(lastRendition.next).not.toHaveBeenCalled();
    });

    it("뷰어를 닫으면 진행 중인 스와이프와 이벤트를 정리한다", async () => {
      const { unmount } = render(<ViewEPUB bookId={1} />);
      await openBook();
      const host = screen.getByTestId("epub-host");
      const rendition = lastRendition;
      emit("touchstart", [touch(250)], host);
      unmount();
      emit("touchend", [], host, [touch(100)]);
      fireEvent.touchStart(host, { touches: [touch(250)] });
      fireEvent.touchEnd(host, { touches: [], changedTouches: [touch(100)] });
      expect(rendition.next).not.toHaveBeenCalled();
      expect(rendition.off).toHaveBeenCalledWith("rendered", expect.any(Function));
      expect(rendition.off).toHaveBeenCalledWith("removed", expect.any(Function));
    });
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

  // ─── 커버리지 보강: 표지·진행 이동·키 입력의 경계 경로 ───────────
  async function openWithProgress() {
    await openBook();
    await act(async () => {
      lastRendition._emit("relocated", {
        start: { cfi: "epubcfi(/6/4)", displayed: { page: 3, total: 12 } },
      });
    });
  }

  it("책의 처음 버튼은 비율 0 위치로 이동한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openWithProgress();
    fireEvent.click(screen.getByRole("button", { name: "책의 처음으로" }));
    expect(lastBook.locations.cfiFromPercentage).toHaveBeenCalledWith(0);
    expect(lastRendition.display).toHaveBeenLastCalledWith("epubcfi(ratio-0)");
  });

  it("위치 변환이 실패(-1 또는 빈 값)하면 이동하지 않는다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openWithProgress();
    const before = lastRendition.display.mock.calls.length;

    lastBook.locations.cfiFromPercentage.mockReturnValueOnce(-1);
    fireEvent.click(screen.getByRole("button", { name: "책의 끝으로" }));
    lastBook.locations.cfiFromPercentage.mockReturnValueOnce(null);
    fireEvent.click(screen.getByRole("button", { name: "책의 끝으로" }));

    expect(lastRendition.display.mock.calls.length).toBe(before);
  });

  it("위치 목록이 비어 있으면 End 키로 이동하지 않는다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    lastBook.locations.total = 0;
    const before = lastRendition.display.mock.calls.length;

    fireEvent.keyDown(document, { key: "End" });

    expect(lastRendition.display.mock.calls.length).toBe(before);
    expect(lastBook.locations.cfiFromPercentage).not.toHaveBeenCalled();
  });

  it("표지 화면에서 진행 이동을 하면 표지를 먼저 닫는다", async () => {
    mockCoverUrl = "blob:epub-cover";
    render(<ViewEPUB bookId={1} />);
    await openWithProgress();
    expect(screen.queryByTestId("epub-cover-page")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "책의 끝으로" }));

    expect(screen.queryByTestId("epub-cover-page")).toBeNull();
    expect(lastRendition.display).toHaveBeenLastCalledWith("epubcfi(ratio-1)");
  });

  it("슬라이더를 연속으로 움직이면 이전 debounce 타이머를 취소한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openWithProgress();
    vi.useFakeTimers();
    const slider = screen.getByRole("slider", { name: "읽기 진행률" });
    fireEvent.change(slider, { target: { value: "200" } });
    fireEvent.change(slider, { target: { value: "700" } });
    await act(async () => {
      vi.advanceTimersByTime(500);
    });
    vi.useRealTimers();

    expect(lastBook.locations.cfiFromPercentage).toHaveBeenCalledTimes(1);
    expect(lastRendition.display).toHaveBeenLastCalledWith("epubcfi(ratio-0.7)");
  });

  it("표지 화면에서 PageDown은 표지만 넘기고 PageUp은 무시한다", async () => {
    mockCoverUrl = "blob:epub-cover";
    render(<ViewEPUB bookId={1} />);
    await openBook();

    await act(async () => {
      fireEvent.keyDown(document, { key: "PageUp" });
    });
    expect(screen.queryByTestId("epub-cover-page")).toBeTruthy();

    await act(async () => {
      fireEvent.keyDown(document, { key: "PageDown" });
    });
    expect(screen.queryByTestId("epub-cover-page")).toBeNull();
    expect(lastRendition.next).not.toHaveBeenCalled();
    expect(lastRendition.prev).not.toHaveBeenCalled();
  });

  it("페이지 점프 중 들어온 추가 PageDown은 무시한다", async () => {
    render(<ViewEPUB bookId={1} />);
    await openBook();
    lastRendition.next.mockImplementation(() => new Promise(() => {}));

    await act(async () => {
      fireEvent.keyDown(document, { key: "PageDown" });
      fireEvent.keyDown(document, { key: "PageDown" });
    });

    expect(lastRendition.next).toHaveBeenCalledTimes(1);
  });

  it("표지 화면에서 Home은 본문 첫 페이지로 이동하지 않는다", async () => {
    mockCoverUrl = "blob:epub-cover";
    render(<ViewEPUB bookId={1} />);
    await openBook();
    const before = lastRendition.display.mock.calls.length;

    fireEvent.keyDown(document, { key: "Home" });

    expect(lastRendition.display.mock.calls.length).toBe(before);
  });

  it("위치 계산이 끝나기 전에 언마운트하면 결과를 저장하지 않는다", async () => {
    deferGenerate = true;
    const { unmount } = render(<ViewEPUB bookId={1} />);
    await waitFor(() => expect(lastBook?.locations.generate).toHaveBeenCalled());
    const book = lastBook;

    unmount();
    await act(async () => {
      resolveGenerate();
    });

    expect(book.locations.save).not.toHaveBeenCalled();
  });

  it("위치 캐시를 읽다 예외가 나도 위치를 새로 계산한다", async () => {
    localStorageMock.getItem.mockImplementation((key) => {
      if (String(key).startsWith("epub_locations_")) throw new Error("denied");
      return localStorageMock._get(key);
    });
    render(<ViewEPUB bookId={1} />);
    await openBook();

    await waitFor(() => expect(lastBook.locations.generate).toHaveBeenCalled());
    expect(lastBook.locations.load).not.toHaveBeenCalled();
  });

  describe("편집 화면의 표지 이미지 버튼", () => {
    const PREVIEW_URL = "http://localhost:8000/pdf-pages/7/insert-preview";
    const SAVE_URL = "http://localhost:8000/epub-cover/7";

    // preview 요청과 저장 요청에 줄 응답. 나머지(EPUB 본문)는 기본 응답을 쓴다.
    const respond = ({ preview, save } = {}) => {
      const base = globalThis.fetch.getMockImplementation();
      globalThis.fetch.mockImplementation((url, init) => {
        if (url === PREVIEW_URL && preview) return Promise.resolve(preview);
        if (url === SAVE_URL && save) return Promise.resolve(save);
        return base(url, init);
      });
    };
    const blobResponse = () => ({
      ok: true,
      status: 200,
      blob: () => Promise.resolve(new Blob(["x"], { type: "image/jpeg" })),
    });
    const textResponse = (status, text) => ({
      ok: false,
      status,
      text: () => Promise.resolve(text),
    });
    const jsonResponse = (body) => ({
      ok: true,
      status: 200,
      json: () => Promise.resolve(body),
    });

    // 표지가 있는 책을 열고 새 표지를 "삽입 표시"까지 해 둔다.
    async function openWithPendingCover() {
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={7} preview editable />);
      await openBook();
      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
      fireEvent.click(screen.getByText("삽입 표시"));
      await screen.findByTestId("epub-cover-pending");
    }

    beforeEach(() => {
      globalThis.URL.createObjectURL = vi.fn(() => "blob:new-cover");
      globalThis.URL.revokeObjectURL = vi.fn();
    });

    it("editable 이 아니면 표지 버튼을 그리지 않는다", async () => {
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={1} />);
      await openBook();

      expect(screen.queryByRole("button", { name: /표지 이미지/ })).toBeNull();
    });

    it("표지가 있는 첫 쪽에는 교체 버튼을 둔다", async () => {
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={1} preview editable />);
      await openBook();

      expect(
        screen.getByRole("button", { name: "표지 이미지 교체" }),
      ).toBeTruthy();
      expect(screen.queryByRole("button", { name: "저장" })).toBeNull();
    });

    it("표지가 없어도 책의 첫 위치에서는 추가 버튼을 둔다", async () => {
      render(<ViewEPUB bookId={1} preview editable />);
      await openBook();
      await act(async () => {
        lastRendition._emit("relocated", { atStart: true, start: {} });
      });

      expect(
        screen.getByRole("button", { name: "표지 이미지 추가" }),
      ).toBeTruthy();
    });

    it("표지를 넘겨 첫 위치를 벗어나면 버튼이 사라진다", async () => {
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={1} preview editable />);
      await openBook();
      await act(async () => {
        lastRendition._emit("relocated", { atStart: false, start: {} });
      });

      fireEvent.click(screen.getByRole("button", { name: "다음 페이지" }));

      expect(screen.queryByRole("button", { name: /표지 이미지/ })).toBeNull();
    });

    it("버튼을 누르면 모달을 열고 표지 유무를 넘긴다", async () => {
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={7} preview editable />);
      await openBook();

      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));

      expect(screen.getByTestId("cover-modal").getAttribute("data-has-cover")).toBe(
        "true",
      );
      fireEvent.click(screen.getByText("모달 닫기"));
      expect(screen.queryByTestId("cover-modal")).toBeNull();
    });

    it("삽입 표시는 insert-preview 로 받은 이미지를 저장 전 표지로 보여 준다", async () => {
      respond({ preview: blobResponse() });
      await openWithPendingCover();

      expect(screen.queryByTestId("cover-modal")).toBeNull();
      expect(screen.getByRole("img", { name: "저장 전 새 표지" }).getAttribute("src")).toBe(
        "blob:new-cover",
      );
      expect(screen.getByText("새 표지 · 저장 전")).toBeTruthy();
      const call = globalThis.fetch.mock.calls.find(([u]) => u === PREVIEW_URL);
      expect(JSON.parse(call[1].body)).toEqual({ url: "https://img.example.com/c.jpg" });
      // 저장하기 전에는 서버의 EPUB 을 건드리지 않는다.
      expect(globalThis.fetch.mock.calls.some(([u]) => u === SAVE_URL)).toBe(false);
      expect(screen.getByRole("button", { name: "저장" })).toBeTruthy();
    });

    it("미리보기 400 은 사유를 모달에 돌려주고 표시하지 않는다", async () => {
      respond({ preview: textResponse(400, "내부 주소의 이미지는 사용할 수 없습니다") });
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={7} preview editable />);
      await openBook();
      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
      fireEvent.click(screen.getByText("삽입 표시"));

      expect(await screen.findByText("내부 주소의 이미지는 사용할 수 없습니다")).toBeTruthy();
      expect(screen.getByTestId("cover-modal")).toBeTruthy();
      expect(screen.queryByTestId("epub-cover-pending")).toBeNull();
    });

    it("미리보기 400 에 본문이 없거나 다른 오류면 상태 코드를 돌려준다", async () => {
      respond({ preview: textResponse(400, "") });
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={7} preview editable />);
      await openBook();
      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
      fireEvent.click(screen.getByText("삽입 표시"));
      expect(await screen.findByText("HTTP 400")).toBeTruthy();

      respond({ preview: textResponse(502, "bad gateway") });
      fireEvent.click(screen.getByText("삽입 표시"));
      expect(await screen.findByText("HTTP 502 - bad gateway")).toBeTruthy();
    });

    it("미리보기 네트워크 오류도 모달에 돌려준다", async () => {
      const base = globalThis.fetch.getMockImplementation();
      globalThis.fetch.mockImplementation((url, init) =>
        url === PREVIEW_URL ? Promise.reject(new Error("network down")) : base(url, init),
      );
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={7} preview editable />);
      await openBook();
      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
      fireEvent.click(screen.getByText("삽입 표시"));

      expect(await screen.findByText("network down")).toBeTruthy();
    });

    it("다시 열면 표시한 URL 을 채우고, 새 이미지로 바꾸면 이전 미리보기를 해제한다", async () => {
      respond({ preview: blobResponse() });
      await openWithPendingCover();

      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
      expect(screen.getByTestId("cover-modal").getAttribute("data-initial-url")).toBe(
        "https://img.example.com/c.jpg",
      );
      fireEvent.click(screen.getByText("삽입 표시"));
      await waitFor(() => expect(screen.queryByTestId("cover-modal")).toBeNull());

      expect(globalThis.URL.revokeObjectURL).toHaveBeenCalledWith("blob:new-cover");
    });

    it("취소하면 표시를 지우고 미리보기를 해제한다", async () => {
      respond({ preview: blobResponse() });
      await openWithPendingCover();

      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 삽입 취소" }));

      expect(screen.queryByTestId("epub-cover-pending")).toBeNull();
      expect(globalThis.URL.revokeObjectURL).toHaveBeenCalledWith("blob:new-cover");
    });

    it("저장은 확인창에서 취소하면 요청하지 않는다", async () => {
      respond({ preview: blobResponse() });
      vi.spyOn(window, "confirm").mockReturnValue(false);
      await openWithPendingCover();

      fireEvent.click(screen.getByRole("button", { name: "저장" }));

      expect(window.confirm).toHaveBeenCalled();
      expect(globalThis.fetch.mock.calls.some(([u]) => u === SAVE_URL)).toBe(false);
      expect(screen.getByTestId("epub-cover-pending")).toBeTruthy();
    });

    it("저장하면 epub-cover 를 호출하고 표시를 지운 뒤 EPUB 을 다시 불러온다", async () => {
      respond({ preview: blobResponse(), save: jsonResponse({ status: "success", result: {} }) });
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
      await openWithPendingCover();
      const before = globalThis.fetch.mock.calls.filter(([u]) => u.includes("/preview/7")).length;

      fireEvent.click(screen.getByRole("button", { name: "저장" }));

      await waitFor(() => expect(screen.queryByTestId("epub-cover-pending")).toBeNull());
      expect(confirm.mock.calls[0][0]).toBe(
        "표지 이미지 교체를 저장합니다. 원본은 0_trash에 보존됩니다. 저장할까요?",
      );
      const save = globalThis.fetch.mock.calls.find(([u]) => u === SAVE_URL);
      expect(save[1].method).toBe("POST");
      expect(JSON.parse(save[1].body)).toEqual({ url: "https://img.example.com/c.jpg" });
      await waitFor(() =>
        expect(
          globalThis.fetch.mock.calls.filter(([u]) => u.includes("/preview/7")).length,
        ).toBe(before + 1),
      );
    });

    describe("이 기기의 이미지", () => {
      const UPLOAD_URL = "http://localhost:8000/pdf-pages/7/insert-preview-upload";

      async function openWithPendingFile() {
        mockCoverUrl = "blob:epub-cover";
        render(<ViewEPUB bookId={7} preview editable />);
        await openBook();
        fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
        fireEvent.click(screen.getByText("파일 표시"));
        await screen.findByTestId("epub-cover-pending");
      }
      const respondUpload = ({ preview, save } = {}) => {
        const base = globalThis.fetch.getMockImplementation();
        globalThis.fetch.mockImplementation((url, init) => {
          if (url === UPLOAD_URL && preview) return Promise.resolve(preview);
          if (url === SAVE_URL && save) return Promise.resolve(save);
          return base(url, init);
        });
      };

      it("파일을 고르면 insert-preview-upload 로 올려 저장 전 표지로 보여 준다", async () => {
        respondUpload({ preview: blobResponse() });
        await openWithPendingFile();

        const call = globalThis.fetch.mock.calls.find(([u]) => u === UPLOAD_URL);
        expect(call[1].body.get("file").name).toBe("cover.jpg");
        expect(screen.getByRole("img", { name: "저장 전 새 표지" }).getAttribute("src")).toBe(
          "blob:new-cover",
        );
        // URL 이 없으므로 툴팁에는 파일 이름을 쓴다.
        expect(screen.getByText("새 표지 · 저장 전").title).toBe("cover.jpg");
        expect(globalThis.fetch.mock.calls.some(([u]) => u === SAVE_URL)).toBe(false);
      });

      it("저장하면 미리 본 JPEG 를 image_data(base64)로 보내고 url 은 보내지 않는다", async () => {
        respondUpload({
          preview: blobResponse(),
          save: jsonResponse({ status: "success", result: {} }),
        });
        vi.spyOn(window, "confirm").mockReturnValue(true);
        await openWithPendingFile();

        fireEvent.click(screen.getByRole("button", { name: "저장" }));

        await waitFor(() => {
          const save = globalThis.fetch.mock.calls.find(([u]) => u === SAVE_URL);
          expect(save[1].method).toBe("POST");
          // 미리보기 응답의 Blob 내용은 "x" 다.
          expect(JSON.parse(save[1].body)).toEqual({ image_data: "eA==" });
        });
      });

      it("서버가 거절하면 사유를 모달에 돌려주고 표시하지 않는다", async () => {
        respondUpload({ preview: textResponse(400, "이미지 형식을 읽을 수 없습니다") });
        mockCoverUrl = "blob:epub-cover";
        render(<ViewEPUB bookId={7} preview editable />);
        await openBook();
        fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
        fireEvent.click(screen.getByText("파일 표시"));

        expect(await screen.findByText("이미지 형식을 읽을 수 없습니다")).toBeTruthy();
        expect(screen.queryByTestId("epub-cover-pending")).toBeNull();
      });
    });

    it("표지가 없던 책의 확인 문구는 '추가'다", async () => {
      respond({ preview: blobResponse(), save: jsonResponse({ status: "success", result: {} }) });
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
      render(<ViewEPUB bookId={7} preview editable />);
      await openBook();
      await act(async () => {
        lastRendition._emit("relocated", { atStart: true, start: {} });
      });
      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 추가" }));
      fireEvent.click(screen.getByText("삽입 표시"));
      await screen.findByTestId("epub-cover-pending");

      fireEvent.click(screen.getByRole("button", { name: "저장" }));

      await waitFor(() => expect(confirm).toHaveBeenCalled());
      expect(confirm.mock.calls[0][0]).toBe(
        "표지 이미지 추가를 저장합니다. 원본은 0_trash에 보존됩니다. 저장할까요?",
      );
    });

    it("저장 실패(status failure)는 툴바에 사유를 보이고 표시를 유지한다", async () => {
      respond({
        preview: blobResponse(),
        save: jsonResponse({ status: "failure", error: "EPUB이 변경되었습니다" }),
      });
      vi.spyOn(window, "confirm").mockReturnValue(true);
      await openWithPendingCover();

      fireEvent.click(screen.getByRole("button", { name: "저장" }));

      expect((await screen.findByRole("alert")).textContent).toContain("EPUB이 변경되었습니다");
      expect(screen.getByTestId("epub-cover-pending")).toBeTruthy();
    });

    it("저장 HTTP 오류와 본문 없는 failure 도 보여 준다", async () => {
      respond({ preview: blobResponse(), save: textResponse(403, "forbidden") });
      vi.spyOn(window, "confirm").mockReturnValue(true);
      await openWithPendingCover();
      fireEvent.click(screen.getByRole("button", { name: "저장" }));
      expect((await screen.findByRole("alert")).textContent).toContain("HTTP 403 - forbidden");

      respond({ save: jsonResponse({ status: "failure" }) });
      fireEvent.click(screen.getByRole("button", { name: "저장" }));
      await waitFor(() =>
        expect(screen.getByRole("alert").textContent).toContain("저장에 실패했습니다."),
      );
    });

    it("저장 중에는 버튼을 잠그고 '저장 중' 을 보인다", async () => {
      let finish;
      respond({
        preview: blobResponse(),
        save: new Promise((resolve) => {
          finish = () => resolve(jsonResponse({ status: "success", result: {} }));
        }),
      });
      // respond 는 응답 객체를 기대하므로 Promise 도 그대로 통과한다.
      vi.spyOn(window, "confirm").mockReturnValue(true);
      await openWithPendingCover();

      fireEvent.click(screen.getByRole("button", { name: "저장" }));

      expect(await screen.findByText("저장 중...")).toBeTruthy();
      expect(screen.getByRole("button", { name: "저장" }).disabled).toBe(true);
      // 저장 중 재클릭은 요청을 더 만들지 않는다.
      fireEvent.click(screen.getByRole("button", { name: "저장" }));
      expect(globalThis.fetch.mock.calls.filter(([u]) => u === SAVE_URL)).toHaveLength(1);
      await act(async () => finish());
    });

    it("메시지 없는 예외도 기본 문구로 보여 준다", async () => {
      const base = globalThis.fetch.getMockImplementation();
      globalThis.fetch.mockImplementation((url, init) =>
        url === PREVIEW_URL ? Promise.reject({}) : base(url, init),
      );
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={7} preview editable />);
      await openBook();
      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
      fireEvent.click(screen.getByText("삽입 표시"));
      expect(await screen.findByText("이미지를 가져오지 못했습니다")).toBeTruthy();

      globalThis.fetch.mockImplementation((url, init) => {
        if (url === PREVIEW_URL) return Promise.resolve(blobResponse());
        if (url === SAVE_URL) return Promise.reject({});
        return base(url, init);
      });
      vi.spyOn(window, "confirm").mockReturnValue(true);
      fireEvent.click(screen.getByText("삽입 표시"));
      await screen.findByTestId("epub-cover-pending");
      fireEvent.click(screen.getByRole("button", { name: "저장" }));
      await waitFor(() =>
        expect(screen.getByRole("alert").textContent).toContain("저장에 실패했습니다."),
      );
    });

    describe("표지 삭제 표시(휴지통)", () => {
      const trashButton = () =>
        screen.getByRole("button", { name: "표지 삭제 표시" });

      async function openWithCover() {
        mockCoverUrl = "blob:epub-cover";
        render(<ViewEPUB bookId={7} preview editable />);
        await openBook();
      }

      it("표지가 있으면 휴지통 버튼을 둔다", async () => {
        await openWithCover();
        expect(trashButton()).toBeTruthy();
      });

      it("표지가 없으면 지울 것이 없으므로 휴지통 버튼을 두지 않는다", async () => {
        render(<ViewEPUB bookId={7} preview editable />);
        await openBook();
        await act(async () => {
          lastRendition._emit("relocated", { atStart: true, start: {} });
        });

        expect(screen.getByRole("button", { name: "표지 이미지 추가" })).toBeTruthy();
        expect(screen.queryByRole("button", { name: "표지 삭제 표시" })).toBeNull();
      });

      it("누르면 회색 덮개와 저장 버튼이 나오고 다시 누르면 해제한다", async () => {
        await openWithCover();
        expect(screen.queryByTestId("epub-cover-delete-overlay")).toBeNull();
        expect(screen.queryByRole("button", { name: "저장" })).toBeNull();

        fireEvent.click(trashButton());

        expect(screen.getByTestId("epub-cover-delete-overlay")).toBeTruthy();
        expect(screen.getByRole("button", { name: "저장" })).toBeTruthy();
        expect(trashButton().getAttribute("aria-pressed")).toBe("true");
        expect(trashButton().className).toContain("active");
        expect(trashButton().title).toBe("삭제 표시 해제");
        // 서버는 저장하기 전까지 건드리지 않는다.
        expect(globalThis.fetch.mock.calls.some(([u]) => u === SAVE_URL)).toBe(false);

        fireEvent.click(trashButton());

        expect(screen.queryByTestId("epub-cover-delete-overlay")).toBeNull();
        expect(screen.queryByRole("button", { name: "저장" })).toBeNull();
        expect(trashButton().title).toBe("표지를 삭제 표시");
      });

      it("새 표지 삽입 표시가 있으면 삭제 표시가 그것을 지운다", async () => {
        respond({ preview: blobResponse() });
        await openWithPendingCover();

        fireEvent.click(trashButton());

        expect(screen.queryByTestId("epub-cover-pending")).toBeNull();
        expect(screen.getByTestId("epub-cover-delete-overlay")).toBeTruthy();
        expect(globalThis.URL.revokeObjectURL).toHaveBeenCalledWith("blob:new-cover");
      });

      it("삭제 표시가 있으면 새 표지 삽입 표시가 그것을 지운다", async () => {
        respond({ preview: blobResponse() });
        await openWithCover();
        fireEvent.click(trashButton());

        fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
        fireEvent.click(screen.getByText("삽입 표시"));
        await screen.findByTestId("epub-cover-pending");

        expect(screen.queryByTestId("epub-cover-delete-overlay")).toBeNull();
        expect(trashButton().getAttribute("aria-pressed")).toBe("false");
      });

      it("확인창에서 취소하면 요청하지 않고 표시를 유지한다", async () => {
        vi.spyOn(window, "confirm").mockReturnValue(false);
        await openWithCover();
        fireEvent.click(trashButton());

        fireEvent.click(screen.getByRole("button", { name: "저장" }));

        expect(window.confirm).toHaveBeenCalledWith(
          "표지 삭제를 저장합니다. 원본은 0_trash에 보존됩니다. 저장할까요?",
        );
        expect(globalThis.fetch.mock.calls.some(([u]) => u === SAVE_URL)).toBe(false);
        expect(screen.getByTestId("epub-cover-delete-overlay")).toBeTruthy();
      });

      it("저장하면 DELETE 를 호출하고 표시를 지운 뒤 EPUB 을 다시 불러온다", async () => {
        respond({ save: jsonResponse({ status: "success", result: {} }) });
        vi.spyOn(window, "confirm").mockReturnValue(true);
        await openWithCover();
        const loads = () =>
          globalThis.fetch.mock.calls.filter(([u]) => u.includes("/preview/7")).length;
        const before = loads();
        fireEvent.click(trashButton());

        fireEvent.click(screen.getByRole("button", { name: "저장" }));

        await waitFor(() =>
          expect(screen.queryByTestId("epub-cover-delete-overlay")).toBeNull(),
        );
        const call = globalThis.fetch.mock.calls.find(([u]) => u === SAVE_URL);
        expect(call[1].method).toBe("DELETE");
        expect(call[1].body).toBeUndefined();
        await waitFor(() => expect(loads()).toBe(before + 1));
      });

      it("서버가 거절하면 사유를 보이고 삭제 표시를 유지한다", async () => {
        respond({
          save: jsonResponse({
            status: "failure",
            error: "이 EPUB의 표지는 이 도구가 넣은 것이 아니라 삭제할 수 없습니다",
          }),
        });
        vi.spyOn(window, "confirm").mockReturnValue(true);
        await openWithCover();
        fireEvent.click(trashButton());

        fireEvent.click(screen.getByRole("button", { name: "저장" }));

        expect((await screen.findByRole("alert")).textContent).toContain(
          "이 도구가 넣은 것이 아니라",
        );
        expect(screen.getByTestId("epub-cover-delete-overlay")).toBeTruthy();
      });

      it("저장 중에는 휴지통도 잠근다", async () => {
        let finish;
        respond({
          save: new Promise((resolve) => {
            finish = () => resolve(jsonResponse({ status: "success", result: {} }));
          }),
        });
        vi.spyOn(window, "confirm").mockReturnValue(true);
        await openWithCover();
        fireEvent.click(trashButton());

        fireEvent.click(screen.getByRole("button", { name: "저장" }));

        await screen.findByText("저장 중...");
        expect(trashButton().disabled).toBe(true);
        await act(async () => finish());
      });
    });

    it("화면을 닫은 뒤 도착한 미리보기 응답은 버리고 blob URL 을 만들지 않는다", async () => {
      let arrive;
      const base = globalThis.fetch.getMockImplementation();
      globalThis.fetch.mockImplementation((url, init) =>
        url === PREVIEW_URL
          ? new Promise((resolve) => {
              arrive = () => resolve(blobResponse());
            })
          : base(url, init),
      );
      mockCoverUrl = "blob:epub-cover";
      render(<ViewEPUB bookId={7} preview editable />);
      await openBook();
      fireEvent.click(screen.getByRole("button", { name: "표지 이미지 교체" }));
      fireEvent.click(screen.getByText("삽입 표시"));
      await waitFor(() => expect(arrive).toBeTypeOf("function"));

      cleanup();
      await act(async () => arrive());

      expect(globalThis.URL.createObjectURL).not.toHaveBeenCalled();
    });

    it("삽입 표시한 표지는 저장 전 표시(점선)와 PDF 와 같은 버튼 모양을 쓴다", async () => {
      respond({ preview: blobResponse() });
      await openWithPendingCover();

      expect(screen.getByTestId("epub-cover-pending").className).toContain(
        "viewer-edit-pending",
      );
      const insert = screen.getByRole("button", { name: "표지 이미지 교체" });
      expect(insert.className).toContain("viewer-edit-btn");
      expect(insert.className).toContain("active");
      expect(insert.getAttribute("aria-pressed")).toBe("true");
      expect(screen.getByRole("button", { name: "저장" }).title).toBe("표시한 변경을 저장");
      expect(screen.getByRole("button", { name: "표지 이미지 삽입 취소" }).textContent).toBe("×");
    });

    it("화면을 닫으면 저장 전 미리보기를 해제한다", async () => {
      respond({ preview: blobResponse() });
      await openWithPendingCover();

      cleanup();

      expect(globalThis.URL.revokeObjectURL).toHaveBeenCalledWith("blob:new-cover");
    });
  });
});
