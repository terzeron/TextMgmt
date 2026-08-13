// @vitest-environment jsdom
/* eslint-disable react/prop-types */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const { mockJsonGetReq, mockJsonDeleteReq } = vi.hoisted(() => ({
  mockJsonGetReq: vi.fn(),
  mockJsonDeleteReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  jsonGetReq: mockJsonGetReq,
  jsonDeleteReq: mockJsonDeleteReq,
}));

vi.mock("@fortawesome/react-fontawesome", () => ({
  FontAwesomeIcon: () => <span />,
}));

vi.mock("react-bootstrap", () => {
  const Passthrough = ({ children }) => <div>{children}</div>;
  const Modal = ({ children }) => <div>{children}</div>;
  Modal.Header = Passthrough;
  Modal.Title = Passthrough;
  Modal.Body = Passthrough;
  Modal.Footer = Passthrough;

  return {
    Alert: Passthrough,
    Badge: Passthrough,
    Button: ({ children, onClick }) => (
      <button onClick={onClick}>{children}</button>
    ),
    ButtonGroup: Passthrough,
    Modal,
    Spinner: () => <span>loading</span>,
    Table: ({ children }) => <table>{children}</table>,
  };
});

import LoginSessionAdmin from "../src/LoginSessionAdmin";

describe("LoginSessionAdmin defensive modal action", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject, final) => {
      resolve({ items: [], summary: null, pagination: null });
      final?.();
    });
  });

  it("폐기 대상이 없으면 confirm handler가 삭제 요청을 보내지 않는다", () => {
    render(<LoginSessionAdmin />);

    fireEvent.click(screen.getByRole("button", { name: "폐기" }));

    expect(mockJsonDeleteReq).not.toHaveBeenCalled();
  });
});
