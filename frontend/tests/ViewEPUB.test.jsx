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

afterEach(cleanup);

// ReactReader mock - autoLoad=true이면 즉시 locationChanged 호출
let autoLoad = true;
let capturedGetRendition = null;
const mockReactReader = vi.fn(
  ({
    url,
    locationChanged,
    getRendition,
    title,
    epubOptions: _epubOptions,
  }) => {
    if (getRendition && url) {
      capturedGetRendition = getRendition;
    }
    if (url && autoLoad) {
      setTimeout(() => locationChanged?.("epubcfi(/1)"), 0);
    }
    return (
      <div data-testid="react-reader">
        {title && <span data-testid="book-title">{title}</span>}ReactReader
      </div>
    );
  },
);

vi.mock("react-reader", () => ({
  ReactReader: (props) => mockReactReader(props),
}));

vi.mock("../src/Common", () => ({
  getApiUrlPrefix: () => "http://localhost:8000",
}));

vi.mock("../src/ViewEPUB.css", () => ({}));

// fetch mock: ArrayBuffer 반환
const mockArrayBuffer = new ArrayBuffer(8);
const _mockArrayBuffer2 = new ArrayBuffer(16);

function createFetchResponse(buf = mockArrayBuffer) {
  return {
    ok: true,
    headers: {
      get: () => null,
    },
    arrayBuffer: () => Promise.resolve(buf),
    text: () => Promise.resolve(""),
  };
}

function createErrorFetchResponse(status = 500, body = "") {
  return {
    ok: false,
    status,
    headers: { get: () => null },
    text: () => Promise.resolve(body),
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
    clear: vi.fn(() => {
      store = {};
    }),
    _getStore: () => store,
  };
})();
Object.defineProperty(globalThis, "localStorage", {
  value: localStorageMock,
  writable: true,
});

