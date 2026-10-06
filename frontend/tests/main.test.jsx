// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { mockRender, mockCreateRoot, mockInitLogging } = vi.hoisted(() => {
  const render = vi.fn();
  return {
    mockRender: render,
    mockCreateRoot: vi.fn(() => ({ render })),
    mockInitLogging: vi.fn(),
  };
});

vi.mock("react-dom/client", () => ({
  default: { createRoot: mockCreateRoot },
  createRoot: mockCreateRoot,
}));
vi.mock("../src/App", () => ({ default: () => null }));
vi.mock("../src/ErrorBoundary", () => ({ default: ({ children }) => children }));
vi.mock("../src/clientLogger", () => ({
  initGlobalErrorLogging: mockInitLogging,
}));
vi.mock("../src/index.css", () => ({}));

describe("main entry", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    document.body.innerHTML = '<div id="root"></div>';
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("전역 오류 로깅을 시작하고 #root에 앱을 렌더링한다", async () => {
    await import("../src/main");

    expect(mockInitLogging).toHaveBeenCalledTimes(1);
    expect(mockCreateRoot).toHaveBeenCalledWith(document.getElementById("root"));
    expect(mockRender).toHaveBeenCalledTimes(1);
  });
});
