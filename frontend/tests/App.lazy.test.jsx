// @vitest-environment jsdom
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Outlet } from "react-router-dom";

vi.mock("../src/Navigation", () => ({
  default: () => <Outlet />,
}));

vi.mock("../src/Home", () => ({
  default: () => <div>Home route</div>,
}));

vi.mock("../src/BookView", () => ({
  default: () => <div>BookView route</div>,
}));

vi.mock("../src/ComicsView", () => ({
  default: () => <div>ComicsView route</div>,
}));

vi.mock("../src/LatestBooks", () => ({
  default: ({ contentType = "book" }) => (
    <div>{contentType === "comic" ? "Latest comics route" : "Latest books route"}</div>
  ),
}));

vi.mock("../src/ViewSingle", () => ({
  default: () => <div>ViewSingle route</div>,
}));

vi.mock("../src/Admin", () => ({
  default: () => <div>Admin route</div>,
}));

vi.mock("../src/BookEdit", () => ({
  default: () => <div>BookEdit lazy route</div>,
}));

vi.mock("../src/ComicsEdit", () => ({
  default: () => <div>ComicsEdit lazy route</div>,
}));

import App from "../src/App";

describe("App lazy routes", () => {
  it("loads BookEdit through the lazy route factory", async () => {
    render(
      <MemoryRouter initialEntries={["/book-edit"]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText("BookEdit lazy route")).toBeTruthy();
  });

  it("loads ComicsEdit through the lazy route factory", async () => {
    render(
      <MemoryRouter initialEntries={["/comics-edit"]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText("ComicsEdit lazy route")).toBeTruthy();
  });

  it("routes /book-latest to latest books", async () => {
    render(
      <MemoryRouter initialEntries={["/book-latest"]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Latest books route")).toBeTruthy();
  });

  it("routes /comics-latest to latest comics", async () => {
    render(
      <MemoryRouter initialEntries={["/comics-latest"]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Latest comics route")).toBeTruthy();
  });
});
