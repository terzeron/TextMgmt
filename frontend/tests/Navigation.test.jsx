// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import Navigation from "../src/Navigation";
import React from "react";
import * as Common from "../src/Common";

// Mock @react-oauth/google
vi.mock("@react-oauth/google", () => ({
  GoogleOAuthProvider: ({ children }) => (
    <div data-testid="google-oauth-provider">{children}</div>
  ),
  GoogleLogin: ({ onSuccess, onError }) => (
    <div>
      <button
        data-testid="google-login"
        onClick={() => onSuccess({ credential: "test-token" })}
      >
        Google Login Mock
      </button>
      <button
        data-testid="google-login-error"
        onClick={() => onError && onError()}
      >
        Google Login Error Mock
      </button>
    </div>
  ),
  googleLogout: vi.fn(),
}));

// Mock Common.js
vi.mock("../src/Common", async () => {
  const actual = await vi.importActual("../src/Common");
  return {
    ...actual,
    rawJsonGetReq: vi.fn(),
    getApiUrlPrefix: vi.fn(() => "/api"),
    tryRefreshToken: vi.fn(),
    startProactiveRefresh: vi.fn(),
    stopProactiveRefresh: vi.fn(),
  };
});

// Mock window.matchMedia
if (!window.matchMedia) {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: vi.fn().mockImplementation((query) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
}

describe("Navigation Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.__ENV__ = {
      VITE_API_URL_PREFIX: "/api",
      VITE_GOOGLE_CLIENT_ID: "test-client-id",
    };
    vi.stubGlobal("fetch", vi.fn());
    vi.stubGlobal("alert", vi.fn());
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("loads session on mount", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: { role: "admin", name: "Test User", email: "test@example.com" },
      }),
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });
    expect(fetch).toHaveBeenCalledWith("/api/auth/me", expect.anything());
  });

  it("handles login success", async () => {
    // Initial session check fails
    fetch.mockResolvedValueOnce({ ok: false });

    // Google auth verification success
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        role: "viewer",
        name: "Viewer User",
        email: "viewer@example.com",
      }),
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const loginButton = await screen.findByTestId("google-login");
    fireEvent.click(loginButton);

    await waitFor(() => {
      expect(screen.queryByText("책 편집")).toBeNull();
      expect(screen.getByText("책")).toBeDefined();
    });
  });

  it("handles logout", async () => {
    // Initial session check success
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });
    // Logout API success
    fetch.mockResolvedValueOnce({ ok: true });

    const { container } = render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    // Wait for admin menu to appear
    await screen.findByText("관리");

    // Find user dropdown toggle. It might not have a button role because it's a div
    const userDropdown = container.querySelector(".dropdown-toggle");
    fireEvent.click(userDropdown);

    const logoutButton = screen.getByText("로그아웃");
    fireEvent.click(logoutButton);

    await waitFor(() => {
      expect(fetch).toHaveBeenCalledWith("/api/auth/logout", expect.anything());
      expect(screen.getByTestId("google-login")).toBeDefined();
    });
  });

  it("performs search when keyword is entered and button clicked", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({
          status: "success",
          result: [{ id: 1, title: "Found Book" }],
          total: 1,
        });
      }
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "python" } });

    const searchButton = screen.getByRole("button", { name: /검색/i });
    fireEvent.click(searchButton);

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("search/python"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("loads hidden categories for viewer", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "viewer" } }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("hidden-categories")) {
        resolve({ status: "success", result: ["cat1", "cat2"] });
      }
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("hidden-categories"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("calls error callback when hidden-categories fetch fails", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        status: "success",
        result: { role: "viewer", name: "V" },
      }),
    });

    Common.rawJsonGetReq.mockImplementation((url, _resolve, reject) => {
      if (url.includes("hidden-categories")) {
        reject(new Error("network error"));
      }
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("hidden-categories"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("appends exclude_categories when viewer has hidden categories and searches", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        status: "success",
        result: { role: "viewer", name: "V" },
      }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("hidden-categories")) {
        resolve({ status: "success", result: ["cat1", "cat2"] });
      }
      if (url.includes("search")) {
        resolve({ status: "success", result: [], total: 0 });
      }
    });

    render(
      <MemoryRouter initialEntries={["/book-view"]}>
        <Routes>
          <Route path="/book-view" element={<Navigation />} />
        </Routes>
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "test" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      const searchCall = Common.rawJsonGetReq.mock.calls.find((c) =>
        c[0].includes("search"),
      );
      expect(searchCall[0]).toContain("exclude_categories=");
      expect(searchCall[0]).toContain("cat1");
    });
  });

  it("handles search returning non-success status", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({ status: "error" });
      }
    });

    render(
      <MemoryRouter initialEntries={["/book-view"]}>
        <Routes>
          <Route path="/book-view" element={<Navigation />} />
        </Routes>
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "test" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("search/test"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("handles search error callback", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    Common.rawJsonGetReq.mockImplementation((url, _resolve, reject) => {
      if (url.includes("search")) {
        reject(new Error("search failed"));
      }
    });

    render(
      <MemoryRouter initialEntries={["/book-view"]}>
        <Routes>
          <Route path="/book-view" element={<Navigation />} />
        </Routes>
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "test" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("search/test"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("handles form submit event for search", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({ status: "success", result: [], total: 0 });
      }
    });

    render(
      <MemoryRouter initialEntries={["/book-view"]}>
        <Routes>
          <Route path="/book-view" element={<Navigation />} />
        </Routes>
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "formtest" } });
    fireEvent.submit(input.closest("form"));

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("search/formtest"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("logs error and returns early when clientId is missing", async () => {
    window.__ENV__ = {};

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    // Should not call /auth/me when clientId is missing
    expect(fetch).not.toHaveBeenCalled();
  });

  it("alerts on Google login API error (non-ok response)", async () => {
    fetch.mockResolvedValueOnce({ ok: false }); // session check
    fetch.mockResolvedValueOnce({ ok: false, status: 500 }); // google auth fails

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const loginButton = await screen.findByTestId("google-login");
    fireEvent.click(loginButton);

    await waitFor(() => {
      expect(alert).toHaveBeenCalledWith(
        "Google 로그인 처리 중 오류가 발생했습니다.",
      );
    });
  });

  it("alerts on Google login onError callback", async () => {
    fetch.mockResolvedValueOnce({ ok: false }); // session check

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const errorButton = await screen.findByTestId("google-login-error");
    fireEvent.click(errorButton);

    await waitFor(() => {
      expect(alert).toHaveBeenCalledWith("Google 로그인 실패");
    });
  });

  it("redirects viewer on non-allowed path", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        status: "success",
        result: { role: "viewer", name: "V" },
      }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("hidden-categories")) {
        resolve({ status: "success", result: [] });
      }
    });

    render(
      <MemoryRouter initialEntries={["/admin"]}>
        <Routes>
          <Route path="/admin" element={<Navigation />} />
          <Route path="/" element={<div>Home Redirect</div>} />
        </Routes>
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("Home Redirect")).toBeDefined();
    });
  });

  it("does not set login when Google auth returns no role", async () => {
    fetch.mockResolvedValueOnce({ ok: false }); // session check fails
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ name: "NoRole User", email: "norole@example.com" }),
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const loginButton = await screen.findByTestId("google-login");
    fireEvent.click(loginButton);

    // role is missing so login state won't be set, login button stays visible
    await waitFor(() => {
      expect(screen.getByTestId("google-login")).toBeDefined();
    });
  });

  it("handles handleLoadMore success callback", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    let loadMoreFn;
    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({
          status: "success",
          result: [{ id: 1, title: "Book 1" }],
          total: 20,
        });
      }
    });

    // Use Outlet context consumer to get handleLoadMore
    const LoadMoreConsumer = () => {
      const { useOutletContext } = require("react-router-dom");
      const ctx = useOutletContext();
      loadMoreFn = ctx.handleLoadMore;
      return <div>Outlet Content</div>;
    };

    render(
      <MemoryRouter initialEntries={["/book-view"]}>
        <Routes>
          <Route path="/book-view" element={<Navigation />}>
            <Route index element={<LoadMoreConsumer />} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );

    // Trigger a search first to set searchKeyword
    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "loadtest" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("search/loadtest"),
        expect.any(Function),
        expect.any(Function),
      );
    });

    // Now mock for load more
    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({
          status: "success",
          result: [{ id: 2, title: "Book 2" }],
          total: 20,
        });
      }
    });

    // Call handleLoadMore
    await waitFor(() => {
      expect(loadMoreFn).toBeDefined();
    });
    loadMoreFn();

    await waitFor(() => {
      // Should have been called with offset=1 (one existing result)
      const calls = Common.rawJsonGetReq.mock.calls.filter((c) =>
        c[0].includes("search"),
      );
      expect(calls.length).toBeGreaterThanOrEqual(2);
    });
  });

  it("handles handleLoadMore error callback", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    let loadMoreFn;
    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({
          status: "success",
          result: [{ id: 1, title: "Book 1" }],
          total: 20,
        });
      }
    });

    const LoadMoreConsumer = () => {
      const { useOutletContext } = require("react-router-dom");
      const ctx = useOutletContext();
      loadMoreFn = ctx.handleLoadMore;
      return <div>Outlet Content</div>;
    };

    render(
      <MemoryRouter initialEntries={["/book-view"]}>
        <Routes>
          <Route path="/book-view" element={<Navigation />}>
            <Route index element={<LoadMoreConsumer />} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "errtest" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("search/errtest"),
        expect.any(Function),
        expect.any(Function),
      );
    });

    // Now mock for load more to fail
    Common.rawJsonGetReq.mockImplementation((url, _resolve, reject) => {
      if (url.includes("search")) {
        reject(new Error("load more failed"));
      }
    });

    await waitFor(() => {
      expect(loadMoreFn).toBeDefined();
    });
    loadMoreFn();

    await waitFor(() => {
      const calls = Common.rawJsonGetReq.mock.calls.filter((c) =>
        c[0].includes("search"),
      );
      expect(calls.length).toBeGreaterThanOrEqual(2);
    });
  });

  it("navigates to book-view when searching from home", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({ status: "success", result: [] });
      }
    });

    render(
      <MemoryRouter initialEntries={["/"]}>
        <Routes>
          <Route path="/" element={<Navigation />} />
          <Route path="/book-view" element={<div>Book View Page</div>} />
        </Routes>
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "test" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      expect(screen.getByText("Book View Page")).toBeDefined();
    });
  });

  it("refreshes token and retries /auth/me on 401", async () => {
    // 1st /auth/me → 401 (access token expired)
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });
    // tryRefreshToken succeeds
    Common.tryRefreshToken.mockResolvedValueOnce(true);
    // 2nd /auth/me → success (after refresh)
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Refreshed User",
          email: "refreshed@example.com",
        },
      }),
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });
    expect(Common.tryRefreshToken).toHaveBeenCalledTimes(1);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it("stays logged out when refresh fails on 401", async () => {
    // /auth/me → 401
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });
    // tryRefreshToken fails
    Common.tryRefreshToken.mockResolvedValueOnce(false);

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByTestId("google-login")).toBeDefined();
    });
    expect(Common.tryRefreshToken).toHaveBeenCalledTimes(1);
    // Should NOT retry /auth/me after failed refresh
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("stays logged out when refresh succeeds but retry /auth/me fails", async () => {
    // 1st /auth/me → 401
    fetch.mockResolvedValueOnce({ ok: false, status: 401 });
    // tryRefreshToken succeeds
    Common.tryRefreshToken.mockResolvedValueOnce(true);
    // 2nd /auth/me → still fails (e.g., server error)
    fetch.mockResolvedValueOnce({ ok: false, status: 500 });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByTestId("google-login")).toBeDefined();
    });
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it("starts proactive refresh after successful session load", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Test",
          email: "test@example.com",
          expires_in: 3600,
        },
      }),
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });
    expect(Common.startProactiveRefresh).toHaveBeenCalledWith(3600);
  });

  it("starts proactive refresh with default when expires_in is absent", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Test",
          email: "test@example.com",
        },
      }),
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });
    // expires_in이 없으면 기본값 7200 사용
    expect(Common.startProactiveRefresh).toHaveBeenCalledWith(7200);
  });

  it("refreshes token on visibility change to visible", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Test",
          email: "test@example.com",
          expires_in: 7200,
        },
      }),
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });

    Common.tryRefreshToken.mockClear();

    // 탭이 다시 활성화되는 이벤트 시뮬레이션
    Object.defineProperty(document, "visibilityState", {
      value: "visible",
      writable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));

    expect(Common.tryRefreshToken).toHaveBeenCalledTimes(1);
  });

  it("does not refresh token when tab becomes hidden", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Test",
          email: "test@example.com",
          expires_in: 7200,
        },
      }),
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });

    Common.tryRefreshToken.mockClear();

    // 탭이 숨겨지는 이벤트 — refresh 호출 안 됨
    Object.defineProperty(document, "visibilityState", {
      value: "hidden",
      writable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));

    expect(Common.tryRefreshToken).not.toHaveBeenCalled();
  });

  it("cleans up visibility listener and timer on unmount", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Test",
          email: "test@example.com",
          expires_in: 7200,
        },
      }),
    });

    const { unmount } = render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });

    Common.stopProactiveRefresh.mockClear();
    unmount();

    expect(Common.stopProactiveRefresh).toHaveBeenCalled();
  });

  it("stops proactive refresh on logout", async () => {
    // 로그인 상태로 시작
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Test",
          email: "test@example.com",
          expires_in: 7200,
        },
      }),
    });

    const { container } = render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });

    Common.stopProactiveRefresh.mockClear();

    // 드롭다운 열기 후 로그아웃 클릭
    fetch.mockResolvedValueOnce({ ok: true, status: 200 });
    const userDropdown = container.querySelector(".dropdown-toggle");
    fireEvent.click(userDropdown);
    const logoutBtn = screen.getByText("로그아웃");
    fireEvent.click(logoutBtn);

    await waitFor(() => {
      expect(Common.stopProactiveRefresh).toHaveBeenCalled();
    });
  });
});
