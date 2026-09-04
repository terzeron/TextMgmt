// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  cleanup,
  fireEvent,
  within,
  act,
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

import { ThemeProvider, createTheme } from "@mui/material/styles";

import CategoryAdminBase, {
  formatErrorMessage,
  getAutoClassifyRemainingCount,
  getReloadRemainingCount,
} from "../src/CategoryAdmin";

// ── 헬퍼 ──

const CategoryAdmin = (props) => (
  <CategoryAdminBase initialShowOnlyAbnormal={false} {...props} />
);

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
const LATEST_EXCLUDED_RESPONSE = ["2_science"];

function setupMockResponses(
  categoriesResult,
  mismatchResult,
  {
    categoriesError,
    mismatchError,
    apiPrefix = "",
    mappingsResult = MAPPINGS_RESPONSE,
    hiddenResult = HIDDEN_RESPONSE,
    latestExcludedResult = LATEST_EXCLUDED_RESPONSE,
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
    } else if (url === apiPrefix + "/category-mismatches/reload-status") {
      resolve({ status: "idle" });
    } else if (url === apiPrefix + "/categories/auto-classify-status") {
      resolve({ status: "idle" });
    } else if (url.startsWith("/category-mappings")) {
      resolve(mappingsResult);
    } else if (url.startsWith("/hidden-categories")) {
      resolve(hiddenResult);
    } else if (url.startsWith("/latest-excluded-categories")) {
      resolve(latestExcludedResult);
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

  // ── 초기 렌더링 ──
  // 카드 헤더가 없어졌으므로 마운트 즉시 본문이 렌더링된다.

  it("마운트 직후 본문을 렌더링한다", async () => {
    setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("카테고리 없음")).toBeTruthy();
    });
  });

  it("카테고리가 있으면 트리 뷰로 펼쳐진다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
  });

  // ── 데이터 로딩 ──

  it("필수 데이터 API와 최신 자료 제외 설정 API를 모두 호출한다", async () => {
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
      expect(mockJsonGetReq).toHaveBeenCalledWith(
        "/latest-excluded-categories?content_type=book",
        null,
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it('카테고리가 없으면 "카테고리 없음" 메시지를 표시한다', async () => {
    setupMockResponses({}, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("카테고리 없음")).toBeTruthy();
    });
  });

  it("트리에 모든 카테고리를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
      expect(screen.getByText("2_science")).toBeTruthy();
      expect(screen.getByText("3_history")).toBeTruthy();
    });
  });

  it("fs_only 카테고리도 트리에 포함된다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("4_fs_only_cat")).toBeTruthy();
    });
  });

  it("_root 카테고리가 트리에 포함된다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
    expect(screen.getByText("_root")).toBeTruthy();
  });

  it("디렉토리 헤더에 이상 항목만 보기 토글을 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const toggle = within(header).getByLabelText("이상 항목만 보기");
    expect(toggle).toBeTruthy();
    expect(toggle.checked).toBe(false);
  });

  it("디렉토리 헤더 컨트롤을 레이블, 토글, 재적재 버튼, 자동 분류 순서의 형제로 배치한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const label = within(header).getByText("디렉토리");
    const toggle = within(header)
      .getByLabelText("이상 항목만 보기")
      .closest(".form-check");
    const bulkReloadButton = within(header).getByRole("button", {
      name: /일괄 재적재/,
    });
    const mismatchReloadButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });
    const autoClassifyButton = within(header).getByTitle("자동 분류");

    expect(label.parentElement).toBe(header);
    expect(toggle.parentElement).toBe(header);
    expect(bulkReloadButton.parentElement).toBe(header);
    expect(mismatchReloadButton.parentElement).toBe(header);
    expect(autoClassifyButton.parentElement).toBe(header);
    expect(within(header).getByText("이상 항목만")).toBeTruthy();
    expect(bulkReloadButton.textContent).toContain("일괄");
    expect(bulkReloadButton.textContent).not.toContain("재적재");
    expect(mismatchReloadButton.textContent).toContain("이상 항목");
    expect(mismatchReloadButton.textContent).not.toContain("재적재");
    expect(Array.from(header.children)).toEqual([
      label,
      toggle,
      bulkReloadButton,
      mismatchReloadButton,
      autoClassifyButton,
    ]);
  });

  it("작업 중이 아니면 디렉토리 헤더 버튼에 잔여 0건을 표시하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const bulkReloadButton = within(header).getByRole("button", {
      name: /일괄 재적재/,
    });
    const mismatchReloadButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });
    const autoClassifyButton = within(header).getByTitle("자동 분류");

    expect(bulkReloadButton.textContent).toContain("일괄");
    expect(bulkReloadButton.textContent).not.toContain("잔여");
    expect(mismatchReloadButton.textContent).toContain("이상 항목");
    expect(mismatchReloadButton.textContent).not.toContain("잔여");
    expect(autoClassifyButton.textContent).toContain("자동 분류");
  });

  it("작업 중이 아니면 선택 카테고리 이상 항목 버튼에 잔여 0건을 표시하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const mismatchReloadButton = screen.getByTitle("이상 항목만 ES 재적재");

    expect(mismatchReloadButton.textContent).toContain("이상 항목 재적재");
    expect(mismatchReloadButton.textContent).not.toContain("잔여");
  });

  it("기본값으로 이상 항목만 보기 토글이 켜져 있다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdminBase />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    expect(screen.getByLabelText("이상 항목만 보기").checked).toBe(true);
    expect(screen.getByText("1_fiction")).toBeTruthy();
    expect(screen.queryByText("3_history")).toBeNull();
  });

  it("이상 항목만 보기 활성화 시 정상 카테고리를 숨긴다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("3_history")).toBeTruthy();
    });

    fireEvent.click(screen.getByLabelText("이상 항목만 보기"));
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
      expect(screen.getByText("2_science")).toBeTruthy();
      expect(screen.getByText("4_fs_only_cat")).toBeTruthy();
      expect(screen.queryByText("3_history")).toBeNull();
      expect(screen.queryByText("_root")).toBeNull();
    });
  });

  it("이상 항목만 보기 상태에서 펼친 디렉토리의 정상 하위 항목을 숨긴다", async () => {
    const categories = {
      parent: 10,
      "parent/abnormal_child": 3,
      "parent/normal_child": 7,
    };
    const mismatchData = {
      mismatches: [{ category: "parent", es_count: 10, fs_count: 9, diff: 1 }],
      es_only: [{ category: "parent/abnormal_child", es_count: 3 }],
      fs_only: [],
    };
    setupMockResponses(categories, mismatchData);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("parent")).toBeTruthy();
    });

    fireEvent.click(screen.getByLabelText("이상 항목만 보기"));
    await waitFor(() => {
      const tree = screen.getByRole("tree");
      expect(within(tree).getByText("parent")).toBeTruthy();
      expect(within(tree).queryByText("normal_child")).toBeNull();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/category-mismatches") resolve(mismatchData);
      else if (url.startsWith("/category-mismatches/parent")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Missing File",
              file_type: "pdf",
              file_path: "parent/missing.pdf",
            },
          ],
          fs_only: [],
          duplicates: [],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("parent"));
    await waitFor(() => {
      const tree = screen.getByRole("tree");
      expect(within(tree).getByText("Missing File.pdf")).toBeTruthy();
      expect(within(tree).queryByText("normal_child")).toBeNull();
    });
  });

  it("디렉토리를 펼친 뒤 이상 항목만 보기로 전환해도 정상 하위 항목을 숨긴다", async () => {
    const categories = {
      parent: 10,
      "parent/abnormal_child": 3,
      "parent/normal_child": 7,
    };
    const mismatchData = {
      mismatches: [{ category: "parent", es_count: 10, fs_count: 9, diff: 1 }],
      es_only: [{ category: "parent/abnormal_child", es_count: 3 }],
      fs_only: [],
    };
    setupMockResponses(categories, mismatchData);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("parent")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/category-mismatches") resolve(mismatchData);
      else if (url.startsWith("/category-mismatches/parent")) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Missing File",
              file_type: "pdf",
              file_path: "parent/missing.pdf",
            },
          ],
          fs_only: [],
          duplicates: [],
        });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("parent"));
    await waitFor(() => {
      const tree = screen.getByRole("tree");
      expect(within(tree).getByText("normal_child")).toBeTruthy();
      expect(within(tree).getByText("Missing File.pdf")).toBeTruthy();
    });

    fireEvent.click(screen.getByLabelText("이상 항목만 보기"));
    await waitFor(() => {
      const tree = screen.getByRole("tree");
      expect(within(tree).getByText("Missing File.pdf")).toBeTruthy();
      expect(within(tree).queryByText("normal_child")).toBeNull();
    });
  });

  it("만화 카테고리 관리에도 이상 항목만 보기 토글을 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, {
      apiPrefix: "/comics",
    });
    render(<CategoryAdmin contentType="comic" />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    expect(within(header).getByLabelText("이상 항목만 보기")).toBeTruthy();
  });

  it("불일치 항목이 있으면 일괄 재적재 버튼을 활성화한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    expect(screen.getByRole("button", { name: /일괄 재적재/ }).disabled).toBe(
      false,
    );
  });

  it("불일치 항목이 없으면 일괄 재적재 버튼을 비활성화한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    expect(screen.getByRole("button", { name: /일괄 재적재/ }).disabled).toBe(
      true,
    );
  });

  it("불일치 항목이 없으면 디렉토리 헤더의 이상 항목 재적재 버튼을 비활성화한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const mismatchReloadButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });
    expect(mismatchReloadButton.disabled).toBe(true);
  });

  it("디렉토리 헤더에서 카테고리 미선택 상태에서는 전체 이상 항목을 재적재한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const mismatchReloadButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });
    expect(mismatchReloadButton.disabled).toBe(false);
    fireEvent.click(mismatchReloadButton);

    const modal = await screen.findByRole("dialog");
    expect(
      within(modal).getByText(
        /현재 불일치 카테고리 3개의 이상 항목 19건을 ES에 재적재합니다/,
      ),
    ).toBeTruthy();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "done",
          category: null,
          indexed_count: 1,
          deleted_count: 1,
          after_count: 0,
          failed_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/reload-all",
        null,
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
      expect(screen.getByText(/이상 항목 일괄 ES 재적재 완료/)).toBeTruthy();
    });
  });

  it("디렉토리 헤더에서 선택 카테고리의 이상 항목만 재적재한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const mismatchReloadButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });
    expect(mismatchReloadButton.disabled).toBe(false);

    fireEvent.click(screen.getByText("1_fiction"));
    expect(mismatchReloadButton.disabled).toBe(false);
    fireEvent.click(mismatchReloadButton);

    const modal = await screen.findByRole("dialog");
    expect(within(modal).getByText(/2건만 ES에 재적재합니다/)).toBeTruthy();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({
          indexed_count: 1,
          deleted_count: 1,
          after_count: 0,
          failed_count: 0,
        });
        if (done) done();
      },
    );
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/reload-mismatches",
        { category: "1_fiction" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  // ── 로딩 상태 ──

  it("API 응답 전에 펼치면 로딩 스피너를 표시한다", async () => {
    const resolvers = {};
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      resolvers[url] = resolve;
    });
    render(<CategoryAdmin />);
    expect(screen.getByText("로딩 중...")).toBeTruthy();

    // tree 렌더링에 필요한 4개 API resolve
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

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, final) => {
        resolve({ book_id: 999 });
        final();
      },
    );

    fireEvent.click(screen.getByText("ES 적재"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/index-file",
        { file_path: "1_fiction/orphan.txt" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });

    await waitFor(() => {
      expect(screen.getByText("ES에 적재되었습니다.")).toBeTruthy();
    });
    expect(screen.queryByText("orphan.txt")).toBeNull();
  });

  it("ES 적재 요청 중 버튼 내부에 spinner를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
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

    mockJsonPostReq.mockImplementation(() => {});

    fireEvent.click(screen.getByText("ES 적재"));

    const indexButton = screen.getByRole("button", { name: /ES 적재/ });
    expect(indexButton.disabled).toBe(true);
    expect(indexButton.querySelector(".spinner-border")).toBeTruthy();
  });

  it("ES 적재 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
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

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, final) => {
        reject("적재 오류");
        final();
      },
    );

    fireEvent.click(screen.getByText("ES 적재"));
    await waitFor(() => {
      expect(screen.getByText("ES 적재 실패: 적재 오류")).toBeTruthy();
    });
  });

  it("파일 삭제 버튼 클릭 시 POST /category-mismatches/delete-file API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
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
    it("마운트 직후 만화 본문을 렌더링한다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
        apiPrefix: "/comics",
      });
      render(<CategoryAdmin contentType="comic" />);
      await waitFor(() => {
        expect(screen.getByText("디렉토리")).toBeTruthy();
      });
    });

    it("/comics prefix로 API를 호출한다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
        apiPrefix: "/comics",
      });
      render(<CategoryAdmin contentType="comic" />);
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
        expect(mockJsonGetReq).toHaveBeenCalledWith(
          "/latest-excluded-categories?content_type=comic",
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
      render(<CategoryAdmin contentType="comic" />);
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
      render(<CategoryAdmin contentType="comic" />);
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

  it("최신 자료 검색 제외 설정이 있으면 체크박스를 선택 상태로 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
      latestExcludedResult: ["2_science"],
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("2_science")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("2_science"));
    await waitFor(() => {
      expect(screen.getByLabelText("최신 자료 검색 제외").checked).toBe(true);
    });
  });

  it("최신 자료 검색 제외 체크박스 클릭 시 POST /latest-excluded-categories API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
      latestExcludedResult: [],
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("최신 자료 검색 제외")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve(["1_fiction"]);
    });

    fireEvent.click(screen.getByLabelText("최신 자료 검색 제외"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/latest-excluded-categories/1_fiction?content_type=book",
        { excluded: true },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("만화 카테고리에서 최신 자료 검색 제외 체크박스 클릭 시 comic content_type으로 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
      apiPrefix: "/comics",
      latestExcludedResult: [],
    });
    render(<CategoryAdmin contentType="comic" />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("최신 자료 검색 제외")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve(["1_fiction"]);
    });

    fireEvent.click(screen.getByLabelText("최신 자료 검색 제외"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/latest-excluded-categories/1_fiction?content_type=comic",
        { excluded: true },
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

  it("자동 분류 버튼은 카테고리 미선택 시 최상위 _root 파일을 non-recursive로 분류한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories/auto-classify-status") {
        resolve({
          status: "done",
          source_category: "_root",
          moved_count: 2,
          skipped_count: 1,
          failed_count: 0,
          remaining_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(within(header).getByTitle("자동 분류"));

    const modal = await screen.findByRole("dialog");
    expect(
      within(modal).getByText("자동 분류", { selector: ".modal-title" }),
    ).toBeTruthy();
    expect(
      within(modal).getByText(/최상위 디렉토리의 바로 아래 파일을/),
    ).toBeTruthy();
    expect(within(modal).queryByLabelText("하위 디렉토리 포함")).toBeNull();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({ started: true });
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "자동 분류" }));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/categories/auto-classify",
        { category: "_root", recursive: false, async_mode: true },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
      expect(
        screen.getByText(
          /최상위 디렉토리 자동 분류 완료 \(이동 2건, 제외 1건, 실패 0건\)/,
        ),
      ).toBeTruthy();
    });
  });

  it("자동 분류 버튼 클릭 시 선택 카테고리 파일을 non-recursive로 자동 분류한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories/auto-classify-status") {
        resolve({
          status: "done",
          source_category: "1_fiction",
          moved_count: 2,
          skipped_count: 1,
          failed_count: 0,
          remaining_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(within(header).getByTitle("자동 분류"));

    const modal = await screen.findByRole("dialog");
    expect(
      within(modal).getByText("자동 분류", { selector: ".modal-title" }),
    ).toBeTruthy();
    expect(
      within(modal).getByText(/카테고리 '1_fiction'의 바로 아래 파일을/),
    ).toBeTruthy();
    expect(within(modal).queryByLabelText("하위 디렉토리 포함")).toBeNull();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({ started: true });
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "자동 분류" }));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/categories/auto-classify",
        { category: "1_fiction", recursive: false, async_mode: true },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
      expect(
        screen.getByText(/자동 분류 완료 \(이동 2건, 제외 1건, 실패 0건\)/),
      ).toBeTruthy();
    });
  });

  it("자동 분류 진행 중에는 버튼에 잔여 건 수를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories/auto-classify-status") {
        resolve({
          status: "running",
          source_category: "_root",
          total_count: 8,
          processed_count: 3,
          remaining_count: 5,
          moved_count: 2,
          skipped_count: 1,
          failed_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(within(header).getByTitle("자동 분류"));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({ started: true });
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "자동 분류" }));

    await waitFor(() => {
      expect(
        within(screen.getByTitle("자동 분류")).getByText("잔여 5건"),
      ).toBeTruthy();
    });
  });

  it("자동 분류 완료 상태에 카운트와 source_category가 없어도 기본 완료 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories/auto-classify-status") {
        resolve({ status: "done" });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(within(header).getByTitle("자동 분류"));
    const modal = await screen.findByRole("dialog");
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({ started: true });
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "자동 분류" }));

    await waitFor(() => {
      expect(
        screen.getByText(
          /최상위 디렉토리 자동 분류 완료 \(이동 0건, 제외 0건, 실패 0건\)/,
        ),
      ).toBeTruthy();
    });
  });

  it("자동 분류 작업이 실패 상태로 끝나면 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories/auto-classify-status") {
        resolve({ status: "failed", error: "자동 분류 실패" });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(within(header).getByTitle("자동 분류"));
    const modal = await screen.findByRole("dialog");
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({ started: true });
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "자동 분류" }));

    await waitFor(() => {
      expect(screen.getByText("자동 분류 실패")).toBeTruthy();
    });
  });

  it("자동 분류 시작 응답이 already_running이면 진행 상태를 즉시 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/categories/auto-classify-status") {
        resolve({
          status: "running",
          total_count: 7,
          processed_count: 2,
          remaining_count: 5,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(within(header).getByTitle("자동 분류"));
    const modal = await screen.findByRole("dialog");
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({
          already_running: true,
          status: "running",
          total_count: 7,
          processed_count: 2,
          remaining_count: 5,
        });
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "자동 분류" }));

    await waitFor(() => {
      expect(
        within(screen.getByTitle("자동 분류")).getByText("잔여 5건"),
      ).toBeTruthy();
    });
  });

  it("자동 분류 시작 요청이 실패하면 버튼을 원래 상태로 돌리고 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(within(header).getByTitle("자동 분류"));
    const modal = await screen.findByRole("dialog");
    mockJsonPostReq.mockImplementation(
      (url, payload, _resolve, reject, final) => {
        reject(null);
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "자동 분류" }));

    await waitFor(() => {
      const autoClassifyButton = screen.getByTitle("자동 분류");
      expect(autoClassifyButton.textContent).toContain("자동 분류");
      expect(autoClassifyButton.querySelector(".spinner-border")).toBeNull();
      expect(screen.getByText("자동 분류에 실패했습니다.")).toBeTruthy();
    });
  });

  it("선택 카테고리 버튼 그룹에는 자동 분류 버튼을 표시하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const actionGroup = screen.getByTitle("이름 변경").closest(".d-flex");

    expect(within(actionGroup).queryByTitle("자동 분류")).toBeNull();
    expect(
      Array.from(actionGroup.querySelectorAll("button")).map(
        (button) => button.title,
      ),
    ).toEqual([
      "이름 변경",
      "카테고리 삭제",
      "ES 재적재",
      "이상 항목만 ES 재적재",
    ]);
  });

  it("만화 카테고리 자동 분류는 /comics prefix와 non-recursive 옵션을 사용한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
      apiPrefix: "/comics",
    });
    render(<CategoryAdmin contentType="comic" />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(within(header).getByTitle("자동 분류"));

    const modal = await screen.findByRole("dialog");
    expect(within(modal).queryByLabelText("하위 디렉토리 포함")).toBeNull();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({ started: true });
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "자동 분류" }));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/comics/categories/auto-classify",
        { category: "1_fiction", recursive: false, async_mode: true },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("ES 재적재 버튼 클릭 시 모달이 뜨고 재적재 API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
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

  it("일괄 재적재 실행 시 reload-all API를 호출하고 결과를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));

    const modal = await screen.findByRole("dialog");
    expect(within(modal).getByText("불일치 일괄 재적재")).toBeTruthy();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, final) => {
        resolve({ started: true });
        final();
      },
    );
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "done",
          category: null,
          indexed_count: 4,
          deleted_count: 2,
          after_count: 1,
          failed_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/reload-all",
        null,
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    await waitFor(() => {
      expect(screen.getByText(/이상 항목 일괄 ES 재적재 완료/)).toBeTruthy();
      expect(screen.getByText(/남은 이상 1건/)).toBeTruthy();
    });
  });

  // ── 키워드 매핑 ──

  it("키워드 추가 시 POST API를 호출한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
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
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
  });

  // ── 불일치 상세 조회 에러 (lines 392-393) ──

  it("불일치 상세 조회 실패 시 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
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

  it("이상 항목 재적재 버튼은 선택 카테고리의 불일치만 재적재한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
    const modal = await screen.findByRole("dialog");
    expect(within(modal).getByText(/2건만 ES에 재적재합니다/)).toBeTruthy();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "done",
          category: "1_fiction",
          indexed_count: 1,
          deleted_count: 1,
          after_count: 0,
          failed_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/reload-mismatches",
        { category: "1_fiction" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
      expect(screen.getByText(/이상 항목 ES 재적재 완료/)).toBeTruthy();
    });
  });

  // ── 모달 onHide (X 버튼) (lines 984, 1017, 1040) ──

  it("이름 변경 모달 X 버튼 클릭 시 모달이 닫힌다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
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
      render(<CategoryAdmin contentType="comic" />);
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

  // ── 자식이 있는 실제 부모 폴더의 enrichItem 재귀 (line 388) ──

  it("자식이 있는 부모 카테고리를 트리에 부모-자식 구조로 표시한다", async () => {
    // parent "a"가 자기 자신으로도 존재하고 "a/x" 자식도 가지므로
    // 가상 부모가 아닌 실제 부모 노드가 되고, enrichItem이 children으로 재귀한다.
    const categories = { a: 5, "a/x": 3, b: 2 };
    const mismatchData = { mismatches: [], es_only: [], fs_only: [] };
    setupMockResponses(categories, mismatchData);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });
    // 부모 "a"와 형제 "b"가 표시된다
    expect(screen.getByText("a")).toBeTruthy();
    expect(screen.getByText("b")).toBeTruthy();
    // 부모 "a" 클릭 시 확장되어 자식 "x"가 노출된다 (enrichItem 재귀 결과)
    fireEvent.click(screen.getByText("a"));
    await waitFor(() => {
      expect(screen.getByText("x")).toBeTruthy();
    });
  });

  // ── 자식이 있는 부모 폴더 비노출 토글 시 children 재귀 갱신 (line 692) ──

  it("자식이 있는 부모 카테고리 비노출 토글 시 자식 노드도 재귀적으로 갱신된다", async () => {
    const categories = { a: 5, "a/x": 3 };
    const mismatchData = { mismatches: [], es_only: [], fs_only: [] };
    setupMockResponses(categories, mismatchData, { hiddenResult: [] });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("a")).toBeTruthy();
    });

    // 실제 부모 "a" 선택
    fireEvent.click(screen.getByText("a"));
    await waitFor(() => {
      expect(screen.getByLabelText("사용자 비노출")).toBeTruthy();
    });

    // 토글 → handleToggleHidden이 setFolderData에서 updateHidden을 children에 재귀 적용
    mockJsonPostReq.mockImplementation((url, payload, resolve) => {
      resolve(["a"]);
    });

    fireEvent.click(screen.getByLabelText("사용자 비노출"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/hidden-categories/a?content_type=book",
        { hidden: true },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    // 갱신 후에도 부모/자식 노드가 유지된다 (재귀 갱신이 정상 동작)
    // "a"는 트리와 우측 패널 헤더에 동시에 존재하므로 getAllByText 사용
    await waitFor(() => {
      expect(screen.getAllByText("a").length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText("x")).toBeTruthy();
    });
  });
});

