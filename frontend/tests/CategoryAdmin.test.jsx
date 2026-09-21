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

afterEach(() => {
  cleanup();
  window.sessionStorage.clear();
});

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

import {
  formatErrorMessage,
  getReloadRemainingCount,
} from "../src/categoryAdminUtils";
import CategoryAdminBase from "../src/CategoryAdmin";

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

  it("디렉토리 헤더 컨트롤을 레이블, 토글, 재적재 버튼 순서의 형제로 배치한다", async () => {
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

    expect(label.parentElement).toBe(header);
    expect(toggle.parentElement).toBe(header);
    expect(bulkReloadButton.parentElement).toBe(header);
    expect(mismatchReloadButton.parentElement).toBe(header);
    expect(within(header).getByText("이상 항목만")).toBeTruthy();
    expect(bulkReloadButton.textContent).toContain("일괄");
    expect(bulkReloadButton.textContent).not.toContain("재적재");
    expect(mismatchReloadButton.textContent).toContain("이상 항목");
    expect(mismatchReloadButton.textContent).not.toContain("재적재");
    expect(within(header).queryByTitle("자동 분류")).toBeNull();
    expect(Array.from(header.children)).toEqual([
      label,
      toggle,
      bulkReloadButton,
      mismatchReloadButton,
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

    expect(bulkReloadButton.textContent).toContain("일괄");
    expect(bulkReloadButton.textContent).not.toContain("잔여");
    expect(mismatchReloadButton.textContent).toContain("이상 항목");
    expect(mismatchReloadButton.textContent).not.toContain("잔여");
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

  it("버튼 그룹이 키워드 영역보다 아래에 온다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => expect(screen.getByText("1_fiction")).toBeTruthy());
    fireEvent.click(screen.getByText("1_fiction"));

    const keywordInput = await screen.findByPlaceholderText("새 키워드 입력");
    const renameButton = screen.getByRole("button", { name: /이름 변경/ });

    // 버튼이 키워드 영역보다 문서 뒤쪽에 있어야 한다
    expect(
      keywordInput.compareDocumentPosition(renameButton) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("건수 차이(diff)가 아니라 실제 이상 항목 수(anomaly_count)를 집계한다", async () => {
    // ES 10건과 FS 10건이라 차이는 0이지만, 경로가 서로 다른 항목이 4건 있다.
    const mismatchData = {
      mismatches: [
        {
          category: "1_fiction",
          es_count: 10,
          fs_count: 10,
          diff: 0,
          anomaly_count: 4,
        },
      ],
      es_only: [],
      fs_only: [],
    };
    setupMockResponses(CATEGORIES_RESPONSE, mismatchData);
    render(<CategoryAdminBase />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    fireEvent.click(
      within(header).getByRole("button", { name: /이상 항목 재적재/ }),
    );

    const modal = await screen.findByRole("dialog");
    expect(
      within(modal).getByText(/전체 이상 항목 4건을 ES에 재적재합니다/),
    ).toBeTruthy();
  });

  it("자기 이상 항목이 없는 부모 디렉토리는 상세를 조회하지 않는다", async () => {
    const categories = { parent: 10, "parent/child": 3 };
    const mismatchData = {
      mismatches: [],
      es_only: [{ category: "parent/child", es_count: 3, anomaly_count: 3 }],
      fs_only: [],
    };
    setupMockResponses(categories, mismatchData);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("parent")).toBeTruthy();
    });

    mockJsonGetReq.mockClear();
    fireEvent.click(screen.getByText("parent"));

    const detailCalls = mockJsonGetReq.mock.calls.filter(([url]) =>
      url.startsWith("/category-mismatches/parent"),
    );
    expect(detailCalls).toEqual([]);
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

  it("이상 항목만 보기 기본 상태에서 미선택이어도 이상 항목 재적재 모달에서 전체 이상 항목을 재적재한다", async () => {
    const largeMismatchData = {
      mismatches: [
        {
          category: "1_fiction",
          es_count: 50000,
          fs_count: 0,
          diff: 50000,
        },
      ],
      es_only: [{ category: "2_science", es_count: 8 }],
      fs_only: [{ category: "4_fs_only_cat", fs_count: 9 }],
    };
    setupMockResponses(CATEGORIES_RESPONSE, largeMismatchData);
    render(<CategoryAdminBase />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    expect(within(header).getByLabelText("이상 항목만 보기").checked).toBe(
      true,
    );

    const mismatchReloadButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });
    expect(mismatchReloadButton.disabled).toBe(false);
    fireEvent.click(mismatchReloadButton);

    const modal = await screen.findByRole("dialog");
    expect(within(modal).queryByText("불일치 일괄 재적재")).toBeNull();
    expect(
      within(modal).getAllByText("이상 항목 재적재").length,
    ).toBeGreaterThan(0);
    expect(
      within(modal).getByText(/전체 이상 항목 50017건을 ES에 재적재합니다/),
    ).toBeTruthy();
    expect(within(modal).getByText("작업 대상: 50017건")).toBeTruthy();

    const confirmButton = within(modal).getByRole("button", {
      name: "이상 항목 재적재",
    });
    expect(confirmButton.disabled).toBe(false);

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );
    fireEvent.click(confirmButton);

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/reload-all",
        { reload_source: "mismatch" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("디렉토리 헤더에서 선택 카테고리의 이상 항목만 재적재한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    expect(
      within(header).getByRole("button", {
        name: /이상 항목 재적재/,
      }).disabled,
    ).toBe(false);

    fireEvent.click(screen.getByText("1_fiction"));
    const mismatchReloadButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });
    expect(mismatchReloadButton.disabled).toBe(false);
    fireEvent.click(mismatchReloadButton);

    const modal = await screen.findByRole("dialog");
    expect(within(modal).getByText(/2건만 ES에 재적재합니다/)).toBeTruthy();
    expect(within(modal).getByText("작업 대상: 2건")).toBeTruthy();

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

  it("이상 항목 선택 상태에서도 해당 카테고리의 이상 항목 재적재를 실행한다", async () => {
    const mismatchData = {
      mismatches: [
        { category: "1_fiction", es_count: 12345, fs_count: 0, diff: 12345 },
      ],
      es_only: [],
      fs_only: [],
    };
    setupMockResponses(CATEGORIES_RESPONSE, mismatchData);
    render(<CategoryAdminBase />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(mismatchData);
      else if (
        url.startsWith("/category-mismatches/1_fiction") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
        resolve({
          es_only: [
            {
              book_id: 101,
              title: "Missing File",
              file_type: "pdf",
              file_path: "1_fiction/missing.pdf",
            },
          ],
          fs_only: [],
          duplicates: [],
        });
      } else if (url.startsWith("/category-mismatches/reload-status")) {
        resolve({ status: "idle" });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByText("Missing File.pdf")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("Missing File.pdf"));

    const header = screen.getByText("디렉토리").closest(".card-header");
    const mismatchReloadButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });
    expect(mismatchReloadButton.disabled).toBe(false);
    fireEvent.click(mismatchReloadButton);

    const modal = await screen.findByRole("dialog");
    // 상세를 펼친 뒤에는 요약 건수(12345)가 아니라 실제로 받아온 항목 수를 대상으로 삼는다.
    expect(within(modal).getByText(/1건만 ES에 재적재합니다/)).toBeTruthy();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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

  it("파일 삭제는 확인 모달에서 취소하면 API를 호출하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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

    mockJsonPostReq.mockClear();
    fireEvent.click(await screen.findByRole("button", { name: "파일 삭제" }));

    const modal = await screen.findByRole("dialog");
    expect(within(modal).getByText(/되돌릴 수 없습니다/)).toBeTruthy();
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));

    expect(mockJsonPostReq).not.toHaveBeenCalled();
    // 트리에 항목이 그대로 남아 있다 (모달 본문에도 같은 경로가 찍히므로 getAll로 본다).
    expect(screen.getAllByText("orphan.txt").length).toBeGreaterThan(0);
  });

  it("파일 삭제 모달은 닫기(X)로도 닫힌다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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

    mockJsonPostReq.mockClear();
    fireEvent.click(await screen.findByRole("button", { name: "파일 삭제" }));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "Close" }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    expect(mockJsonPostReq).not.toHaveBeenCalled();
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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

    fireEvent.click(screen.getByRole("button", { name: "파일 삭제" }));
    const deleteFileModal = await screen.findByRole("dialog");
    fireEvent.click(
      within(deleteFileModal).getByRole("button", { name: "삭제" }),
    );
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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

    fireEvent.click(screen.getByRole("button", { name: "파일 삭제" }));
    const deleteFileModal = await screen.findByRole("dialog");
    fireEvent.click(
      within(deleteFileModal).getByRole("button", { name: "삭제" }),
    );
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
        else if (
          url.startsWith("/comics/category-mismatches/") &&
          !url.startsWith("/comics/category-mismatches/reload-status")
        ) {
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
        else if (
          url.startsWith("/comics/category-mismatches/") &&
          !url.startsWith("/comics/category-mismatches/reload-status")
        ) {
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

    fireEvent.click(screen.getAllByText("2_science")[0]);
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

  it("_root 선택 시 이름 변경/삭제 버튼을 막는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("_root")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("_root"));

    const renameBtn = screen.getByTitle(
      "최상위 디렉토리는 이름을 변경할 수 없습니다",
    );
    const deleteBtn = screen.getByTitle("최상위 디렉토리는 삭제할 수 없습니다");
    expect(renameBtn.disabled).toBe(true);
    expect(deleteBtn.disabled).toBe(true);
  });

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

  it("분류 제안 버튼 클릭 시 선택 카테고리로 분류 제안을 시작한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({
          started: true,
          status: "running",
          source_category: "1_fiction",
          total_count: 0,
          processed_count: 0,
        });
        if (final) final();
      },
    );

    fireEvent.click(screen.getByTitle("분류 제안"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/categories/classify-proposal",
        { category: "1_fiction" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("분류 제안 시작 응답이 already_running이면 진행 상태를 즉시 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({
          already_running: true,
          status: "running",
          source_category: "1_fiction",
          total_count: 4,
          processed_count: 1,
          items: [],
        });
        if (final) final();
      },
    );

    fireEvent.click(screen.getByTitle("분류 제안"));

    await waitFor(() => {
      expect(screen.getByText("분류 제안: 1_fiction")).toBeTruthy();
      expect(screen.getByText("1 / 4")).toBeTruthy();
    });
    // 진행 중에는 분류 제안 버튼도 다시 누를 수 없다.
    expect(screen.getByTitle("분류 제안").disabled).toBe(true);
  });

  it("분류 제안 시작 요청이 실패하면 버튼을 원래 상태로 돌리고 에러 메시지를 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    mockJsonPostReq.mockImplementation(
      (url, payload, _resolve, reject, final) => {
        reject(null);
        if (final) final();
      },
    );

    fireEvent.click(screen.getByTitle("분류 제안"));

    await waitFor(() => {
      const proposalButton = screen.getByTitle("분류 제안");
      expect(proposalButton.textContent).toContain("분류 제안");
      expect(proposalButton.querySelector(".spinner-border")).toBeNull();
      expect(screen.getByText("분류 제안에 실패했습니다.")).toBeTruthy();
    });
  });

  it("선택 카테고리 버튼 그룹에는 이상 항목 재적재 다음으로 분류 제안 버튼을 표시한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const actionGroup = screen.getByTitle("이름 변경").closest(".d-flex");

    expect(within(actionGroup).getByTitle("분류 제안")).toBeTruthy();
    expect(
      Array.from(actionGroup.querySelectorAll("button")).map(
        (button) => button.title,
      ),
    ).toEqual([
      "이름 변경",
      "카테고리 삭제",
      "ES 재적재",
      "이상 항목만 ES 재적재",
      "분류 제안",
    ]);
  });

  it("만화 카테고리 분류 제안은 /comics prefix를 사용한다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY, {
      apiPrefix: "/comics",
    });
    render(<CategoryAdmin contentType="comic" />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({ started: true, status: "running" });
        if (final) final();
      },
    );

    fireEvent.click(screen.getByTitle("분류 제안"));
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/comics/categories/classify-proposal",
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

  it("일괄 재적재 실행 시 reload-all API를 호출하고 완료 배너 없이 끝낸다", async () => {
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
        { reload_source: "bulk" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /일괄 재적재/ }).textContent,
      ).toContain("일괄");
    });
    expect(screen.queryByText(/ES 재적재 완료/)).toBeNull();
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

  it("한글 조합 중의 Enter는 키워드를 등록하지 않는다", async () => {
    // 한글 입력에서 Enter는 먼저 조합을 확정한다. 그걸 등록으로 받으면 확정 Enter와
    // 전송 Enter가 각각 요청을 만들어 같은 키워드가 두 번 나간다. 두 번째는 중복이라
    // 예전에는 화면에 실패 창이 떴다 — 등록은 되어 있는데도.
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "중학생" } });
    mockJsonPostReq.mockImplementation((url, payload, resolve) => resolve([]));

    // 조합을 확정하는 Enter
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(mockJsonPostReq).not.toHaveBeenCalled();

    // 조합이 끝난 뒤의 Enter만 등록한다
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledTimes(1);
    });
  });

  it("이미 등록된 키워드를 다시 넣어도 목록이 서버 기준으로 유지된다", async () => {
    // 서버가 돌려준 목록을 그대로 쓴다. 직접 append 하면 같은 배지가 두 번 보인다.
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("1_fiction"));
    const input = screen.getByPlaceholderText("새 키워드 입력");
    fireEvent.change(input, { target: { value: "중학생" } });
    mockJsonPostReq.mockImplementation((url, payload, resolve) =>
      resolve({ result: ["중학생"], warning: "이미 등록된 키워드입니다." }),
    );

    fireEvent.click(screen.getByText("추가"));

    await waitFor(() => {
      expect(screen.getByText("이미 등록된 키워드입니다.")).toBeTruthy();
    });
    expect(screen.getAllByText("중학생")).toHaveLength(1);
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
      } else if (url.startsWith("/category-mismatches/reload-status")) {
        resolve({ status: "idle" });
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      )
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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

  it("이미 등록된 키워드는 추가 요청을 보내지 않는다", async () => {
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
    expect(mockJsonPostReq).not.toHaveBeenCalled();
    expect(screen.queryByText("이미 등록된 키워드입니다.")).toBeNull();
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

  it("현재 이름과 동일한 이름으로 변경하면 요청을 보내지 않는다", async () => {
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

    expect(mockJsonPutReq).not.toHaveBeenCalled();
    expect(screen.queryByText("현재 이름과 동일합니다.")).toBeNull();
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

  it("ES 재적재 성공 시 완료 배너를 띄우지 않는다", async () => {
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
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/reload",
        { category: "1_fiction" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    expect(screen.queryByText(/재적재 완료/)).toBeNull();
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
      if (url === "/category-mismatches/reload-status?category=1_fiction") {
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
    });
    expect(screen.queryByText(/ES 재적재 완료/)).toBeNull();
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      ) {
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
        else if (
          url.startsWith("/comics/category-mismatches/") &&
          !url.startsWith("/comics/category-mismatches/reload-status")
        ) {
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
      else if (
        url.startsWith("/category-mismatches/") &&
        !url.startsWith("/category-mismatches/reload-status")
      )
        reject(null);
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
      } else if (url.startsWith("/category-mismatches/reload-status")) {
        resolve({ status: "idle" });
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
        expect(alert.className).toContain("alert-danger");
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

// ── 분류 제안 검토 · 승인 ──

describe("CategoryAdmin 분류 제안", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  const PROPOSAL_ITEM_CERTAIN = {
    file_path: "1_fiction/a.epub",
    title: "확실한 책",
    current_category: "1_fiction",
    target_category: "2_science",
    grade: "certain",
    confidence: 0.9,
    candidates: [
      { category: "2_science", source: "model", detail: "모델 판정" },
    ],
    apply_status: "pending",
    apply_error: null,
  };
  const PROPOSAL_ITEM_UNSURE = {
    file_path: "1_fiction/b.epub",
    title: "애매한 책",
    current_category: "1_fiction",
    target_category: "3_history",
    grade: "unsure",
    confidence: 0.3,
    candidates: [
      { category: "3_history", source: "bookstore", detail: "서점 1곳 일치" },
    ],
    apply_status: "pending",
    apply_error: null,
  };
  const PROPOSAL_ITEM_MOVED = {
    file_path: "1_fiction/c.epub",
    title: "이미 옮긴 책",
    current_category: "1_fiction",
    target_category: "2_science",
    grade: "certain",
    confidence: 0.95,
    candidates: [],
    apply_status: "moved",
    apply_error: null,
  };

  // resultRef.current를 매 GET 호출 시점에 다시 읽어, 테스트 중간에 다음 폴링
  // 응답을 바꿔치기할 수 있게 한다.
  function mockClassifyProposalGet(resultRef) {
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url === "/categories/classify-proposal")
        resolve(resultRef.current);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });
  }

  const PROPOSAL_ITEM_WITH_ID = {
    ...PROPOSAL_ITEM_CERTAIN,
    file_path: "1_fiction/d.epub",
    title: "지울 책",
    book_id: 909,
  };

  async function renderReadyProposalWithDeletableItem() {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 2,
        processed_count: 2,
        items: [PROPOSAL_ITEM_CERTAIN, PROPOSAL_ITEM_WITH_ID],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("지울 책")).toBeTruthy();
    });
  }

  it("삭제를 취소하면 아무 요청도 나가지 않는다", async () => {
    await renderReadyProposalWithDeletableItem();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    try {
      mockJsonDeleteReq.mockClear();
      fireEvent.click(screen.getByLabelText("지울 책 삭제"));
      // 되돌릴 수 없는 작업이라 확인 전에는 파일을 건드리면 안 된다.
      expect(mockJsonDeleteReq.mock.calls.map((c) => c[0])).toEqual([]);
      expect(screen.getByText("지울 책")).toBeTruthy();
    } finally {
      window.confirm.mockRestore();
    }
  });

  it("삭제를 확인하면 책과 제안 행을 지우고 표에서 없앤다", async () => {
    await renderReadyProposalWithDeletableItem();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    try {
      mockJsonDeleteReq.mockClear();
      mockJsonDeleteReq.mockImplementation((url, _payload, resolve) => resolve({}));
      fireEvent.click(screen.getByLabelText("지울 책 삭제"));

      await waitFor(() => {
        expect(screen.queryByText("지울 책")).toBeNull();
      });
      const urls = mockJsonDeleteReq.mock.calls.map(([url]) => url);
      // 파일·ES 문서를 지우는 일과 제안 행을 없애는 일은 다른 자원이라 두 번 부른다.
      expect(urls[0]).toBe("/books/909");
      expect(urls[1]).toBe(
        "/categories/classify-proposal/item?file_path=1_fiction%2Fd.epub",
      );
      // 다른 행은 그대로 남는다.
      expect(screen.getByText("확실한 책")).toBeTruthy();
    } finally {
      window.confirm.mockRestore();
    }
  });

  it("책 삭제가 실패하면 행을 남기고 제안 행도 건드리지 않는다", async () => {
    await renderReadyProposalWithDeletableItem();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    try {
      mockJsonDeleteReq.mockClear();
      mockJsonDeleteReq.mockImplementation((url, _payload, _resolve, reject) =>
        reject("서버 오류"),
      );
      fireEvent.click(screen.getByLabelText("지울 책 삭제"));

      await waitFor(() => {
        expect(screen.getByText("서버 오류")).toBeTruthy();
      });
      // 파일이 남았는데 행만 사라지면 표에서 다시 손댈 길이 없어진다.
      expect(screen.getByText("지울 책")).toBeTruthy();
      // 제안 행 삭제까지 가면 안 된다 — 파일이 남았는데 행만 사라진다.
      expect(mockJsonDeleteReq.mock.calls).toHaveLength(1);
    } finally {
      window.confirm.mockRestore();
    }
  });

  it("책은 지웠는데 제안 행 삭제가 실패하면 표에서는 치우고 알린다", async () => {
    // 행을 남기면 다음 조회에서 없는 파일을 가리키는 행으로 되살아난다.
    await renderReadyProposalWithDeletableItem();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    try {
      mockJsonDeleteReq.mockClear();
      mockJsonDeleteReq.mockImplementation((url, _payload, resolve, reject) => {
        if (url.startsWith("/books/")) resolve({});
        else reject("행 삭제 실패");
      });
      fireEvent.click(screen.getByLabelText("지울 책 삭제"));

      await waitFor(() => {
        expect(screen.getByText("행 삭제 실패")).toBeTruthy();
      });
      expect(screen.queryByText("지울 책")).toBeNull();
      expect(screen.getByText("확실한 책")).toBeTruthy();
    } finally {
      window.confirm.mockRestore();
    }
  });

  it("삭제가 도는 동안 같은 버튼을 다시 눌러도 요청이 겹치지 않는다", async () => {
    await renderReadyProposalWithDeletableItem();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    try {
      mockJsonDeleteReq.mockClear();
      // 응답을 주지 않아 삭제가 도는 상태로 묶어 둔다.
      mockJsonDeleteReq.mockImplementation(() => {});
      const button = screen.getByLabelText("지울 책 삭제");
      fireEvent.click(button);
      fireEvent.click(button);

      expect(mockJsonDeleteReq.mock.calls).toHaveLength(1);
    } finally {
      window.confirm.mockRestore();
    }
  });

  it("제목이 없는 행은 파일 경로로 확인 문구를 만든다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [{ ...PROPOSAL_ITEM_WITH_ID, title: "" }],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByLabelText("1_fiction/d.epub 삭제")).toBeTruthy();
    });

    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    try {
      fireEvent.click(screen.getByLabelText("1_fiction/d.epub 삭제"));
      expect(confirmSpy.mock.calls[0][0]).toContain("1_fiction/d.epub");
    } finally {
      window.confirm.mockRestore();
    }
  });

  it("상태가 ready면 표가 보이고 certain 항목이 기본 선택된다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 3,
        processed_count: 3,
        items: [
          PROPOSAL_ITEM_CERTAIN,
          PROPOSAL_ITEM_UNSURE,
          PROPOSAL_ITEM_MOVED,
        ],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(
        screen.getByLabelText("1_fiction/a.epub 추천 1 선택").checked,
      ).toBe(true);
    });
    expect(screen.getByLabelText("1_fiction/b.epub 추천 1 선택").checked).toBe(
      false,
    );
    // 이미 옮긴 행은 목적지를 다시 고를 수 없다.
    expect(screen.getByLabelText("1_fiction/c.epub 목적지").disabled).toBe(
      true,
    );
  });

  it("추천 1이 현재 디렉토리와 같으면 certain이라도 기본 선택하지 않는다", async () => {
    // 제자리 이동이라 옮길 것이 없다. 자동으로 승인 목록에 들어가면 실제로
    // 옮겨야 할 행이 몇 개인지 숫자로 안 보인다.
    const sameCategoryItem = {
      ...PROPOSAL_ITEM_CERTAIN,
      file_path: "1_fiction/same.epub",
      title: "제자리 책",
      target_category: "1_fiction",
      candidates: [
        { category: "1_fiction", source: "model", detail: "모델 판정" },
      ],
    };
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 2,
        processed_count: 2,
        items: [PROPOSAL_ITEM_CERTAIN, sameCategoryItem],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(
        screen.getByLabelText("1_fiction/a.epub 추천 1 선택").checked,
      ).toBe(true);
    });
    expect(
      screen.getByLabelText("1_fiction/same.epub 추천 1 선택").checked,
    ).toBe(false);
  });

  it("상태가 failed면 에러 메시지를 보여준다 (I2)", async () => {
    const resultRef = {
      current: {
        status: "failed",
        source_category: "1_fiction",
        total_count: 1200,
        processed_count: 34,
        error: "분류 제안에 실패했습니다.",
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    // 배너 텍스트가 <strong>과 형제 텍스트 노드로 나뉘어 있어 getByText의 기본
    // 정확 일치로는 못 찾는다 — 정규식으로 부분 일치를 확인한다.
    await waitFor(() => {
      expect(screen.getByText(/분류 제안에 실패했습니다/)).toBeTruthy();
    });
    // 34/1200처럼 처리량이 전체에 못 미치면, 이 제안이 끝나지 않았다는 사실도
    // 명시해야 한다 — 그러지 않으면 관리자가 미완성 제안을 완성됐다고 착각한다.
    expect(screen.getByText(/1200건 중 34건까지만/)).toBeTruthy();
    // failed에서도 승인은 계속 켜져 있어야 한다 — C1/I4가 재시도할 수 있어야 한다.
    expect(screen.getByRole("button", { name: /분류 승인/ }).disabled).toBe(
      false,
    );
  });

  it("failed여도 전체를 다 처리했으면 미완성 문구를 보여주지 않는다", async () => {
    const resultRef = {
      current: {
        status: "failed",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        error:
          "백엔드가 다시 시작되어 진행 중이던 작업이 중단되었습니다. 남은 항목을 확인한 뒤 진행하세요.",
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(
        screen.getByText(/진행 중이던 작업이 중단되었습니다/),
      ).toBeTruthy();
    });
    expect(screen.queryByText(/까지만 처리됐습니다/)).toBeNull();
  });

  it("상태가 ready면 에러 배너를 보여주지 않는다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(screen.getByText("확실한 책")).toBeTruthy();
    });
    expect(screen.queryByText("작업이 중단됐습니다.")).toBeNull();
  });

  it("이미 검토한 제안이 있으면 분류 제안 버튼은 확인 모달을 거친 뒤에만 다시 시작한다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(screen.getByTitle("분류 제안")).toBeTruthy();
    });

    fireEvent.click(screen.getByTitle("분류 제안"));
    // 모달을 거치지 않고는 재요청이 나가면 안 된다 — 검토 결과를 잃기 전에
    // 반드시 확인을 받아야 한다.
    expect(mockJsonPostReq).not.toHaveBeenCalled();

    const modal = await screen.findByRole("dialog");
    expect(
      within(modal).getByText("분류 제안 다시 시작", {
        selector: ".modal-title",
      }),
    ).toBeTruthy();

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({
          started: true,
          status: "running",
          source_category: "1_fiction",
          total_count: 0,
          processed_count: 0,
        });
        if (final) final();
      },
    );

    fireEvent.click(within(modal).getByRole("button", { name: "다시 시작" }));

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/categories/classify-proposal",
        { category: "1_fiction" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    // 모달은 react-bootstrap fade 전환이 끝나야 DOM 에서 빠진다. POST 가 나간
    // 직후에 동기로 보면 아직 남아 있어 간헐적으로 실패한다.
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("기존 제안이 없으면 분류 제안 버튼은 확인 모달 없이 바로 시작한다", async () => {
    const resultRef = { current: { status: "idle" } };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("1_fiction"));

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({
          started: true,
          status: "running",
          source_category: "1_fiction",
          total_count: 0,
          processed_count: 0,
        });
        if (final) final();
      },
    );

    fireEvent.click(screen.getByTitle("분류 제안"));

    // 잃을 게 없으니 모달 없이 곧바로 요청이 나간다.
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/categories/classify-proposal",
        { category: "1_fiction" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("상태가 running이면 지금까지의 항목이 표에 보이고 승인 버튼이 비활성이다", async () => {
    const resultRef = {
      current: {
        status: "running",
        source_category: "1_fiction",
        total_count: 5,
        processed_count: 1,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(screen.getByText("확실한 책")).toBeTruthy();
    });
    expect(screen.getByRole("button", { name: /분류 승인/ }).disabled).toBe(
      true,
    );
  });

  it("이상 항목 스캔이 안 끝나도 디렉토리를 먼저 그리고 제안 표도 함께 보여준다", async () => {
    // 이상 항목 스캔만 몇 초 걸린다. 그 몇 초 동안 왼쪽 디렉토리가 아예 없고 제안
    // 표만 보였다. 이제 디렉토리를 먼저 그리고, 스캔 결과는 나중에 건수로 얹는다.
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      // 불일치 스캔은 응답하지 않는다 — 그래도 디렉토리는 나와야 한다.
      else if (url === "/category-mismatches") return;
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url === "/categories/classify-proposal")
        resolve({
          status: "running",
          source_category: "1_fiction",
          total_count: 5,
          processed_count: 2,
          items: [PROPOSAL_ITEM_CERTAIN],
        });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });
    render(<CategoryAdmin />);

    // 왼쪽 디렉토리가 스캔을 기다리지 않고 나온다.
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });
    // 제안 표의 목적지 select에도 같은 이름이 있으므로 트리 안에서만 확인한다.
    expect(
      within(screen.getByRole("tree")).getByText("3_history"),
    ).toBeTruthy();
    // 스캔이 도는 동안은 이상 항목 집계 중임을 헤더에서 알린다.
    expect(screen.getByLabelText("이상 항목 집계 중")).toBeTruthy();
    // 복원된 선택 카테고리의 제안 표와 진행 수도 같이 보인다.
    expect(screen.getByText("확실한 책")).toBeTruthy();
    expect(screen.getByText("분류 제안: 1_fiction")).toBeTruthy();
    expect(screen.getByText(/2 \/ 5/)).toBeTruthy();
  });

  it("이상 항목 스캔이 오면 건수와 이상 항목만 필터를 적용한다", async () => {
    // 스캔 전에는 건수가 없어 필터를 걸 수 없다. 그대로 걸러내면 빈 트리가 되므로
    // 그때까지는 전체를 보여주고, 스캔이 오면 이상 항목만 남긴다.
    let resolveMismatches = null;
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolveMismatches = resolve;
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url === "/categories/classify-proposal")
        resolve({ status: "idle" });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });
    render(<CategoryAdminBase />);

    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });
    // 토글은 켜져 있지만 건수가 없어 아직 아무것도 숨기지 않는다.
    expect(screen.getByLabelText("이상 항목만 보기").checked).toBe(true);
    expect(screen.getByText("3_history")).toBeTruthy();

    await act(async () => {
      resolveMismatches(MISMATCH_RESPONSE_WITH_DATA);
    });

    await waitFor(() => {
      expect(screen.queryByText("3_history")).toBeNull();
    });
    expect(screen.getByText("1_fiction")).toBeTruthy();
    expect(screen.getByText("4_fs_only_cat")).toBeTruthy();
    expect(screen.queryByLabelText("이상 항목 집계 중")).toBeNull();
  });

  it("다른 디렉토리를 선택하면 그 제안 표는 보이지 않는다", async () => {
    // 제안은 한 번에 하나만 있고 만들어진 카테고리에 속한다. 다른 디렉토리로 옮겼는데
    // 표가 남아 있으면 지금 보는 디렉토리의 책이 그렇게 분류된 것으로 읽히고,
    // 그대로 승인하면 엉뚱한 책이 옮겨진다.
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(screen.getByText("확실한 책")).toBeTruthy();
    });

    fireEvent.click(screen.getAllByText("2_science")[0]);

    await waitFor(() => {
      expect(screen.queryByText("확실한 책")).toBeNull();
    });
    expect(screen.queryByRole("button", { name: /분류 승인/ })).toBeNull();
  });

  it("다시 그 디렉토리로 돌아오면 제안 표가 다시 보인다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(screen.getByText("확실한 책")).toBeTruthy();
    });
    fireEvent.click(screen.getAllByText("2_science")[0]);
    await waitFor(() => {
      expect(screen.queryByText("확실한 책")).toBeNull();
    });

    fireEvent.click(screen.getAllByText("1_fiction")[0]);

    await waitFor(() => {
      expect(screen.getByText("확실한 책")).toBeTruthy();
    });
  });

  it("분류가 도는 동안 분류 제안 버튼은 계속 돌고 진행 수를 보여준다", async () => {
    // 시작 요청이 끝나면 스피너를 내리던 예전 동작은, 정작 오래 걸리는 분류 자체가
    // 도는 동안 버튼이 멈춘 것처럼 보이게 했다. 작업이 끝날 때까지 돌아야 한다.
    const resultRef = {
      current: {
        status: "running",
        source_category: "1_fiction",
        total_count: 5,
        processed_count: 2,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    const button = await screen.findByTitle("분류 제안");
    await waitFor(() => {
      expect(button.querySelector(".spinner-border")).toBeTruthy();
    });
    expect(button.textContent).toContain("2/5");
    expect(button.disabled).toBe(true);
  });

  it("분류가 끝나면 분류 제안 버튼의 스피너가 멈춘다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 5,
        processed_count: 5,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    const button = await screen.findByTitle("분류 제안");
    await waitFor(() => {
      expect(screen.getByText("확실한 책")).toBeTruthy();
    });
    expect(button.querySelector(".spinner-border")).toBeNull();
  });

  it("승인 요청 payload에 선택된 행만, 사용자가 고친 목적지로 담긴다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 2,
        processed_count: 2,
        items: [PROPOSAL_ITEM_CERTAIN, PROPOSAL_ITEM_UNSURE],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(
        screen.getByLabelText("1_fiction/a.epub 추천 1 선택").checked,
      ).toBe(true);
    });

    // 애매한 책은 기본 선택되지 않으므로 직접 선택으로 목적지를 준다.
    fireEvent.change(screen.getByLabelText("1_fiction/b.epub 목적지"), {
      target: { value: "2_science" },
    });

    // 확실한 책은 목적지를 사용자가 다른 곳으로 고친다 — 추천 체크는 꺼진다.
    fireEvent.change(screen.getByLabelText("1_fiction/a.epub 목적지"), {
      target: { value: "3_history" },
    });
    expect(screen.getByLabelText("1_fiction/a.epub 추천 1 선택").checked).toBe(
      false,
    );

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, final) => {
        resolve({ started: true, total_count: 2 });
        if (final) final();
      },
    );

    fireEvent.click(screen.getByRole("button", { name: /분류 승인/ }));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "승인" }));

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/categories/classify-proposal/apply",
        {
          items: [
            { file_path: "1_fiction/a.epub", target_category: "3_history" },
            { file_path: "1_fiction/b.epub", target_category: "2_science" },
          ],
        },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  it("목적지 없는 항목은 선택돼 있어도 승인 payload에 안 들어간다 (2차 방어)", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(
        screen.getByLabelText("1_fiction/a.epub 추천 1 선택").checked,
      ).toBe(true);
    });

    // 추천 체크를 풀면 그 행에는 목적지가 없다. 승인 버튼은 대상이 0건이 되어
    // 비활성화된다 — CategoryAdmin의 proposalApplyItems가 isSelectable로 거른
    // 결과와 화면이 항상 일치해야 한다.
    fireEvent.click(screen.getByLabelText("1_fiction/a.epub 추천 1 선택"));

    const approveButton = screen.getByRole("button", { name: /분류 승인/ });
    expect(approveButton.textContent).toContain("분류 승인 (0건)");
    expect(approveButton.disabled).toBe(true);
  });

  it("완료 기록 삭제는 moved 행이 없으면 비활성이다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [PROPOSAL_ITEM_UNSURE],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(screen.getByText("애매한 책")).toBeTruthy();
    });
    expect(
      screen.getByRole("button", { name: "완료 기록 삭제" }).disabled,
    ).toBe(true);
  });

  it("완료 기록 삭제는 moved 행이 있으면 활성이고 DELETE를 부른다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 2,
        processed_count: 2,
        items: [PROPOSAL_ITEM_UNSURE, PROPOSAL_ITEM_MOVED],
      },
    };
    mockClassifyProposalGet(resultRef);
    mockJsonDeleteReq.mockImplementation(
      (url, _payload, resolve, _reject, final) => {
        resolve({ deleted_count: 1 });
        if (final) final();
      },
    );
    render(<CategoryAdmin />);

    await waitFor(() => {
      expect(screen.getByText("이미 옮긴 책")).toBeTruthy();
    });

    const clearButton = screen.getByRole("button", { name: "완료 기록 삭제" });
    expect(clearButton.disabled).toBe(false);
    fireEvent.click(clearButton);
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "삭제" }));

    await waitFor(() => {
      expect(mockJsonDeleteReq).toHaveBeenCalledWith(
        "/categories/classify-proposal/applied",
        null,
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
  });

  async function renderReadyProposal(items) {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: items.length,
        processed_count: items.length,
        items,
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /분류 승인/ })).toBeTruthy();
    });
  }

  it("분류 승인 모달은 취소와 닫기로 모두 닫힌다", async () => {
    await renderReadyProposal([PROPOSAL_ITEM_CERTAIN]);

    fireEvent.click(screen.getByRole("button", { name: /분류 승인/ }));
    let modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    expect(mockJsonPostReq).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /분류 승인/ }));
    modal = await screen.findByRole("dialog");
    // 헤더의 닫기(X)는 onHide 로 들어온다.
    fireEvent.click(within(modal).getByRole("button", { name: "Close" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    expect(mockJsonPostReq).not.toHaveBeenCalled();
  });

  it("분류 승인이 실패하면 메시지를 보여준다", async () => {
    await renderReadyProposal([PROPOSAL_ITEM_CERTAIN]);

    mockJsonPostReq.mockImplementation(
      (url, payload, _resolve, reject, final) => {
        reject("승인 서버 오류");
        if (final) final();
      },
    );
    fireEvent.click(screen.getByRole("button", { name: /분류 승인/ }));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "승인" }));

    await waitFor(() => {
      expect(screen.getByText("승인 서버 오류")).toBeTruthy();
    });
  });

  it("완료 기록 삭제 모달은 취소와 닫기로 모두 닫힌다", async () => {
    await renderReadyProposal([PROPOSAL_ITEM_UNSURE, PROPOSAL_ITEM_MOVED]);
    const clearButton = screen.getByRole("button", { name: "완료 기록 삭제" });

    fireEvent.click(clearButton);
    let modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    fireEvent.click(clearButton);
    modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "Close" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    expect(mockJsonDeleteReq).not.toHaveBeenCalled();
  });

  it("완료 기록 삭제가 실패하면 메시지를 보여준다", async () => {
    await renderReadyProposal([PROPOSAL_ITEM_UNSURE, PROPOSAL_ITEM_MOVED]);

    mockJsonDeleteReq.mockImplementation(
      (url, _payload, _resolve, reject, final) => {
        reject("기록 삭제 오류");
        if (final) final();
      },
    );
    fireEvent.click(screen.getByRole("button", { name: "완료 기록 삭제" }));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "삭제" }));

    await waitFor(() => {
      expect(screen.getByText("기록 삭제 오류")).toBeTruthy();
    });
  });

  it("완료 기록은 지웠는데 표를 다시 못 불러오면 그 사실을 알린다", async () => {
    // DELETE 는 성공했다. 재조회만 실패하면 표가 그대로라 삭제 여부를 알 길이 없다.
    await renderReadyProposal([PROPOSAL_ITEM_UNSURE, PROPOSAL_ITEM_MOVED]);

    mockJsonDeleteReq.mockImplementation(
      (url, _payload, resolve, _reject, final) => {
        resolve({ deleted_count: 1 });
        if (final) final();
      },
    );
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url === "/categories/classify-proposal") reject("재조회 오류");
      else if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });

    fireEvent.click(screen.getByRole("button", { name: "완료 기록 삭제" }));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "삭제" }));

    await waitFor(() => {
      expect(screen.getByText("재조회 오류")).toBeTruthy();
    });
  });

  it("분류 제안 다시 시작 모달은 닫기(X)로도 닫힌다", async () => {
    await renderReadyProposal([PROPOSAL_ITEM_CERTAIN]);

    fireEvent.click(screen.getByTitle("분류 제안"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "Close" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    expect(mockJsonPostReq).not.toHaveBeenCalled();
  });

  it("분류 제안 다시 시작 모달은 취소로도 닫힌다", async () => {
    await renderReadyProposal([PROPOSAL_ITEM_CERTAIN]);

    fireEvent.click(screen.getByTitle("분류 제안"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "취소" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    expect(mockJsonPostReq).not.toHaveBeenCalled();
  });

  it("마운트 시 상태가 ready면 source_category가 선택된다", async () => {
    const resultRef = {
      current: {
        status: "ready",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [PROPOSAL_ITEM_CERTAIN],
      },
    };
    mockClassifyProposalGet(resultRef);
    render(<CategoryAdmin />);

    // 카테고리 상세 카드(분류 제안 버튼 포함)는 selectedCategory가 있어야
    // 보인다 — source_category로 복원됐는지는 이 카드의 노출로 확인한다.
    await waitFor(() => {
      expect(screen.getByTitle("분류 제안")).toBeTruthy();
    });
    expect(screen.queryByText("왼쪽에서 디렉토리를 선택하세요.")).toBeNull();
  });

  it("분류가 도는 동안 새로 분류된 책이 새로고침 없이 표에 나타난다", async () => {
    // 표는 3초 간격 폴링으로 갱신된다. 페이지를 다시 열거나 새로고침해야 보인다면
    // 진행 상황을 지켜보라고 만든 화면이 제 역할을 못 한다.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const resultRef = {
        current: {
          status: "running",
          source_category: "1_fiction",
          total_count: 2,
          processed_count: 1,
          items: [PROPOSAL_ITEM_CERTAIN],
        },
      };
      mockClassifyProposalGet(resultRef);
      render(<CategoryAdmin />);

      await waitFor(() => {
        expect(screen.getByText("확실한 책")).toBeTruthy();
      });
      expect(screen.queryByText("애매한 책")).toBeNull();

      // 다음 폴링에서 한 권이 더 분류돼 돌아온다.
      resultRef.current = {
        ...resultRef.current,
        processed_count: 2,
        items: [PROPOSAL_ITEM_CERTAIN, PROPOSAL_ITEM_UNSURE],
      };
      await act(async () => {
        vi.advanceTimersByTime(3000);
      });

      await waitFor(() => {
        expect(screen.getByText("애매한 책")).toBeTruthy();
      });
      expect(screen.getByTitle("분류 제안").textContent).toContain("2/2");
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("상태가 applying이면 폴링하고, 종료 상태가 되면 멈춘다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const resultRef = {
        current: {
          status: "applying",
          source_category: "1_fiction",
          total_count: 1,
          processed_count: 1,
          items: [{ ...PROPOSAL_ITEM_CERTAIN, apply_status: "pending" }],
        },
      };
      mockClassifyProposalGet(resultRef);
      render(<CategoryAdmin />);

      await waitFor(() => {
        expect(screen.getByText("확실한 책")).toBeTruthy();
      });
      const getCallsAtApplying = mockJsonGetReq.mock.calls.filter(
        ([url]) => url === "/categories/classify-proposal",
      ).length;

      // 3초 뒤 다음 폴링에서 이동 완료로 응답한다 — 종료 상태이므로 이 뒤로는
      // 더 이상 폴링하지 않아야 한다.
      resultRef.current = {
        ...resultRef.current,
        status: "done",
        items: [{ ...PROPOSAL_ITEM_CERTAIN, apply_status: "moved" }],
      };
      await act(async () => {
        vi.advanceTimersByTime(3000);
      });
      await waitFor(() => {
        expect(screen.getByText("이동 완료")).toBeTruthy();
      });

      const getCallsAtDone = mockJsonGetReq.mock.calls.filter(
        ([url]) => url === "/categories/classify-proposal",
      ).length;
      expect(getCallsAtDone).toBeGreaterThan(getCallsAtApplying);

      const callsRightAfterDone = getCallsAtDone;
      await act(async () => {
        vi.advanceTimersByTime(6000);
      });
      const getCallsLater = mockJsonGetReq.mock.calls.filter(
        ([url]) => url === "/categories/classify-proposal",
      ).length;
      expect(getCallsLater).toBe(callsRightAfterDone);
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
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

    // 서버가 공유하는 재적재 작업 상태를 흉내낸다. POST는 즉시 시작만 알리고,
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

    // 미선택 상태의 전체 작업은 "일괄" 버튼만 소유한다.
    const bulkButton = within(
      screen.getByText("디렉토리").closest(".card-header"),
    ).getByRole("button", { name: /일괄 재적재/ });

    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    // 즉시 1회 폴링되어 잔여 이상 항목 건수가 표시된다
    await waitFor(() => {
      expect(within(bulkButton).getByText("잔여 4건")).toBeTruthy();
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
      expect(within(bulkButton).getByText("잔여 0건")).toBeTruthy();
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
      // 응답을 영원히 보류한다. 화면 이탈 시점까지 재적재가 진행 중인 상태.
    });

    const getCallCountFor = (url) =>
      mockJsonGetReq.mock.calls.filter((call) => call[0] === url).length;

    const bulkButton = within(
      screen.getByText("디렉토리").closest(".card-header"),
    ).getByRole("button", { name: /일괄 재적재/ });

    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));
    await waitFor(() => {
      expect(within(bulkButton).getByText("잔여 18건")).toBeTruthy();
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

  it("미선택 이상 항목 재적재를 누르면 일괄 버튼이 아니라 이상 항목 버튼만 스피닝한다", async () => {
    const largeMismatchData = {
      mismatches: [
        {
          category: "1_fiction",
          es_count: 50000,
          fs_count: 0,
          diff: 50000,
        },
      ],
      es_only: [{ category: "2_science", es_count: 8 }],
      fs_only: [{ category: "4_fs_only_cat", fs_count: 9 }],
    };
    setupMockResponses(CATEGORIES_RESPONSE, largeMismatchData);

    let allReloadStarted = false;
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve(
          allReloadStarted
            ? {
                status: "running",
                category: null,
                before_count: 50017,
                indexed_count: 17,
                deleted_count: 0,
              }
            : { status: "idle" },
        );
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    render(<CategoryAdminBase />);
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

    fireEvent.click(mismatchButton);
    const modal = await screen.findByRole("dialog");
    allReloadStarted = true;
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 50000건")).toBeTruthy();
    });
    expect(bulkButton.querySelector(".spinner-border")).toBeNull();
    expect(bulkButton.textContent).toContain("일괄");
    expect(bulkButton.textContent).not.toContain("잔여");
    expect(mockJsonPostReq).toHaveBeenCalledWith(
      "/category-mismatches/reload-all",
      { reload_source: "mismatch" },
      expect.any(Function),
      expect.any(Function),
      expect.any(Function),
    );
  });

  it("미선택 이상 항목 재적재 진행 중 새로 마운트해도 일괄 버튼에 진행 상태를 붙이지 않는다", async () => {
    const largeMismatchData = {
      mismatches: [
        {
          category: "1_fiction",
          es_count: 50000,
          fs_count: 0,
          diff: 50000,
        },
      ],
      es_only: [{ category: "2_science", es_count: 8 }],
      fs_only: [{ category: "4_fs_only_cat", fs_count: 9 }],
    };
    setupMockResponses(CATEGORIES_RESPONSE, largeMismatchData);

    let allReloadStarted = false;
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve(
          allReloadStarted
            ? {
                status: "running",
                category: null,
                before_count: 50017,
                indexed_count: 17,
                deleted_count: 0,
              }
            : { status: "idle" },
        );
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

    const { unmount } = render(<CategoryAdminBase />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const header = screen.getByText("디렉토리").closest(".card-header");
    const mismatchButton = within(header).getByRole("button", {
      name: /이상 항목 재적재/,
    });

    fireEvent.click(mismatchButton);
    const modal = await screen.findByRole("dialog");
    allReloadStarted = true;
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(mockJsonPostReq).toHaveBeenCalledWith(
        "/category-mismatches/reload-all",
        { reload_source: "mismatch" },
        expect.any(Function),
        expect.any(Function),
        expect.any(Function),
      );
    });
    unmount();

    render(<CategoryAdminBase />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    const remountedBulkButton = screen.getByTitle(
      "불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)",
    );
    const remountedMismatchButton =
      screen.getByTitle("전체 이상 항목 ES 재적재");

    await waitFor(() => {
      expect(
        within(remountedMismatchButton).getByText("잔여 50000건"),
      ).toBeTruthy();
    });
    expect(remountedBulkButton.querySelector(".spinner-border")).toBeNull();
    expect(remountedBulkButton.textContent).toContain("일괄");
    expect(remountedBulkButton.textContent).not.toContain("잔여");
  });

  it("backend가 이상 항목 source로 보고한 전체 재적재는 session 없이도 이상 항목 버튼에 붙는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    window.sessionStorage.clear();

    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status") {
        resolve({
          status: "running",
          category: null,
          reload_source: "mismatch",
          before_count: 50017,
          indexed_count: 17,
          deleted_count: 0,
        });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    render(<CategoryAdminBase />);

    const bulkButton = await screen.findByTitle(
      "불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)",
    );
    const mismatchButton = screen.getByTitle("전체 이상 항목 ES 재적재");

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 50000건")).toBeTruthy();
    });
    expect(bulkButton.querySelector(".spinner-border")).toBeNull();
    expect(bulkButton.textContent).toContain("일괄");
    expect(bulkButton.textContent).not.toContain("잔여");
  });

  it("카테고리별 이상 항목 재적재를 누르면 일괄 버튼은 스피닝하지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const header = screen.getByText("디렉토리").closest(".card-header");
    const bulkButton = within(header).getByRole("button", {
      name: /일괄 재적재/,
    });
    const mismatchButton = screen.getByTitle("이상 항목만 ES 재적재");

    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        resolve({ started: true });
        if (done) done();
      },
    );
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status?category=1_fiction") {
        resolve({
          status: "running",
          category: "1_fiction",
          before_count: 10,
          indexed_count: 1,
          deleted_count: 0,
        });
        return;
      }
      if (url === "/category-mismatches/reload-status") {
        resolve({ status: "idle" });
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
    expect(mockJsonPostReq).toHaveBeenCalledWith(
      "/category-mismatches/reload-mismatches",
      { category: "1_fiction" },
      expect.any(Function),
      expect.any(Function),
      expect.any(Function),
    );
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

  it("재적재 시작 전 마운트 시점 폴링 응답이 시작 후에 뒤늦게 도착해도 새 폴링 결과를 덮어쓰지 않는다", async () => {
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

    fireEvent.click(bulkButton);
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(within(bulkButton).getByText("잔여 8건")).toBeTruthy();
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
      expect(within(bulkButton).getByText("잔여 8건")).toBeTruthy();
    });
    expect(mismatchButton.querySelector(".spinner-border")).toBeNull();
  });

  it("선택 카테고리 이상 항목 재적재 시작 후 지연된 초기 idle 응답이 와도 진행 표시를 지우지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    let resolveInitialCategoryStatus;
    let categoryReloadStatusCalls = 0;
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status?category=1_fiction") {
        categoryReloadStatusCalls += 1;
        if (categoryReloadStatusCalls === 1) {
          resolveInitialCategoryStatus = resolve;
          return;
        }
        resolve({
          status: "running",
          category: "1_fiction",
          before_count: 10,
          indexed_count: 2,
          deleted_count: 0,
        });
        return;
      }
      if (url === "/category-mismatches/reload-status") {
        resolve({ status: "idle" });
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
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    const mismatchButton = screen.getByTitle("이상 항목만 ES 재적재");

    fireEvent.click(mismatchButton);
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 8건")).toBeTruthy();
    });

    act(() => {
      resolveInitialCategoryStatus({ status: "idle" });
    });

    await waitFor(() => {
      expect(within(mismatchButton).getByText("잔여 8건")).toBeTruthy();
    });
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

  it("카운트 필드가 없는 완료 응답에도 배너 없이 진행 표시를 끈다", async () => {
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
        screen.getByRole("button", { name: /일괄 재적재/ }).textContent,
      ).toContain("일괄");
    });
    expect(screen.queryByText(/ES 재적재 완료/)).toBeNull();
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

  it("마운트 시 이미 일괄 재적재가 진행 중이면 일괄 버튼에만 자동으로 붙는다", async () => {
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
    // 스피닝 중에는 접근성 이름이 "잔여 N건"으로 바뀌어 role+name으로
    // 못 찾으므로, 상태와 무관하게 고정된 title 속성으로 찾는다.
    await waitFor(() => {
      const bulkButton = screen.getByTitle(
        "불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)",
      );
      expect(within(bulkButton).getByText("잔여 0건")).toBeTruthy();
    });

    const mismatchButton = screen.getByTitle("전체 이상 항목 ES 재적재");
    expect(mismatchButton.querySelector(".spinner-border")).toBeNull();
    expect(mismatchButton.disabled).toBe(true);
  });

  it("카테고리 선택 후 그 카테고리 재적재가 이미 진행 중이면 이상 항목 버튼에 자동으로 붙고, 일괄 버튼은 영향받지 않는다", async () => {
    setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_WITH_DATA);
    const originalGetImpl = mockJsonGetReq.getMockImplementation();
    mockJsonGetReq.mockImplementation((url, payload, resolve, reject) => {
      if (url === "/category-mismatches/reload-status?category=1_fiction") {
        resolve({
          status: "running",
          category: "1_fiction",
          indexed_count: 1,
          deleted_count: 1,
        });
        return;
      }
      if (url === "/category-mismatches/reload-status") {
        resolve({ status: "idle" });
        return;
      }
      originalGetImpl(url, payload, resolve, reject);
    });

    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("1_fiction"));

    await waitFor(() => {
      const mismatchButton = screen.getByTitle(
        "선택 디렉토리 이상 항목만 ES 재적재",
      );
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
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
    let modal = await screen.findByRole("dialog");
    fireEvent.click(modal.querySelector(".btn-close"));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
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

      fireEvent.click(screen.getByText("1_fiction"));
      fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
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

describe("CategoryAdmin 재적재 충돌 및 폴링 방어 처리", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  // 재적재 상태 폴링 응답을 보류시킨다. POST 응답이 만든 상태가 곧바로 도착한
  // idle 폴링 응답에 덮이면 검증하려는 분기의 결과를 관찰할 수 없다.
  function setupWithHeldReloadStatus({
    mismatchResult = MISMATCH_RESPONSE_WITH_DATA,
    detailResult = null,
  } = {}) {
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(mismatchResult);
      else if (url.startsWith("/category-mismatches/reload-status")) return;
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
      else if (detailResult && url.startsWith("/category-mismatches/"))
        resolve(detailResult);
    });
  }

  // 확인 모달이 사라지는 동안에는 모달 버튼과 헤더 버튼의 접근성 이름이 겹치므로
  // 헤더 안에서만 버튼을 찾는다.
  function headerButton(name) {
    const header = screen.getByText("디렉토리").closest(".card-header");
    return within(header).getByRole("button", { name });
  }

  function resolvePost(result) {
    mockJsonPostReq.mockImplementation(
      (_url, _payload, resolve, _reject, done) => {
        resolve(result);
        if (done) done();
      },
    );
  }

  // ── 일괄/전체 재적재 시작 응답 처리 ──

  it("일괄 재적재 시작이 다른 카테고리 작업에 막히면 스피너를 끄고 차단 메시지를 표시한다", async () => {
    setupWithHeldReloadStatus();
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    resolvePost({ already_running: true, category: "2_science" });
    fireEvent.click(headerButton("일괄 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(
        screen.getByText(
          /카테고리 '2_science' 재적재가 이미 진행 중이라 지금은 일괄 재적재를 실행할 수 없습니다/,
        ),
      ).toBeTruthy();
    });
    expect(
      headerButton("일괄 재적재").querySelector(".spinner-border"),
    ).toBeNull();
    expect(
      window.sessionStorage.getItem("CategoryAdmin.allReloadOwner.book"),
    ).toBeNull();
  });

  it("일괄 재적재 시작 응답이 카테고리 없는 already_running이면 진행 상태를 그대로 반영한다", async () => {
    setupWithHeldReloadStatus();
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    resolvePost({
      already_running: true,
      status: "running",
      remaining_count: 3,
    });
    fireEvent.click(headerButton("일괄 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(
        within(headerButton("일괄 재적재")).getByText("잔여 3건"),
      ).toBeTruthy();
    });
    expect(
      window.sessionStorage.getItem("CategoryAdmin.allReloadOwner.book"),
    ).toBe("bulk");
  });

  it("일괄 재적재 시작 응답의 reload_source가 이상 항목이면 이상 항목 스피너로 전환한다", async () => {
    setupWithHeldReloadStatus();
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    resolvePost({ reload_source: "mismatch" });
    fireEvent.click(headerButton("일괄 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(
        within(headerButton("이상 항목 재적재")).getByText("잔여 19건"),
      ).toBeTruthy();
    });
    expect(
      headerButton("일괄 재적재").querySelector(".spinner-border"),
    ).toBeNull();
    expect(
      window.sessionStorage.getItem("CategoryAdmin.allReloadOwner.book"),
    ).toBe("mismatch");
  });

  it("전체 이상 항목 재적재 시작 응답의 reload_source가 일괄이면 일괄 스피너로 전환한다", async () => {
    setupWithHeldReloadStatus();
    render(<CategoryAdminBase />);
    await waitFor(() => {
      expect(screen.getByText("디렉토리")).toBeTruthy();
    });

    resolvePost({ reload_source: "bulk" });
    fireEvent.click(headerButton("이상 항목 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(
        within(headerButton("일괄 재적재")).getByText("잔여 19건"),
      ).toBeTruthy();
    });
    expect(
      headerButton("이상 항목 재적재").querySelector(".spinner-border"),
    ).toBeNull();
  });

  // ── 카테고리별 이상 항목 재적재 시작 응답 처리 ──

  it("카테고리 이상 항목 재적재가 다른 카테고리 작업에 막히면 차단 메시지를 표시한다", async () => {
    setupWithHeldReloadStatus();
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    resolvePost({ already_running: true, category: "2_science" });
    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(
        screen.getByText(
          /카테고리 '2_science' 재적재가 이미 진행 중이라 지금은 실행할 수 없습니다/,
        ),
      ).toBeTruthy();
    });
    expect(
      headerButton("이상 항목 재적재").querySelector(".spinner-border"),
    ).toBeNull();
  });

  it("카테고리 이상 항목 재적재가 일괄 작업에 막히면 일괄 진행 중 메시지를 표시한다", async () => {
    setupWithHeldReloadStatus();
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    resolvePost({ already_running: true });
    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(
        screen.getByText(
          /일괄 재적재가 이미 진행 중이라 지금은 실행할 수 없습니다/,
        ),
      ).toBeTruthy();
    });
  });

  it("같은 카테고리의 already_running 응답은 폴링과 같은 경로로 상태를 반영한다", async () => {
    setupWithHeldReloadStatus();
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    resolvePost({
      already_running: true,
      category: "1_fiction",
      status: "done",
      indexed_count: 3,
      deleted_count: 1,
      after_count: 0,
      failed_count: 0,
    });
    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );

    await waitFor(() => {
      expect(screen.getByTitle("이상 항목만 ES 재적재").textContent).toContain(
        "이상 항목 재적재",
      );
    });
    expect(screen.queryByText(/ES 재적재 완료/)).toBeNull();
  });

  // ── 폴링 실패 및 stale 응답 ──

  it("카테고리별 재적재 상태 폴링이 실패해도 선택 화면을 유지한다", async () => {
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/reload-status?category="))
        reject("status error");
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByTitle("이상 항목만 ES 재적재")).toBeTruthy();
    });
  });

  it("분류 제안 폴링 요청이 실패해도 진행 표시를 유지한다", async () => {
    let failPoll = false;
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url === "/categories/classify-proposal") {
        if (failPoll) reject("poll error");
        else resolve({ status: "idle" });
      } else if (url.startsWith("/category-mappings"))
        resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    failPoll = true;
    resolvePost({
      started: true,
      status: "running",
      source_category: "1_fiction",
      total_count: 1,
      processed_count: 0,
      items: [],
    });
    fireEvent.click(screen.getByTitle("분류 제안"));

    // 폴링(GET)이 실패해도 진행 중 표시(버튼 비활성)는 그대로 유지돼야 한다.
    await waitFor(() => {
      expect(screen.getByTitle("분류 제안").disabled).toBe(true);
    });
  });

  it("언마운트 뒤 도착한 분류 제안 폴링 응답은 무시한다", async () => {
    const heldResolvers = [];
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches") resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url === "/categories/classify-proposal")
        heldResolvers.push(resolve);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });
    const { unmount } = render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    resolvePost({
      started: true,
      status: "running",
      source_category: "1_fiction",
      total_count: 1,
      processed_count: 0,
      items: [],
    });
    fireEvent.click(screen.getByTitle("분류 제안"));

    await waitFor(() => {
      expect(heldResolvers.length).toBeGreaterThan(1);
    });

    const lastResolve = heldResolvers[heldResolvers.length - 1];
    unmount();
    expect(() =>
      lastResolve({
        status: "running",
        source_category: "1_fiction",
        total_count: 1,
        processed_count: 1,
        items: [],
      }),
    ).not.toThrow();
  });

  it("콘텐츠 타입이 바뀌면 이전 최신 자료 제외 응답을 무시한다", async () => {
    const latestExcludedCalls = [];
    mockJsonGetReq.mockImplementation((url, _payload, resolve, reject) => {
      if (url.endsWith("/categories")) resolve(CATEGORIES_RESPONSE);
      else if (url.endsWith("/category-mismatches"))
        resolve(MISMATCH_RESPONSE_EMPTY);
      else if (url.includes("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        latestExcludedCalls.push({ resolve, reject });
    });

    const { rerender } = render(<CategoryAdmin contentType="book" />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    rerender(<CategoryAdmin contentType="comic" />);
    await waitFor(() => {
      expect(latestExcludedCalls.length).toBe(2);
    });

    // 첫 요청(=이전 contentType)의 응답은 늦게 도착하더라도 반영되지 않는다.
    await act(async () => {
      latestExcludedCalls[0].resolve(["1_fiction"]);
      latestExcludedCalls[0].reject("stale error");
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByLabelText("최신 자료 검색 제외").checked).toBe(false);
    });
  });

  // ── 재적재 소유자 세션 저장 ──

  it("sessionStorage 접근이 차단되어도 재적재 소유자 없이 렌더링한다", async () => {
    const getItemSpy = vi
      .spyOn(Storage.prototype, "getItem")
      .mockImplementation(() => {
        throw new Error("sessionStorage blocked");
      });
    try {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("1_fiction")).toBeTruthy();
      });
      expect(getItemSpy).toHaveBeenCalled();
    } finally {
      getItemSpy.mockRestore();
    }
  });

  it("일괄 재적재 차단 메시지는 5초 뒤 사라진다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      setupWithHeldReloadStatus();
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("디렉토리")).toBeTruthy();
      });

      resolvePost({ already_running: true, category: "2_science" });
      fireEvent.click(headerButton("일괄 재적재"));
      const modal = await screen.findByRole("dialog");
      fireEvent.click(
        within(modal).getByRole("button", { name: "일괄 재적재" }),
      );

      const blockedMessage =
        /카테고리 '2_science' 재적재가 이미 진행 중이라 지금은 일괄 재적재를 실행할 수 없습니다/;
      await waitFor(() => {
        expect(screen.getByText(blockedMessage)).toBeTruthy();
      });
      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
      expect(screen.queryByText(blockedMessage)).toBeNull();
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("카테고리 이상 항목 재적재 차단 메시지는 5초 뒤 사라진다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      setupWithHeldReloadStatus();
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("1_fiction")).toBeTruthy();
      });

      fireEvent.click(screen.getByText("1_fiction"));
      resolvePost({ already_running: true, category: "2_science" });
      fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
      const modal = await screen.findByRole("dialog");
      fireEvent.click(
        within(modal).getByRole("button", { name: "이상 항목 재적재" }),
      );

      const blockedMessage =
        /카테고리 '2_science' 재적재가 이미 진행 중이라 지금은 실행할 수 없습니다/;
      await waitFor(() => {
        expect(screen.getByText(blockedMessage)).toBeTruthy();
      });
      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
      expect(screen.queryByText(blockedMessage)).toBeNull();
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("sessionStorage 저장이 차단되어도 일괄 재적재를 시작한다", async () => {
    const setItemSpy = vi
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => {
        throw new Error("sessionStorage blocked");
      });
    try {
      setupWithHeldReloadStatus();
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("디렉토리")).toBeTruthy();
      });

      resolvePost({ started: true });
      fireEvent.click(headerButton("일괄 재적재"));
      const modal = await screen.findByRole("dialog");
      fireEvent.click(
        within(modal).getByRole("button", { name: "일괄 재적재" }),
      );

      await waitFor(() => {
        expect(
          headerButton("일괄 재적재").querySelector(".spinner-border"),
        ).toBeTruthy();
      });
      expect(setItemSpy).toHaveBeenCalled();
    } finally {
      setItemSpy.mockRestore();
    }
  });

  it("contentType이 비어 있으면 기본 book 세션 키로 재적재 소유자를 읽는다", async () => {
    const getItemSpy = vi.spyOn(Storage.prototype, "getItem");
    try {
      setupMockResponses(CATEGORIES_RESPONSE, MISMATCH_RESPONSE_EMPTY);
      render(
        <CategoryAdminBase contentType="" initialShowOnlyAbnormal={false} />,
      );
      await waitFor(() => {
        expect(screen.getByText("1_fiction")).toBeTruthy();
      });
      expect(getItemSpy).toHaveBeenCalledWith(
        "CategoryAdmin.allReloadOwner.book",
      );
    } finally {
      getItemSpy.mockRestore();
    }
  });
});