beforeEach(() => {
  autoLoad = true;
  mockReactReader.mockClear();
  capturedGetRendition = null;
  localStorageMock.clear();
  localStorageMock.getItem.mockClear();
  localStorageMock.setItem.mockClear();
  globalThis.fetch = vi.fn(() => Promise.resolve(createFetchResponse()));
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

import ViewEPUB from "../src/ViewEPUB";

// rendition mock을 만들어주는 헬퍼
function createMockRendition({
  destroyThrows = false,
  locationsTotal = 300,
  readyReject = null,
} = {}) {
  const handlers = {};
  const locations = {
    generate: vi.fn(() => {
      // epubjs: total = _locations.length - 1 (0-indexed 최대값)
      locations.total = locationsTotal - 1;
      return Promise.resolve(Array(locationsTotal));
    }),
    locationFromCfi: vi.fn(() => 44),
    total: 0,
  };
  return {
    display: vi.fn(() => Promise.resolve()),
    destroy: destroyThrows
      ? vi.fn(() => {
          throw new Error("already destroyed");
        })
      : vi.fn(),
    themes: {
      fontSize: vi.fn(),
      font: vi.fn(),
    },
    on: vi.fn((event, handler) => {
      handlers[event] = handler;
    }),
    book: {
      spine: {
        get: vi.fn((target) => target || null),
      },
      loaded: {
        metadata: Promise.resolve({ title: "테스트 책 제목" }),
      },
      ready: readyReject ? Promise.reject(readyReject) : Promise.resolve(),
      locations,
    },
    _handlers: handlers,
    _emitRelocated: (location) => {
      if (handlers["relocated"]) handlers["relocated"](location);
    },
  };
}

describe("ViewEPUB", () => {
  beforeEach(() => {
    mockReactReader.mockClear();
    capturedGetRendition = null;
    autoLoad = true;
  });

  // ── 유효성 검사 ──

  it("bookId가 없으면 에러를 표시한다", () => {
    render(<ViewEPUB bookId={0} />);
    expect(
      screen.getByText(/유효한 bookId가 제공되지 않았습니다/),
    ).toBeTruthy();
  });

  it("bookId가 없으면 fetch를 호출하지 않는다", () => {
    render(<ViewEPUB bookId={0} />);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  // ── 초기 로딩 ──

  it("preview=true이면 chapters=10로 첫 요청한다", async () => {
    render(<ViewEPUB bookId={42} preview={true} />);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/preview/42?chapters=10",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalledWith(
        expect.objectContaining({ url: mockArrayBuffer }),
      );
    });
  });

  it("preview=false(전체보기)이면 chapters=0으로 첫 요청한다", async () => {
    render(<ViewEPUB bookId={42} />);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/preview/42?chapters=0",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalledWith(
        expect.objectContaining({ url: mockArrayBuffer }),
      );
    });
  });

  // ── 미리보기: 추가 로드 없음 ──

  it("미리보기 모드에서 2챕터 이후 추가 페칭하지 않는다", async () => {
    render(<ViewEPUB bookId={42} preview={true} />);

    // 초기 로드 완료 대기
    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    // 추가 로드 없음 확인
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    expect(globalThis.fetch).toHaveBeenCalledTimes(1);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/preview/42?chapters=10",
      expect.any(Object),
    );
  });

  // ── 단일 fetch 검증 ──

  it("전체보기에서 chapters=0으로 요청하고 단일 fetch만 수행한다", async () => {
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    expect(globalThis.fetch).toHaveBeenCalledTimes(1);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/preview/42?chapters=0",
      expect.any(Object),
    );
  });

  // ── "더보기" 버튼 없음 ──

  it('"더 보기" 버튼이 표시되지 않는다', async () => {
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    expect(screen.queryByText(/더 보기/)).toBeNull();
    expect(screen.queryByRole("button", { name: /챕터/ })).toBeNull();
  });

  // ── %2F 회귀 방지 ──

  it("preview URL에 파일 경로나 %2F가 포함되지 않는다", () => {
    render(<ViewEPUB bookId={42} />);
    const fetchUrl = globalThis.fetch.mock.calls[0][0];
    expect(fetchUrl).toBe("http://localhost:8000/preview/42?chapters=0");
    expect(fetchUrl).not.toContain("%2F");
  });

  // ── bookId / preview 변경 ──

  it("bookId 변경 시 새로 fetch한다", async () => {
    const { rerender } = render(<ViewEPUB bookId={1} />);
    await waitFor(() => {
      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://localhost:8000/preview/1?chapters=0",
        expect.any(Object),
      );
    });

    rerender(<ViewEPUB bookId={2} />);
    await waitFor(() => {
      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://localhost:8000/preview/2?chapters=0",
        expect.any(Object),
      );
    });
  });

  it("preview 전환 시 새로 fetch한다", async () => {
    const { rerender } = render(<ViewEPUB bookId={1} preview={true} />);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://localhost:8000/preview/1?chapters=10",
      expect.any(Object),
    );

    globalThis.fetch.mockClear();
    rerender(<ViewEPUB bookId={1} preview={false} />);
    await waitFor(() => {
      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://localhost:8000/preview/1?chapters=0",
        expect.any(Object),
      );
    });
  });

  // ── 컨테이너 높이 ──

  it("preview=true이면 컨테이너 높이가 60vh이다", () => {
    const { container } = render(<ViewEPUB bookId={1} preview={true} />);
    const div = container.firstChild;
    expect(div.style.height).toBe("60vh");
  });

  it("preview=false이면 컨테이너 높이가 100dvh이다", () => {
    const { container } = render(<ViewEPUB bookId={1} />);
    const div = container.firstChild;
    expect(div.style.height).toBe("100dvh");
  });

  // ── 로딩 상태 ──

  it("초기 로딩 시 스피너를 표시한다", () => {
    render(<ViewEPUB bookId={1} />);
    expect(screen.getByText("로딩 중...")).toBeTruthy();
  });

  // ── 로딩 타임아웃 (30초) ──

  it("30초 내 로딩 미완료 시 타임아웃 에러를 표시한다", async () => {
    vi.useFakeTimers();
    autoLoad = false;

    render(<ViewEPUB bookId={1} />);

    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    await act(async () => {
      vi.advanceTimersByTime(30000);
    });

    expect(
      screen.getByText(/EPUB 렌더링 시간이 초과되었습니다\. \(book_id=1\)/),
    ).toBeTruthy();
    expect(screen.queryByText("로딩 중...")).toBeNull();

    vi.useRealTimers();
  });

  it("로딩 완료 후에는 타임아웃이 발생하지 않는다", async () => {
    vi.useFakeTimers();
    autoLoad = true;

    render(<ViewEPUB bookId={1} />);

    await act(async () => {
      vi.advanceTimersByTime(1);
    });

    act(() => {
      vi.advanceTimersByTime(30000);
    });
    expect(screen.queryByText(/EPUB 렌더링 시간이 초과되었습니다/)).toBeNull();

    vi.useRealTimers();
  });

  // ── fetch 에러 ──

  it("서버 에러(non-ok) 시 에러 메시지를 표시한다", async () => {
    globalThis.fetch = vi.fn(() =>
      Promise.resolve(createErrorFetchResponse(500)),
    );
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText(/EPUB 로딩 실패/)).toBeTruthy();
    });
  });

  it("네트워크 에러(TypeError: Failed to fetch) 시 에러 메시지를 표시한다", async () => {
    globalThis.fetch = vi.fn(() =>
      Promise.reject(new TypeError("Failed to fetch")),
    );
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText(/EPUB 로딩 실패: Failed to fetch/)).toBeTruthy();
    });
  });

  it("AbortError는 에러 메시지로 표시하지 않는다", async () => {
    const abortError = new DOMException(
      "The operation was aborted.",
      "AbortError",
    );
    globalThis.fetch = vi.fn(() => Promise.reject(abortError));
    render(<ViewEPUB bookId={1} />);

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    expect(screen.queryByText(/EPUB 로딩 실패/)).toBeNull();
  });

  // ── 언마운트 시 cleanup ──

  it("언마운트 시 fetch가 abort된다", async () => {
    const abortSpy = vi.spyOn(AbortController.prototype, "abort");
    const { unmount } = render(<ViewEPUB bookId={1} />);

    unmount();
    expect(abortSpy).toHaveBeenCalled();
  });

  // ── 전체보기: 책 제목 및 툴바 ──

  it("전체보기 모드에서 getRendition으로 책 제목을 추출한다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
      await new Promise((r) => setTimeout(r, 10));
    });

    await waitFor(() => {
      expect(screen.getByTestId("book-title")).toBeTruthy();
      expect(screen.getByText("테스트 책 제목")).toBeTruthy();
    });
  });

  it("미리보기 모드에서는 책 제목을 title prop에 전달하지 않는다", async () => {
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    const lastCall =
      mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
    expect(lastCall[0].title).toBeUndefined();
  });

  it("전체보기에서 A+/A- 버튼이 표시된다", async () => {
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(screen.getByLabelText("글자 크기 늘리기")).toBeTruthy();
      expect(screen.getByLabelText("글자 크기 줄이기")).toBeTruthy();
    });
  });

  it("미리보기에서는 툴바가 표시되지 않는다", async () => {
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    expect(screen.queryByTestId("epub-toolbar")).toBeNull();
  });

  it("A+ 클릭 시 rendition.themes.fontSize가 호출된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    const plusButton = screen.getByLabelText("글자 크기 늘리기");
    await act(async () => {
      fireEvent.click(plusButton);
    });

    expect(rendition.themes.fontSize).toHaveBeenCalledWith("120%");
  });

  it("A- 클릭 시 글자 크기가 줄어든다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    const minusButton = screen.getByLabelText("글자 크기 줄이기");
    await act(async () => {
      fireEvent.click(minusButton);
    });

    expect(rendition.themes.fontSize).toHaveBeenCalledWith("80%");
  });

  it("글자 크기가 최소(80%)이면 A- 버튼이 비활성화된다", async () => {
    localStorageMock._getStore()["epub_fontSize"] = "80";
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    const minusButton = await waitFor(() =>
      screen.getByLabelText("글자 크기 줄이기"),
    );
    expect(minusButton.disabled).toBe(true);
  });

  it("글자 크기가 최대(160%)이면 A+ 버튼이 비활성화된다", async () => {
    localStorageMock._getStore()["epub_fontSize"] = "160";
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    const plusButton = await waitFor(() =>
      screen.getByLabelText("글자 크기 늘리기"),
    );
    expect(plusButton.disabled).toBe(true);
  });

  it("getRendition에서 저장된 글자 크기를 적용한다", async () => {
    localStorageMock._getStore()["epub_fontSize"] = "140";
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(rendition.themes.fontSize).toHaveBeenCalledWith("140%");
  });

  it("전체보기에서 글꼴 선택 드롭다운이 표시된다", async () => {
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(screen.getByLabelText("글꼴 선택")).toBeTruthy();
    });
  });

  it("글꼴 변경 시 rendition.themes.font가 호출된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    const select = screen.getByLabelText("글꼴 선택");
    await act(async () => {
      fireEvent.change(select, {
        target: { value: "'Nanum Gothic', sans-serif" },
      });
    });

    expect(rendition.themes.font).toHaveBeenCalledWith(
      "'Nanum Gothic', sans-serif",
    );
  });

  it("getRendition에서 저장된 글꼴을 적용한다", async () => {
    localStorageMock._getStore()["epub_fontFamily"] = "serif";
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(rendition.themes.font).toHaveBeenCalledWith("serif");
  });

  // ── 페이지 정보 ──

  it("전체보기에서 locations 미준비 시 relocated 이벤트가 페이지 정보를 업데이트하지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    // generate가 완료되지 않도록 막음
    rendition.book.locations.generate = vi.fn(() => new Promise(() => {}));

    await act(async () => {
      capturedGetRendition(rendition);
    });

    // locations.generate 완료 전에 relocated 트리거
    await act(async () => {
      rendition._emitRelocated({ start: { cfi: "epubcfi(/1)" } });
    });

    expect(screen.getByText("페이지 계산 중...")).toBeTruthy();
  });

  it("미리보기에서는 페이지 정보가 표시되지 않는다", async () => {
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    expect(screen.queryByTestId("epub-page-info")).toBeNull();
  });

  // ── 위치 저장 및 복원 ──

  it("전체보기에서 페이지 변경 시 위치를 localStorage에 저장한다", async () => {
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    const lastCall =
      mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
    await act(async () => {
      lastCall[0].locationChanged("epubcfi(/6)");
    });

    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "epub_location_42",
      "epubcfi(/6)",
    );
  });

  it("미리보기에서는 읽기 위치를 저장하지 않는다", async () => {
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    localStorageMock.setItem.mockClear();

    const lastCall =
      mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
    await act(async () => {
      lastCall[0].locationChanged("epubcfi(/6)");
    });

    expect(localStorageMock.setItem).not.toHaveBeenCalled();
  });

  it("전체보기에서 첫 렌더 후 저장된 위치로 display() 복원한다", async () => {
    localStorageMock._getStore()["epub_location_42"] =
      "epubcfi(/saved/location)";
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    const displaySpy = rendition.display;
    await act(async () => {
      capturedGetRendition(rendition);
    });

    // 첫 렌더 시뮬레이션 (수동으로 locationChanged 호출)
    const lastCall =
      mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
    await act(async () => {
      lastCall[0].locationChanged("epubcfi(/1)");
    });

    // display가 wrapped되었으므로, spy를 통해 검증
    expect(displaySpy).toHaveBeenCalledWith("epubcfi(/saved/location)");
  });

  it("저장된 위치 복원 실패 시 localStorage에서 위치를 삭제하고 현재 위치를 유지한다", async () => {
    localStorageMock._getStore()["epub_location_42"] = "epubcfi(/invalid)";
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.display = vi.fn((target) => {
      if (target === "epubcfi(/invalid)") {
        return Promise.reject(new Error("Invalid CFI"));
      }
      return Promise.resolve();
    });

    await act(async () => {
      capturedGetRendition(rendition);
    });

    const lastCall =
      mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
    await act(async () => {
      lastCall[0].locationChanged("epubcfi(/1)");
    });

    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    expect(localStorageMock.removeItem).toHaveBeenCalledWith(
      "epub_location_42",
    );
  });

  it("저장된 위치가 없으면 전체보기에서 처음부터 시작한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    const displaySpy = rendition.display;
    await act(async () => {
      capturedGetRendition(rendition);
    });

    const lastCall =
      mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
    await act(async () => {
      lastCall[0].locationChanged("epubcfi(/1)");
    });

    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    // display가 저장된 위치로 호출되지 않음 (복원할 위치가 없음)
    expect(displaySpy).not.toHaveBeenCalled();
  });

  it("전체보기에서 location prop이 전달되지 않는다", async () => {
    localStorageMock._getStore()["epub_location_42"] = "epubcfi(/saved)";
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    const calls = mockReactReader.mock.calls;
    calls.forEach((call) => {
      expect(call[0].location).toBeUndefined();
    });
  });

  it("저장된 위치가 있어도 초기 로드에서 타임아웃이 발생하지 않는다", async () => {
    vi.useFakeTimers();
    localStorageMock._getStore()["epub_location_42"] = "epubcfi(/saved)";
    autoLoad = true;

    render(<ViewEPUB bookId={42} />);

    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    act(() => {
      vi.advanceTimersByTime(30000);
    });

    expect(screen.queryByText(/EPUB 렌더링 시간이 초과되었습니다/)).toBeNull();

    vi.useRealTimers();
  });

  it("미리보기에서는 저장된 읽기 위치를 무시한다", async () => {
    localStorageMock._getStore()["epub_location_42"] = "epubcfi(/saved)";
    autoLoad = false;
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    const displaySpy = rendition.display;
    await act(async () => {
      capturedGetRendition(rendition);
    });

    const lastCall =
      mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
    await act(async () => {
      lastCall[0].locationChanged("epubcfi(/1)");
    });

    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    expect(displaySpy).not.toHaveBeenCalled();
  });

  // ── relocated 이벤트 ──

  it("전체보기에서 getRendition이 relocated 리스너를 등록한다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(rendition.on).toHaveBeenCalledWith(
      "relocated",
      expect.any(Function),
    );
  });

  it("미리보기에서는 getRendition이 relocated 리스너를 등록하지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(rendition.on).not.toHaveBeenCalledWith(
      "relocated",
      expect.any(Function),
    );
  });

  // ── locations.generate ──

  it("전체보기에서 book.locations.generate가 호출된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalledWith(1024);
    });
  });

  it("locations 준비 후 relocated에서 전역 페이지 번호를 표시한다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({ locationsTotal: 300 });
    await act(async () => {
      capturedGetRendition(rendition);
    });

    // locations.generate 완료 대기
    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalled();
    });

    await act(async () => {
      await rendition.book.locations.generate();
    });

    await act(async () => {
      rendition._emitRelocated({ start: { cfi: "epubcfi(/1)" } });
    });

    await waitFor(() => {
      expect(screen.getByText("45 / 300")).toBeTruthy();
    });
  });

  it("locations.generate 완료 시 현재 위치의 전역 페이지로 즉시 업데이트된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    const rendition = createMockRendition({ locationsTotal: 300 });
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalled();
    });

    await act(async () => {
      await rendition.book.locations.generate();
    });

    await waitFor(() => {
      expect(screen.getByText("45 / 300")).toBeTruthy();
    });
  });

  it("locations.generate 실패 시 페이지 계산 중 상태가 유지된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.book.locations.generate = vi.fn(() =>
      Promise.reject(new Error("generate failed")),
    );

    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    expect(screen.getByText("페이지 계산 중...")).toBeTruthy();
  });

  it("미리보기에서는 locations.generate가 호출되지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    expect(rendition.book.locations.generate).not.toHaveBeenCalled();
  });

  it("마지막 페이지에서 relocated 시 page가 total을 초과하지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({ locationsTotal: 50 });
    rendition.book.locations.locationFromCfi = vi.fn(() => 49);

    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalled();
    });

    await act(async () => {
      await rendition.book.locations.generate();
    });

    await act(async () => {
      rendition._emitRelocated({ start: { cfi: "epubcfi(/last)" } });
    });

    await waitFor(() => {
      expect(screen.getByText("50 / 50")).toBeTruthy();
    });
  });

  it("마지막 페이지에서 generate 완료 시에도 page가 total을 초과하지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    const rendition = createMockRendition({ locationsTotal: 50 });
    rendition.book.locations.locationFromCfi = vi.fn(() => 49);

    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalled();
    });

    await act(async () => {
      await rendition.book.locations.generate();
    });

    await waitFor(() => {
      expect(screen.getByText("50 / 50")).toBeTruthy();
    });
  });

  it("locationFromCfi가 음수를 반환하면 페이지 정보를 업데이트하지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({ locationsTotal: 50 });
    rendition.book.locations.locationFromCfi = vi.fn(() => -1);

    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalled();
    });

    await act(async () => {
      await rendition.book.locations.generate();
    });

    await act(async () => {
      rendition._emitRelocated({ start: { cfi: "epubcfi(/invalid)" } });
    });

    // 페이지 번호가 업데이트되지 않고 초기 상태 유지
    expect(screen.getByText("페이지 계산 중...")).toBeTruthy();
  });

  it("locationFromCfi가 예외를 던지면 페이지 정보를 업데이트하지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({ locationsTotal: 50 });
    rendition.book.locations.locationFromCfi = vi.fn(() => {
      throw new Error("Invalid CFI");
    });

    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalled();
    });

    await act(async () => {
      await rendition.book.locations.generate();
    });

    await act(async () => {
      rendition._emitRelocated({ start: { cfi: "epubcfi(/invalid)" } });
    });

    expect(screen.getByText("페이지 계산 중...")).toBeTruthy();
  });

  // ── rendition.destroy() ──

  it("언마운트 시 rendition.destroy()가 호출된다", async () => {
    autoLoad = true;
    const { unmount } = render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    unmount();

    expect(rendition.destroy).toHaveBeenCalled();
  });

  it("bookId 변경 시 이전 rendition.destroy()가 호출된다", async () => {
    autoLoad = true;
    const { rerender } = render(<ViewEPUB bookId={1} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition1 = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition1);
    });

    rerender(<ViewEPUB bookId={2} />);

    await waitFor(() => {
      expect(rendition1.destroy).toHaveBeenCalled();
    });
  });

  it("getRendition 재호출 시 이전 rendition.destroy()가 호출된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition1 = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition1);
    });

    const rendition2 = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition2);
    });

    expect(rendition1.destroy).toHaveBeenCalled();
  });

  it("rendition.destroy()가 에러를 던져도 크래시하지 않는다", async () => {
    autoLoad = true;
    const { unmount } = render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({ destroyThrows: true });
    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(() => unmount()).not.toThrow();
  });

  it("getRendition에서 같은 rendition이 다시 전달되면 destroy하지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    rendition.destroy.mockClear();

    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(rendition.destroy).not.toHaveBeenCalled();
  });

  it("rendition이 없는 상태에서 언마운트해도 에러가 발생하지 않는다", () => {
    const { unmount } = render(<ViewEPUB bookId={42} />);
    expect(() => unmount()).not.toThrow();
  });

  // ── 에러 처리 ──

  it("서버 에러 시 응답 본문을 에러 메시지에 표시한다", async () => {
    globalThis.fetch = vi.fn(() =>
      Promise.resolve(createErrorFetchResponse(500, "Custom error body")),
    );
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => {
      expect(
        screen.getByText(/EPUB 로딩 실패: Custom error body/),
      ).toBeTruthy();
    });
  });

  it("서버 에러 본문이 비어있으면 status 코드를 표시한다", async () => {
    globalThis.fetch = vi.fn(() =>
      Promise.resolve(createErrorFetchResponse(404, "")),
    );
    render(<ViewEPUB bookId={1} />);
    await waitFor(() => {
      expect(
        screen.getByText(/EPUB 로딩 실패: 서버 응답 오류: 404/),
      ).toBeTruthy();
    });
  });

  it("book.ready reject 시 에러 메시지를 표시한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({
      readyReject: new Error("Book parsing failed"),
    });
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(
        screen.getByText(/EPUB 파싱 오류: Book parsing failed/),
      ).toBeTruthy();
    });
  });

  it("타임아웃 에러 메시지에 bookId가 포함된다", async () => {
    vi.useFakeTimers();
    autoLoad = false;

    render(<ViewEPUB bookId={99} />);

    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    await act(async () => {
      vi.advanceTimersByTime(30000);
    });

    expect(screen.getByText(/book_id=99/)).toBeTruthy();

    vi.useRealTimers();
  });

  it("displayerror 이벤트 발생 시 에러 메시지를 표시한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      rendition._handlers["displayerror"](
        new Error("Display rendering failed"),
      );
    });

    await waitFor(() => {
      expect(
        screen.getByText(/EPUB 렌더링 오류: Display rendering failed/),
      ).toBeTruthy();
    });
  });

  it("displayerror 발생 시 로딩 타임아웃이 취소된다", async () => {
    vi.useFakeTimers();
    autoLoad = false;

    render(<ViewEPUB bookId={42} />);

    await act(async () => {
      await vi.runOnlyPendingTimersAsync();
    });

    expect(capturedGetRendition).not.toBeNull();

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      rendition._handlers["displayerror"](new Error("Display error"));
    });

    act(() => {
      vi.advanceTimersByTime(30000);
    });
    expect(screen.queryByText(/EPUB 렌더링 시간이 초과되었습니다/)).toBeNull();

    vi.useRealTimers();
  });

  it("book.ready reject 시 로딩 타임아웃이 취소된다", async () => {
    vi.useFakeTimers();
    autoLoad = false;

    render(<ViewEPUB bookId={42} />);

    await act(async () => {
      await vi.runOnlyPendingTimersAsync();
    });

    expect(capturedGetRendition).not.toBeNull();

    const rendition = createMockRendition({
      readyReject: new Error("Book load failed"),
    });
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      await vi.runOnlyPendingTimersAsync();
    });

    act(() => {
      vi.advanceTimersByTime(30000);
    });
    expect(screen.queryByText(/EPUB 렌더링 시간이 초과되었습니다/)).toBeNull();

    vi.useRealTimers();
  });

  it("미리보기에서도 displayerror가 에러를 표시한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      rendition._handlers["displayerror"](new Error("Preview display error"));
    });

    await waitFor(() => {
      expect(
        screen.getByText(/EPUB 렌더링 오류: Preview display error/),
      ).toBeTruthy();
    });
  });

  it("미리보기에서도 book.ready reject가 에러를 표시한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({
      readyReject: new Error("Preview parsing failed"),
    });
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(
        screen.getByText(/EPUB 파싱 오류: Preview parsing failed/),
      ).toBeTruthy();
    });
  });

  it("displayerror에 문자열이 전달되면 String()으로 표시한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      rendition._handlers["displayerror"]("String error message");
    });

    await waitFor(() => {
      expect(
        screen.getByText(/EPUB 렌더링 오류: String error message/),
      ).toBeTruthy();
    });
  });

  // ── 이벤트 리스너 등록 ──

  it("전체보기에서 displayerror 리스너가 등록된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(rendition.on).toHaveBeenCalledWith(
      "displayerror",
      expect.any(Function),
    );
  });

  it("미리보기에서도 displayerror 리스너가 등록된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(rendition.on).toHaveBeenCalledWith(
      "displayerror",
      expect.any(Function),
    );
  });

  // ── display() monkey-patch ──

  it("rendition.display가 있으면 monkey-patch하여 성공 시 정상 동작한다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    const origDisplay = rendition.display;
    await act(async () => {
      capturedGetRendition(rendition);
    });

    expect(rendition.display).not.toBe(origDisplay);

    await act(async () => {
      await rendition.display("epubcfi(/test)");
    });

    expect(origDisplay).toHaveBeenCalledWith("epubcfi(/test)");
  });

  it("rendition.display가 reject되면 에러 메시지를 표시하고 에러를 재전파한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.display = vi.fn(() =>
      Promise.reject(new Error("Display failed")),
    );

    await act(async () => {
      capturedGetRendition(rendition);
    });

    const wrappedDisplay = rendition.display;
    await expect(async () => {
      await wrappedDisplay();
    }).rejects.toThrow("Display failed");

    await waitFor(() => {
      expect(screen.getByText(/EPUB 표시 실패: Display failed/)).toBeTruthy();
    });
  });

  it("display(target) 실패 시 target 없이 재시도하여 첫 페이지로 폴백한다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    let callCount = 0;
    const _origDisplayFn = rendition.display;
    rendition.display = vi.fn((target) => {
      callCount++;
      if (callCount === 1 && target) {
        return Promise.reject(new Error("Invalid target"));
      }
      return Promise.resolve();
    });

    await act(async () => {
      capturedGetRendition(rendition);
    });

    // getRendition이 display를 래핑했으므로, 원본 함수가 아닌 래핑된 함수를 사용
    const wrappedDisplay = rendition.display;
    await act(async () => {
      await wrappedDisplay("epubcfi(/invalid)");
    });

    // 래핑된 함수 내부에서 origDisplay가 2번 호출됨 (실패 후 fallback)
    expect(callCount).toBe(2);
  });

  it("display 폴백 성공 시 localStorage에서 저장된 위치를 제거한다", async () => {
    localStorageMock._getStore()["epub_location_42"] = "epubcfi(/saved)";
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    let callCount = 0;
    rendition.display = vi.fn((target) => {
      callCount++;
      if (callCount === 1 && target) {
        return Promise.reject(new Error("Invalid"));
      }
      return Promise.resolve();
    });

    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      await rendition.display("epubcfi(/invalid)");
    });

    expect(localStorageMock.removeItem).toHaveBeenCalledWith(
      "epub_location_42",
    );
  });

  it("display(undefined) 실패 시 폴백 없이 바로 에러를 표시한다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    let callCount = 0;
    rendition.display = vi.fn(() => {
      callCount++;
      return Promise.reject(new Error("Display failed"));
    });

    await act(async () => {
      capturedGetRendition(rendition);
    });

    const wrappedDisplay = rendition.display;
    await expect(async () => {
      await wrappedDisplay(undefined);
    }).rejects.toThrow("Display failed");

    expect(callCount).toBe(1);
  });

  it('display("") 실패 시 폴백 없이 바로 에러를 표시한다', async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    let callCount = 0;
    rendition.display = vi.fn(() => {
      callCount++;
      return Promise.reject(new Error("Display failed"));
    });

    await act(async () => {
      capturedGetRendition(rendition);
    });

    const wrappedDisplay = rendition.display;
    await expect(async () => {
      await wrappedDisplay("");
    }).rejects.toThrow("Display failed");

    expect(callCount).toBe(1);
  });

  it("display 폴백 성공 시 에러 메시지와 로딩 상태가 정상 유지된다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    let callCount = 0;
    rendition.display = vi.fn((target) => {
      callCount++;
      if (callCount === 1 && target) {
        return Promise.reject(new Error("Invalid"));
      }
      return Promise.resolve();
    });

    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      await rendition.display("epubcfi(/invalid)");
    });

    expect(screen.queryByText(/EPUB 표시 실패/)).toBeNull();
  });

  it("display(target) + 폴백 모두 실패 시 폴백 에러를 표시하고 재전파한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.display = vi.fn(() => Promise.reject(new Error("All failed")));

    await act(async () => {
      capturedGetRendition(rendition);
    });

    const wrappedDisplay = rendition.display;
    await expect(async () => {
      await wrappedDisplay("epubcfi(/target)");
    }).rejects.toThrow("All failed");

    await waitFor(() => {
      expect(screen.getByText(/EPUB 표시 실패: All failed/)).toBeTruthy();
    });
  });

  // ── 페이지 상태 표시 ──

  it('전체보기에서 locations 생성 전까지 "페이지 계산 중..." 표시', async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.book.locations.generate = vi.fn(() => new Promise(() => {})); // 계속 대기

    await act(async () => {
      capturedGetRendition(rendition);
    });

    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    expect(screen.getByText("페이지 계산 중...")).toBeTruthy();
  });

  it("전체보기에서 locations 준비 완료 후 정확한 전체 페이지 정보를 표시한다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({ locationsTotal: 100 });
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalled();
    });

    await act(async () => {
      await rendition.book.locations.generate();
    });

    await waitFor(() => {
      expect(screen.getByText("45 / 100")).toBeTruthy();
    });
  });

  it("미리보기에서는 페이지 로딩 상태가 표시되지 않는다", async () => {
    render(<ViewEPUB bookId={42} preview={true} />);

    await waitFor(() => {
      expect(mockReactReader).toHaveBeenCalled();
    });

    expect(screen.queryByText(/페이지 계산 중.../)).toBeNull();
    expect(screen.queryByTestId("epub-page-info")).toBeNull();
  });

  it("전체보기에서 cfi 없는 relocated 이벤트는 전역 페이지를 업데이트하지 않는다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({ locationsTotal: 100 });
    await act(async () => {
      capturedGetRendition(rendition);
    });

    await waitFor(() => {
      expect(rendition.book.locations.generate).toHaveBeenCalled();
    });

    await act(async () => {
      await rendition.book.locations.generate();
    });

    await act(async () => {
      rendition._emitRelocated({ start: {} });
    });

    expect(screen.getByText("45 / 100")).toBeTruthy();
  });

  // ── 저장 위치 복원 catch 경로 (handleLocationChanged) ──

  it("첫 렌더 후 저장된 위치 복원이 reject되면 console.warn 후 localStorage에서 위치를 삭제한다", async () => {
    localStorageMock._getStore()["epub_location_42"] = "epubcfi(/saved)";
    autoLoad = false;
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    // 원본 display가 저장 위치와 폴백(undefined) 모두 reject하도록 구성하면
    // getRendition이 래핑한 display가 최종적으로 reject되어
    // handleLocationChanged의 .catch (소스 133-134)가 실행된다.
    const rendition = createMockRendition();
    rendition.display = vi.fn(() => Promise.reject(new Error("restore fail")));

    await act(async () => {
      capturedGetRendition(rendition);
    });

    localStorageMock.removeItem.mockClear();

    const lastCall =
      mockReactReader.mock.calls[mockReactReader.mock.calls.length - 1];
    await act(async () => {
      lastCall[0].locationChanged("epubcfi(/1)");
    });

    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    expect(warnSpy).toHaveBeenCalledWith(
      "[epub.js] 저장된 위치 복원 실패, 현재 위치 유지",
    );
    expect(localStorageMock.removeItem).toHaveBeenCalledWith(
      "epub_location_42",
    );

    warnSpy.mockRestore();
    errorSpy.mockRestore();
  });

  // ── spine.get() 폴백 오버라이드 ──

  it("spine.get()이 누락 항목에 falsy를 반환하면 첫 챕터로 폴백한다", async () => {
    autoLoad = true;
    render(<ViewEPUB bookId={42} />);

    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    const firstChapter = { href: "chapter1.xhtml" };
    // 원본 spine.get: 유효 target은 그대로, 누락(falsy) target은 null,
    // undefined로 호출되면 첫 챕터를 돌려준다.
    rendition.book.spine.get = vi.fn((target) => {
      if (target === undefined) return firstChapter;
      return target || null;
    });

    await act(async () => {
      capturedGetRendition(rendition);
    });

    // getRendition이 spine.get을 오버라이드(소스 187-191)했다.
    const overridden = rendition.book.spine.get;

    // 누락된 항목 접근: 원본이 null을 반환 → 첫 챕터로 폴백 (소스 189-190)
    const fallback = overridden(null);
    expect(fallback).toBe(firstChapter);

    // 유효한 항목은 그대로 반환 (소스 188만 실행)
    const valid = overridden("chapter5.xhtml");
    expect(valid).toBe("chapter5.xhtml");
  });
});