// ── 다크 테마 / 폴백 경로 ──

describe("CategoryAdmin 다크 테마 및 폴백 경로", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  const renderDark = (ui) =>
    render(
      <ThemeProvider theme={createTheme({ palette: { mode: "dark" } })}>
        {ui}
      </ThemeProvider>,
    );

  it("다크 테마에서도 트리를 렌더링한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    renderDark(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
    // 비노출 카테고리(3_history)와 일반 카테고리가 모두 렌더링된다
    expect(screen.getByText("3_history")).toBeTruthy();
  });

  it("불일치 응답에 mismatches/es_only/fs_only 가 없어도 트리를 만든다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, {});
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
  });

  it("매핑/비노출 응답이 null 이어도 기본값으로 처리한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
      mappingsResult: null,
      hiddenResult: null,
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
    // 매핑이 없으므로 키워드 칩이 없다
    fireEvent.click(screen.getByText("1_fiction"));
    expect(screen.queryByText("소설")).toBeNull();
  });

  it("ES 문서 수가 없는 카테고리는 0건으로 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("4_fs_only_cat")).toBeTruthy();
    });
    // 4_fs_only_cat 은 /categories 응답에 없으므로 ?? 0 폴백을 탄다
    fireEvent.click(screen.getByText("4_fs_only_cat"));
    await waitFor(() => {
      expect(screen.getByText("ES 0건")).toBeTruthy();
    });
  });

  it("Enter 이외의 키는 키워드 추가/이름 변경을 트리거하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "무시될키워드" } });
    fireEvent.keyDown(input, { key: "a" });
    expect(mockJsonPostReq).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTitle("이름 변경"));
    const modal = await screen.findByRole("dialog");
    const renameInput = modal.querySelector(
      'input[type="text"]:not([disabled])',
    );
    fireEvent.keyDown(renameInput, { key: "Escape" });
    expect(mockJsonPutReq).not.toHaveBeenCalled();
  });

  it("빈 키워드로 Enter 를 눌러도 API 를 호출하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(mockJsonPostReq).not.toHaveBeenCalled();
  });

  it("빈 이름으로 Enter 를 눌러도 이름 변경 API 를 호출하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이름 변경"));
    const modal = await screen.findByRole("dialog");
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    fireEvent.change(input, { target: { value: "  " } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(mockJsonPutReq).not.toHaveBeenCalled();
  });

  it("매핑이 없던 카테고리에 키워드를 추가하면 배열을 새로 만든다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("3_history")).toBeTruthy();
    });

    // 3_history 는 MAPPINGS_RESPONSE 에 없다
    fireEvent.click(screen.getByText("3_history"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "역사" } });

    mockJsonPostReq.mockImplementation((url, payload, resolve, _r, done) => {
      resolve();
      if (done) done();
    });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(screen.getByText("역사")).toBeTruthy();
    });
  });

  it("비노출 토글 응답이 null 이어도 빈 집합으로 처리한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    mockJsonPostReq.mockImplementation((url, payload, resolve, _r, done) => {
      resolve(null);
      if (done) done();
    });
    fireEvent.click(screen.getByLabelText("사용자 비노출"));

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalled();
    });
  });
});