describe("CategoryAdmin 직전 작업 상태 잔상 처리", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  // 백엔드는 새 작업이 시작되기 전까지 직전 작업의 done/error를 계속 돌려준다.
  // 그 응답을 완료로 처리하면 탭을 열 때마다, 디렉토리를 고를 때마다 같은 배너와
  // 목록 재조회가 되풀이된다.
  function setupWithTerminalStatus(status) {
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve(status);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });
  }

  it("시작하지 않은 작업의 error 상태는 오류 배너로 띄우지 않는다", async () => {
    setupWithTerminalStatus({ status: "error", error: "이전 재적재 실패" });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    expect(screen.queryByText("이전 재적재 실패")).toBeNull();

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByTitle("ES 재적재")).toBeTruthy();
    });
    expect(screen.queryByText("이전 재적재 실패")).toBeNull();
  });

  it("시작하지 않은 작업의 done 상태로는 목록을 다시 조회하지 않는다", async () => {
    setupWithTerminalStatus({
      status: "done",
      category: "1_fiction",
      indexed_count: 3,
      deleted_count: 1,
      after_count: 0,
      failed_count: 0,
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    const countCategoriesCalls = () =>
      mockJsonGetReq.mock.calls.filter((call) => call[0] === "/categories")
        .length;
    const before = countCategoriesCalls();

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByTitle("ES 재적재")).toBeTruthy();
    });

    expect(countCategoriesCalls()).toBe(before);
  });

  it("직접 시작한 작업이 error로 끝나면 오류 배너를 표시한다", async () => {
    setupWithTerminalStatus({ status: "error", error: "재적재 도중 중단" });
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

    fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(within(modal).getByRole("button", { name: "일괄 재적재" }));

    await waitFor(() => {
      expect(screen.getByText("재적재 도중 중단")).toBeTruthy();
    });
  });
});

