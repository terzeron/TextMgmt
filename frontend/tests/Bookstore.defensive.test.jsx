// @vitest-environment jsdom
/* eslint-disable react/prop-types, react/display-name */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";

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
      <button onClick={onClick} disabled={disabled} title={title}>
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

  it("검색어가 없으면 검색 버튼을 비활성화한다", () => {
    render(<Bookstore bookInfo={{ title: "", author: "", isbn: "" }} />);

    expect(screen.getAllByRole("button", { name: "ISBN" })[0].disabled).toBe(true);
    expect(screen.getAllByRole("button", { name: "저자+제목" })[0].disabled).toBe(true);
    expect(mockRawJsonGetReq).not.toHaveBeenCalled();
  });

  it("ISBN 미지원 서점은 ISBN 검색 버튼을 비활성화한다", () => {
    const { container } = render(
      <Bookstore bookInfo={{ title: "제목", author: "저자", isbn: "978" }} />,
    );

    // 줄 순서에 기대지 않는다. 서점이 늘면 인덱스가 밀려 엉뚱한 줄을 누른다.
    const naver = within(container).getByText("네이버쇼핑").parentElement;
    expect(within(naver).getByRole("button", { name: "ISBN" }).disabled).toBe(true);
    expect(mockRawJsonGetReq).not.toHaveBeenCalled();
  });
});
