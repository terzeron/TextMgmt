// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup, waitFor, fireEvent } from "@testing-library/react";

const { mockJsonGetReq } = vi.hoisted(() => ({ mockJsonGetReq: vi.fn() }));

vi.mock("../src/Common", () => ({ jsonGetReq: mockJsonGetReq }));

import ViewHistoryAdmin from "../src/ViewHistoryAdmin";

const USER_WITH_BOTH = {
  email: "reader@example.com",
  last_viewed_at: 1786501200,
  book: [
    {
      book_id: 1,
      title: "책 제목 하나",
      category: "소설",
      viewed_at: 1786501200,
    },
    { book_id: 2, title: "책 제목 둘", category: "", viewed_at: 1786500000 },
  ],
  comic: [
    {
      book_id: 10,
      title: "만화 제목",
      category: "액션",
      viewed_at: 1786500500,
    },
  ],
};

const USER_BOOK_ONLY = {
  email: "booksonly@example.com",
  last_viewed_at: 1786400000,
  book: [
    { book_id: 3, title: "책만 봄", category: "에세이", viewed_at: 1786400000 },
  ],
  comic: [],
};

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

describe("ViewHistoryAdmin", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
  });

  afterEach(cleanup);

  it("마운트 시 /view-history 를 조회한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({ limit: 50, users: [USER_WITH_BOTH] }),
    );

    render(<ViewHistoryAdmin />);

    await waitFor(() => expect(mockJsonGetReq).toHaveBeenCalled());
    expect(mockJsonGetReq.mock.calls[0][0]).toBe("/view-history");
  });

  it("사용자별로 책·만화 목록을 렌더링한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({ limit: 50, users: [USER_WITH_BOTH] }),
    );

    render(<ViewHistoryAdmin />);

    expect(await screen.findByText("reader@example.com")).toBeTruthy();
    expect(screen.getByText("책 제목 하나")).toBeTruthy();
    expect(screen.getByText("책 제목 둘")).toBeTruthy();
    expect(screen.getByText("만화 제목")).toBeTruthy();
    expect(screen.getByText("소설")).toBeTruthy();
    expect(screen.getByText("액션")).toBeTruthy();
  });

  it("책·만화 각각의 건수를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({ limit: 50, users: [USER_WITH_BOTH] }),
    );

    render(<ViewHistoryAdmin />);
    await screen.findByText("reader@example.com");

    // 책 2건, 만화 1건
    expect(screen.getByText("책").textContent).toContain("2");
    expect(screen.getByText("만화").textContent).toContain("1");
  });

  it("한쪽 유형이 비어 있으면 해당 안내를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({ limit: 50, users: [USER_BOOK_ONLY] }),
    );

    render(<ViewHistoryAdmin />);
    await screen.findByText("booksonly@example.com");

    expect(screen.getByText("책만 봄")).toBeTruthy();
    expect(screen.getByText("조회 이력이 없습니다.")).toBeTruthy();
  });

  it("카테고리가 비면 대시로 표시한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({ limit: 50, users: [USER_WITH_BOTH] }),
    );

    const { container } = render(<ViewHistoryAdmin />);
    await screen.findByText("책 제목 둘");

    const cells = [...container.querySelectorAll("td")].map(
      (td) => td.textContent,
    );
    expect(cells).toContain("-");
  });

  it("여러 사용자를 응답 순서대로 렌더링한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({ limit: 50, users: [USER_WITH_BOTH, USER_BOOK_ONLY] }),
    );

    const { container } = render(<ViewHistoryAdmin />);
    await screen.findByText("reader@example.com");

    const headings = [...container.querySelectorAll("h6")].map(
      (h) => h.textContent,
    );
    expect(headings).toEqual(["reader@example.com", "booksonly@example.com"]);
  });

  it("요약에 사용자 수와 유형별 상한을 표시한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({ limit: 50, users: [USER_WITH_BOTH, USER_BOOK_ONLY] }),
    );

    render(<ViewHistoryAdmin />);

    expect(
      await screen.findByText(/사용자 2명 · 유형별 최근 50건/),
    ).toBeTruthy();
  });

  it("빈 상태를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith({ limit: 50, users: [] }));

    render(<ViewHistoryAdmin />);

    expect(
      await screen.findByText("조회 이력이 있는 사용자가 없습니다."),
    ).toBeTruthy();
  });

  it("응답 result가 비어 있으면 users/limit fallback을 사용한다", async () => {
    mockJsonGetReq.mockImplementation(resolveWith(null));

    render(<ViewHistoryAdmin />);

    expect(
      await screen.findByText("조회 이력이 있는 사용자가 없습니다."),
    ).toBeTruthy();
    expect(screen.queryByText(/유형별 최근/)).toBeNull();
  });

  it("조회 시각이 없으면 대시로 표시한다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({
        limit: 50,
        users: [
          {
            ...USER_BOOK_ONLY,
            last_viewed_at: 0,
            book: [{ ...USER_BOOK_ONLY.book[0], viewed_at: null }],
          },
        ],
      }),
    );

    const { container } = render(<ViewHistoryAdmin />);
    await screen.findByText("booksonly@example.com");

    const cells = [...container.querySelectorAll("td")].map(
      (td) => td.textContent,
    );
    expect(cells).toContain("-");
    expect(container.textContent).toContain("마지막 조회 -");
  });

  it("조회 실패 시 에러를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(rejectWith(new Error("boom")));

    render(<ViewHistoryAdmin />);

    expect(
      await screen.findByText("조회 목록을 불러오지 못했습니다."),
    ).toBeTruthy();
  });

  it("조회 실패 알림을 닫을 수 있다", async () => {
    mockJsonGetReq.mockImplementation(rejectWith(new Error("boom")));

    render(<ViewHistoryAdmin />);

    expect(
      await screen.findByText("조회 목록을 불러오지 못했습니다."),
    ).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Close alert"));
    expect(screen.queryByText("조회 목록을 불러오지 못했습니다.")).toBeNull();
  });

  it("로딩 상태를 표시한다", async () => {
    mockJsonGetReq.mockImplementation(() => {});

    render(<ViewHistoryAdmin />);

    expect(await screen.findByText("불러오는 중...")).toBeTruthy();
  });

  it("새로고침 버튼을 두지 않는다", async () => {
    mockJsonGetReq.mockImplementation(
      resolveWith({ limit: 50, users: [USER_WITH_BOTH] }),
    );

    render(<ViewHistoryAdmin />);
    await screen.findByText("reader@example.com");

    expect(screen.queryByRole("button")).toBeNull();
  });
});