// ── 에러 객체가 비어 있을 때의 기본 메시지 + 메시지 자동 소멸 ──

describe("CategoryAdmin 기본 에러 메시지 폴백", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
  });

  // 패널을 펼치고 1_fiction 을 선택한 상태까지 진행하는 헬퍼
  const openAndSelect = async (category = "1_fiction") => {
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText(category)).toBeTruthy();
    });
    fireEvent.click(screen.getByText(category));
  };

  it("불일치 상세 조회 에러가 비어 있으면 기본 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/")) reject(null);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("불일치 상세 조회에 실패했습니다.")).toBeTruthy();
    });

    // 5초 뒤 메시지가 자동으로 사라진다
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    await waitFor(() => {
      expect(screen.queryByText("불일치 상세 조회에 실패했습니다.")).toBeNull();
    });
  });

  it("키워드 추가 에러가 비어 있으면 기본 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    mockJsonPostReq.mockImplementation((url, payload, _res, reject, done) => {
      reject(undefined);
      if (done) done();
    });

    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "새키워드" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(
        screen.getByText("이미 등록된 키워드이거나 추가에 실패했습니다."),
      ).toBeTruthy();
    });
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    await waitFor(() => {
      expect(
        screen.queryByText("이미 등록된 키워드이거나 추가에 실패했습니다."),
      ).toBeNull();
    });
  });

  it("키워드 삭제 에러가 비어 있으면 기본 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    mockJsonDeleteReq.mockImplementation((url, payload, _res, reject, done) => {
      reject(null);
      if (done) done();
    });

    // 1_fiction 의 키워드 칩("소설")에 달린 삭제 버튼 클릭
    const chip = screen.getByText("소설").closest(".badge, .chip, span");
    const removeBtn =
      chip?.querySelector("button, svg") ||
      within(chip.parentElement).getAllByRole("button")[0];
    fireEvent.click(removeBtn);

    await waitFor(() => {
      expect(screen.getByText("삭제에 실패했습니다.")).toBeTruthy();
    });
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
  });

  it("비노출 토글 에러가 비어 있으면 기본 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    mockJsonPostReq.mockImplementation((url, payload, _res, reject, done) => {
      reject(null);
      if (done) done();
    });
    fireEvent.click(screen.getByLabelText("사용자 비노출"));

    await waitFor(() => {
      expect(screen.getByText("비노출 설정 변경에 실패했습니다.")).toBeTruthy();
    });
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
  });

  it("이름 변경 에러가 비어 있으면 기본 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    fireEvent.click(screen.getByTitle("이름 변경"));
    const modal = await screen.findByRole("dialog");
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    fireEvent.change(input, { target: { value: "새이름" } });

    mockJsonPutReq.mockImplementation((url, payload, _res, reject, done) => {
      reject(null);
      if (done) done();
    });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(screen.getByText("이름 변경에 실패했습니다.")).toBeTruthy();
    });
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
  });

  it("카테고리 삭제 에러가 비어 있으면 기본 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    fireEvent.click(screen.getByTitle("카테고리 삭제"));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation((url, payload, _res, reject, done) => {
      reject(null);
      if (done) done();
    });
    fireEvent.click(within(modal).getByRole("button", { name: "삭제" }));

    await waitFor(() => {
      expect(screen.getByText("삭제에 실패했습니다.")).toBeTruthy();
    });
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
  });

  it("ES 재적재 에러가 비어 있으면 기본 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    fireEvent.click(screen.getByTitle("ES 재적재"));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation((url, payload, _res, reject, done) => {
      reject(null);
      if (done) done();
    });
    fireEvent.click(within(modal).getByRole("button", { name: "재적재" }));

    await waitFor(() => {
      expect(screen.getByText("ES 재적재에 실패했습니다.")).toBeTruthy();
    });
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
  });
});

