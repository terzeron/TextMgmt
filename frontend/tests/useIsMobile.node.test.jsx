// @vitest-environment node
import { describe, it, expect } from "vitest";
import { renderToString } from "react-dom/server";
import useIsMobile from "../src/useIsMobile";

function Probe() {
  return <span>{useIsMobile() ? "mobile" : "desktop"}</span>;
}

describe("useIsMobile without window", () => {
  it("falls back to desktop during server-side rendering", () => {
    expect(renderToString(<Probe />)).toContain("desktop");
  });
});
