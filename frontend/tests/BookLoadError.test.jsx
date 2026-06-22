// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup, fireEvent } from "@testing-library/react";

afterEach(cleanup);

const { mockJsonDeleteReq, mockNavigate } = vi.hoisted(() => ({
  mockJsonDeleteReq: vi.fn(),
  mockNavigate: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  jsonDeleteReq: mockJsonDeleteReq,
}));

vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual("react-router-dom");
  return { ...actual, useNavigate: () => mockNavigate };
});

import BookLoadError from "../src/BookLoadError";

describe("BookLoadError", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("모든 사용자에게 안내 메시지를 표시한다", () => {
    render(
      <BookLoadError
        bookId="19411"
        category="3_판타지"
        error="Book not found"
        role="viewer"
        apiPrefix=""
      />,
    );
    expect(screen.getByText("책 정보를 불러오지 못했습니다.")).toBeTruthy();
    expect(screen.getByText(/19411/)).toBeTruthy();
    expect(screen.getByText(/3_판타지/)).toBeTruthy();
  });

  it("viewer에게는 사유 원문과 액션 버튼을 노출하지 않는다", () => {
    render(
      <BookLoadError
        bookId="19411"
        category="3_판타지"
        error="Book not found"
        role="viewer"
        apiPrefix=""
      />,
    );
    expect(screen.queryByTestId("book-load-error-reason")).toBeNull();
    expect(screen.queryByText("ES 잔존 문서 삭제")).toBeNull();
    expect(screen.queryByText("카테고리 불일치 관리로 이동")).toBeNull();
  });

  it("admin에게는 사유 원문과 액션 버튼을 노출한다", () => {
    render(
      <BookLoadError
        bookId="19411"
        category="3_판타지"
        error="Book not found"
        role="admin"
        apiPrefix=""
      />,
    );
    expect(screen.getByTestId("book-load-error-reason").textContent).toContain(
      "Book not found",
    );
    expect(screen.getByText("ES 잔존 문서 삭제")).toBeTruthy();
    expect(screen.getByText("카테고리 불일치 관리로 이동")).toBeTruthy();
  });

  it("ES 잔존 문서 삭제 버튼 클릭 시 confirm 후 jsonDeleteReq를 호출한다", () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(
      <BookLoadError
        bookId="19411"
        category="3_판타지"
        error="Book not found"
        role="admin"
        apiPrefix=""
      />,
    );

    fireEvent.click(screen.getByText("ES 잔존 문서 삭제"));

    expect(confirmSpy).toHaveBeenCalled();
    expect(mockJsonDeleteReq).toHaveBeenCalled();
    expect(mockJsonDeleteReq.mock.calls[0][0]).toBe(
      "/category-mismatches/es-doc/19411",
    );
    confirmSpy.mockRestore();
  });

  it("comic(apiPrefix=/comics)일 때 ES 삭제 URL에 prefix가 포함된다", () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(
      <BookLoadError
        bookId="500"
        category="만화"
        error="not found"
        role="admin"
        apiPrefix="/comics"
      />,
    );

    fireEvent.click(screen.getByText("ES 잔존 문서 삭제"));

    expect(mockJsonDeleteReq.mock.calls[0][0]).toBe(
      "/comics/category-mismatches/es-doc/500",
    );
    confirmSpy.mockRestore();
  });

  it("confirm을 취소하면 jsonDeleteReq를 호출하지 않는다", () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(
      <BookLoadError
        bookId="19411"
        category="3_판타지"
        error="Book not found"
        role="admin"
        apiPrefix=""
      />,
    );

    fireEvent.click(screen.getByText("ES 잔존 문서 삭제"));

    expect(mockJsonDeleteReq).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it("카테고리 불일치 관리로 이동 버튼 클릭 시 /admin으로 이동한다", () => {
    render(
      <BookLoadError
        bookId="19411"
        category="3_판타지"
        error="Book not found"
        role="admin"
        apiPrefix=""
      />,
    );
    fireEvent.click(screen.getByText("카테고리 불일치 관리로 이동"));
    expect(mockNavigate).toHaveBeenCalledWith("/admin");
  });

  it("bookId가 없으면 ES 삭제 버튼이 비활성화된다", () => {
    render(
      <BookLoadError
        category="3_판타지"
        error="not found"
        role="admin"
        apiPrefix=""
      />,
    );
    expect(
      screen.getByText("ES 잔존 문서 삭제").closest("button").disabled,
    ).toBe(true);
  });

  it("ES 삭제 성공 시 결과 메시지를 표시한다", () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) => resolve());
    render(
      <BookLoadError
        bookId="19411"
        category="3_판타지"
        error="not found"
        role="admin"
        apiPrefix=""
      />,
    );

    fireEvent.click(screen.getByText("ES 잔존 문서 삭제"));

    expect(screen.getByText(/ES 잔존 문서를 삭제했습니다/)).toBeTruthy();
    confirmSpy.mockRestore();
  });

  it("ES 삭제 실패 시 실패 메시지를 표시한다", () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    mockJsonDeleteReq.mockImplementation((url, payload, resolve, reject) =>
      reject("서버 오류"),
    );
    render(
      <BookLoadError
        bookId="19411"
        category="3_판타지"
        error="not found"
        role="admin"
        apiPrefix=""
      />,
    );

    fireEvent.click(screen.getByText("ES 잔존 문서 삭제"));

    expect(screen.getByText(/ES 문서 삭제 실패: 서버 오류/)).toBeTruthy();
    confirmSpy.mockRestore();
  });
});
