// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  fireEvent,
  cleanup,
} from "@testing-library/react";

afterEach(cleanup);

const { mockTextGetReq } = vi.hoisted(() => ({
  mockTextGetReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  textGetReq: mockTextGetReq,
}));

import ViewTXT from "../src/ViewTXT";

describe("ViewTXT", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("로딩 중 상태를 표시한다", () => {
    mockTextGetReq.mockImplementation(() => {}); // 응답 안 함
    render(<ViewTXT bookId={1} />);
    expect(screen.getByText("로딩 중...")).toBeTruthy();
  });

  it("텍스트 파일 내용을 렌더링한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("첫 번째 줄\n두 번째 줄\n세 번째 줄");
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("첫 번째 줄")).toBeTruthy();
      expect(screen.getByText("두 번째 줄")).toBeTruthy();
      expect(screen.getByText("세 번째 줄")).toBeTruthy();
    });
  });

  it("lineCount로 줄 수를 제한한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("줄1\n줄2\n줄3\n줄4\n줄5");
    });
    render(<ViewTXT bookId={1} lineCount={2} />);
    await waitFor(() => {
      expect(screen.getByText("줄1")).toBeTruthy();
      expect(screen.getByText("줄2")).toBeTruthy();
      expect(screen.queryByText("줄3")).toBeNull();
    });
  });

  it("에러 발생 시 에러 메시지를 표시한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve, reject) => {
      reject("네트워크 오류");
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText(/파일을 불러올 수 없습니다/)).toBeTruthy();
    });
  });

  it("bookId가 없으면 에러 메시지를 표시한다", async () => {
    render(<ViewTXT bookId={0} />);
    await waitFor(() => {
      expect(
        screen.getByText(/유효한 bookId가 제공되지 않았습니다/),
      ).toBeTruthy();
    });
  });

  it("lineCount가 0이면 모든 줄을 표시한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("줄1\n줄2\n줄3");
    });
    render(<ViewTXT bookId={1} lineCount={0} />);
    await waitFor(() => {
      expect(screen.getByText("줄1")).toBeTruthy();
      expect(screen.getByText("줄3")).toBeTruthy();
    });
  });

  // ── 줄 합치기 (merged mode) ──

  it('로딩 완료 후 "줄 합치기" 버튼을 표시한다', async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("줄1\n줄2");
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("줄 합치기")).toBeTruthy();
    });
  });

  it("줄 합치기 토글 시 merged 모드로 전환된다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("제목\n\n본문 내용입니다.");
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("줄 합치기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("줄 합치기"));
    // merged 모드에서 빈 줄 기준 컨트롤이 표시됨
    await waitFor(() => {
      expect(screen.getByText("빈 줄 기준")).toBeTruthy();
    });
  });

  it("merged 모드에서 다시 토글하면 원래 줄 표시로 복귀한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("줄1\n줄2");
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("줄 합치기")).toBeTruthy();
    });

    // 토글 ON
    fireEvent.click(screen.getByText("줄 합치기"));
    await waitFor(() => {
      expect(screen.getByText("빈 줄 기준")).toBeTruthy();
    });

    // 토글 OFF
    fireEvent.click(screen.getByText("줄 합치기"));
    await waitFor(() => {
      expect(screen.queryByText("빈 줄 기준")).toBeNull();
    });
  });

  // ── 빈 줄 기준 컨트롤 ──

  it("빈 줄 기준 + 버튼으로 minBlank를 증가시킨다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("줄1\n\n줄2");
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("줄 합치기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("줄 합치기"));
    await waitFor(() => {
      expect(screen.getByText("1")).toBeTruthy(); // 초기값
    });

    fireEvent.click(screen.getByText("+"));
    await waitFor(() => {
      expect(screen.getByText("2")).toBeTruthy();
    });
  });

  it("빈 줄 기준 − 버튼으로 값을 다시 1로 낮춘다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("줄1\n\n줄2");
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("줄 합치기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("줄 합치기"));
    fireEvent.click(screen.getByText("+"));
    await waitFor(() => {
      expect(screen.getByText("2")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("−"));
    await waitFor(() => {
      expect(screen.getByText("1")).toBeTruthy();
    });
  });

  it("빈 줄 기준 − 버튼은 1 미만으로 내릴 수 없다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("줄1\n줄2");
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("줄 합치기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("줄 합치기"));
    await waitFor(() => {
      expect(screen.getByText("빈 줄 기준")).toBeTruthy();
    });

    // minBlank=1일 때 − 버튼은 disabled
    const minusBtn = screen.getByText("−");
    expect(minusBtn.disabled).toBe(true);
  });

  it("apiPrefix를 URL에 포함한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("내용");
    });
    render(<ViewTXT bookId={1} apiPrefix="/comics" />);
    await waitFor(() => {
      expect(mockTextGetReq).toHaveBeenCalledWith(
        "/comics/download/1",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("응답이 문자열이 아니고 falsy이면 빈 문자열로 변환한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve(null);
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("줄 합치기")).toBeTruthy();
    });
  });

  it("응답이 문자열이 아니고 truthy이면 String으로 변환한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve(12345);
    });
    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("12345")).toBeTruthy();
    });
  });

  it("merged 모드에서 separator, dialogue, header 블록을 렌더링한다", async () => {
    mockTextGetReq.mockImplementation((url, payload, resolve) => {
      resolve("아무 내용");
    });

    const formatTextMock = vi
      .spyOn(await import("../src/textFormatter"), "formatText")
      .mockReturnValue([
        { type: "separator", text: "" },
        { type: "header", text: "제목 블록" },
        { type: "dialogue", text: "대사 블록" },
      ]);

    render(<ViewTXT bookId={1} />);
    await waitFor(() => {
      expect(screen.getByText("줄 합치기")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("줄 합치기"));

    await waitFor(() => {
      expect(document.querySelector(".txt-separator")).toBeTruthy();
      expect(document.querySelector(".txt-header")?.textContent).toBe(
        "제목 블록",
      );
      expect(document.querySelector(".txt-dialogue")?.textContent).toBe(
        "대사 블록",
      );
    });

    formatTextMock.mockRestore();
  });
});
