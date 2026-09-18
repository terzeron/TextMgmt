// @vitest-environment jsdom
/* eslint-disable react/prop-types, react/display-name */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";

const { mockRawJsonGetReq } = vi.hoisted(() => ({
  mockRawJsonGetReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  rawJsonGetReq: mockRawJsonGetReq,
}));

vi.mock("@fortawesome/react-fontawesome", () => ({
  FontAwesomeIcon: () => <span data-testid="icon" />,
}));

vi.mock("react-bootstrap", () => {
  const Card = ({ children }) => <div>{children}</div>;
  Card.Header = ({ children }) => <div>{children}</div>;
  Card.Body = ({ children }) => <div>{children}</div>;

  return {
    Button: ({ children, onClick, disabled, title }) => (
      <button onClick={onClick} data-disabled={String(Boolean(disabled))} title={title}>
        {children}
      </button>
    ),
    ButtonGroup: ({ children }) => <div>{children}</div>,
    Card,
    Spinner: () => <span>loading</span>,
    Tab: ({ title, children }) => (
      <section aria-label={title}>
        <h2>{title}</h2>
        {children}
      </section>
    ),
    Tabs: ({ children }) => <div>{children}</div>,
  };
});

import Bookstore from "../src/Bookstore";

describe("Bookstore defensive search handlers", () => {
  beforeEach(() => {
    mockRawJsonGetReq.mockReset();
  });

  it("검색 파라미터가 비어 있으면 API 호출 없이 에러를 표시한다", async () => {
    render(<Bookstore bookInfo={{ title: "", author: "", isbn: "" }} />);

    fireEvent.click(screen.getAllByRole("button", { name: "ISBN" })[0]);

    await waitFor(() => {
      expect(screen.getByText("검색어가 없습니다.")).toBeTruthy();
    });
    expect(mockRawJsonGetReq).not.toHaveBeenCalled();
  });

  it("ISBN 미지원 서점에서 ISBN handler가 호출되면 지원하지 않는다는 에러를 표시한다", async () => {
    render(<Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />);

    // 탭 순서에 기대지 않는다. 서점이 늘면 인덱스가 밀려 엉뚱한 탭을 누른다.
    // 탭은 중복 렌더링될 수 있어 첫 번째만 쓴다.
    const naver = screen.getAllByRole("region", { name: "네이버쇼핑" })[0];
    fireEvent.click(within(naver).getByRole("button", { name: "ISBN" }));

    await waitFor(() => {
      expect(
        screen.getByText("이 서점은 ISBN 검색을 지원하지 않습니다."),
      ).toBeTruthy();
    });
    expect(mockRawJsonGetReq).not.toHaveBeenCalled();
  });
});