// ── placeholder / 성공 메시지 자동 소멸 / 만화 조회 링크 ──

describe("CategoryAdmin 잔여 분기", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  it("placeholder(로딩 중...) 노드 클릭은 무시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    // 카테고리를 펼쳐야 placeholder child 가 렌더된다
    fireEvent.click(screen.getByText("1_fiction"));
    const placeholder = await screen.findByText("로딩 중...");
    fireEvent.click(placeholder);

    // placeholder 는 선택 대상이 아니므로 1_fiction 선택이 유지된다
    expect(screen.getByPlaceholderText("새 키워드 입력")).toBeTruthy();
  });

  it("만화 카테고리의 중복 문서 조회는 comics-view 로 연결된다", async () => {
    const openSpy = vi.fn();
    const originalOpen = window.open;
    window.open = openSpy;

    try {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA, {
        apiPrefix: "/comics",
      });
      render(<CategoryAdmin contentType="comic" />);
      await waitFor(() => {
        expect(screen.getByRole("tree")).toBeTruthy();
      });

      mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
        if (url.startsWith("/comics/category-mismatches/1_fiction")) {
          resolve({
            duplicates: [
              {
                file_path: "1_fiction/dup.pdf",
                file_exists: true,
                docs: [
                  {
                    book_id: 2001,
                    title: "만화중복",
                    author: "A",
                    file_linked: false,
                  },
                ],
              },
            ],
          });
        } else if (url === "/comics/categories") {
          resolve(CATEGORIES_RESPONSE);
        } else if (url === "/comics/category-mismatches") {
          resolve(MISMATCH_RESPONSE_WITH_DATA);
        } else if (url.startsWith("/category-mappings")) {
          resolve(MAPPINGS_RESPONSE);
        } else if (url.startsWith("/hidden-categories")) {
          resolve(HIDDEN_RESPONSE);
        }
      });

      fireEvent.click(screen.getByText("1_fiction"));
      await waitFor(() => {
        expect(screen.getByText(/\[중복\] dup\.pdf/)).toBeTruthy();
      });
      fireEvent.click(screen.getByText(/\[중복\] dup\.pdf/));
      await waitFor(() => {
        expect(screen.getByText("만화중복")).toBeTruthy();
      });

      fireEvent.click(screen.getByRole("button", { name: "조회" }));
      expect(openSpy).toHaveBeenCalledWith(
        expect.stringContaining("/comics-view/2001"),
        "_blank",
        "noopener",
      );
    } finally {
      window.open = originalOpen;
    }
  });

  it("중복 ES 문서 삭제 에러가 비어 있으면 기본 메시지를 표시한다", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
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
                  book_id: 3001,
                  title: "미연결중복",
                  author: "A",
                  file_linked: false,
                },
              ],
            },
          ],
        });
      } else {
        setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA)(
          url,
          _payload,
          resolve,
        );
      }
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText(/\[중복\] dup\.pdf/)).toBeTruthy();
    });
    fireEvent.click(screen.getByText(/\[중복\] dup\.pdf/));
    await waitFor(() => {
      expect(screen.getByText("미연결중복")).toBeTruthy();
    });

    mockJsonDeleteReq.mockImplementation((url, payload, _res, reject) => {
      reject(null);
    });
    fireEvent.click(screen.getByRole("button", { name: "삭제" }));

    await waitFor(() => {
      expect(screen.getByText("삭제 실패")).toBeTruthy();
    });
  });
});

