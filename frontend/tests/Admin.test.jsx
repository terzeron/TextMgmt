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
  default: ({ contentType, title }) => (
    <div data-testid={`category-admin-${contentType || "book"}`}>
      {title || "카테고리 관리"}
    </div>
  ),
}));

vi.mock("../src/LoginSessionAdmin", () => ({
  default: () => <div data-testid="login-session-admin">로그인 세션</div>,
}));

import Admin from "../src/Admin";

const SUBTAB_LABELS = [
  "책 카테고리 관리",
  "만화 카테고리 관리",
  "로그인 세션 관리",
];

describe("Admin", () => {
  it("2차 탭 3개를 렌더링한다", () => {
    render(<Admin />);

    for (const label of SUBTAB_LABELS) {
      expect(screen.getByRole("tab", { name: label })).toBeTruthy();
    }
  });

  it("기본으로 책 카테고리 탭을 보여준다", () => {
    render(<Admin />);

    expect(screen.getByTestId("category-admin-book")).toBeTruthy();
    // 선택되지 않은 탭의 내용은 렌더링하지 않는다
    expect(screen.queryByTestId("category-admin-comic")).toBeNull();
    expect(screen.queryByTestId("login-session-admin")).toBeNull();
  });

  it("만화 카테고리 탭으로 전환한다", async () => {
    render(<Admin />);

    fireEvent.click(screen.getByRole("tab", { name: "만화 카테고리 관리" }));

    await waitFor(() =>
      expect(screen.getByTestId("category-admin-comic")).toBeTruthy(),
    );
    expect(screen.queryByTestId("category-admin-book")).toBeNull();
  });

  it("로그인 세션 탭으로 전환한다", async () => {
    render(<Admin />);

    fireEvent.click(screen.getByRole("tab", { name: "로그인 세션 관리" }));

    await waitFor(() =>
      expect(screen.getByTestId("login-session-admin")).toBeTruthy(),
    );
    expect(screen.queryByTestId("category-admin-book")).toBeNull();
  });

  it("탭에 접근성 role 과 선택 상태가 있다", () => {
    render(<Admin />);

    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(3);
    // 키보드 내비게이션을 위해 선택된 탭이 aria-selected 로 노출된다
    const selected = tabs.filter(
      (t) => t.getAttribute("aria-selected") === "true",
    );
    expect(selected).toHaveLength(1);
    expect(selected[0].textContent).toBe("책 카테고리 관리");
  });
});
