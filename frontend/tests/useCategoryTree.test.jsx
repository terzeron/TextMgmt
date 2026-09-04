// @vitest-environment jsdom
/* eslint-disable react/prop-types */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useState } from "react";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { useCategoryTree } from "../src/useCategoryTree";

const { mockJsonGetReq, mockRawJsonGetReq } = vi.hoisted(() => ({
  mockJsonGetReq: vi.fn(),
  mockRawJsonGetReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  jsonGetReq: mockJsonGetReq,
  rawJsonGetReq: mockRawJsonGetReq,
}));

function Probe({ role, onError, apiPrefix = "" }) {
  const tree = useCategoryTree({ apiPrefix, role, onError });
  const [result, setResult] = useState("");
  const withCallbacks = (bookId) =>
    tree.loadBookById(
      bookId,
      (book) => setResult("success:" + book.book_id),
      (error) => setResult("failure:" + error),
    );
  return (
    <div>
      <pre data-testid="state">{JSON.stringify(tree.folderData)}</pre>
      <div data-testid="result">{result}</div>
      <button onClick={() => tree.loadCategoryPage("소설")}>load novel</button>
      <button onClick={() => tree.loadCategoryPage("없음")}>
        load missing
      </button>
      <button onClick={() => tree.loadBookById(1)}>hidden book</button>
      <button onClick={() => tree.loadBookById(2)}>visible book</button>
      <button onClick={() => tree.loadBookById(3)}>missing book</button>
      <button onClick={() => withCallbacks(1)}>hidden book with cb</button>
      <button onClick={() => withCallbacks(2)}>visible book with cb</button>
      <button onClick={() => withCallbacks(3)}>missing book with cb</button>
      <button onClick={() => withCallbacks(4)}>no-category book with cb</button>
    </div>
  );
}

function setupCategorySuccess({ hidden = [] } = {}) {
  mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
    if (url === "/categories") {
      resolve({ 소설: 2 });
    } else if (url === "/hidden-categories?content_type=book") {
      resolve(hidden);
    } else if (url === "/books/1") {
      resolve({
        book_id: 1,
        title: "비공개",
        file_type: "epub",
        category: "비공개/하위",
      });
    } else if (url === "/books/2") {
      resolve({
        book_id: 2,
        title: "공개",
        file_type: "epub",
        category: "소설",
      });
    } else if (url === "/books/3") {
      reject("not found");
    } else if (url === "/books/4") {
      resolve({ book_id: 4, title: "카테고리 없음", file_type: "epub" });
    }
  });
}