describe("CategoryAdmin 재적재 중 버튼 비활성 일관성", () => {
  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  // 헤더의 "일괄"·"이상 항목" 버튼과 오른쪽 액션 영역의 버튼은 같은 작업을 실행하므로
  // 진행 중 비활성 조건도 같아야 한다.
  function setupRunning(runningStatus) {
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(CATEGORIES_RESPONSE);
      else if (url === "/category-mismatches")
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve(runningStatus);
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve(HIDDEN_RESPONSE);
      else if (url.startsWith("/latest-excluded-categories"))
        resolve(LATEST_EXCLUDED_RESPONSE);
    });
  }

  it("일괄 재적재 진행 중에는 오른쪽 액션 버튼도 함께 비활성이 된다", async () => {
    setupRunning({ status: "running", category: null, remaining_count: 5 });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByTitle("ES 재적재")).toBeTruthy();
    });

    expect(screen.getByRole("button", { name: /일괄 재적재/ }).disabled).toBe(
      true,
    );
    expect(screen.getByTitle("ES 재적재").disabled).toBe(true);
    expect(screen.getByTitle("이상 항목만 ES 재적재").disabled).toBe(true);
  });

  it("카테고리 이상 항목 재적재 진행 중에는 오른쪽 액션 버튼도 함께 비활성이 된다", async () => {
    setupRunning({
      status: "running",
      category: "1_fiction",
      remaining_count: 2,
    });
    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });

    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByTitle("ES 재적재").disabled).toBe(true);
    });
    expect(screen.getByTitle("이상 항목만 ES 재적재").disabled).toBe(true);
  });
});