describe("CategoryAdmin 성공 메시지 자동 소멸", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
  });

  const openAndSelect = async () => {
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("1_fiction"));
  };

  it("이름 변경 성공 메시지는 5초 뒤 사라진다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    fireEvent.click(screen.getByTitle("이름 변경"));
    const modal = await screen.findByRole("dialog");
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    fireEvent.change(input, { target: { value: "1_fiction_new" } });

    mockJsonPutReq.mockImplementation((url, payload, resolve, _r, done) => {
      resolve();
      if (done) done();
    });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(mockJsonPutReq).toHaveBeenCalled();
    });
    // 성공 핸들러가 등록한 5초 타이머가 정상적으로 실행된다
    // (loadData() 가 메시지를 즉시 비우므로 화면에는 남지 않는다)
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    expect(screen.queryByText(/변경했습니다/)).toBeNull();
  });

  it("현재 이름과 동일하다는 안내는 3초 뒤 사라진다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    fireEvent.click(screen.getByTitle("이름 변경"));
    const modal = await screen.findByRole("dialog");
    const input = modal.querySelector('input[type="text"]:not([disabled])');
    fireEvent.change(input, { target: { value: "1_fiction" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(screen.getByText("현재 이름과 동일합니다.")).toBeTruthy();
    });
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    await waitFor(() => {
      expect(screen.queryByText("현재 이름과 동일합니다.")).toBeNull();
    });
  });

  it("카테고리 삭제 성공 메시지는 5초 뒤 사라진다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    fireEvent.click(screen.getByTitle("카테고리 삭제"));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation((url, payload, resolve, _r, done) => {
      resolve({ deleted_count: 3 });
      if (done) done();
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
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    expect(screen.queryByText(/삭제되었습니다/)).toBeNull();
  });

  it("ES 재적재 성공 메시지는 5초 뒤 사라진다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    await openAndSelect();

    fireEvent.click(screen.getByTitle("ES 재적재"));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation((url, payload, resolve, _r, done) => {
      resolve({ processed_count: 7 });
      if (done) done();
    });
    fireEvent.click(within(modal).getByRole("button", { name: "재적재" }));

    await waitFor(() => {
      expect(screen.getByText(/7건 처리/)).toBeTruthy();
    });
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    await waitFor(() => {
      expect(screen.queryByText(/7건 처리/)).toBeNull();
    });
  });

  describe("진행 잔여 건수 계산", () => {
    it("재적재 status 형태별 잔여 건수를 계산한다", () => {
      expect(getReloadRemainingCount(null)).toBe(0);
      expect(getReloadRemainingCount(null, 4)).toBe(4);
      expect(
        getReloadRemainingCount({
          status: "running",
          remaining_count: 6,
        }),
      ).toBe(6);
      expect(
        getReloadRemainingCount({
          status: "running",
          before_count: 10,
          indexed_count: 2,
          deleted_count: 3,
        }),
      ).toBe(5);
      expect(
        getReloadRemainingCount({
          status: "running",
          before_count: 4,
        }),
      ).toBe(4);
      expect(getReloadRemainingCount({ status: "running" }, 7)).toBe(7);
      expect(getReloadRemainingCount({ status: "done", after_count: 3 })).toBe(
        3,
      );
      expect(getReloadRemainingCount({ status: "done" }, 2)).toBe(2);
      expect(getReloadRemainingCount({ status: "done" })).toBe(0);
    });

    it("자동 분류 status 형태별 잔여 건수를 계산한다", () => {
      expect(getAutoClassifyRemainingCount(null)).toBeNull();
      expect(
        getAutoClassifyRemainingCount({
          status: "idle",
          remaining_count: 5,
        }),
      ).toBeNull();
      expect(
        getAutoClassifyRemainingCount({
          status: "running",
          total_count: 0,
          processed_count: 0,
          remaining_count: 0,
        }),
      ).toBeNull();
      expect(
        getAutoClassifyRemainingCount({
          status: "running",
          remaining_count: 5,
          total_count: 8,
          processed_count: 3,
        }),
      ).toBe(5);
      expect(
        getAutoClassifyRemainingCount({
          status: "running",
          total_count: 8,
          processed_count: 3,
        }),
      ).toBe(5);
      expect(
        getAutoClassifyRemainingCount({
          status: "running",
          total_count: 8,
        }),
      ).toBe(8);
      expect(
        getAutoClassifyRemainingCount({
          status: "running",
          processed_count: 3,
        }),
      ).toBeNull();
    });
  });

  describe("formatErrorMessage & 에러 객체 핸들링", () => {
    it("다양한 형태의 에러 입력(문자열, Error 인스턴스, 객체, null)을 안전하게 문자열로 변환한다", () => {
      expect(formatErrorMessage(null, "기본 오류")).toBe("기본 오류");
      expect(formatErrorMessage("서버 통신 실패")).toBe("서버 통신 실패");
      expect(formatErrorMessage(new Error("네트워크 연결 오류"))).toBe(
        "네트워크 연결 오류",
      );
      expect(formatErrorMessage({ message: "상세 오류 메시지" })).toBe(
        "상세 오류 메시지",
      );
      expect(formatErrorMessage({ detail: "FastAPI 유효성 검사 실패" })).toBe(
        "FastAPI 유효성 검사 실패",
      );
      expect(formatErrorMessage({ error: "DB 연결 실패" })).toBe(
        "DB 연결 실패",
      );
      expect(formatErrorMessage(12345)).toBe("12345");
    });

    it("API 실패 시 reject 콜백에 Error 객체가 전달되어도 TypeError 없이 alert 박스에 표시된다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
      await openAndSelect();

      mockJsonPostReq.mockImplementation(
        (url, payload, _resolve, reject, done) => {
          reject(new Error("Internal Server Error (500)"));
          if (done) done();
        },
      );

      const input = screen.getByPlaceholderText("새 키워드 입력");
      fireEvent.change(input, { target: { value: "새키워드" } });
      fireEvent.keyDown(input, { key: "Enter" });

      await waitFor(() => {
        const alert = screen.getByText("Internal Server Error (500)");
        expect(alert).toBeTruthy();
        expect(alert.className).toContain("alert-info");
      });
    });

    it("실패/오류 단어가 포함된 Error 객체 메시지는 alert-danger 스타일이 적용된다", async () => {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
      await openAndSelect();

      mockJsonPostReq.mockImplementation(
        (url, payload, _resolve, reject, done) => {
          reject(new Error("키워드 저장 실패"));
          if (done) done();
        },
      );

      const input = screen.getByPlaceholderText("새 키워드 입력");
      fireEvent.change(input, { target: { value: "새키워드" } });
      fireEvent.keyDown(input, { key: "Enter" });

      await waitFor(() => {
        const alert = screen.getByText("키워드 저장 실패");
        expect(alert).toBeTruthy();
        expect(alert.className).toContain("alert-danger");
      });
    });
  });
});

