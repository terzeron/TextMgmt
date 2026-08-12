// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup, waitFor, fireEvent } from "@testing-library/react";

const { mockJsonGetReq, mockJsonDeleteReq } = vi.hoisted(() => ({
  mockJsonGetReq: vi.fn(),
  mockJsonDeleteReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  jsonGetReq: mockJsonGetReq,
  jsonDeleteReq: mockJsonDeleteReq,
}));

import LoginSessionAdmin from "../src/LoginSessionAdmin";

const ACTIVE_SESSION = {
  session_id: "a".repeat(32),
  session_label: "aaaaaaaa...",
  email: "admin@example.com",
  status: "active",
  created_at: 1786500000,
  last_seen_at: 1786501200,
  expires_at: 1787106000,
  revoked_at: null,
  revoke_reason: null,
  token_count: 6,
  valid_token_count: 1,
  is_current: true,
};

const REVOKED_SESSION = {
  ...ACTIVE_SESSION,
  session_id: "b".repeat(32),
  session_label: "bbbbbbbb...",
  email: "viewer@example.com",
  status: "revoked",
  revoke_reason: "logout",
  revoked_at: 1786502000,
  valid_token_count: 0,
  is_current: false,
};

function page(items, overrides = {}) {
  return {
    items,
    pagination: {
      page: 1,
      pageSize: 50,
      totalItems: items.length,
      totalPages: 1,
    },
    summary: { active: 1, expired: 0, revoked: 1, total: items.length },
    ...overrides,
  };
}

// jsonGetReq(url, payload, resolve, reject, final) 시그니처를 그대로 흉내낸다.
function resolveWith(result) {
  return (url, payload, resolve, reject, final) => {
    resolve(result);
    if (final) final();
  };
}

function rejectWith(error) {
  return (url, payload, resolve, reject, final) => {
    if (reject) reject(error);
    if (final) final();
  };
}