// ── 재적재 완료 후 재조회 범위와 화면 유지 ──
// 재적재는 백그라운드 작업이다. 완료를 반영할 때 화면 전체를 "로딩 중..."으로 덮거나
// 코퍼스 전수 요약 스캔(/category-mismatches)을 다시 돌리면, 사용자는 버튼 하나를
// 눌렀는데 서브탭에 다시 들어온 것과 같은 대기를 겪는다.

describe("CategoryAdmin 재적재 완료 반영", () => {
  const DETAIL_WITH_ANOMALIES = {
    es_only: [
      {
        book_id: 101,
        title: "Book A",
        file_type: "pdf",
        file_path: "1_fiction/a.pdf",
      },
    ],
    fs_only: [{ file_name: "orphan.txt", file_path: "1_fiction/orphan.txt" }],
    duplicates: [],
    fs_count: 8,
  };

  const IDLE = { status: "idle" };

  beforeEach(() => {
    mockJsonGetReq.mockReset();
    mockJsonDeleteReq.mockReset();
    mockJsonPostReq.mockReset();
    mockJsonPutReq.mockReset();
  });

  // 백엔드는 카테고리별 락과 전체(일괄) 락을 따로 들고 있다. reload-status는
  // category 쿼리가 있으면 그 카테고리 락을, 없으면 전체 락을 돌려준다. 두 락을 같은
  // 값으로 흉내내면 한 작업의 상태가 다른 쪽 버튼까지 움직여 검사가 무의미해진다.
  function mockServer({ categoryStatusRef, bulkStatusRef, detailRef }) {
    const counts = { summaryScan: 0, detail: 0, categories: 0 };
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") {
        counts.categories++;
        resolve(CATEGORIES_RESPONSE);
      } else if (url === "/category-mismatches") {
        counts.summaryScan++;
        resolve(MISMATCH_RESPONSE_WITH_DATA);
      } else if (url === "/category-mismatches/reload-status") {
        resolve(bulkStatusRef.current);
      } else if (url.startsWith("/category-mismatches/reload-status?")) {
        resolve(categoryStatusRef.current);
      } else if (url.startsWith("/category-mismatches/")) {
        counts.detail++;
        resolve(detailRef.current);
      } else if (url.startsWith("/category-mappings")) {
        resolve(MAPPINGS_RESPONSE);
      } else if (url.startsWith("/hidden-categories")) {
        resolve(HIDDEN_RESPONSE);
      } else if (url.startsWith("/latest-excluded-categories")) {
        resolve(LATEST_EXCLUDED_RESPONSE);
      }
    });
    return counts;
  }

  function findTreeItemByText(text) {
    return screen
      .getAllByRole("treeitem")
      .find((item) => item.textContent.includes(text));
  }

  function mockStartedPost(statusRef, nextStatus) {
    mockJsonPostReq.mockImplementation(
      (url, payload, resolve, _reject, done) => {
        statusRef.current = nextStatus;
        resolve({ started: true, category: payload?.category ?? null });
        if (done) done();
      },
    );
  }

  async function selectFictionAndStartCategoryReload(categoryStatusRef) {
    await waitFor(() => {
      expect(screen.getByText("1_fiction")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("1_fiction"));
    await waitFor(() => {
      expect(screen.getByTitle("이상 항목만 ES 재적재").disabled).toBe(false);
    });

    mockStartedPost(categoryStatusRef, {
      status: "running",
      category: "1_fiction",
      before_count: 2,
      indexed_count: 0,
      deleted_count: 0,
    });
    fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
    const modal = await screen.findByRole("dialog");
    fireEvent.click(
      within(modal).getByRole("button", { name: "이상 항목 재적재" }),
    );
    await waitFor(() => {
      expect(screen.getAllByText(/잔여/).length).toBeGreaterThan(0);
    });
  }

  it("카테고리 재적재가 진행 중이면 전체 락이 비어 있어도 진행 표시를 유지한다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const categoryStatusRef = { current: IDLE };
      const bulkStatusRef = { current: IDLE };
      const detailRef = { current: DETAIL_WITH_ANOMALIES };
      mockServer({ categoryStatusRef, bulkStatusRef, detailRef });
      render(<CategoryAdmin />);
      await selectFictionAndStartCategoryReload(categoryStatusRef);

      // 전체 락을 여러 번 폴링해도 카테고리 작업의 스피너를 끄지 않아야 한다.
      await act(async () => {
        vi.advanceTimersByTime(20000);
      });
      expect(screen.getAllByText(/잔여/).length).toBeGreaterThan(0);
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("카테고리 재적재가 완료되면 전체 요약 스캔을 다시 돌리지 않고 그 카테고리만 다시 센다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const categoryStatusRef = { current: IDLE };
      const bulkStatusRef = { current: IDLE };
      const detailRef = { current: DETAIL_WITH_ANOMALIES };
      const counts = mockServer({ categoryStatusRef, bulkStatusRef, detailRef });
      render(<CategoryAdmin />);
      await selectFictionAndStartCategoryReload(categoryStatusRef);

      const scansBeforeCompletion = counts.summaryScan;
      const detailsBeforeCompletion = counts.detail;

      detailRef.current = { ...DETAIL_WITH_ANOMALIES, es_only: [], fs_only: [] };
      categoryStatusRef.current = {
        status: "done",
        category: "1_fiction",
        indexed_count: 2,
        deleted_count: 0,
        after_count: 0,
        failed_count: 0,
      };
      await act(async () => {
        vi.advanceTimersByTime(10000);
      });

      await waitFor(() => {
        expect(counts.detail).toBeGreaterThan(detailsBeforeCompletion);
      });
      expect(counts.summaryScan).toBe(scansBeforeCompletion);
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("카테고리 재적재가 완료돼도 펼친 디렉토리와 선택이 유지된다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const categoryStatusRef = { current: IDLE };
      const bulkStatusRef = { current: IDLE };
      const detailRef = { current: DETAIL_WITH_ANOMALIES };
      mockServer({ categoryStatusRef, bulkStatusRef, detailRef });
      render(<CategoryAdmin />);
      await selectFictionAndStartCategoryReload(categoryStatusRef);
      await waitFor(() => {
        expect(screen.getByText("Book A.pdf")).toBeTruthy();
      });

      // 두 건 중 한 건만 해소된 상태. 남은 항목은 그대로 펼쳐져 있어야 한다.
      detailRef.current = { ...DETAIL_WITH_ANOMALIES, fs_only: [] };
      categoryStatusRef.current = {
        status: "done",
        category: "1_fiction",
        after_count: 1,
        failed_count: 0,
      };
      await act(async () => {
        vi.advanceTimersByTime(10000);
      });

      // 전체 재조회가 돌면 expandedItems가 비워지고 트리가 처음부터 다시 만들어진다.
      await waitFor(() => {
        expect(screen.queryByText("orphan.txt")).toBeNull();
      });
      expect(
        findTreeItemByText("1_fiction")?.getAttribute("aria-expanded"),
      ).toBe("true");
      expect(screen.getByText("Book A.pdf")).toBeTruthy();
      expect(screen.getByTitle("이상 항목만 ES 재적재")).toBeTruthy();
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("카테고리 재적재가 완료되면 그 카테고리의 이상 항목 건수를 전체 집계에서도 뺀다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const categoryStatusRef = { current: IDLE };
      const bulkStatusRef = { current: IDLE };
      const detailRef = { current: DETAIL_WITH_ANOMALIES };
      mockServer({ categoryStatusRef, bulkStatusRef, detailRef });
      render(<CategoryAdmin />);
      await selectFictionAndStartCategoryReload(categoryStatusRef);

      // 요약 스캔의 전체 이상 항목은 1_fiction 2건 + 2_science 8건 + 4_fs_only_cat 9건.
      detailRef.current = { ...DETAIL_WITH_ANOMALIES, es_only: [], fs_only: [] };
      categoryStatusRef.current = {
        status: "done",
        category: "1_fiction",
        after_count: 0,
        failed_count: 0,
      };
      await act(async () => {
        vi.advanceTimersByTime(10000);
      });

      fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
      const modal = await screen.findByRole("dialog");
      await waitFor(() => {
        expect(modal.textContent).toContain("이상 항목 17건");
      });
      expect(modal.textContent).toContain("불일치 카테고리 2개");
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("일괄 재적재가 완료되면 요약 스캔은 다시 돌리지만 펼친 디렉토리는 유지한다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const categoryStatusRef = { current: IDLE };
      const bulkStatusRef = { current: IDLE };
      const detailRef = { current: DETAIL_WITH_ANOMALIES };
      const counts = mockServer({ categoryStatusRef, bulkStatusRef, detailRef });
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("1_fiction")).toBeTruthy();
      });
      fireEvent.click(screen.getByText("1_fiction"));
      await waitFor(() => {
        expect(screen.getByText("Book A.pdf")).toBeTruthy();
      });

      mockStartedPost(bulkStatusRef, {
        status: "running",
        category: null,
        reload_source: "bulk",
        before_count: 4,
      });
      fireEvent.click(screen.getByRole("button", { name: /일괄 재적재/ }));
      const modal = await screen.findByRole("dialog");
      fireEvent.click(
        within(modal).getByRole("button", { name: "일괄 재적재" }),
      );
      await waitFor(() => {
        expect(screen.getAllByText(/잔여/).length).toBeGreaterThan(0);
      });

      const scansBeforeCompletion = counts.summaryScan;
      bulkStatusRef.current = {
        status: "done",
        category: null,
        reload_source: "bulk",
        after_count: 0,
        failed_count: 0,
      };
      await act(async () => {
        vi.advanceTimersByTime(10000);
      });

      await waitFor(() => {
        expect(counts.summaryScan).toBeGreaterThan(scansBeforeCompletion);
      });
      expect(screen.getByText("디렉토리")).toBeTruthy();
      expect(
        findTreeItemByText("1_fiction")?.getAttribute("aria-expanded"),
      ).toBe("true");
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });

  it("하위 카테고리를 다시 세면 부모의 합계 배지도 같이 맞춘다", async () => {
    const categories = { parent: 10, "parent/child": 3 };
    const mismatchData = {
      mismatches: [],
      es_only: [
        { category: "parent", es_count: 10, anomaly_count: 2 },
        { category: "parent/child", es_count: 3, anomaly_count: 3 },
      ],
      fs_only: [],
    };
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/category-mismatches") resolve(mismatchData);
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url === "/category-mismatches/parent")
        resolve({
          es_only: [
            {
              book_id: 1,
              title: "P1",
              file_type: "pdf",
              file_path: "parent/p1.pdf",
            },
            {
              book_id: 2,
              title: "P2",
              file_type: "pdf",
              file_path: "parent/p2.pdf",
            },
          ],
          fs_only: [],
          duplicates: [],
          fs_count: 10,
        });
      else if (url === "/category-mismatches/parent/child")
        resolve({
          es_only: [
            {
              book_id: 3,
              title: "C1",
              file_type: "pdf",
              file_path: "parent/child/c1.pdf",
            },
          ],
          fs_only: [],
          duplicates: [],
          fs_count: 3,
        });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve([]);
      else if (url.startsWith("/latest-excluded-categories")) resolve([]);
    });

    render(<CategoryAdmin />);
    const treeLabel = (text) =>
      within(screen.getByRole("tree")).getByText(text).parentElement
        .textContent;

    await waitFor(() => {
      expect(screen.getByRole("tree")).toBeTruthy();
    });

    // 요약 스캔 기준 부모 배지는 자기 2건 + 하위 3건 = 5건.
    fireEvent.click(within(screen.getByRole("tree")).getByText("parent"));
    await waitFor(() => {
      expect(treeLabel("parent")).toBe("parent5");
    });

    // 하위를 실제로 세니 3건이 아니라 1건이다. 부모 배지도 3건으로 내려가야 한다.
    fireEvent.click(within(screen.getByRole("tree")).getByText("child"));
    await waitFor(() => {
      expect(treeLabel("child")).toBe("child1");
    });
    expect(treeLabel("parent")).toBe("parent3");
  });

  it("가상 부모 아래 하위 카테고리를 다시 세도 합계 배지를 맞춘다", async () => {
    // 부모 카테고리가 ES에 없으면 트리는 가상 부모를 만든다. 가상 부모는 자체 건수가
    // 0이라 합계가 하위 건수만으로 정해진다.
    // 공통 접두어가 생기지 않게 다른 최상위 카테고리를 하나 같이 둔다.
    const categories = { "a/x": 3, b: 2 };
    const mismatchData = {
      mismatches: [],
      es_only: [{ category: "a/x", es_count: 3, anomaly_count: 3 }],
      fs_only: [],
    };
    mockJsonGetReq.mockImplementation((url, _payload, resolve) => {
      if (url === "/categories") resolve(categories);
      else if (url === "/category-mismatches") resolve(mismatchData);
      else if (url.startsWith("/category-mismatches/reload-status"))
        resolve({ status: "idle" });
      else if (url === "/category-mismatches/a/x")
        resolve({
          es_only: [
            {
              book_id: 7,
              title: "X1",
              file_type: "pdf",
              file_path: "a/x/x1.pdf",
            },
          ],
          fs_only: [],
          duplicates: [],
          fs_count: 3,
        });
      else if (url.startsWith("/category-mappings")) resolve(MAPPINGS_RESPONSE);
      else if (url.startsWith("/hidden-categories")) resolve([]);
      else if (url.startsWith("/latest-excluded-categories")) resolve([]);
    });

    render(<CategoryAdmin />);
    const treeLabel = (text) =>
      within(screen.getByRole("tree")).getByText(text).parentElement
        .textContent;
    await waitFor(() => {
      expect(treeLabel("a")).toBe("a3");
    });

    fireEvent.click(within(screen.getByRole("tree")).getByText("a"));
    fireEvent.click(within(screen.getByRole("tree")).getByText("x"));
    await waitFor(() => {
      expect(treeLabel("x")).toBe("x1");
    });
    expect(treeLabel("a")).toBe("a1");
  });

  it("전에 남은 전체 재적재 소유자 기록은 전체 락이 비어 있으면 지운다", async () => {
    window.sessionStorage.setItem("CategoryAdmin.allReloadOwner.book", "bulk");
    const categoryStatusRef = { current: IDLE };
    const bulkStatusRef = { current: IDLE };
    const detailRef = { current: DETAIL_WITH_ANOMALIES };
    mockServer({ categoryStatusRef, bulkStatusRef, detailRef });

    render(<CategoryAdmin />);
    await waitFor(() => {
      expect(
        window.sessionStorage.getItem("CategoryAdmin.allReloadOwner.book"),
      ).toBeNull();
    });
  });

  it("시작 요청 응답 전에 도착한 직전 작업의 done 상태를 내 작업 완료로 보지 않는다", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      // 직전 작업이 남긴 done. 백엔드는 새 락을 잡기 전까지 이 상태를 계속 돌려준다.
      const categoryStatusRef = {
        current: {
          status: "done",
          category: "1_fiction",
          after_count: 0,
          failed_count: 0,
        },
      };
      const bulkStatusRef = { current: IDLE };
      const detailRef = { current: DETAIL_WITH_ANOMALIES };
      const counts = mockServer({ categoryStatusRef, bulkStatusRef, detailRef });
      render(<CategoryAdmin />);
      await waitFor(() => {
        expect(screen.getByText("1_fiction")).toBeTruthy();
      });
      fireEvent.click(screen.getByText("1_fiction"));
      await waitFor(() => {
        expect(screen.getByTitle("이상 항목만 ES 재적재").disabled).toBe(false);
      });

      const scansBeforeClick = counts.summaryScan;
      const detailsBeforeClick = counts.detail;

      // POST 응답을 보류한다. 그 사이에 폴링이 stale done을 읽는다.
      let releasePost = null;
      mockJsonPostReq.mockImplementation(
        (url, payload, resolve, _reject, done) => {
          releasePost = () => {
            resolve({ started: true, category: payload?.category ?? null });
            if (done) done();
          };
        },
      );

      fireEvent.click(screen.getByTitle("이상 항목만 ES 재적재"));
      const modal = await screen.findByRole("dialog");
      fireEvent.click(
        within(modal).getByRole("button", { name: "이상 항목 재적재" }),
      );

      await act(async () => {
        await Promise.resolve();
      });

      // stale done을 완료로 오인하면 여기서 재조회가 돌고 스피너가 꺼진다.
      expect(counts.summaryScan).toBe(scansBeforeClick);
      expect(counts.detail).toBe(detailsBeforeClick);
      expect(screen.getAllByText(/잔여/).length).toBeGreaterThan(0);

      releasePost();
      await act(async () => {
        await Promise.resolve();
      });
    } finally {
      vi.runOnlyPendingTimers();
      vi.useRealTimers();
    }
  });
});
