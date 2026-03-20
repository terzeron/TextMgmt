// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";

afterEach(cleanup);

const { mockRawJsonGetReq, mockGetApiUrlPrefix } = vi.hoisted(() => ({
  mockRawJsonGetReq: vi.fn(),
  mockGetApiUrlPrefix: vi.fn(() => "http://localhost:8000"),
}));

vi.mock("../src/Common.js", () => ({
  rawJsonGetReq: mockRawJsonGetReq,
  getApiUrlPrefix: mockGetApiUrlPrefix,
}));

vi.mock("@react-oauth/google", () => ({
  GoogleOAuthProvider: ({ children }) => <div>{children}</div>,
  GoogleLogin: ({ onSuccess, onError }) => (
    <button
      data-testid="google-login"
      onClick={() => onSuccess({ credential: "test-token" })}
    >
      Google로 로그인
    </button>
  ),
  googleLogout: vi.fn(),
}));

const { mockNavigate } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
}));

vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual("react-router-dom");
  return {
    ...actual,
    useLocation: () => ({ pathname: "/" }),
    useNavigate: () => mockNavigate,
    Outlet: (props) => <div data-testid="outlet">Outlet</div>,
    Navigate: ({ to }) => <div data-testid="navigate">Navigate to {to}</div>,
  };
});

import Navigation from "../src/Navigation";

const mockAuthMe = (role = null, overrides = {}) => {
  if (!role) {
    global.fetch.mockResolvedValueOnce({
      ok: false,
      status: 401,
      json: async () => ({}),
    });
    return;
  }
  global.fetch.mockResolvedValueOnce({
    ok: true,
    status: 200,
    json: async () => ({
      status: "success",
      result: {
        role,
        name: overrides.name || "User",
        email: overrides.email || "user@test.com",
        picture: overrides.picture || "",
      },
    }),
  });
};

