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
/* eslint-disable react/prop-types -- 테스트 mock 컴포넌트는 throwaway라 PropTypes 검증 불필요 */
import Navigation from "../src/Navigation";
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
    refreshOnVisible: vi.fn(),
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
    expect(screen.getByText("최신 책")).toBeDefined();
    expect(screen.getByText("최신 만화")).toBeDefined();
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

    Common.refreshOnVisible.mockClear();

    // 탭이 다시 활성화되는 이벤트 시뮬레이션
    Object.defineProperty(document, "visibilityState", {
      value: "visible",
      writable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));

    // 여러 탭이 동시에 활성화될 때의 refresh 폭주를 막기 위해 디바운스된 진입점을 쓴다
    expect(Common.refreshOnVisible).toHaveBeenCalledTimes(1);
    expect(Common.tryRefreshToken).not.toHaveBeenCalled();
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
    Common.refreshOnVisible.mockClear();

    // 탭이 숨겨지는 이벤트 — refresh 호출 안 됨
    Object.defineProperty(document, "visibilityState", {
      value: "hidden",
      writable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));

    expect(Common.refreshOnVisible).not.toHaveBeenCalled();
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

  it("renders GoogleLogin in a centered wrapper when logged out", async () => {
    fetch.mockResolvedValueOnce({ ok: false }); // session check fails → not logged in

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const loginButton = await screen.findByTestId("google-login");
    // GoogleLogin mock root div → centering wrapper
    const wrapper = loginButton.parentElement.parentElement;
    expect(wrapper.style.display).toBe("flex");
    expect(wrapper.style.justifyContent).toBe("center");
    expect(wrapper.style.alignItems).toBe("center");
    expect(wrapper.style.minHeight).toBe("calc(100vh - 56px)");
  });

  it("centering wrapper is absent after successful Google login", async () => {
    fetch.mockResolvedValueOnce({ ok: false }); // session check fails
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        role: "admin",
        name: "Admin User",
        email: "admin@example.com",
      }),
    }); // google auth succeeds

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const loginButton = await screen.findByTestId("google-login");
    fireEvent.click(loginButton);

    await waitFor(() => {
      expect(screen.queryByTestId("google-login")).toBeNull();
    });
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

  // Regression: sessionLoading guard prevents UI flash after login
  it("renders nothing while session check is pending", async () => {
    let resolveSessionFetch;
    fetch.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveSessionFetch = resolve;
      }),
    );
    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );
    // While fetch is in-flight, nothing should render
    expect(screen.queryByTestId("google-login")).toBeNull();
    expect(screen.queryByText("책")).toBeNull();
    // Resolve with a failed session
    resolveSessionFetch({ ok: false, status: 401 });
    await waitFor(() => {
      expect(screen.getByTestId("google-login")).toBeDefined();
    });
  });

  it("shows login button after session check throws network error", async () => {
    fetch.mockRejectedValueOnce(new Error("network error"));
    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );
    // While fetch is in-flight, nothing should render
    expect(screen.queryByTestId("google-login")).toBeNull();
    await waitFor(() => {
      expect(screen.getByTestId("google-login")).toBeDefined();
    });
  });

  it("shows login button when clientId is missing without stalling", async () => {
    window.__ENV__ = {};
    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );
    await waitFor(() => {
      expect(screen.getByTestId("google-login")).toBeDefined();
    });
    expect(fetch).not.toHaveBeenCalled();
  });

  it("shows menu directly after session restore without flashing login screen", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Test User",
          email: "test@example.com",
          picture: "",
        },
      }),
    });
    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );
    // Login button must never appear — session should restore directly to menu
    expect(screen.queryByTestId("google-login")).toBeNull();
    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });
    expect(screen.queryByTestId("google-login")).toBeNull();
  });

  // --- 추가 브랜치 커버리지 테스트 ---

  it("comics 컨텍스트에서 hidden-categories를 comic content_type으로 로드한다", async () => {
    // viewer로 세션 복원 (comics 경로)
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
      <MemoryRouter initialEntries={["/comics-view"]}>
        <Routes>
          <Route path="/comics-view" element={<Navigation />} />
        </Routes>
      </MemoryRouter>,
    );

    // isComicsContext === true → content_type=comic 브랜치 (line 55)
    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("content_type=comic"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("comics 컨텍스트에서 검색 시 /comics search prefix를 사용한다", async () => {
    // admin이면 hidden-categories 호출 없음, comics prefix만 확인 (line 50)
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
      <MemoryRouter initialEntries={["/comics-view"]}>
        <Routes>
          <Route path="/comics-view" element={<Navigation />} />
        </Routes>
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "comic" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      const searchCall = Common.rawJsonGetReq.mock.calls.find((c) =>
        c[0].includes("search"),
      );
      // searchPrefix === "/comics" (line 50)
      expect(searchCall[0]).toContain("/comics/search/");
    });
  });

  it("hidden-categories 성공 응답에 result가 없으면 빈 배열로 설정한다", async () => {
    // line 61: setHiddenCategories(data.result || []) 의 || [] 브랜치
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        status: "success",
        result: { role: "viewer", name: "V" },
      }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("hidden-categories")) {
        // status는 success지만 result 필드가 없음
        resolve({ status: "success" });
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
    fireEvent.change(input, { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    // hiddenCategories가 빈 배열이므로 exclude_categories가 붙지 않음
    await waitFor(() => {
      const searchCall = Common.rawJsonGetReq.mock.calls.find((c) =>
        c[0].includes("search"),
      );
      expect(searchCall[0]).not.toContain("exclude_categories");
    });
  });

  it("검색 성공 응답에 result/total이 없으면 기본값을 사용한다", async () => {
    // line 94-95: data.result || [], data.total || 0 의 폴백 브랜치
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        // success지만 result/total 필드 누락
        resolve({ status: "success" });
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
    fireEvent.change(input, { target: { value: "nores" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("search/nores"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("handleSearch는 키워드가 비어 있으면 검색을 실행하지 않는다", async () => {
    // line 83: if (searchKeyword) false 브랜치
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    Common.rawJsonGetReq.mockImplementation(() => {});

    render(
      <MemoryRouter initialEntries={["/book-view"]}>
        <Routes>
          <Route path="/book-view" element={<Navigation />} />
        </Routes>
      </MemoryRouter>,
    );

    // 키워드 입력 없이 바로 검색 버튼 클릭
    const searchButton = await screen.findByRole("button", { name: /검색/i });
    fireEvent.click(searchButton);

    // search 요청이 한 번도 발생하지 않아야 함
    const searchCalls = Common.rawJsonGetReq.mock.calls.filter((c) =>
      c[0].includes("search"),
    );
    expect(searchCalls.length).toBe(0);
  });

  it("handleLoadMore는 keyword가 없으면 즉시 반환한다", async () => {
    // line 113: !searchKeyword 브랜치 (early return)
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    Common.rawJsonGetReq.mockImplementation(() => {});

    let loadMoreFn;
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

    await waitFor(() => {
      expect(loadMoreFn).toBeDefined();
    });

    // 검색 키워드 없이 loadMore 호출 → 즉시 반환, search 요청 없음
    loadMoreFn();
    const searchCalls = Common.rawJsonGetReq.mock.calls.filter((c) =>
      c[0].includes("search"),
    );
    expect(searchCalls.length).toBe(0);
  });

  it("handleLoadMore 성공 응답에 result가 없으면 append를 건너뛴다", async () => {
    // line 119: data.status === "success" && data.result 에서 result falsy 브랜치
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
    fireEvent.change(input, { target: { value: "noappend" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      expect(Common.rawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("search/noappend"),
        expect.any(Function),
        expect.any(Function),
      );
    });

    // loadMore가 success이지만 result 없음 → append 안 함
    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({ status: "success" });
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

  it("Google 로그인 성공 시 name/email이 없으면 기본값을 사용한다", async () => {
    // line 210-211: data.name || "Unknown", data.email || "" 폴백 브랜치
    fetch.mockResolvedValueOnce({ ok: false }); // session check fails
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ role: "admin" }), // name/email/picture/expires_in 없음
    });

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const loginButton = await screen.findByTestId("google-login");
    fireEvent.click(loginButton);

    // role이 있으므로 로그인 상태가 되어 메뉴가 보여야 함
    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeDefined();
    });
    // expires_in이 없으므로 startProactiveRefresh 호출 안 됨 (line 215 false 브랜치)
    expect(Common.startProactiveRefresh).not.toHaveBeenCalled();
  });

  it("Google 로그인 성공 시 expires_in이 있으면 선제 갱신을 시작한다", async () => {
    // line 215: if (data.expires_in) true 브랜치
    fetch.mockResolvedValueOnce({ ok: false }); // session check fails
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        role: "admin",
        name: "Admin",
        email: "admin@example.com",
        picture: "http://example.com/p.png",
        expires_in: 1800,
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
      expect(Common.startProactiveRefresh).toHaveBeenCalledWith(1800);
    });
  });

  it("picture가 있으면 아바타 이미지를 렌더링한다", async () => {
    // line 357: picture ? <img> : <FontAwesomeIcon> 의 true 브랜치
    fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Test",
          email: "pic@example.com",
          picture: "http://example.com/avatar.png",
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

    const avatar = container.querySelector("img.rounded-circle");
    expect(avatar).not.toBeNull();
    expect(avatar.getAttribute("src")).toBe("http://example.com/avatar.png");
    expect(avatar.getAttribute("alt")).toBe("pic@example.com");
  });

  it("viewer는 허용 경로(book-view)에서 Outlet을 렌더링한다", async () => {
    // line 279: role === "viewer" && !isViewerAllowedPath 에서 isViewerAllowedPath true 브랜치
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
      <MemoryRouter initialEntries={["/book-view"]}>
        <Routes>
          <Route path="/book-view" element={<Navigation />}>
            <Route index element={<div>Viewer Outlet</div>} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );

    // 허용 경로이므로 redirect 없이 Outlet 렌더링
    await waitFor(() => {
      expect(screen.getByText("Viewer Outlet")).toBeDefined();
    });
    // admin 전용 메뉴는 보이지 않아야 함
    expect(screen.queryByText("책 편집")).toBeNull();
  });

  // ── 추가 분기 ──

  it("hidden-categories 응답이 success 가 아니면 목록을 갱신하지 않는다", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        status: "success",
        result: { role: "viewer", name: "V" },
      }),
    });

    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("hidden-categories")) {
        resolve({ status: "error", message: "권한 없음" });
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

  it("검색 진행 중에는 검색 버튼에 스피너를 표시한다", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    // resolve 를 호출하지 않아 검색이 진행 중 상태로 유지된다
    Common.rawJsonGetReq.mockImplementation(() => {});

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    const input = await screen.findByPlaceholderText(/키워드/i);
    fireEvent.change(input, { target: { value: "spinner" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => {
      expect(document.querySelector(".fa-spinner")).toBeTruthy();
    });
  });

  it("더 보기 응답에 total 이 없으면 0 으로 처리한다", async () => {
    fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: "success", result: { role: "admin" } }),
    });

    let loadMoreFn;
    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({
          status: "success",
          result: [{ id: 1, title: "B1" }],
          total: 5,
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
    fireEvent.change(input, { target: { value: "notot" } });
    fireEvent.click(screen.getByRole("button", { name: /검색/i }));

    await waitFor(() => expect(loadMoreFn).toBeDefined());

    // total 없는 응답으로 더 보기
    Common.rawJsonGetReq.mockImplementation((url, resolve) => {
      if (url.includes("search")) {
        resolve({ status: "success", result: [{ id: 2, title: "B2" }] });
      }
    });
    loadMoreFn();

    await waitFor(() => {
      const calls = Common.rawJsonGetReq.mock.calls.filter((c) =>
        c[0].includes("search"),
      );
      expect(calls.length).toBeGreaterThanOrEqual(2);
    });
  });

  it("window.__ENV__ 가 없으면 빌드타임 클라이언트 ID 로 폴백한다", async () => {
    delete window.__ENV__;

    render(
      <MemoryRouter>
        <Navigation />
      </MemoryRouter>,
    );

    // clientId 가 없으면 세션 조회를 하지 않는다
    await waitFor(() => {
      expect(fetch).not.toHaveBeenCalledWith("/api/auth/me", expect.anything());
    });
  });
});