// ── 재적재 진행 중 잔여 이상 항목 건수 폴링 ──

describe("CategoryAdmin 재적재 진행 상태 폴링", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
  });

  it("일괄 재적재 진행 중에는 10초마다 작업 상태를 폴링해 잔여 이상 항목 건수를 표시하고, 완료되면 멈춘다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    const modal = await screen.findByRole("dialog");

    // 서버가 공유하는 재적재 작업 상태를 흉내낸다 — POST는 즉시 시작만 알리고,
    // 실제 진행/완료는 reload-status 폴링 결과로 흘러들어온다.
    let reloadStatus = { status: "idle" };
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve(reloadStatus);
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        reloadStatus = {
          status: "running",
          category: null,
          before_count: 4,
          indexed_count: 0,
          deleted_count: 0,
        };
        resolve({ started: true });
        if (done) done();
      },
    );

    const getCallCountFor = (url) =>
      mockJsonGetReq.mock.calls.filter((call) => call[0] === url).length;

    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    // 즉시 1회 폴링되어 잔여 이상 항목 건수가 표시된다
    await waitFor(() => {
      expect(screen.getByText("잔여 4건")).toBeTruthy();
    });
    const countAfterFirstPoll = getCallCountFor(
      "/category-mismatches/reload-status",
    );

    reloadStatus = {
      status: "running",
      category: null,
      before_count: 4,
      indexed_count: 3,
      deleted_count: 1,
    };
    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    expect(getCallCountFor("/category-mismatches/reload-status")).toBe(
      countAfterFirstPoll + 1,
    );
    await waitFor(() => {
      expect(screen.getByText("잔여 0건")).toBeTruthy();
    });

    // 재적재가 완료되면 폴링이 멈추고 잔여 건수 표시가 사라진다
    reloadStatus = {
      status: "done",
      category: null,
      indexed_count: 3,
      deleted_count: 1,
      after_count: 0,
      failed_count: 0,
    };
    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    await waitFor(() => {
      const bulkButton = screen.getByTitle(
        "불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)",
      );
      expect(bulkButton.querySelector(".spinner-border")).toBeNull();
    });

    const countAfterCompletion = getCallCountFor(
      "/category-mismatches/reload-status",
    );
    await act(async () => {
      vi.advanceTimersByTime(30000);
    });
    expect(getCallCountFor("/category-mismatches/reload-status")).toBe(
      countAfterCompletion,
    );
  });

  it("컴포넌트가 언마운트되면 재적재 상태 폴링을 멈춘다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const { unmount } = render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    const modal = await screen.findByRole("dialog");

    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "running",
          category: null,
          before_count: 19,
          indexed_count: 1,
          deleted_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    mockJsonPostReq.mockImplementation(() => {
      // 응답을 영원히 보류 — 화면 이탈 시점까지 재적재가 진행 중인 상태.
    });

    const getCallCountFor = (url) =>
      mockJsonGetReq.mock.calls.filter((call) => call[0] === url).length;

    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));
    await waitFor(() => {
      expect(screen.getByText("잔여 18건")).toBeTruthy();
    });

    const countAtUnmount = getCallCountFor(
      "/category-mismatches/reload-status",
    );
    unmount();

    await act(async () => {
      vi.advanceTimersByTime(30000);
    });
    expect(getCallCountFor("/category-mismatches/reload-status")).toBe(
      countAtUnmount,
    );
  });
});

