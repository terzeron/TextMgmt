// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";

const outletContext = { role: undefined };

vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual("react-router-dom");
  return {
    ...actual,
    useOutletContext: () => outletContext,
  };
});

import Home from "../src/Home";

describe("Home", () => {
  beforeEach(() => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve({ ok: true })),
    );
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    outletContext.role = undefined;
  });

  it("role 이 없으면 /wake 를 호출하지 않는다", () => {
    outletContext.role = undefined;
    render(<Home />);
    expect(screen.getByText("우진은채네 책방")).toBeDefined();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("role 이 있으면 /wake 를 호출한다", () => {
    outletContext.role = "admin";
    render(<Home />);
    expect(fetch).toHaveBeenCalledWith("/wake");
  });

  it("/wake 호출이 실패해도 렌더링을 막지 않는다", () => {
    outletContext.role = "user";
    fetch.mockReturnValueOnce(Promise.reject(new Error("offline")));
    render(<Home />);
    expect(screen.getByText("우진은채네 책방")).toBeDefined();
  });

  it("빌드 ID를 화면 하단에 고정해 표시한다", () => {
    render(<Home />);
    const footer = screen.getByTestId("build-id");
    // vite define이 타임스탬프나 이미지 태그를 박으므로 값 형식은 검증하지 않는다
    expect(footer.textContent).toMatch(/^build: .+$/);
    expect(footer.style.position).toBe("fixed");
    expect(footer.style.bottom).toBe("0px");
    expect(footer.style.left).toBe("0px");
    expect(footer.style.right).toBe("0px");
  });
});
