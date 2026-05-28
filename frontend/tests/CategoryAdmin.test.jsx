// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  cleanup,
  fireEvent,
  within,
} from "@testing-library/react";

afterEach(cleanup);

// mock 함수 호이스팅
const { mockJsonGetReq, mockJsonDeleteReq, mockJsonPostReq, mockJsonPutReq } =
  vi.hoisted(() => ({
    mockJsonGetReq: vi.fn(),
    mockJsonDeleteReq: vi.fn(),
    mockJsonPostReq: vi.fn(),
    mockJsonPutReq: vi.fn(),
  }));

vi.mock("../src/Common", () => ({
  jsonGetReq: mockJsonGetReq,
  jsonDeleteReq: mockJsonDeleteReq,
  jsonPostReq: mockJsonPostReq,
  jsonPutReq: mockJsonPutReq,
  getApiUrlPrefix: () => "http://localhost:8000",
}));

vi.mock("../src/categoryMappingCache", () => ({
  fetchCategoryMappings: vi.fn(() => Promise.resolve({})),
  updateCachedMappings: vi.fn(),
}));

import CategoryAdmin from "../src/CategoryAdmin";

// ── 헬퍼 ──

const CATEGORIES_RESPONSE = {
  "1_fiction": 10,
  "2_science": 8,
  "3_history": 5,
  _root: 2,
};

const MISMATCH_RESPONSE_WITH_DATA = {
  mismatches: [{ category: "1_fiction", es_count: 10, fs_count: 8, diff: 2 }],
  es_only: [{ category: "2_science", es_count: 8 }],
  fs_only: [{ category: "4_fs_only_cat", fs_count: 9 }],
};

const MISMATCH_RESPONSE_EMPTY = {
  mismatches: [],
  es_only: [],
  fs_only: [],
};

const MAPPINGS_RESPONSE = {
  "1_fiction": ["소설", "문학"],
  "2_science": ["과학"],
};

const HIDDEN_RESPONSE = ["3_history"];

function setupMockResponses(
  categoriesResult,
  mismatchResult,
  {
    categoriesError,
    mismatchError,
    apiPrefix = "",
    mappingsResult = MAPPINGS_RESPONSE,
    hiddenResult = HIDDEN_RESPONSE,
  } = {},
) {
  mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
    if (url === apiPrefix + "/categories") {
      if (categoriesError) {
        reject(categoriesError);
      } else {
        resolve(categoriesResult);
      }
    } else if (url === apiPrefix + "/category-mismatches") {
      if (mismatchError) {
        reject(mismatchError);
      } else {
        resolve(mismatchResult);
      }
    } else if (url.startsWith("/category-mappings")) {
      resolve(mappingsResult);
    } else if (url.startsWith("/hidden-categories")) {
      resolve(hiddenResult);
    }
  });
}

