// @vitest-environment jsdom
/* eslint-disable react/prop-types */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import { useCategoryTree } from "../src/useCategoryTree";

const { mockJsonGetReq, mockRawJsonGetReq } = vi.hoisted(() => ({
  mockJsonGetReq: vi.fn(),
  mockRawJsonGetReq: vi.fn(),
}));

vi.mock("../src/Common", () => ({
  jsonGetReq: mockJsonGetReq,
  rawJsonGetReq: mockRawJsonGetReq,
}));

function Probe({ role, onError }) {
  const tree = useCategoryTree({ apiPrefix: "", role, onError });
  return (
    <div>
      <pre data-testid="state">{JSON.stringify(tree.folderData)}</pre>
      <button onClick={() => tree.loadCategoryPage("소설")}>load novel</button>
      <button onClick={() => tree.loadCategoryPage("없음")}>load missing</button>
      <button onClick={() => tree.loadBookById(1)}>hidden book</button>
      <button onClick={() => tree.loadBookById(2)}>visible book</button>
      <button onClick={() => tree.loadBookById(3)}>missing book</button>
    </div>
  );
}

function setupCategorySuccess({ hidden = [] } = {}) {
  mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
    if (url === "/categories") {
      resolve({ "소설": 2 });
    } else if (url === "/hidden-categories?content_type=book") {
      resolve(hidden);
    } else if (url === "/books/1") {
      resolve({ book_id: 1, title: "비공개", file_type: "epub", category: "비공개/하위" });
    } else if (url === "/books/2") {
      resolve({ book_id: 2, title: "공개", file_type: "epub", category: "소설" });
    } else if (url === "/books/3") {
      reject("not found");
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

    await waitFor(() => expect(mockJsonGetReq).toHaveBeenCalledWith(
      "/categories",
      null,
      expect.any(Function),
      expect.any(Function),
    ));
    expect(screen.getByTestId("state").textContent).toBe("[]");
  });

  it("없는 폴더를 로드하려 하면 요청하지 않는다", async () => {
    setupCategorySuccess();

    render(<Probe />);
    await waitFor(() => expect(screen.getByTestId("state").textContent).toContain("소설"));

    fireEvent.click(screen.getByRole("button", { name: "load missing" }));

    expect(mockRawJsonGetReq).not.toHaveBeenCalled();
  });

  it("이미 로딩 중인 폴더는 중복 요청하지 않는다", async () => {
    setupCategorySuccess();
    mockRawJsonGetReq.mockImplementation(() => {});

    render(<Probe />);
    await waitFor(() => expect(screen.getByTestId("state").textContent).toContain("소설"));

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
    await waitFor(() => expect(screen.getByTestId("state").textContent).toContain("소설"));

    fireEvent.click(screen.getByRole("button", { name: "load novel" }));

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain('"loadingBooks":false'),
    );
  });

  it("책 목록 성공 응답의 result/total/cursor가 없으면 빈 목록 fallback을 사용한다", async () => {
    setupCategorySuccess();
    mockRawJsonGetReq.mockImplementation((url, resolve) => {
      resolve({ status: "success" });
    });

    render(<Probe />);
    await waitFor(() => expect(screen.getByTestId("state").textContent).toContain("소설"));

    fireEvent.click(screen.getByRole("button", { name: "load novel" }));

    await waitFor(() =>
      expect(screen.getByTestId("state").textContent).toContain('"booksLoaded":true'),
    );
    expect(screen.getByTestId("state").textContent).not.toContain("더 보기");
  });

  it("책 단건 조회 callback이 없어도 hidden/성공/실패 경로를 처리한다", async () => {
    setupCategorySuccess({ hidden: ["비공개"] });

    render(<Probe role="viewer" />);
    await waitFor(() => expect(screen.getByTestId("state").textContent).toContain("소설"));

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
});
