import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { reportClientError } = vi.hoisted(() => ({
  reportClientError: vi.fn(),
}));
vi.mock("../src/clientLogger", () => ({ reportClientError }));

import { endPdfTrace, startPdfTrace, tracePdf } from "../src/pdfViewerTrace";

const KEY = "tm.pdfTrace";
const stored = () => JSON.parse(localStorage.getItem(KEY));

describe("pdfViewerTrace", () => {
  beforeEach(() => {
    endPdfTrace();
    localStorage.clear();
    reportClientError.mockClear();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("처음 열릴 때는 서버로 아무것도 보내지 않고 기록을 시작한다", () => {
    expect(startPdfTrace({ bookId: 7 })).toBe(false);
    expect(reportClientError).not.toHaveBeenCalled();
    expect(stored()).toMatchObject({ bookId: 7, clean: false, events: [] });
  });

  it("이벤트를 상태와 함께 저장한다", () => {
    startPdfTrace({ bookId: 7 });
    tracePdf("render-done p9", { live: 4, mpx: 12.5 });
    const record = stored();
    expect(record.events).toHaveLength(1);
    expect(record.events[0]).toMatch(
      /^\+\d+ms render-done p9 live=4 mpx=12.5 vis=visible$/,
    );
    expect(record.last).toBe(record.events[0]);
  });

  it("탭이 숨겨진 상태를 기록에 남긴다", () => {
    startPdfTrace({ bookId: 7 });
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    tracePdf("turn", {});
    expect(stored().last).toContain("vis=hidden");
  });

  it("최근 이벤트 24개만 남긴다", () => {
    startPdfTrace({ bookId: 7 });
    for (let i = 0; i < 30; i++) tracePdf(`e${i}`);
    const { events } = stored();
    expect(events).toHaveLength(24);
    expect(events[0]).toContain(" e6 ");
    expect(events[23]).toContain(" e29 ");
  });

  it("정상 종료하면 clean 으로 표시하고 다음 열림에서 보내지 않는다", () => {
    startPdfTrace({ bookId: 7 });
    tracePdf("turn");
    endPdfTrace();
    expect(stored().clean).toBe(true);
    expect(startPdfTrace({ bookId: 8 })).toBe(false);
    expect(reportClientError).not.toHaveBeenCalled();
  });

  it("정상 종료하지 않은 기록이 남아 있으면 다음 열림에서 서버로 보낸다", () => {
    startPdfTrace({ bookId: 7 });
    tracePdf("render-start p9", { live: 8 });
    tracePdf("render-done p9", { live: 9 });
    // endPdfTrace 없이 새로 열림 = 탭이 죽었다가 다시 불러와진 상황
    expect(startPdfTrace({ bookId: 7 })).toBe(true);
    expect(reportClientError).toHaveBeenCalledTimes(1);
    const arg = reportClientError.mock.calls[0][0];
    expect(arg.errorType).toBe("CUSTOM_ERROR");
    expect(arg.message).toContain("[PDF_TRACE]");
    expect(arg.message).toContain("book=7");
    expect(arg.message).toContain("render-done p9");
    expect(arg.stack.split("\n")).toHaveLength(2);
  });

  it("뷰어를 시작하기 전의 tracePdf·endPdfTrace 는 아무 일도 하지 않는다", () => {
    expect(() => tracePdf("x")).not.toThrow();
    expect(() => endPdfTrace()).not.toThrow();
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it("저장소를 읽거나 쓸 수 없어도 예외를 던지지 않는다", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(() => startPdfTrace({ bookId: 7 })).not.toThrow();
    expect(() => tracePdf("x")).not.toThrow();
    expect(() => endPdfTrace()).not.toThrow();
  });

  it("손상된 저장값은 무시하고 새로 시작한다", () => {
    localStorage.setItem(KEY, "{not json");
    expect(startPdfTrace({ bookId: 7 })).toBe(false);
    expect(reportClientError).not.toHaveBeenCalled();
  });

  it("이전 기록에 events 가 없어도 보낼 수 있다", () => {
    localStorage.setItem(KEY, JSON.stringify({ bookId: 3, clean: false }));
    expect(startPdfTrace({ bookId: 3 })).toBe(true);
    expect(reportClientError.mock.calls[0][0].stack).toBe("");
  });
});
