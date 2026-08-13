// @vitest-environment jsdom
/* eslint-disable react/prop-types */
import { describe, it, expect, afterEach } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import useIsMobile from "../src/useIsMobile";

afterEach(cleanup);

function Probe({ breakpoint = 768 }) {
  return <div>{useIsMobile(breakpoint) ? "mobile" : "desktop"}</div>;
}

describe("useIsMobile", () => {
  it("uses the current window width for the initial value", () => {
    window.innerWidth = 500;

    render(<Probe />);

    expect(screen.getByText("mobile")).toBeTruthy();
  });

  it("updates when the viewport is resized across the breakpoint", () => {
    window.innerWidth = 900;
    render(<Probe breakpoint={800} />);
    expect(screen.getByText("desktop")).toBeTruthy();

    act(() => {
      window.innerWidth = 700;
      window.dispatchEvent(new Event("resize"));
    });

    expect(screen.getByText("mobile")).toBeTruthy();
  });
});