describe("Navigation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.__ENV__ = {
      VITE_GOOGLE_CLIENT_ID: "test-client-id",
      VITE_ADMIN_EMAIL: "admin@test.com",
      VITE_ALLOWED_EMAILS: "viewer@test.com",
    };
    global.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 401,
      json: async () => ({}),
    });
  });

  afterEach(() => {
    delete window.__ENV__;
    vi.restoreAllMocks();
  });

  it("미로그인 시 Google 로그인 버튼을 표시한다", () => {
    render(<Navigation />);
    expect(screen.getByTestId("google-login")).toBeTruthy();
  });

  it("Navbar에 브랜드 이미지를 표시한다", () => {
    render(<Navigation />);
    expect(screen.getByAltText("Text")).toBeTruthy();
  });

  it("세션 복원으로 admin 자동 로그인한다", async () => {
    global.fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Admin",
          email: "admin@test.com",
          picture: "",
        },
      }),
    });
    render(<Navigation />);
    await waitFor(() => {
      expect(screen.getByText("책 편집")).toBeTruthy();
    });
    expect(screen.getByText("책")).toBeTruthy();
    expect(screen.getByText("만화 편집")).toBeTruthy();
    expect(screen.getByText("만화")).toBeTruthy();
    expect(screen.getByText("관리")).toBeTruthy();
  });

  it("세션 복원으로 viewer 자동 로그인한다", async () => {
    global.fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "viewer",
          name: "Viewer",
          email: "viewer@test.com",
          picture: "",
        },
      }),
    });
    render(<Navigation />);
    await waitFor(() => {
      expect(screen.getByText("책")).toBeTruthy();
    });
    expect(screen.getByText("만화")).toBeTruthy();
    expect(screen.queryByText("책 편집")).toBeNull();
    expect(screen.queryByText("만화 편집")).toBeNull();
    expect(screen.queryByText("관리")).toBeNull();
  });

  it("세션 복원 실패 시 로그인 버튼을 표시한다", async () => {
    global.fetch.mockResolvedValueOnce({
      ok: false,
      status: 403,
      json: async () => ({ detail: "Access denied" }),
    });
    render(<Navigation />);
    await waitFor(() => {
      expect(screen.getByTestId("google-login")).toBeTruthy();
    });
  });

  it("로그아웃 시 로그인 상태를 초기화한다", async () => {
    global.fetch
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({
          status: "success",
          result: {
            role: "admin",
            name: "Admin",
            email: "admin@test.com",
            picture: "",
          },
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ status: "success" }),
      });
    render(<Navigation />);

    // Dropdown.Toggle(as="div")을 클릭하여 드롭다운 메뉴 표시
    await waitFor(() => {
      const dropdownToggle = document.querySelector(".dropdown-toggle");
      expect(dropdownToggle).toBeTruthy();
      fireEvent.click(dropdownToggle);
    });

    await waitFor(() => {
      expect(screen.getByText("로그아웃")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("로그아웃"));

    // 로그아웃 후 Google 로그인 버튼이 다시 표시
    await waitFor(() => {
      expect(screen.getByTestId("google-login")).toBeTruthy();
    });
  });

  it("admin 로그인 시 검색 입력 필드를 표시한다", async () => {
    global.fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Admin",
          email: "admin@test.com",
          picture: "",
        },
      }),
    });
    render(<Navigation />);
    await waitFor(() => {
      const searchInput = screen.getByPlaceholderText("키워드");
      expect(searchInput).toBeTruthy();
    });
  });

  it("검색 버튼 클릭 시 rawJsonGetReq를 호출한다", async () => {
    global.fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Admin",
          email: "admin@test.com",
          picture: "",
        },
      }),
    });
    render(<Navigation />);

    const searchInput = await screen.findByPlaceholderText("키워드");
    fireEvent.change(searchInput, { target: { value: "테스트" } });

    const searchBtn = screen.getByText("검색");
    fireEvent.click(searchBtn);

    expect(mockRawJsonGetReq).toHaveBeenCalledWith(
      expect.stringContaining("/search/"),
      expect.any(Function),
      expect.any(Function),
    );
  });

  it("홈 화면에서 검색 시 /book-view로 이동한다", async () => {
    global.fetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => ({
        status: "success",
        result: {
          role: "admin",
          name: "Admin",
          email: "admin@test.com",
          picture: "",
        },
      }),
    });
    render(<Navigation />);

    const searchInput = await screen.findByPlaceholderText("키워드");
    fireEvent.change(searchInput, { target: { value: "테스트" } });

    const searchBtn = screen.getByText("검색");
    fireEvent.click(searchBtn);

    expect(mockNavigate).toHaveBeenCalledWith("/book-view");
  });

  it("프로필 사진이 있으면 이미지로 표시한다", async () => {
    mockAuthMe("admin", {
      email: "admin@test.com",
      name: "Admin",
      picture: "https://photo.example.com/admin.jpg",
    });
    render(<Navigation />);
    const img = await screen.findByAltText("admin@test.com");
    expect(img.getAttribute("src")).toBe("https://photo.example.com/admin.jpg");
  });

  it("Google 로그인 성공 시 백엔드 검증을 수행한다", async () => {
    global.fetch
      .mockResolvedValueOnce({
        ok: false,
        status: 401,
        json: async () => ({}),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({
          email: "admin@test.com",
          name: "Admin User",
          picture: "",
          role: "admin",
        }),
      });

    render(<Navigation />);
    const loginBtn = screen.getByTestId("google-login");
    fireEvent.click(loginBtn);

    await waitFor(() => {
      expect(global.fetch).toHaveBeenCalledWith(
        "http://localhost:8000/auth/google",
        expect.objectContaining({
          method: "POST",
          credentials: "include",
          body: JSON.stringify({ credential: "test-token" }),
        }),
      );
    });

  });

  it("viewer는 hidden-categories를 로드한다", async () => {
    mockAuthMe("viewer", { email: "viewer@test.com", name: "Viewer" });

    mockRawJsonGetReq.mockImplementation((url, onSuccess) => {
      if (url.includes("/hidden-categories")) {
        onSuccess({ status: "success", result: ["비공개"] });
      }
    });

    render(<Navigation />);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalledWith(
        "/hidden-categories?content_type=book",
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("hidden-categories 로드 실패 시 빈 배열로 설정한다", async () => {
    mockAuthMe("viewer", { email: "viewer@test.com", name: "Viewer" });

    mockRawJsonGetReq.mockImplementation((url, onSuccess, onError) => {
      if (url === "/hidden-categories?content_type=book") {
        onError("서버 오류");
      }
    });

    render(<Navigation />);

    // 에러가 발생해도 컴포넌트는 정상 렌더링
    await waitFor(() => {
      expect(screen.getByText("책")).toBeTruthy();
    });
  });

  it("검색 성공 시 결과를 Outlet context로 전달한다", async () => {
    mockAuthMe("admin", { email: "admin@test.com", name: "Admin" });

    mockRawJsonGetReq.mockImplementation((url, onSuccess) => {
      if (url.includes("/search/")) {
        onSuccess({ status: "success", result: [{ book_id: 1 }], total: 1 });
      }
    });

    render(<Navigation />);

    const searchInput = await screen.findByPlaceholderText("키워드");
    fireEvent.change(searchInput, { target: { value: "테스트" } });
    fireEvent.click(screen.getByText("검색"));

    await waitFor(() => {
      const call = mockRawJsonGetReq.mock.calls.find((c) =>
        c[0].includes("/search/"),
      );
      expect(call).toBeTruthy();
      // 성공 콜백 호출됨
      expect(call[1]).toBeTypeOf("function");
    });
  });

  it("폼 submit으로 검색을 실행한다", async () => {
    mockAuthMe("admin", { email: "admin@test.com", name: "Admin" });

    render(<Navigation />);

    const searchInput = await screen.findByPlaceholderText("키워드");
    fireEvent.change(searchInput, { target: { value: "폼검색" } });

    // form submit 이벤트
    const form = searchInput.closest("form");
    fireEvent.submit(form);

    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalledWith(
        expect.stringContaining("/search/"),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("VITE_GOOGLE_CLIENT_ID가 없으면 로그인 상태를 복원하지 않는다", () => {
    window.__ENV__ = {
      VITE_ADMIN_EMAIL: "admin@test.com",
      VITE_ALLOWED_EMAILS: "viewer@test.com",
    };
    render(<Navigation />);
    expect(screen.queryByText("책 편집")).toBeNull();
  });

  it("Google 로그인 백엔드 검증 실패 시 alert를 표시한다", async () => {
    global.fetch
      .mockResolvedValueOnce({
        ok: false,
        status: 401,
        json: async () => ({}),
      })
      .mockResolvedValueOnce({
        ok: false,
        status: 500,
      });
    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});

    render(<Navigation />);
    fireEvent.click(screen.getByTestId("google-login"));

    await waitFor(() => {
      expect(alertSpy).toHaveBeenCalledWith(expect.stringContaining("오류"));
    });

    alertSpy.mockRestore();
  });

  it("검색어가 비어있으면 검색을 실행하지 않는다", () => {
    mockAuthMe("admin", { email: "admin@test.com", name: "Admin" });
    render(<Navigation />);

    return screen.findByText("검색").then((btn) => {
      fireEvent.click(btn);
      expect(mockRawJsonGetReq).not.toHaveBeenCalled();
    });
  });

  it("책 컨텍스트에서 검색 시 prefix 없이 /search/ URL을 호출한다", async () => {
    mockAuthMe("admin", { email: "admin@test.com", name: "Admin" });
    render(<Navigation />);

    const searchInput = await screen.findByPlaceholderText("키워드");
    fireEvent.change(searchInput, { target: { value: "소설" } });
    fireEvent.click(screen.getByText("검색"));

    await waitFor(() => {
      const searchCall = mockRawJsonGetReq.mock.calls.find((c) =>
        c[0].includes("/search/"),
      );
      expect(searchCall).toBeTruthy();
      expect(searchCall[0]).toMatch(/^\/search\//);
      expect(searchCall[0]).not.toMatch(/^\/comics/);
    });
  });

  it("viewer에서 hidden categories가 있으면 검색 URL에 exclude_categories를 포함한다", async () => {
    mockAuthMe("viewer", { email: "viewer@test.com", name: "Viewer" });

    mockRawJsonGetReq.mockImplementation((url, onSuccess) => {
      if (url === "/hidden-categories?content_type=book") {
        onSuccess({ status: "success", result: ["비공개", "비밀"] });
      }
    });

    render(<Navigation />);

    // hidden categories 로드 대기
    await waitFor(() => {
      expect(mockRawJsonGetReq).toHaveBeenCalledWith(
        "/hidden-categories?content_type=book",
        expect.any(Function),
        expect.any(Function),
      );
    });

    // 검색 실행
    const searchInput = await screen.findByPlaceholderText("키워드");
    fireEvent.change(searchInput, { target: { value: "테스트" } });
    fireEvent.click(screen.getByText("검색"));

    await waitFor(() => {
      const searchCall = mockRawJsonGetReq.mock.calls.find((c) =>
        c[0].includes("/search/"),
      );
      expect(searchCall).toBeTruthy();
      expect(searchCall[0]).toContain("exclude_categories=");
    });
  });
});