describe("LoginSessionAdmin", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
  });

  afterEach(cleanup);

  it("마운트 시 활성 세션을 기본으로 조회한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([ACTIVE_SESSION])));

    render(<LoginSessionAdmin />);

    await waitFor(() => expect(mockJsonGetReq).toHaveBeenCalled());
    const url = mockJsonGetReq.mock.calls[0][0];
    expect(url).toContain("/auth/sessions");
    expect(url).toContain("status=active");
    expect(url).toContain("page=1");
    expect(url).toContain("pageSize=50");
  });

  it("세션 테이블에 필수 컬럼을 렌더링한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith(page([ACTIVE_SESSION, REVOKED_SESSION])),
    );

    render(<LoginSessionAdmin />);

    for (const header of [
      "상태",
      "계정",
      "세션",
      "생성 시각",
      "마지막 갱신",
      "만료 시각",
      "폐기 사유",
      "작업",
    ]) {
      expect(
        await screen.findByRole("columnheader", { name: header }),
      ).toBeTruthy();
    }
    expect(screen.getByText("admin@example.com")).toBeTruthy();
    expect(screen.getByText("viewer@example.com")).toBeTruthy();
  });

  it("현재 세션에는 배지를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith(page([ACTIVE_SESSION, REVOKED_SESSION])),
    );

    render(<LoginSessionAdmin />);

    expect(await screen.findByText("현재 세션")).toBeTruthy();
    // is_current 가 아닌 세션에는 배지가 없다 (한 개만 존재)
    expect(screen.getAllByText("현재 세션")).toHaveLength(1);
  });

  it("session_id 전체를 화면에 노출하지 않는다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([ACTIVE_SESSION])));

    const { container } = render(<LoginSessionAdmin />);

    await screen.findByText("aaaaaaaa...");
    expect(container.textContent).not.toContain("a".repeat(32));
  });

  it("빈 목록 상태를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([])));

    render(<LoginSessionAdmin />);

    expect(await screen.findByText("표시할 세션이 없습니다.")).toBeTruthy();
  });

  it("조회 실패 시 에러를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(rejectWith(new Error("boom")));

    render(<LoginSessionAdmin />);

    expect(
      await screen.findByText("세션 목록을 불러오지 못했습니다."),
    ).toBeTruthy();
  });

  it("로딩 상태를 표시한다", async () => {
    // resolve 를 호출하지 않아 로딩 상태가 유지된다
    mockJsonGetReq.mockImplementation(() => {});

    render(<LoginSessionAdmin />);

    expect(await screen.findByText("불러오는 중...")).toBeTruthy();
  });

  it("전체 필터를 누르면 status=all 로 다시 조회한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([ACTIVE_SESSION])));

    render(<LoginSessionAdmin />);
    await screen.findByText("admin@example.com");

    fireEvent.click(screen.getByRole("button", { name: "전체" }));

    await waitFor(() => {
      const urls = mockJsonGetReq.mock.calls.map((c) => c[0]);
      expect(urls.some((u) => u.includes("status=all"))).toBe(true);
    });
  });

  it("새로고침 버튼이 목록을 다시 조회한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([ACTIVE_SESSION])));

    render(<LoginSessionAdmin />);
    await screen.findByText("admin@example.com");
    const before = mockJsonGetReq.mock.calls.length;

    fireEvent.click(screen.getByRole("button", { name: /새로고침/ }));

    await waitFor(() =>
      expect(mockJsonGetReq.mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("확인 모달을 승인하기 전에는 폐기 요청을 보내지 않는다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([ACTIVE_SESSION])));

    render(<LoginSessionAdmin />);
    await screen.findByText("admin@example.com");

    fireEvent.click(screen.getByRole("button", { name: /세션 폐기/ }));

    // 모달이 열렸지만 아직 요청은 없다
    expect(await screen.findByRole("dialog")).toBeTruthy();
    expect(mockJsonDeleteReq).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "취소" }));
    expect(mockJsonDeleteReq).not.toHaveBeenCalled();
  });

  it("확인 후 DELETE /auth/sessions/{id} 를 호출한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([ACTIVE_SESSION])));
    mockJsonDeleteReq.mockImplementation(
      resolveWith({ revoked: true, revoked_current: false, status: "revoked" }),
    );

    render(<LoginSessionAdmin />);
    await screen.findByText("admin@example.com");

    fireEvent.click(screen.getByRole("button", { name: /세션 폐기/ }));
    fireEvent.click(await screen.findByRole("button", { name: "폐기" }));

    await waitFor(() => expect(mockJsonDeleteReq).toHaveBeenCalledTimes(1));
    expect(mockJsonDeleteReq.mock.calls[0][0]).toBe(
      `/auth/sessions/${"a".repeat(32)}`,
    );
    expect(await screen.findByText("세션을 폐기했습니다.")).toBeTruthy();
  });

  it("현재 세션을 폐기하면 미인증 흐름으로 리로드한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([ACTIVE_SESSION])));
    mockJsonDeleteReq.mockImplementation(
      resolveWith({ revoked: true, revoked_current: true, status: "revoked" }),
    );
    const reload = vi.fn();
    Object.defineProperty(window, "location", {
      value: { ...window.location, reload },
      writable: true,
    });

    render(<LoginSessionAdmin />);
    await screen.findByText("admin@example.com");

    fireEvent.click(screen.getByRole("button", { name: /세션 폐기/ }));
    fireEvent.click(await screen.findByRole("button", { name: "폐기" }));

    await waitFor(() => expect(reload).toHaveBeenCalled());
  });

  it("폐기 실패 시 에러를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([ACTIVE_SESSION])));
    mockJsonDeleteReq.mockImplementation(rejectWith(new Error("nope")));

    render(<LoginSessionAdmin />);
    await screen.findByText("admin@example.com");

    fireEvent.click(screen.getByRole("button", { name: /세션 폐기/ }));
    fireEvent.click(await screen.findByRole("button", { name: "폐기" }));

    expect(await screen.findByText("세션을 폐기하지 못했습니다.")).toBeTruthy();
  });

  it("활성이 아닌 세션은 폐기 버튼이 비활성이다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(page([REVOKED_SESSION])));

    render(<LoginSessionAdmin />);
    await screen.findByText("viewer@example.com");

    expect(screen.getByRole("button", { name: /세션 폐기/ }).disabled).toBe(
      true,
    );
  });

  it("여러 페이지일 때만 페이지 이동 버튼을 보여준다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith(
        page([ACTIVE_SESSION], {
          pagination: { page: 1, pageSize: 50, totalItems: 80, totalPages: 2 },
        }),
      ),
    );

    render(<LoginSessionAdmin />);
    await screen.findByText("admin@example.com");

    const next = screen.getByRole("button", { name: "다음" });
    expect(screen.getByRole("button", { name: "이전" }).disabled).toBe(true);

    fireEvent.click(next);

    await waitFor(() => {
      const urls = mockJsonGetReq.mock.calls.map((c) => c[0]);
      expect(urls.some((u) => u.includes("page=2"))).toBe(true);
    });
  });
});