describe("CategoryAdmin 재적재 버튼별 스피너 및 실패 처리", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  it("카테고리 미선택 상태에서 이상 항목 재적재를 눌러도 일괄 재적재 버튼은 스피닝하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const bulkButton = within(header).getByRole("button", {
      name: /일괄 재적재/,
    });
    const mismatchButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        // 내부적으로는 reload-all(카테고리 없음)이 진행 중이지만, 사용자는
        // "이상 항목 재적재" 버튼을 눌렀으므로 그 버튼에만 스피너가 켜져야 한다.
        resolve({
          status: "running",
          category: null,
          before_count: 10,
          indexed_count: 1,
          deleted_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    fireEvent.click(mismatchButton);
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 9건")).toBeTruthy();
    });
    expect(bulkButton.querySelector(".spinner-border")).toBeNull();
  });

  it("재적재 시작 응답 전 idle 폴링 응답이 와도 시작 중 표시를 유지한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const bulkButton = within(header).getByRole("button", {
      name: /일괄 재적재/,
    });
    mockJsonPostReq.mockImplementation(() => {
      // 시작 요청이 아직 끝나지 않은 상태를 유지한다.
    });

    fireEvent.click(bulkButton);
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(bulkButton.querySelector(".spinner-border")).not.toBeNull();
      expect(within(bulkButton).getByText("잔여 19건")).toBeTruthy();
    });
  });

  it("이상 항목 재적재 클릭 후 지연된 초기 상태 응답이 와도 일괄 버튼으로 스피너를 옮기지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    let resolveInitialStatus;
    let reloadStatusCalls = 0;
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        reloadStatusCalls += 1;
        if (reloadStatusCalls === 1) {
          resolveInitialStatus = resolve;
          return;
        }
        resolve({
          status: "running",
          category: null,
          before_count: 10,
          indexed_count: 2,
          deleted_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );

    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const bulkButton = within(header).getByRole("button", {
      name: /일괄 재적재/,
    });
    const mismatchButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });

    fireEvent.click(mismatchButton);
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 8건")).toBeTruthy();
    });

    act(() => {
      resolveInitialStatus({
        status: "running",
        category: null,
        before_count: 10,
        indexed_count: 7,
        deleted_count: 0,
      });
    });

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 3건")).toBeTruthy();
    });
    expect(bulkButton.querySelector(".spinner-border")).toBeNull();
  });

  it("재적재 시작 후 지연된 초기 idle 응답이 와도 진행 표시를 지우지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    let resolveInitialStatus;
    let reloadStatusCalls = 0;
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        reloadStatusCalls += 1;
        if (reloadStatusCalls === 1) {
          resolveInitialStatus = resolve;
          return;
        }
        resolve({
          status: "running",
          category: null,
          before_count: 10,
          indexed_count: 2,
          deleted_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );

    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const bulkButton = within(header).getByRole("button", {
      name: /일괄 재적재/,
    });
    const mismatchButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });

    fireEvent.click(mismatchButton);
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 8건")).toBeTruthy();
    });

    act(() => {
      resolveInitialStatus({ status: "idle" });
    });

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 8건")).toBeTruthy();
    });
    expect(bulkButton.querySelector(".spinner-border")).toBeNull();
  });

  it("일괄 재적재 시작 요청 자체가 실패하면 에러 메시지를 표시하고 스피너를 멈춘다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
      reject(null);
    });

    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(
        screen.getByText(/불일치 일괄 ES 재적재 시작에 실패했습니다/),
      ).toBeTruthy();
    });
  });

  it("카테고리별 이상 항목 재적재 시작 요청 자체가 실패하면 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
      reject(null);
    });

    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(
        screen.getByText(/이상 항목 ES 재적재 시작에 실패했습니다/),
      ).toBeTruthy();
    });
  });

  it("재적재 작업이 서버에서 실패로 끝나면 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({ status: "failed", category: null, error: "ES 연결 실패" });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(screen.getByText("ES 연결 실패")).toBeTruthy();
    });
  });

  it("완료 시 실패 건수가 있으면 메시지에 함께 표시하고, 카운트 필드가 없으면 0으로 처리한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "done",
          category: null,
          after_count: 3,
          failed_count: 2,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(
        screen.getByText(/적재 0건, ES 정리 0건, 남은 이상 3건, 실패 2건/),
      ).toBeTruthy();
    });
  });

  it("선택 카테고리 재적재가 진행 중일 때 잔여 이상 항목 건수를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
    const modal = await screen.findByRole("dialog");

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "running",
          category: "1_fiction",
          before_count: 5,
          indexed_count: 2,
          deleted_count: 1,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(screen.getAllByText("잔여 2건").length).toBeGreaterThan(0);
    });
  });

  it("마운트 시 이미 일괄 재적재가 진행 중이면 일괄 재적재 버튼에 자동으로 붙는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "running",
          category: null,
          indexed_count: 4,
          deleted_count: 1,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    render(<CategoryAdmin />);
    // 두 버튼 모두 스피닝 중에는 접근성 이름이 "잔여 N건"으로 바뀌어 role+name으로
    // 못 찾으므로, 상태와 무관하게 고정된 title 속성으로 찾는다.
    await waitFor(() => {
      const bulkButton = screen.getByTitle(
        "불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)",
      );
      expect(within(bulkButton).getByText("잔여 0건")).toBeTruthy();
    });

    const mismatchButton = screen.getByTitle("하위 전체 이상 항목 ES 재적재");
    expect(mismatchButton.querySelector(".spinner-border")).toBeNull();
  });

  it("마운트 시 특정 카테고리 재적재가 진행 중이면 이상 항목 재적재 버튼에 자동으로 붙는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "running",
          category: "1_fiction",
          indexed_count: 1,
          deleted_count: 1,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    render(<CategoryAdmin />);
    await waitFor(() => {
      const mismatchButton = screen.getByTitle("하위 전체 이상 항목 ES 재적재");
      expect(within(mismatchButton).getByText("잔여 0건")).toBeTruthy();
    });

    const bulkButton = screen.getByTitle(
      "불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)",
    );
    expect(bulkButton.querySelector(".spinner-border")).toBeNull();
  });

  it("재적재 상태 조회가 실패해도 화면이 깨지지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        reject("boom");
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /일괄 재적재/ })).toBeTruthy();
    });
  });
});