describe("CategoryAdmin", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  // ── 초기 렌더링 (접힌 상태) ──

  it("초기 상태에서 타이틀 헤더를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin title="책 카테고리 관리" />);
    await waitFor(() => {
      expect(screen.getByText("책 카테고리 관리")).toBeTruthy();
    });
  });

  // ── 펼치기/접기 ──

  it("헤더 클릭 시 카드가 펼쳐진다", async () => {
    setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("카테고리 관리")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("카테고리 없음")).toBeTruthy();
    });
  });

  it("펼친 상태에서 헤더 클릭 시 접힌다", async () => {
    setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("카테고리 관리")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("카테고리 없음")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.queryByText("카테고리 없음")).toBeNull();
    });
  });

  it("카테고리가 있으면 트리 뷰로 펼쳐진다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("카테고리 관리")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
  });

  // ── 데이터 로딩 ──

  it("4개 API를 모두 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/categories",
        null,
        expect.any(Function),
        expect.any(Function),
      );
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/category-mismatches",
        null,
        expect.any(Function),
        expect.any(Function),
      );
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/category-mappings?content_type=book",
        null,
        expect.any(Function),
        expect.any(Function),
      );
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/hidden-categories?content_type=book",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it('카테고리가 없으면 "카테고리 없음" 메시지를 표시한다', async () => {
    setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("카테고리 없음")).toBeTruthy();
    });
  });

  it("트리에 모든 카테고리를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
      expect(screen.getByText("2_science")).toBeTruthy();
      expect(screen.getByText("3_history")).toBeTruthy();
    });
  });

  it("fs_only 카테고리도 트리에 포함된다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("4_fs_only_cat")).toBeTruthy();
    });
  });

  it("_root 카테고리가 트리에 포함된다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
    expect(screen.getByText("_root")).toBeTruthy();
  });

  // ── 로딩 상태 ──

  it("API 응답 전에 펼치면 로딩 스피너를 표시한다", async () => {
    const resolvers = {};
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      resolvers[url] = resolve;
    });
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    expect(screen.getByText("로딩 중...")).toBeTruthy();

    // 4개 API 모두 resolve
    resolvers["/categories"](CATEGORIES_RESPONSE);
    resolvers["/category-mismatches"](MISMATCH_RESPONSE_EMPTY);
    resolvers["/category-mappings?content_type=book"](MAPPINGS_RESPONSE);
    resolvers["/hidden-categories?content_type=book"](HIDDEN_RESPONSE);

    await waitFor(() => {
      expect(screen.queryByText("로딩 중...")).toBeNull();
    });
  });

  // ── 에러 처리 ──

  it("카테고리 API 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(null, MISMATCH_RESPONSE_EMPTY, {
      categoriesError: "Network error",
    });
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(
        screen.getByText(/카테고리 목록을 불러올 수 없습니다/),
      ).toBeTruthy();
    });
  });

  it("불일치 API 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, null, {
      mismatchError: "Server error",
    });
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(
        screen.getByText(/불일치 데이터를 불러올 수 없습니다/),
      ).toBeTruthy();
    });
  });

  it("에러 발생 시 로딩이 종료된다", async () => {
    setupMockResponses(null, MISMATCH_RESPONSE_EMPTY, {
      categoriesError: "fail",
    });
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(
        screen.getByText(/카테고리 목록을 불러올 수 없습니다/),
      ).toBeTruthy();
    });
    expect(screen.queryByText("로딩 중...")).toBeNull();
  });

  // ── 폴더 클릭 시 불일치 항목 로딩 ──

  it("불일치가 있는 폴더 클릭 시 /category-mismatches/{id} API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    // 폴더 클릭용 mock 재설정
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Book A",
              file_type: "pdf",
              file_path: "1_fiction/a.pdf",
            },
          ],
          fs_only: [
            { file_name: "orphan.txt", file_path: "1_fiction/orphan.txt" },
          ],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));

    await waitFor(() => {
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/category-mismatches/1_fiction",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });

    await waitFor(() => {
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
      expect(screen.getByText("orphan.txt")).toBeTruthy();
    });
  });

  it("booksLoaded 플래그로 중복 API 호출을 방지한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    let categoryApiCallCount = 0;
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        categoryApiCallCount++;
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Book A",
              file_type: "pdf",
              file_path: "1_fiction/a.pdf",
            },
          ],
          fs_only: [],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
    });
    expect(categoryApiCallCount).toBe(1);

    // 두 번째 클릭 — booksLoaded가 true이므로 API 호출 없음
    // 트리와 오른쪽 패널에 동일 텍스트가 있으므로 getAllByText 사용
    fireEvent.click(screen.getAllByText("1_fiction")[0]);
    await new Promise((r) => setTimeout(r, 50));
    expect(categoryApiCallCount).toBe(1);
  });

  // ── ES-only 삭제 ──

  it("ES-only 항목 선택 시 삭제/편집/조회 버튼이 표시된다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Book A",
              file_type: "pdf",
              file_path: "1_fiction/a.pdf",
            },
          ],
          fs_only: [],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("Book A.pdf"));
    await waitFor(() => {
      expect(screen.getByText("삭제")).toBeTruthy();
      expect(screen.getByText("편집")).toBeTruthy();
      expect(screen.getByText("조회")).toBeTruthy();
    });
  });

  it("삭제 버튼 클릭 시 DELETE /books/{bookId} API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Book A",
              file_type: "pdf",
              file_path: "1_fiction/a.pdf",
            },
          ],
          fs_only: [],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("Book A.pdf"));
    await waitFor(() => {
      expect(screen.getByText("삭제")).toBeTruthy();
    });

    mockJsonDeleteReq.mockImplementation((url, _payload, resolve) => {
      resolve("Ok");
    });

    fireEvent.click(screen.getByText("삭제"));
    await waitFor(() => {
      expect(mockJsonDeleteReq).toHaveBeenCalledWith(
        "/books/101",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });

    await waitFor(() => {
      expect(screen.getByText("책 정보가 삭제되었습니다.")).toBeTruthy();
    });
    expect(screen.queryByText("Book A.pdf")).toBeNull();
  });

  it("삭제 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Book A",
              file_type: "pdf",
              file_path: "1_fiction/a.pdf",
            },
          ],
          fs_only: [],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("Book A.pdf"));
    await waitFor(() => {
      expect(screen.getByText("삭제")).toBeTruthy();
    });

    mockJsonDeleteReq.mockImplementation((url, _payload, resolve, reject) => {
      reject("서버 오류");
    });

    fireEvent.click(screen.getByText("삭제"));
    await waitFor(() => {
      expect(screen.getByText("삭제 실패: 서버 오류")).toBeTruthy();
    });
    expect(screen.getAllByText("Book A.pdf").length).toBeGreaterThanOrEqual(1);
  });

  // ── FS-only 적재 ──

  it("FS-only 항목 선택 시 ES 적재/파일 삭제 버튼이 표시된다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [],
          fs_only: [
            { file_name: "orphan.txt", file_path: "1_fiction/orphan.txt" },
          ],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("orphan.txt")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("orphan.txt"));
    await waitFor(() => {
      expect(screen.getByText("ES 적재")).toBeTruthy();
      expect(screen.getByText("파일 삭제")).toBeTruthy();
    });
  });

  it("ES 적재 버튼 클릭 시 POST /category-mismatches/index-file API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [],
          fs_only: [
            { file_name: "orphan.txt", file_path: "1_fiction/orphan.txt" },
          ],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("orphan.txt")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("orphan.txt"));
    await waitFor(() => {
      expect(screen.getByText("ES 적재")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve({ book_id: 999 });
    });

    fireEvent.click(screen.getByText("ES 적재"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/index-file",
        { file_path: "1_fiction/orphan.txt" },
        expect.any(Function),
        expect.any(Function),
      );
    });

    await waitFor(() => {
      expect(screen.getByText("ES에 적재되었습니다.")).toBeTruthy();
    });
    expect(screen.queryByText("orphan.txt")).toBeNull();
  });

  it("ES 적재 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [],
          fs_only: [
            { file_name: "orphan.txt", file_path: "1_fiction/orphan.txt" },
          ],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("orphan.txt")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("orphan.txt"));
    await waitFor(() => {
      expect(screen.getByText("ES 적재")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
      reject("적재 오류");
    });

    fireEvent.click(screen.getByText("ES 적재"));
    await waitFor(() => {
      expect(screen.getByText("ES 적재 실패: 적재 오류")).toBeTruthy();
    });
  });

  it("파일 삭제 버튼 클릭 시 POST /category-mismatches/delete-file API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [],
          fs_only: [
            { file_name: "orphan.txt", file_path: "1_fiction/orphan.txt" },
          ],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("orphan.txt")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("orphan.txt"));
    await waitFor(() => {
      expect(screen.getByText("파일 삭제")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve({ success: true });
    });

    fireEvent.click(screen.getByText("파일 삭제"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/delete-file",
        { file_path: "1_fiction/orphan.txt" },
        expect.any(Function),
        expect.any(Function),
      );
    });

    await waitFor(() => {
      expect(screen.getByText("파일이 삭제되었습니다.")).toBeTruthy();
    });
    expect(screen.queryByText("orphan.txt")).toBeNull();
  });

  it("파일 삭제 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [],
          fs_only: [
            { file_name: "orphan.txt", file_path: "1_fiction/orphan.txt" },
          ],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("orphan.txt")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("orphan.txt"));
    await waitFor(() => {
      expect(screen.getByText("파일 삭제")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
      reject("삭제 실패");
    });

    fireEvent.click(screen.getByText("파일 삭제"));
    await waitFor(() => {
      expect(screen.getByText("파일 삭제 실패: 삭제 실패")).toBeTruthy();
    });
  });

  // ── 만화 contentType 테스트 ──

  describe('contentType="comic"', () => {
    it("만화 카테고리 관리 타이틀을 표시한다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
        apiPrefix: "/comics",
      });
      render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
      await waitFor(() => {
        expect(screen.getByText("만화 카테고리 관리")).toBeTruthy();
      });
    });

    it("/comics prefix로 API를 호출한다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
        apiPrefix: "/comics",
      });
      render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
      await waitFor(() => {
        expect(mockJsonGetReq).toHaveBeenCalledWith(
          "/comics/categories",
          null,
          expect.any(Function),
          expect.any(Function),
        );
        expect(mockJsonGetReq).toHaveBeenCalledWith(
          "/comics/category-mismatches",
          null,
          expect.any(Function),
          expect.any(Function),
        );
        expect(mockJsonGetReq).toHaveBeenCalledWith(
          "/category-mappings?content_type=comic",
          null,
          expect.any(Function),
          expect.any(Function),
        );
        expect(mockJsonGetReq).toHaveBeenCalledWith(
          "/hidden-categories?content_type=comic",
          null,
          expect.any(Function),
          expect.any(Function),
        );
      });
    });

    it("ES-only 항목 삭제 시 /comics/books/{id} API를 호출한다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, {
        apiPrefix: "/comics",
      });
      render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
      fireEvent.click(screen.getByText("만화 카테고리 관리"));
      await waitFor(() => {
        expect(screen.getByRole("tree")).toBeTruthy();
      });

      mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
        if (url === "/comics/categories") resolve(CATEGORIES_RESPONSE);
        else if (url === "/comics/category-mismatches")
          resolve(MISMATCH_RESPONSE_WITH_DATA);
        else if (url.startsWith("/comics/category-mismatches/")) {
          resolve({
            es_only: [
              {
                book_id: 201,
                title: "Comic A",
                file_type: "zip",
                file_path: "1_fiction/a.zip",
              },
            ],
            fs_only: [],
          });
        } else if (url.startsWith("/category-mappings"))
          resolve(MAPPINGS_RESPONSE);
        else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      });

      fireEvent.click(screen.getByText("1_fiction"));
      await waitFor(() => {
        expect(screen.getByText("Comic A.zip")).toBeTruthy();
      });

      fireEvent.click(screen.getByText("Comic A.zip"));
      await waitFor(() => {
        expect(screen.getByText("삭제")).toBeTruthy();
      });

      mockJsonDeleteReq.mockImplementation((url, _payload, resolve) => {
        resolve("Ok");
      });

      fireEvent.click(screen.getByText("삭제"));
      await waitFor(() => {
        expect(mockJsonDeleteReq).toHaveBeenCalledWith(
          "/comics/books/201",
          null,
          expect.any(Function),
          expect.any(Function),
        );
      });

      await waitFor(() => {
        expect(screen.getByText("만화 정보가 삭제되었습니다.")).toBeTruthy();
      });
    });

    it("만화 정보 설명 텍스트를 올바르게 표시한다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, {
        apiPrefix: "/comics",
      });
      render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
      fireEvent.click(screen.getByText("만화 카테고리 관리"));
      await waitFor(() => {
        expect(screen.getByRole("tree")).toBeTruthy();
      });

      mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
        if (url === "/comics/categories") resolve(CATEGORIES_RESPONSE);
        else if (url === "/comics/category-mismatches")
          resolve(MISMATCH_RESPONSE_WITH_DATA);
        else if (url.startsWith("/comics/category-mismatches/")) {
          resolve({
            es_only: [
              {
                book_id: 201,
                title: "Comic A",
                file_type: "zip",
                file_path: "1_fiction/a.zip",
              },
            ],
            fs_only: [],
          });
        } else if (url.startsWith("/category-mappings"))
          resolve(MAPPINGS_RESPONSE);
        else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      });

      fireEvent.click(screen.getByText("1_fiction"));
      await waitFor(() => {
        expect(screen.getByText("Comic A.zip")).toBeTruthy();
      });

      fireEvent.click(screen.getByText("Comic A.zip"));
      await waitFor(() => {
        expect(
          screen.getByText(
            "만화 정보만 존재하고 파일시스템에는 존재하지 않습니다.",
          ),
        ).toBeTruthy();
      });
    });
  });

  // ── 비노출 설정 ──

  it("비노출 체크박스 클릭 시 POST /hidden-categories API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("사용자 비노출")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve(["1_fiction", "3_history"]);
    });

    fireEvent.click(screen.getByLabelText("사용자 비노출"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/hidden-categories/1_fiction?content_type=book",
        { hidden: true },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  // ── 카테고리 관리 (이름 변경 / 삭제 / 재적재) ──

  it("이름 변경 버튼 클릭 시 모달이 뜨고 변경 API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const renameBtn = screen.getByTitle("이름 변경");
    fireEvent.click(renameBtn);

    const modal = await screen.findByRole("dialog");
    // getByLabelText 대신 직접 쿼리
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    fireEvent.change(input, { target: { value: "1_fiction_new" } });

    mockJsonPutReq.mockImplementation((url, payload, resolve) => {
      resolve();
    });

    fireEvent.click(within(modal).getByRole("button", { name: "변경" }));
    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledWith(
        "/categories/rename",
        { old_category: "1_fiction", new_category: "1_fiction_new" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("삭제 버튼 클릭 시 모달이 뜨고 삭제 API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const deleteBtn = screen.getByTitle("카테고리 삭제");
    fireEvent.click(deleteBtn);

    const modal = await screen.findByRole("dialog");
    expect(within(modal).getByText("카테고리 삭제")).toBeTruthy();

    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve({ deleted_count: 10 });
    });

    fireEvent.click(within(modal).getByRole("button", { name: "삭제" }));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/categories/delete",
        { category: "1_fiction" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("ES 재적재 버튼 클릭 시 모달이 뜨고 재적재 API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const reloadBtn = screen.getByTitle("ES 재적재");
    fireEvent.click(reloadBtn);

    const modal = await screen.findByRole("dialog");
    expect(within(modal).getByText("ES 재적재")).toBeTruthy();

    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve({ processed_count: 5 });
    });

    fireEvent.click(within(modal).getByRole("button", { name: "재적재" }));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/reload",
        { category: "1_fiction" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  // ── 키워드 매핑 ──

  it("키워드 추가 시 POST API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "새키워드" } });

    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve();
    });

    fireEvent.click(screen.getByText("추가"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mappings/1_fiction/keywords?content_type=book",
        { keyword: "새키워드" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("키워드 삭제 시 DELETE API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("소설")).toBeTruthy();
    });

    const deleteIcon = screen
      .getByText("소설")
      .querySelector('svg[data-icon="trash"]');
    fireEvent.click(deleteIcon);

    mockJsonDeleteReq.mockImplementation((url, payload, resolve) => {
      resolve();
    });

    await waitFor(() => {
      expect(mockJsonDeleteReq).toHaveBeenCalledWith(
        "/category-mappings/1_fiction/keywords/%EC%86%8C%EC%84%A4?content_type=book",
        null,
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  // ── 중복(Duplicate) 불일치 처리 ──

  it("중복 항목 선택 시 중복 문서 테이블을 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url.startsWith("/category-mismatches/1_fiction")) {
        resolve({
          duplicates: [
            {
              file_path: "1_fiction/dup.pdf",
              file_exists: true,
              docs: [
                {
                  book_id: 1001,
                  title: "Dup 1",
                  author: "Author",
                  file_linked: true,
                },
                {
                  book_id: 1002,
                  title: "Dup 2",
                  author: "Author",
                  file_linked: false,
                },
              ],
            },
          ],
        });
      } else
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA)(
          url,
          _payload,
          resolve,
        );
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText(/\[중복\] dup\.pdf/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/\[중복\] dup\.pdf/));
    await waitFor(() => {
      expect(screen.getByText("Dup 1")).toBeTruthy();
      expect(screen.getByText("Dup 2")).toBeTruthy();
      expect(screen.getByText("연결됨")).toBeTruthy();
      expect(screen.getByText("미연결")).toBeTruthy();
    });
  });

  it("계층 구조 카테고리를 트리 구조로 표시한다", async () => {
    const categories = {
      "prefix/fiction": 10,
      "prefix/science": 8,
    };
    const mismatchData = {
      mismatches: [
        { category: "prefix/fiction", es_count: 10, fs_count: 8, diff: 2 },
      ],
      es_only: [],
      fs_only: [],
    };
    setupMockResponses(categories, mismatchData);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
  });

  // ── 매핑/히든 API 실패 시 graceful fallback (lines 308, 314) ──

  it("매핑 API 실패 시 빈 매핑으로 진행한다", async () => {
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url.startsWith("/category-mappings")) reject("매핑 오류");
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
  });

  it("히든 카테고리 API 실패 시 빈 목록으로 진행한다", async () => {
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) reject("히든 오류");
    });
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
  });

  // ── 불일치 상세 조회 에러 (lines 392-393) ──

  it("불일치 상세 조회 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/"))
        reject("상세 조회 오류");
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText(/상세 조회 오류/)).toBeTruthy();
    });
  });

  // ── fs_count 저장 (line 378) ──

  it("불일치 상세 응답에 fs_count가 있으면 파일 건수를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Book A",
              file_type: "pdf",
              file_path: "1_fiction/a.pdf",
            },
          ],
          fs_only: [],
          fs_count: 42,
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
    });

    // 다시 카테고리 선택해서 오른쪽 패널에 fs_count Badge 표시
    // (이미 선택된 상태이므로 접힌다 → 다시 열기)
    fireEvent.click(screen.getAllByText("1_fiction")[0]);
    fireEvent.click(screen.getAllByText("1_fiction")[0]);
    await waitFor(() => {
      expect(screen.getByText("파일 42건")).toBeTruthy();
    });
  });

  // ── 키워드 추가: 빈 값/중복 (lines 445, 449-451) ──

  it("빈 키워드로 추가 시 API를 호출하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.click(screen.getByText("추가"));
    expect(mockJsonPostReq).not.toHaveBeenCalled();
  });

  it("이미 등록된 키워드 추가 시 경고 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("소설")).toBeTruthy();
    });

    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "소설" } });
    fireEvent.click(screen.getByText("추가"));
    await waitFor(() => {
      expect(screen.getByText("이미 등록된 키워드입니다.")).toBeTruthy();
    });
    expect(mockJsonPostReq).not.toHaveBeenCalled();
  });

  // ── 키워드 추가 에러 콜백 (lines 470-473) ──

  it("키워드 추가 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "새단어" } });

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        reject("추가 실패 오류");
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(screen.getByText("추가"));
    await waitFor(() => {
      expect(screen.getByText(/추가 실패 오류/)).toBeTruthy();
    });
  });

  // ── 키워드 추가 성공 시 매핑 업데이트 (line 461) ──

  it("키워드 추가 성공 시 매핑 목록에 새 키워드가 추가된다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "판타지" } });

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        resolve();
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(screen.getByText("추가"));
    await waitFor(() => {
      expect(screen.getByText("판타지")).toBeTruthy();
    });
  });

  // ── 키워드 삭제 에러 콜백 (lines 493-495) ──

  it("키워드 삭제 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("소설")).toBeTruthy();
    });

    mockJsonDeleteReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        reject("키워드 삭제 실패");
        if (onFinally) onFinally();
      },
    );

    const badge = screen.getByText("소설");
    const deleteIcon = badge.querySelector('svg[data-icon="trash"]');
    fireEvent.click(deleteIcon);
    await waitFor(() => {
      expect(screen.getByText(/키워드 삭제 실패/)).toBeTruthy();
    });
  });

  // ── 키워드 삭제 성공 시 매핑에서 제거 (lines 484-487) ──

  it("키워드 삭제 성공 시 매핑 목록에서 키워드가 제거된다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("소설")).toBeTruthy();
    });

    mockJsonDeleteReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        resolve();
        if (onFinally) onFinally();
      },
    );

    const badge = screen.getByText("소설");
    const deleteIcon = badge.querySelector('svg[data-icon="trash"]');
    fireEvent.click(deleteIcon);
    await waitFor(() => {
      expect(screen.queryByText("소설")).toBeNull();
    });
  });

  // ── 비노출 설정 에러 (lines 521-525) ──

  it("비노출 설정 변경 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("사용자 비노출")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        reject("비노출 오류");
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(screen.getByLabelText("사용자 비노출"));
    await waitFor(() => {
      expect(screen.getByText(/비노출 오류/)).toBeTruthy();
    });
  });

  // ── 이름 변경: 동일 이름 (line 534) ──

  it("현재 이름과 동일한 이름으로 변경 시 경고 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이름 변경"));

    const modal = await screen.findByRole("dialog");
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    // 이름이 이미 1_fiction으로 설정됨 → Enter로 제출 (버튼은 disabled)
    fireEvent.change(input, { target: { value: "1_fiction" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(screen.getByText("현재 이름과 동일합니다.")).toBeTruthy();
    });
    expect(mockJsonPutReq).not.toHaveBeenCalled();
  });

  // ── 이름 변경 성공 메시지 (line 544) ──

  it("이름 변경 성공 시 loadData를 다시 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이름 변경"));

    const modal = await screen.findByRole("dialog");
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    fireEvent.change(input, { target: { value: "1_fiction_renamed" } });

    const getCallsBefore = mockJsonGetReq.mock.calls.length;
    mockJsonPutReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        // loadData() 호출 시 GET mock 필요
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
        resolve();
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "변경" }));
    await waitFor(() => {
      // loadData가 다시 호출되어 새로운 GET 요청 발생
      expect(mockJsonGetReq.mock.calls.length).toBeGreaterThan(getCallsBefore);
    });
  });

  // ── 이름 변경 에러 (lines 549-553) ──

  it("이름 변경 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이름 변경"));

    const modal = await screen.findByRole("dialog");
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    fireEvent.change(input, { target: { value: "1_fiction_new" } });

    mockJsonPutReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        reject("이름 변경 서버 오류");
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "변경" }));
    await waitFor(() => {
      expect(screen.getByText(/이름 변경 서버 오류/)).toBeTruthy();
    });
  });

  // ── 카테고리 삭제 성공 메시지 (line 565) ──

  it("카테고리 삭제 성공 시 loadData를 다시 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("카테고리 삭제"));

    const modal = await screen.findByRole("dialog");

    const getCallsBefore = mockJsonGetReq.mock.calls.length;
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        // loadData() 호출 시 GET mock 필요
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
        resolve({ deleted_count: 10 });
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "삭제" }));
    await waitFor(() => {
      expect(mockJsonGetReq.mock.calls.length).toBeGreaterThan(getCallsBefore);
    });
  });

  // ── 카테고리 삭제 에러 (lines 570-574) ──

  it("카테고리 삭제 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("카테고리 삭제"));

    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        reject("삭제 서버 오류");
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "삭제" }));
    await waitFor(() => {
      expect(screen.getByText(/삭제 서버 오류/)).toBeTruthy();
    });
  });

  // ── ES 재적재 성공/에러 (lines 588-594) ──

  it("ES 재적재 성공 시 성공 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("ES 재적재"));

    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        resolve({ processed_count: 5 });
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "재적재" }));
    await waitFor(() => {
      expect(screen.getByText(/재적재 완료.*5건 처리/)).toBeTruthy();
    });
  });

  it("ES 재적재 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("ES 재적재"));

    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        reject("재적재 오류");
        if (onFinally) onFinally();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "재적재" }));
    await waitFor(() => {
      expect(screen.getByText(/재적재 오류/)).toBeTruthy();
    });
  });

  // ── 키보드 핸들러 (lines 664, 671) ──

  it("키워드 입력 필드에서 Enter 키 입력 시 키워드를 추가한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "엔터키워드" } });

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        resolve();
        if (onFinally) onFinally();
      },
    );

    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mappings/1_fiction/keywords?content_type=book",
        { keyword: "엔터키워드" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("이름 변경 모달에서 Enter 키 입력 시 이름을 변경한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이름 변경"));

    const modal = await screen.findByRole("dialog");
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    fireEvent.change(input, { target: { value: "1_fiction_enter" } });

    mockJsonPutReq.mockImplementation(
      (url, payload, resolve, reject, onFinally) => {
        resolve();
        if (onFinally) onFinally();
      },
    );

    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalledWith(
        "/categories/rename",
        { old_category: "1_fiction", new_category: "1_fiction_enter" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  // ── ES-only 편집/조회 버튼 window.open (lines 930, 936) ──

  it("ES-only 항목의 편집 버튼 클릭 시 window.open을 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Book A",
              file_type: "pdf",
              file_path: "1_fiction/a.pdf",
            },
          ],
          fs_only: [],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("Book A.pdf"));
    await waitFor(() => {
      expect(screen.getByText("편집")).toBeTruthy();
    });

    const spy = vi.spyOn(window, "open").mockImplementation(() => null);
    fireEvent.click(screen.getByText("편집"));
    expect(spy).toHaveBeenCalledWith(
      "/book-edit/101?category=1_fiction",
      "_blank",
      "noopener",
    );

    fireEvent.click(screen.getByText("조회"));
    expect(spy).toHaveBeenCalledWith(
      "/book-view/101?category=1_fiction",
      "_blank",
      "noopener",
    );
    spy.mockRestore();
  });

  // ── 중복 항목 조회/삭제 버튼 (lines 896, 903, 906, 911) ──

  it("중복 항목의 조회 버튼 클릭 시 window.open을 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url.startsWith("/category-mismatches/1_fiction")) {
        resolve({
          duplicates: [
            {
              file_path: "1_fiction/dup.pdf",
              file_exists: true,
              docs: [
                {
                  book_id: 1001,
                  title: "Dup 1",
                  author: "Author",
                  file_type: "pdf",
                  file_linked: true,
                },
                {
                  book_id: 1002,
                  title: "Dup 2",
                  author: "Author",
                  file_type: "pdf",
                  file_linked: false,
                },
              ],
            },
          ],
        });
      } else if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText(/\[중복\] dup\.pdf/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/\[중복\] dup\.pdf/));
    await waitFor(() => {
      expect(screen.getByText("Dup 1")).toBeTruthy();
      expect(screen.getByText("Dup 2")).toBeTruthy();
    });

    const spy = vi.spyOn(window, "open").mockImplementation(() => null);

    // 조회 버튼 클릭 (첫 번째 행)
    const viewButtons = screen.getAllByText("조회");
    fireEvent.click(viewButtons[0]);
    expect(spy).toHaveBeenCalledWith(
      "/book-view/1001?category=1_fiction",
      "_blank",
      "noopener",
    );

    spy.mockRestore();
  });

  it("중복 항목의 미연결 문서 삭제 버튼 클릭 시 API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url.startsWith("/category-mismatches/1_fiction")) {
        resolve({
          duplicates: [
            {
              file_path: "1_fiction/dup.pdf",
              file_exists: true,
              docs: [
                {
                  book_id: 1001,
                  title: "Dup 1",
                  author: "Author",
                  file_type: "pdf",
                  file_linked: true,
                },
                {
                  book_id: 1002,
                  title: "Dup 2",
                  author: "Author",
                  file_type: "pdf",
                  file_linked: false,
                },
              ],
            },
          ],
        });
      } else if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText(/\[중복\] dup\.pdf/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/\[중복\] dup\.pdf/));
    await waitFor(() => {
      expect(screen.getByText("Dup 2")).toBeTruthy();
    });

    // 미연결 문서의 삭제 버튼 (Dup 2, book_id 1002)
    vi.spyOn(window, "confirm").mockReturnValue(true);
    mockJsonDeleteReq.mockImplementation((url, payload, resolve) => {
      resolve();
    });

    const deleteButtons = screen.getAllByText("삭제");
    // 미연결인 1002의 삭제 버튼 클릭
    fireEvent.click(deleteButtons[0]);
    await waitFor(() => {
      expect(mockJsonDeleteReq).toHaveBeenCalledWith(
        "/category-mismatches/es-doc/1002",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });

    window.confirm.mockRestore();
  });

  it("중복 항목의 삭제 confirm 취소 시 API를 호출하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url.startsWith("/category-mismatches/1_fiction")) {
        resolve({
          duplicates: [
            {
              file_path: "1_fiction/dup.pdf",
              file_exists: true,
              docs: [
                {
                  book_id: 1002,
                  title: "Dup 2",
                  author: "Author",
                  file_type: "pdf",
                  file_linked: false,
                },
              ],
            },
          ],
        });
      } else if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText(/\[중복\] dup\.pdf/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/\[중복\] dup\.pdf/));
    await waitFor(() => {
      expect(screen.getByText("Dup 2")).toBeTruthy();
    });

    vi.spyOn(window, "confirm").mockReturnValue(false);
    const deleteButtons = screen.getAllByText("삭제");
    fireEvent.click(deleteButtons[0]);
    expect(mockJsonDeleteReq).not.toHaveBeenCalled();
    window.confirm.mockRestore();
  });

  it("중복 항목 삭제 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url.startsWith("/category-mismatches/1_fiction")) {
        resolve({
          duplicates: [
            {
              file_path: "1_fiction/dup.pdf",
              file_exists: true,
              docs: [
                {
                  book_id: 1002,
                  title: "Dup 2",
                  author: "Author",
                  file_type: "pdf",
                  file_linked: false,
                },
              ],
            },
          ],
        });
      } else if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText(/\[중복\] dup\.pdf/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/\[중복\] dup\.pdf/));
    await waitFor(() => {
      expect(screen.getByText("Dup 2")).toBeTruthy();
    });

    vi.spyOn(window, "confirm").mockReturnValue(true);
    mockJsonDeleteReq.mockImplementation((url, payload, resolve, reject) => {
      reject("중복 삭제 오류");
    });

    const deleteButtons = screen.getAllByText("삭제");
    fireEvent.click(deleteButtons[0]);
    await waitFor(() => {
      expect(screen.getByText(/중복 삭제 오류/)).toBeTruthy();
    });
    window.confirm.mockRestore();
  });

  // ── 중복 항목 파일 없음 표시 (line 890) ──

  it('중복 항목에서 파일이 없는 경우 "파일 없음" 뱃지를 표시한다', async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url.startsWith("/category-mismatches/1_fiction")) {
        resolve({
          duplicates: [
            {
              file_path: "1_fiction/dup.pdf",
              file_exists: false,
              docs: [
                {
                  book_id: 1001,
                  title: "Dup 1",
                  author: "Author",
                  file_type: "pdf",
                  file_linked: false,
                },
              ],
            },
          ],
        });
      } else if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText(/\[중복\] dup\.pdf/)).toBeTruthy();
    });

    fireEvent.click(screen.getByText(/\[중복\] dup\.pdf/));
    await waitFor(() => {
      expect(screen.getByText("파일 없음")).toBeTruthy();
    });
  });

  // ── 모달 취소 버튼 (lines 1005, 1028, 1051) ──

  it("이름 변경 모달 취소 버튼 클릭 시 모달이 닫힌다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이름 변경"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("삭제 모달 취소 버튼 클릭 시 모달이 닫힌다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("카테고리 삭제"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("ES 재적재 모달 취소 버튼 클릭 시 모달이 닫힌다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("ES 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  // ── 모달 onHide (X 버튼) (lines 984, 1017, 1040) ──

  it("이름 변경 모달 X 버튼 클릭 시 모달이 닫힌다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이름 변경"));
    const modal = await screen.findByRole("dialog");
    const closeBtn = modal.querySelector(".btn-close");
    fireEvent.click(closeBtn);
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("삭제 모달 X 버튼 클릭 시 모달이 닫힌다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("카테고리 삭제"));
    const modal = await screen.findByRole("dialog");
    const closeBtn = modal.querySelector(".btn-close");
    fireEvent.click(closeBtn);
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("ES 재적재 모달 X 버튼 클릭 시 모달이 닫힌다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("ES 재적재"));
    const modal = await screen.findByRole("dialog");
    const closeBtn = modal.querySelector(".btn-close");
    fireEvent.click(closeBtn);
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  // ── ES-only 삭제 시 warning 포함 응답 (line 608-609) ──

  it("ES-only 삭제 시 warning이 있으면 warning 메시지도 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Book A",
              file_type: "pdf",
              file_path: "1_fiction/a.pdf",
            },
          ],
          fs_only: [],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("Book A.pdf"));
    await waitFor(() => {
      expect(screen.getByText("삭제")).toBeTruthy();
    });

    mockJsonDeleteReq.mockImplementation((url, _payload, resolve) => {
      resolve({ warning: "파일 경로 참조 존재" });
    });

    fireEvent.click(screen.getByText("삭제"));
    await waitFor(() => {
      expect(screen.getByText(/파일 경로 참조 존재/)).toBeTruthy();
    });
  });

  // ── 만화 contentType: 편집/조회 URL 경로 (lines 896, 930, 936) ──

  describe('contentType="comic" window.open 경로', () => {
    it("만화 ES-only 항목의 편집/조회 URL이 comics-edit/comics-view를 사용한다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, {
        apiPrefix: "/comics",
      });
      render(<CategoryAdmin contentType="comic" title="만화 카테고리 관리" />);
      fireEvent.click(screen.getByText("만화 카테고리 관리"));
      await waitFor(() => {
        expect(screen.getByRole("tree")).toBeTruthy();
      });

      mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
        if (url === "/comics/categories") resolve(CATEGORIES_RESPONSE);
        else if (url === "/comics/category-mismatches")
          resolve(MISMATCH_RESPONSE_WITH_DATA);
        else if (url.startsWith("/comics/category-mismatches/")) {
          resolve({
            es_only: [
              {
                book_id: 201,
                title: "Comic A",
                file_type: "zip",
                file_path: "1_fiction/a.zip",
              },
            ],
            fs_only: [],
          });
        } else if (url.startsWith("/category-mappings"))
          resolve(MAPPINGS_RESPONSE);
        else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      });

      fireEvent.click(screen.getByText("1_fiction"));
      await waitFor(() => {
        expect(screen.getByText("Comic A.zip")).toBeTruthy();
      });

      fireEvent.click(screen.getByText("Comic A.zip"));
      await waitFor(() => {
        expect(screen.getByText("편집")).toBeTruthy();
      });

      const spy = vi.spyOn(window, "open").mockImplementation(() => null);
      fireEvent.click(screen.getByText("편집"));
      expect(spy).toHaveBeenCalledWith(
        "/comics-edit/201?category=1_fiction",
        "_blank",
        "noopener",
      );

      fireEvent.click(screen.getByText("조회"));
      expect(spy).toHaveBeenCalledWith(
        "/comics-view/201?category=1_fiction",
        "_blank",
        "noopener",
      );
      spy.mockRestore();
    });
  });

  // ── 서브카테고리 선택 시 키워드 섹션 미표시 ──

  it("서브카테고리 선택 시 키워드 입력 섹션이 표시되지 않는다", async () => {
    const categories = { "parent/sub1": 5 };
    const mismatchData = { mismatches: [], es_only: [], fs_only: [] };
    setupMockResponses(categories, mismatchData);
    render(<CategoryAdmin />);
    fireEvent.click(screen.getByText("카테고리 관리"));
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    // parent가 virtual parent, sub1이 leaf
    // parent 폴더를 먼저 클릭하여 확장
    const _treeItems = screen.getAllByRole("treeitem");
    // sub1을 클릭
    fireEvent.click(screen.getByText("sub1"));
    await waitFor(() => {
      expect(screen.getByText("parent/sub1")).toBeTruthy();
    });
    expect(screen.queryByPlaceholderText("새 키워드 입력")).toBeNull();
  });
});
