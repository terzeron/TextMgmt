// @vitest-environment jsdom
/* eslint-disable react/prop-types, react/display-name */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const { mockJsonDeleteReq, mockNavigate } = vi.hoisted(() => ({
  mockJsonDeleteReq: vi.fn(),
  mockNavigate: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  jsonDeleteReq: mockJsonDeleteReq,
}));

vi.mock("react-router-dom", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("react-bootstrap", () => {
  const Card = ({ children }) => <div>{children}</div>;
  Card.Header = ({ children }) => <div>{children}</div>;
  Card.Body = ({ children }) => <div>{children}</div>;

  return {
    Alert: ({ children }) => <div>{children}</div>,
    Button: ({ children, onClick }) => <button onClick={onClick}>{children}</button>,
    Card,
  };
});

import BookLoadError from "../src/BookLoadError";

describe("BookLoadError defensive delete guard", () => {
  beforeEach(() => {
    mockJsonDeleteReq.mockReset();
    mockNavigate.mockReset();
  });

  it("bookId가 없으면 handler가 confirm/delete 없이 반환한다", () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<BookLoadError role="admin" error="not found" apiPrefix="" />);

    fireEvent.click(screen.getByText("ES 잔존 문서 삭제"));

    expect(confirmSpy).not.toHaveBeenCalled();
    expect(mockJsonDeleteReq).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });
});