// ── rendition 이 아직 없거나 비정형 에러인 경우 ──

describe("ViewEPUB 방어 분기", () => {
  beforeEach(() => {
    mockReactReader.mockClear();
    capturedGetRendition = null;
    autoLoad = true;
  });

  it("rendition 이 준비되기 전 글자 크기를 바꿔도 예외가 없다", async () => {
    render(<ViewEPUB bookId={42} />);
    await waitFor(() => {
      expect(screen.getByLabelText("글자 크기 늘리기")).toBeTruthy();
    });

    // getRendition 을 호출하지 않았으므로 renditionRef.current 는 null 이다
    await act(async () => {
      fireEvent.click(screen.getByLabelText("글자 크기 늘리기"));
    });
    expect(localStorage.setItem).toHaveBeenCalledWith(
      "epub_fontSize",
      expect.any(String),
    );
  });

  it("rendition 이 준비되기 전 글꼴을 바꿔도 예외가 없다", async () => {
    render(<ViewEPUB bookId={42} />);
    const select = await screen.findByLabelText("글꼴 선택");

    await act(async () => {
      fireEvent.change(select, { target: { value: "serif" } });
    });
    expect(localStorage.setItem).toHaveBeenCalledWith(
      "epub_fontFamily",
      "serif",
    );
  });

  it("book.ready 가 message 없는 값으로 reject 되면 String() 으로 표시한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);
    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition({ readyReject: { code: 7 } });
    await act(async () => {
      capturedGetRendition(rendition);
      await new Promise((r) => setTimeout(r, 10));
    });

    await waitFor(() => {
      expect(screen.getByText(/EPUB 파싱 오류: \[object Object\]/)).toBeTruthy();
    });
  });

  it("메타데이터에 title 이 없으면 제목을 설정하지 않는다", async () => {
    render(<ViewEPUB bookId={42} />);
    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.book.loaded.metadata = Promise.resolve({});
    await act(async () => {
      capturedGetRendition(rendition);
      await new Promise((r) => setTimeout(r, 10));
    });

    expect(screen.queryByTestId("book-title")).toBeNull();
  });

  it("display 가 함수가 아니면 래핑을 건너뛴다", async () => {
    render(<ViewEPUB bookId={42} />);
    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.display = undefined;
    await act(async () => {
      capturedGetRendition(rendition);
      await new Promise((r) => setTimeout(r, 10));
    });

    expect(rendition.display).toBeUndefined();
  });

  it("display() 가 message 없는 값으로 reject 되면 String() 으로 표시한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);
    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.display = vi.fn(() => Promise.reject({ code: 9 }));
    await act(async () => {
      capturedGetRendition(rendition);
      await new Promise((r) => setTimeout(r, 10));
    });

    // target 없이 호출 → 폴백 없이 바로 에러 메시지
    await act(async () => {
      await rendition.display().catch(() => {});
      await new Promise((r) => setTimeout(r, 10));
    });

    await waitFor(() => {
      expect(screen.getByText(/EPUB 표시 실패: \[object Object\]/)).toBeTruthy();
    });
  });

  it("저장된 위치 이동 실패 후 첫 페이지 이동도 실패하면 에러를 표시한다", async () => {
    autoLoad = false;
    render(<ViewEPUB bookId={42} />);
    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const rendition = createMockRendition();
    rendition.display = vi.fn(() => Promise.reject({ reason: "fail" }));
    await act(async () => {
      capturedGetRendition(rendition);
      await new Promise((r) => setTimeout(r, 10));
    });

    // target 을 주면 저장 위치 삭제 후 첫 페이지 폴백을 시도한다
    await act(async () => {
      await rendition.display("epubcfi(/6/2)").catch(() => {});
      await new Promise((r) => setTimeout(r, 10));
    });

    await waitFor(() => {
      expect(screen.getByText(/EPUB 표시 실패: \[object Object\]/)).toBeTruthy();
    });
    expect(localStorage.removeItem).toHaveBeenCalledWith("epub_location_42");
  });

  it("이전 rendition 의 locations 결과는 무시한다", async () => {
    render(<ViewEPUB bookId={42} />);
    await waitFor(() => {
      expect(capturedGetRendition).not.toBeNull();
    });

    const first = createMockRendition();
    const second = createMockRendition();
    await act(async () => {
      capturedGetRendition(first);
      capturedGetRendition(second);
      await new Promise((r) => setTimeout(r, 20));
    });

    // 교체된 첫 rendition 의 locations 생성 결과는 반영되지 않는다
    expect(second.book.locations.generate).toHaveBeenCalled();
  });
});
