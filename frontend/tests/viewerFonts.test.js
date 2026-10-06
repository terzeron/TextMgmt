// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";
import {
  FONT_FAMILIES,
  readReaderFontFamily,
  saveReaderFontFamily,
} from "../src/viewerFonts";

describe("viewerFonts", () => {
  beforeEach(() => localStorage.clear());

  it("저장된 값이 없으면 빈 문자열을 돌려준다", () => {
    expect(readReaderFontFamily()).toBe("");
  });

  it("reader 키가 없으면 예전 epub 키를 읽는다", () => {
    const font = FONT_FAMILIES.find((item) => item.value);
    localStorage.setItem("epub_fontFamily", font.value);
    expect(readReaderFontFamily()).toBe(font.value);
  });

  it("목록에 없는 값은 빈 문자열로 처리한다", () => {
    localStorage.setItem("reader_fontFamily", "없는 글꼴");
    expect(readReaderFontFamily()).toBe("");
  });

  it("저장하면 두 키에 같은 값을 기록한다", () => {
    const font = FONT_FAMILIES.find((item) => item.value);
    saveReaderFontFamily(font.value);
    expect(localStorage.getItem("reader_fontFamily")).toBe(font.value);
    expect(localStorage.getItem("epub_fontFamily")).toBe(font.value);
  });
});