describe("useCategoryTree", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockRawJsonGetReq.mockReset();
  });

  afterEach(cleanup);

  it("카테고리 로드 실패 시 onError가 없어도 예외 없이 종료한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      reject("boom");
    });

    render(<Probe />);

    await waitFor(() =>
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/categories",
        null,
        expect.any(Function),
        expect.any(Function),
      ),
    );
    expect(screen.getByTestId("state").textContent).toBe("[]");
  });

  it("없는 폴더를 로드하려 하면 요청하지 않는다", async () => {
    setupCategorySuccess();

    render(<Probe />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(screen.getByRole("button", { name: "load missing" }));

    expect(mockRawJsonGetReq).not.toHaveBeenCalled();
  });

  it("이미 로딩 중인 폴더는 중복 요청하지 않는다", async () => {
    setupCategorySuccess();
    mockRawJsonGetReq.mockImplementation(() => {});

    render(<Probe />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(screen.getByRole("button", { name: "load novel" }));
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("loadingBooks"),
    );
    fireEvent.click(screen.getByRole("button", { name: "load novel" }));

    expect(mockRawJsonGetReq).toHaveBeenCalledTimes(1);
  });

  it("책 목록 응답이 success가 아니면 로딩 상태만 해제한다", async () => {
    setupCategorySuccess();
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      resolve({ status: "error" });
    });

    render(<Probe />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(screen.getByRole("button", { name: "load novel" }));

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain(
        '"loadingBooks":false',
      ),
    );
  });

  it("책 목록 성공 응답의 result/total/cursor가 없으면 빈 목록 fallback을 사용한다", async () => {
    setupCategorySuccess();
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      resolve({ status: "success" });
    });

    render(<Probe />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(screen.getByRole("button", { name: "load novel" }));

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain(
        '"booksLoaded":true',
      ),
    );
    expect(screen.getByTestId("state").textContent).not.toContain("더 보기");
  });

  it("책 단건 조회 callback이 없어도 hidden/성공/실패 경로를 처리한다", async () => {
    setupCategorySuccess({ hidden: ["비공개"] });

    render(<Probe role="viewer" />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(screen.getByRole("button", { name: "hidden book" }));
    fireEvent.click(screen.getByRole("button", { name: "visible book" }));
    fireEvent.click(screen.getByRole("button", { name: "missing book" }));

    await waitFor(() => {
      const urls = mockJsonGetReq.mock.calls.map((call) => call[0]);
      expect(urls).toEqual(
        expect.arrayContaining(["/books/1", "/books/2", "/books/3"]),
      );
    });
  });

  // ── 카테고리 목록 로딩 fallback/분기 ──

  it("hidden 목록 조회가 실패하면 전체 카테고리를 표시한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") resolve({ 소설: 2 });
      else if (url === "/hidden-categories?content_type=book")
        reject("hidden fetch failed");
    });

    render(<Probe role="viewer" />);

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );
  });

  it("categoryCounts 응답이 falsy이면 빈 객체로 대체한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve(null);
    });

    render(<Probe />);

    await waitFor(() =>
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/categories",
        null,
        expect.any(Function),
        expect.any(Function),
      ),
    );
    expect(screen.getByTestId("state").textContent).toBe("[]");
  });

  it("카테고리 목록에 빈 문자열 카테고리가 있어도 정렬 시 오류 없이 처리한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 2, "": 1, 만화: 1 });
    });

    render(<Probe />);

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );
    expect(screen.getByTestId("state").textContent).toContain("만화");
  });

  it("카테고리 로드 실패 시 onError가 있으면 호출된다", async () => {
    const onError = vi.fn();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      reject("boom");
    });

    render(<Probe onError={onError} />);

    await waitFor(() => expect(onError).toHaveBeenCalledWith("boom"));
  });

  it("apiPrefix가 comics이고 role이 viewer이면 comic hidden-categories를 조회한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/comics/categories") resolve({ 만화: 1 });
      else if (url === "/hidden-categories?content_type=comic") resolve([]);
    });

    render(<Probe role="viewer" apiPrefix="/comics" />);

    await waitFor(() =>
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/hidden-categories?content_type=comic",
        null,
        expect.any(Function),
        expect.any(Function),
      ),
    );
  });

  it("hidden 목록 응답이 falsy이면 전체 카테고리를 표시한다", async () => {
    setupCategorySuccess({ hidden: null });

    render(<Probe role="viewer" />);

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );
  });

  it("숨김 카테고리와 일치하는 카테고리는 트리에서 제외된다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 2, 비공개: 1 });
      else if (url === "/hidden-categories?content_type=book")
        resolve(["비공개"]);
    });

    render(<Probe role="viewer" />);

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );
    expect(screen.getByTestId("state").textContent).not.toContain("비공개");
  });

  // ── 최상위(_root) 파일 로딩 ──

  it("루트 파일이 있으면 책 목록을 가져와 트리에 추가한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 2, _root: 3 });
      else if (url === "/categories/_root")
        resolve([
          { book_id: 500, title: "가나문학", file_type: "epub" },
          { book_id: 501 }, // title, file_type 없음
          { book_id: 502, title: "다라문학", file_type: "epub" },
        ]);
    });

    render(<Probe />);

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("가나문학"),
    );
    const state = screen.getByTestId("state").textContent;
    expect(state).toContain("다라문학");
    expect(state).toContain('"id":"/501"');
  });

  it("루트 파일 응답이 배열이 아니면 빈 목록으로 처리한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve) => {
      if (url === "/categories") resolve({ 소설: 2, _root: 2 });
      else if (url === "/categories/_root") resolve({ not: "array" });
    });

    render(<Probe />);

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );
  });

  it("루트 파일 조회 실패 시 기존 폴더 데이터만 사용한다", async () => {
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories") resolve({ 소설: 2, _root: 1 });
      else if (url === "/categories/_root") reject();
    });

    render(<Probe />);

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );
  });

  // ── 커서 기반 페이지네이션 ──

  it("다음 페이지 커서로 이어서 책 목록을 불러온다", async () => {
    setupCategorySuccess();
    let callCount = 0;
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      callCount += 1;
      if (callCount === 1) {
        resolve({
          status: "success",
          result: [
            { book_id: 1, title: "A", file_type: "epub" },
            { book_id: 2, title: "B", file_type: "epub" },
          ],
          next_cursor: "CUR1",
          total: 5,
        });
      } else {
        resolve({
          status: "success",
          result: [{ book_id: 3, title: "C", file_type: "epub" }],
          next_cursor: "",
          total: 5,
        });
      }
    });

    render(<Probe />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(screen.getByRole("button", { name: "load novel" }));
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("더 보기"),
    );

    fireEvent.click(screen.getByRole("button", { name: "load novel" }));
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("C.epub"),
    );

    expect(mockRawJsonGetReq).toHaveBeenCalledTimes(2);
    expect(mockRawJsonGetReq.mock.calls[1][0]).toContain("cursor=CUR1");
    expect(screen.getByTestId("state").textContent).not.toContain("더 보기");
  });

  // ── 책 단건 조회 콜백 ──

  it("숨김 카테고리 접근 시 콜백으로 실패를 알린다", async () => {
    setupCategorySuccess({ hidden: ["비공개"] });
    render(<Probe role="viewer" />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(
      screen.getByRole("button", { name: "hidden book with cb" }),
    );

    await waitFor(() =>
      expect(screen.getByTestId("result").textContent).toBe(
        "failure:접근 권한이 없는 카테고리입니다.",
      ),
    );
  });

  it("공개 카테고리 조회 성공 시 콜백으로 성공을 알린다", async () => {
    setupCategorySuccess({ hidden: ["비공개"] });
    render(<Probe role="viewer" />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(
      screen.getByRole("button", { name: "visible book with cb" }),
    );

    await waitFor(() =>
      expect(screen.getByTestId("result").textContent).toBe("success:2"),
    );
  });

  it("책 조회 실패 시 콜백으로 실패를 알린다", async () => {
    setupCategorySuccess({ hidden: ["비공개"] });
    render(<Probe role="viewer" />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(
      screen.getByRole("button", { name: "missing book with cb" }),
    );

    await waitFor(() =>
      expect(screen.getByTestId("result").textContent).toBe(
        "failure:not found",
      ),
    );
  });

  it("카테고리 필드가 없는 책은 숨김 검사를 건너뛰고 성공 처리된다", async () => {
    setupCategorySuccess({ hidden: ["비공개"] });
    render(<Probe role="viewer" />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(
      screen.getByRole("button", { name: "no-category book with cb" }),
    );

    await waitFor(() =>
      expect(screen.getByTestId("result").textContent).toBe("success:4"),
    );
  });

  it("role이 viewer가 아니면 hidden 카테고리 검사를 건너뛴다", async () => {
    setupCategorySuccess();
    render(<Probe />);
    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain("소설"),
    );

    fireEvent.click(
      screen.getByRole("button", { name: "visible book with cb" }),
    );

    await waitFor(() =>
      expect(screen.getByTestId("result").textContent).toBe("success:2"),
    );
  });
});
