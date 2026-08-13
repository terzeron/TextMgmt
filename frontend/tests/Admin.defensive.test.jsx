// @vitest-environment jsdom
/* eslint-disable react/prop-types */
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

vi.mock("react-router-dom", () => ({
  useOutletContext: () => ({}),
}));

vi.mock("../src/CategoryAdmin", () => ({
  default: ({ contentType }) => <div>Category {contentType}</div>,
}));

vi.mock("../src/LoginSessionAdmin", () => ({
  default: () => <div>Login Sessions</div>,
}));

vi.mock("../src/ViewHistoryAdmin", () => ({
  default: () => <div>View History</div>,
}));

vi.mock("react-bootstrap", () => ({
  Tab: ({ children }) => <div>{children}</div>,
  Tabs: ({ children, onSelect }) => (
    <div>
      <button onClick={() => onSelect(null)}>reset tab</button>
      {children}
    </div>
  ),
}));

import Admin from "../src/Admin";

describe("Admin defensive tab fallback", () => {
  it("onSelect가 null을 넘기면 세션 탭으로 되돌린다", () => {
    render(<Admin />);

    fireEvent.click(screen.getByRole("button", { name: "reset tab" }));

    expect(screen.getByText("Login Sessions")).toBeTruthy();
  });
});