describe("CategoryAdmin 잔여 커버리지 보강", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  it("최신 자료 제외 목록 조회가 실패해도 빈 집합으로 처리하고 화면이 깨지지 않는다", async () => {
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url === "/category-mismatches/reload-status")
        resolve({ status: "idle" });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories")) reject("boom");
    });

    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("최신 자료 검색 제외").checked).toBe(false);
    });
  });

  it("최신 자료 검색 제외 설정 변경이 실패하면 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
      latestExcludedResult: [],
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("최신 자료 검색 제외")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, reject, final) => {
        reject(null);
        if (final) final();
      },
    );

    fireEvent.click(screen.getByLabelText("최신 자료 검색 제외"));
    await waitFor(() => {
      expect(
        screen.getByText(/최신 자료 검색 제외 설정 변경에 실패했습니다/),
      ).toBeTruthy();
    });
  });

  it("이상 항목 재적재 확인 모달을 X 버튼과 취소 버튼으로 각각 닫을 수 있다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /이상 항목 재적재/ }));
    let modal = await screen.findByRole("dialog");
    fireEvent.click(modal.querySelector(".btn-close"));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    fireEvent.click(screen.getByRole("button", { name: /이상 항목 재적재/ }));
    modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("불일치 일괄 재적재 확인 모달을 X 버튼과 취소 버튼으로 각각 닫을 수 있다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    let modal = await screen.findByRole("dialog");
    fireEvent.click(modal.querySelector(".btn-close"));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("최신 자료 제외 목록 응답이 null 이어도 빈 집합으로 처리한다", async () => {
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url === "/category-mismatches/reload-status")
        resolve({ status: "idle" });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories")) resolve(null);
    });

    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("최신 자료 검색 제외").checked).toBe(false);
    });
  });

  it("최신 자료 검색 제외 설정 변경 응답이 null 이어도 빈 집합으로 처리한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
      latestExcludedResult: [],
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("최신 자료 검색 제외")).toBeTruthy();
    });

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve(null);
        if (done) done();
      },
    );

    fireEvent.click(screen.getByLabelText("최신 자료 검색 제외"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalled();
    });
    expect(screen.getByLabelText("최신 자료 검색 제외").checked).toBe(false);
  });

  it("마운트 시 진행 중인 작업의 카운트 필드가 없으면 잔여 0건으로 처리한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({ status: "running", category: null });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    render(<CategoryAdmin />);
    await waitFor(() => {
      const bulkButton = screen.getByTitle(
        "불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)",
      );
      expect(within(bulkButton).getByText("잔여 0건")).toBeTruthy();
    });
  });

  it("마운트 직후 언마운트되면 재적재 상태 조회 결과를 무시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    let resolveStatus;
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolveStatus = resolve;
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    const { unmount } = render(<CategoryAdmin />);
    unmount();

    // 언마운트 이후 응답이 도착해도 setState가 호출되지 않고 조용히 무시된다.
    expect(() =>
      resolveStatus({ status: "running", category: null, indexed_count: 1 }),
    ).not.toThrow();
  });

  it("최신 자료 검색 제외 실패 메시지는 3초 뒤 사라진다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
        latestExcludedResult: [],
      });
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("1_fiction")).toBeTruthy();
      });

      fireEvent.click(screen.getByText("1_fiction"));
      await waitFor(() => {
        expect(screen.getByLabelText("최신 자료 검색 제외")).toBeTruthy();
      });

      mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
        reject(null);
      });
      fireEvent.click(screen.getByLabelText("최신 자료 검색 제외"));

      await waitFor(() => {
        expect(
          screen.getByText(/최신 자료 검색 제외 설정 변경에 실패했습니다/),
        ).toBeTruthy();
      });

      await act(async () => {
        vi.advanceTimersByTime(3000);
      });
      expect(
        screen.queryByText(/최신 자료 검색 제외 설정 변경에 실패했습니다/),
      ).toBeNull();
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("카테고리별 재적재 시작 실패 메시지는 5초 뒤 사라진다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("디렉토리")).toBeTruthy();
      });

      mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
        reject(null);
      });

      fireEvent.click(screen.getByRole("button", { name: /이상 항목 재적재/ }));
      const modal = await screen.findByRole("dialog");
      fireEvent.click(
        within(modal).getByRole("button", { name: "이상 항목 재적재" }),
      );
      await waitFor(() => {
        expect(
          screen.getByText(/이상 항목 ES 재적재 시작에 실패했습니다/),
        ).toBeTruthy();
      });
      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
      expect(
        screen.queryByText(/이상 항목 ES 재적재 시작에 실패했습니다/),
      ).toBeNull();
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("일괄 재적재 시작 실패 메시지는 5초 뒤 사라진다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("디렉토리")).toBeTruthy();
      });

      mockJsonPostReq.mockImplementation((url, payload, resolve, reject) => {
        reject(null);
      });

      fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
      const modal = await screen.findByRole("dialog");
      fireEvent.click(
        within(modal).getByRole("button", { name: "일괄 재적재" }),
      );
      await waitFor(() => {
        expect(
          screen.getByText(/불일치 일괄 ES 재적재 시작에 실패했습니다/),
        ).toBeTruthy();
      });
      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
      expect(
        screen.queryByText(/불일치 일괄 ES 재적재 시작에 실패했습니다/),
      ).toBeNull();
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });
});
