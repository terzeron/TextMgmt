// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import {
  render,
  screen,
  cleanup,
  fireEvent,
  waitFor,
} from "@testing-library/react";

afterEach(cleanup);

const { mockUseOutletContext } = vi.hoisted(() => ({
  mockUseOutletContext: vi.fn(() => ({
    searchResults: [],
    hasSearched: false,
  })),
}));

vi.mock("react-router-dom", () => ({
  useOutletContext: mockUseOutletContext,
}));

vi.mock("../src/CategoryAdmin", () => ({
  default: ({ contentType }) => (
    <div data-testid={`category-admin-${contentType || "book"}`}>
      카테고리 관리
    </div>
  ),
}));

vi.mock("../src/LoginSessionAdmin", () => ({
  default: () => <div data-testid="login-session-admin">로그인 세션</div>,
}));

vi.mock("../src/ViewHistoryAdmin", () => ({
  default: () => <div data-testid="view-history-admin">조회 목록</div>,
}));

import Admin from "../src/Admin";

// 요청된 표시 순서
const SUBTAB_LABELS = [
  "로그인 세션 관리",
  "사용자별 조회 목록",
  "책 카테고리 관리",
  "만화 카테고리 관리",
];

describe("Admin", () => {
  it("2차 탭 4개를 요청된 순서대로 렌더링한다", () => {
    render(<Admin />);

    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual(SUBTAB_LABELS);
  });

  it("관리 내부 서브 탭에 전용 modern tab class를 적용한다", () => {
    render(<Admin />);

    const tabList = screen.getByRole("tablist");
    expect(tabList.classList.contains("admin-modern-tabs")).toBe(true);
    expect(tabList.classList.contains("nav-tabs")).toBe(true);
  });

  it("기본으로 로그인 세션 관리 탭을 보여준다", () => {
    render(<Admin />);

    expect(screen.getByTestId("login-session-admin")).toBeTruthy();
    // 선택되지 않은 탭의 내용은 렌더링하지 않는다
    expect(screen.queryByTestId("view-history-admin")).toBeNull();
    expect(screen.queryByTestId("category-admin-book")).toBeNull();
    expect(screen.queryByTestId("category-admin-comic")).toBeNull();
  });

  it("사용자별 조회 목록 탭으로 전환한다", async () => {
    render(<Admin />);

    fireEvent.click(screen.getByRole("tab", { name: "사용자별 조회 목록" }));

    await waitFor(() =>
      expect(screen.getByTestId("view-history-admin")).toBeTruthy(),
    );
    expect(screen.queryByTestId("login-session-admin")).toBeNull();
  });

  it("책 카테고리 관리 탭으로 전환한다", async () => {
    render(<Admin />);

    fireEvent.click(screen.getByRole("tab", { name: "책 카테고리 관리" }));

    await waitFor(() =>
      expect(screen.getByTestId("category-admin-book")).toBeTruthy(),
    );
    expect(screen.queryByTestId("category-admin-comic")).toBeNull();
  });

  it("만화 카테고리 관리 탭으로 전환한다", async () => {
    render(<Admin />);

    fireEvent.click(screen.getByRole("tab", { name: "만화 카테고리 관리" }));

    await waitFor(() =>
      expect(screen.getByTestId("category-admin-comic")).toBeTruthy(),
    );
    expect(screen.queryByTestId("category-admin-book")).toBeNull();
  });

  it("탭에 접근성 role 과 선택 상태가 있다", () => {
    render(<Admin />);

    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(4);
    // 키보드 내비게이션을 위해 선택된 탭이 aria-selected 로 노출된다
    const selected = tabs.filter(
      (t) => t.getAttribute("aria-selected") === "true",
    );
    expect(selected).toHaveLength(1);
    expect(selected[0].textContent).toBe("로그인 세션 관리");
  });
});
