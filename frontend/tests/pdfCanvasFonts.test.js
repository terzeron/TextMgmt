import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

describe("PDF canvas fallback fonts", () => {
  let faces;
  let context;
  const originalFonts = Object.getOwnPropertyDescriptor(document, "fonts");

  afterEach(() => {
    vi.unstubAllGlobals();
    if (originalFonts) Object.defineProperty(document, "fonts", originalFonts);
    else delete document.fonts;
  });

  beforeEach(() => {
    vi.resetModules();
    faces = [];
    vi.stubGlobal("FontFace", class {
      constructor(family, source, options) {
        Object.assign(this, { family, source, ...options });
        faces.push(this);
      }
      async load() { return this; }
    });
    Object.defineProperty(document, "fonts", {
      configurable: true,
      value: { add: vi.fn() },
    });
    let font = "10px sans-serif";
    context = Object.create({
      get font() { return font; },
      set font(value) { font = value; },
    });
  });

  it("loads bundled regular and bold faces once before rendering", async () => {
    const { preparePdfCanvasFonts } = await import("../src/pdfCanvasFonts");
    await preparePdfCanvasFonts(context);
    await preparePdfCanvasFonts(context);
    expect(faces.map(({ weight }) => weight)).toEqual(["400", "700"]);
    expect(faces.every(({ source }) => source.includes("/fonts/noto-serif-kr-"))).toBe(true);
    expect(document.fonts.add).toHaveBeenCalledTimes(2);
  });

  it("puts Noto before the generic fallback while preserving the embedded font", async () => {
    const { preparePdfCanvasFonts } = await import("../src/pdfCanvasFonts");
    await preparePdfCanvasFonts(context);
    context.font = 'bold 20px "g_d0_f1", serif';
    expect(context.font).toBe('bold 20px "g_d0_f1", "TM PDF Noto Serif CJK", serif');
    context.font = '20px "¹ÙÅÁÃ¼", sans-serif';
    expect(context.font).toContain('"TM PDF Noto Serif CJK", sans-serif');
  });

  it("does not duplicate the fallback or alter a font without a generic family", async () => {
    const { preparePdfCanvasFonts } = await import("../src/pdfCanvasFonts");
    await preparePdfCanvasFonts(context);
    context.font = '20px "g_d0_f1"';
    expect(context.font).toBe('20px "g_d0_f1"');
    context.font = "20px serif";
    const savedFont = context.font;
    context.font = savedFont;
    expect(context.font.match(/TM PDF Noto Serif CJK/g)).toHaveLength(1);
  });

  it("continues rendering if the font download fails and retries next time", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { preparePdfCanvasFonts } = await import("../src/pdfCanvasFonts");
    vi.spyOn(FontFace.prototype, "load").mockRejectedValueOnce(new Error("offline"));
    await preparePdfCanvasFonts(context);
    expect(warn).toHaveBeenCalled();
    await preparePdfCanvasFonts(context);
    expect(document.fonts.add).toHaveBeenCalled();
    warn.mockRestore();
  });
});
