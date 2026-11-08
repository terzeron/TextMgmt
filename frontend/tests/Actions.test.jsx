// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup, act } from "@testing-library/react";

afterEach(cleanup);

// 추천 API 대역. 대응 알고리즘은 backend(store_category_matcher)가 맡고 Rust 테스트가 검증한다.
// 여기서는 서점 카테고리 단어가 디렉토리 이름(숫자 접두 제외)에 들어 있으면 맞은 것으로 보는 단순 규칙을 쓴다.
const { mockPost } = vi.hoisted(() => ({ mockPost: vi.fn() }));
const fakeSuggest = ({ suggested, categories, top_n }) => {
  const words = Object.values(suggested).flatMap((v) => v.split(/[/()\s]+/).filter(Boolean));
  return categories
    .map((c) => [c, words.filter((w) => (c.includes("_") ? c.split("_").slice(1).join("_") : c).includes(w)).length])
    .filter(([, n]) => n > 0)
    .sort((a, b) => b[1] - a[1])
    .slice(0, top_n)
    .map(([c]) => c);
};
vi.mock("../src/Common", () => ({
  getRandomLightColor: () => "#cccccc",
  ROOT_DIRECTORY: "$$rootdir$$",
  jsonPostReq: (...args) => mockPost(...args),
}));
const answerNow = () =>
  mockPost.mockImplementation((url, payload, resolve) => resolve({ top: fakeSuggest(payload), debug: null }));
beforeEach(() => {
  mockPost.mockReset();
  answerNow();
});

import Actions from "../src/Actions";

// ── 공통 props ──

const defaultProps = {
  selectedEntryId: "test/1",
  selectedCategory: "",
  otherCategoryList: ["소설", "역사", "과학"],
  newFileName: "test.pdf",
  toNextEntryClicked: vi.fn(),
  moveToUpperButtonClicked: vi.fn(),
  moveToDirectoryButtonClicked: vi.fn(),
  selectDirectoryButtonClicked: vi.fn(),
  suggestedCategories: {},
};

// ── isProcessing 버튼 비활성화 ──

describe("isProcessing 버튼 비활성화", () => {
  it('isProcessing=false이면 "다음 책으로" 버튼이 활성화된다', () => {
    render(<Actions {...defaultProps} isProcessing={false} />);
    expect(screen.getByText("다음 책으로").disabled).toBe(false);
  });

  it('isProcessing=true이면 "다음 책으로" 버튼이 비활성화된다', () => {
    render(<Actions {...defaultProps} isProcessing={true} />);
    expect(screen.getByText("다음 책으로").disabled).toBe(true);
  });

  it('isProcessing=true이면 "상위로" 버튼이 비활성화된다', () => {
    render(<Actions {...defaultProps} isProcessing={true} />);
    const btn = screen.getByText(
      (content, element) =>
        element.tagName === "BUTTON" && element.textContent.includes("상위로"),
    );
    expect(btn.disabled).toBe(true);
  });

  it('isProcessing=true이면 "로 옮기기" 버튼이 비활성화된다', () => {
    render(
      <Actions {...defaultProps} selectedCategory="소설" isProcessing={true} />,
    );
    const btn = screen.getByText(
      (content, element) =>
        element.tagName === "BUTTON" &&
        element.textContent.includes("로 옮기기"),
    );
    expect(btn.disabled).toBe(true);
  });

  it('isProcessing=false이면 "로 옮기기" 버튼이 활성화된다 (selectedCategory 있을 때)', () => {
    render(
      <Actions
        {...defaultProps}
        selectedCategory="소설"
        isProcessing={false}
      />,
    );
    const btn = screen.getByText(
      (content, element) =>
        element.tagName === "BUTTON" &&
        element.textContent.includes("로 옮기기"),
    );
    expect(btn.disabled).toBe(false);
  });

  it("isProcessing가 undefined이면 기존 disabled 로직만 적용된다", () => {
    render(<Actions {...defaultProps} />);
    // "다음 책으로"는 isProcessing 없으므로 활성화
    expect(screen.getByText("다음 책으로").disabled).toBe(false);
  });

  it('isProcessing=true일 때 "다음 책으로" 클릭해도 핸들러가 호출되지 않는다', () => {
    const fn = vi.fn();
    render(
      <Actions {...defaultProps} toNextEntryClicked={fn} isProcessing={true} />,
    );
    const btn = screen.getByText("다음 책으로");
    fireEvent.click(btn);
    expect(fn).not.toHaveBeenCalled();
  });

  it('isProcessing=true일 때 "로 옮기기" 클릭해도 핸들러가 호출되지 않는다', () => {
    const fn = vi.fn();
    render(
      <Actions
        {...defaultProps}
        selectedCategory="소설"
        moveToDirectoryButtonClicked={fn}
        isProcessing={true}
      />,
    );
    const btn = screen.getByText(
      (content, element) =>
        element.tagName === "BUTTON" &&
        element.textContent.includes("로 옮기기"),
    );
    fireEvent.click(btn);
    expect(fn).not.toHaveBeenCalled();
  });
});

// ── 카테고리 버튼 하이라이트/선택 스타일 ──

describe("카테고리 버튼 하이라이트", () => {
  it("유사도 자동 선택 시 highlight 클래스를 유지한다 (노란색)", () => {
    const selectFn = vi.fn();
    render(
      <Actions
        {...defaultProps}
        suggestedCategories={{ yes24: "소설" }}
        selectedCategory="소설"
        selectDirectoryButtonClicked={selectFn}
      />,
    );
    // 자동 선택으로 selectDirectoryButtonClicked가 호출됨
    expect(selectFn).toHaveBeenCalledWith(null, "소설");
    // 버튼에 highlight 클래스가 유지되어야 함
    const btn = screen.getByText("소설");
    expect(btn.className).toContain("highlight");
    // 흰색 배경이 아님
    expect(btn.style.backgroundColor).not.toBe("rgb(255, 255, 255)");
  });

  it("수동 클릭 시 흰색 배경을 적용한다", () => {
    const selectFn = vi.fn();
    const { rerender } = render(
      <Actions
        {...defaultProps}
        suggestedCategories={{ yes24: "소설" }}
        selectedCategory="소설"
        selectDirectoryButtonClicked={selectFn}
      />,
    );
    // 사용자가 '역사' 버튼을 클릭
    fireEvent.click(screen.getByText("역사"));
    expect(selectFn).toHaveBeenCalledWith(expect.anything(), "역사");

    // 부모가 selectedCategory를 '역사'로 업데이트
    rerender(
      <Actions
        {...defaultProps}
        suggestedCategories={{ yes24: "소설" }}
        selectedCategory="역사"
        selectDirectoryButtonClicked={selectFn}
      />,
    );
    const btn = screen.getByText("역사");
    expect(btn.style.backgroundColor).toBe("rgb(255, 255, 255)");
    expect(btn.className).not.toContain("highlight");
  });

  it("수동 클릭 후에도 유사 카테고리의 highlight 클래스는 유지된다", () => {
    const selectFn = vi.fn();
    const { rerender } = render(
      <Actions
        {...defaultProps}
        suggestedCategories={{ yes24: "소설" }}
        selectedCategory="소설"
        selectDirectoryButtonClicked={selectFn}
      />,
    );
    // 사용자가 다른 버튼 클릭
    fireEvent.click(screen.getByText("역사"));
    rerender(
      <Actions
        {...defaultProps}
        suggestedCategories={{ yes24: "소설" }}
        selectedCategory="역사"
        selectDirectoryButtonClicked={selectFn}
      />,
    );
    // '소설' 버튼은 선택 해제되었지만 여전히 highlight 클래스를 가짐
    const novelBtn = screen.getByText("소설");
    expect(novelBtn.className).toContain("highlight");
  });

  it("highlight-secondary 클래스를 유사도 2~5위 카테고리에 적용한다", () => {
    const selectFn = vi.fn();
    render(
      <Actions
        {...defaultProps}
        otherCategoryList={["소설", "역사", "과학", "철학", "예술"]}
        suggestedCategories={{ yes24: "소설" }}
        selectedCategory="소설"
        selectDirectoryButtonClicked={selectFn}
      />,
    );
    // 최소 1위인 소설은 highlight
    const novelBtn = screen.getByText("소설");
    expect(novelBtn.className).toContain("highlight");
  });

  describe("사용자가 직접 고른 카테고리가 우선한다", () => {
    const manualPick = () => {
      const selectFn = vi.fn();
      const view = render(
        <Actions
          {...defaultProps}
          suggestedCategories={{ yes24: "소설" }}
          selectedCategory="소설"
          selectDirectoryButtonClicked={selectFn}
        />,
      );
      fireEvent.click(screen.getByText("역사"));
      selectFn.mockClear();
      return { selectFn, ...view };
    };

    it("클릭한 뒤 새 서점 결과가 도착해도 선택을 바꾸지 않는다", () => {
      const { selectFn, rerender } = manualPick();

      rerender(
        <Actions
          {...defaultProps}
          suggestedCategories={{ yes24: "과학" }}
          selectedCategory="역사"
          selectDirectoryButtonClicked={selectFn}
        />,
      );

      expect(selectFn).not.toHaveBeenCalled();
      expect(screen.getByText("역사").style.backgroundColor).toBe("rgb(255, 255, 255)");
    });

    it("카테고리 목록이 내용은 같고 새 배열로 바뀌어도 선택을 바꾸지 않는다", () => {
      const { selectFn, rerender } = manualPick();

      rerender(
        <Actions
          {...defaultProps}
          otherCategoryList={["소설", "역사", "과학"]}
          suggestedCategories={{ yes24: "소설" }}
          selectedCategory="역사"
          selectDirectoryButtonClicked={selectFn}
        />,
      );

      expect(selectFn).not.toHaveBeenCalled();
    });

    it("검색 결과가 비었다가 다시 채워져도 선택을 바꾸지 않는다", () => {
      const { selectFn, rerender } = manualPick();
      const props = { ...defaultProps, selectedCategory: "역사", selectDirectoryButtonClicked: selectFn };

      rerender(<Actions {...props} suggestedCategories={{}} />);
      rerender(<Actions {...props} suggestedCategories={{ aladin: "과학" }} />);

      expect(selectFn).not.toHaveBeenCalled();
    });

    it("다른 책으로 바뀌면 다시 자동 선택한다", () => {
      const { selectFn, rerender } = manualPick();
      const props = { ...defaultProps, selectedCategory: "역사", selectDirectoryButtonClicked: selectFn };

      rerender(<Actions {...props} selectedEntryId="test/2" suggestedCategories={{}} />);
      rerender(<Actions {...props} selectedEntryId="test/2" selectedCategory="" suggestedCategories={{ yes24: "과학" }} />);

      expect(selectFn).toHaveBeenCalledWith(null, "과학");
    });

    it("직접 고르기 전에는 새 서점 결과로 자동 선택을 갱신한다", () => {
      const selectFn = vi.fn();
      const { rerender } = render(
        <Actions
          {...defaultProps}
          suggestedCategories={{ yes24: "소설" }}
          selectedCategory="소설"
          selectDirectoryButtonClicked={selectFn}
        />,
      );
      selectFn.mockClear();

      rerender(
        <Actions
          {...defaultProps}
          suggestedCategories={{ yes24: "역사" }}
          selectedCategory="소설"
          selectDirectoryButtonClicked={selectFn}
        />,
      );

      expect(selectFn).toHaveBeenCalledWith(null, "역사");
    });
  });
});

describe("Actions 추가 분기", () => {
  it("디렉토리 탐색 버튼을 카테고리 버튼보다 먼저 표시하고 선택한 탐색 동작을 호출한다", () => {
    const previous = vi.fn();
    const next = vi.fn();
    render(
      <Actions
        {...defaultProps}
        directoryMode
        showDirectoryNavigation
        previousDirectoryDisabled={false}
        nextDirectoryDisabled={false}
        onPreviousDirectory={previous}
        onNextDirectory={next}
      />,
    );

    const previousButton = screen.getByText("이전 디렉토리로");
    const nextButton = screen.getByText("다음 디렉토리로");
    expect(previousButton.compareDocumentPosition(screen.getByText("소설")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(previousButton);
    fireEvent.click(nextButton);
    expect(previous).toHaveBeenCalledOnce();
    expect(next).toHaveBeenCalledOnce();
  });

  it("탐색 가능 옵션이 없으면 디렉토리 탐색 버튼을 숨긴다", () => {
    render(<Actions {...defaultProps} directoryMode />);
    expect(screen.queryByText("이전 디렉토리로")).toBeNull();
    expect(screen.queryByText("다음 디렉토리로")).toBeNull();
  });

  it("서점 카테고리를 backend 추천 API로 보낸다", () => {
    render(
      <Actions
        {...defaultProps}
        otherCategoryList={["_root", "소설", "역사"]}
        suggestedCategories={{ yes24_0_0: "소설" }}
      />,
    );

    expect(mockPost).toHaveBeenCalledWith(
      "/bookstore/category-suggestions?content_type=book",
      { suggested: { yes24_0_0: "소설" }, categories: ["소설", "역사"], top_n: 5 },
      expect.any(Function),
      expect.any(Function),
    );
  });

  it("추천 요청이 실패하면 하이라이트하지 않는다", () => {
    mockPost.mockImplementation((url, payload, resolve, reject) => reject("서버 오류"));
    render(<Actions {...defaultProps} suggestedCategories={{ yes24: "소설" }} />);
    expect(document.querySelectorAll(".highlight").length).toBe(0);
  });

  it("늦게 도착한 이전 응답은 무시한다", () => {
    const pending = [];
    mockPost.mockImplementation((url, payload, resolve) => pending.push(() => resolve({ top: fakeSuggest(payload) })));
    const selectFn = vi.fn();
    const props = { ...defaultProps, selectDirectoryButtonClicked: selectFn };
    const { rerender } = render(<Actions {...props} suggestedCategories={{ yes24: "소설" }} />);
    rerender(<Actions {...props} suggestedCategories={{ yes24: "역사" }} />);

    act(() => pending[1]());
    act(() => pending[0]());

    expect(selectFn).toHaveBeenCalledTimes(1);
    expect(selectFn).toHaveBeenCalledWith(null, "역사");
  });

  it("언더스코어가 있는 카테고리는 접두사 색상과 하위 라벨로 렌더링한다", () => {
    render(
      <Actions {...defaultProps} otherCategoryList={["10_history_modern"]} />,
    );

    const button = screen.getByText("history");
    expect(button).toBeTruthy();
    expect(button.style.backgroundColor).toBe("rgb(204, 204, 204)");
  });
});

// ── 추가 분기 ──

describe("Actions 카테고리 하이라이트", () => {
  it("유사도 2~5위 카테고리에 highlight-secondary 클래스를 적용한다", () => {
    render(
      <Actions
        {...defaultProps}
        otherCategoryList={["소설", "역사", "과학"]}
        suggestedCategories={{ yes24: "소설 역사 과학" }}
      />,
    );
    const secondary = document.querySelectorAll(".highlight-secondary");
    expect(secondary.length).toBeGreaterThan(0);
    expect(document.querySelectorAll(".highlight").length).toBeGreaterThan(0);
  });

  it("빈 서점 카테고리가 섞여 있어도 나머지로 하이라이트를 계산한다", () => {
    render(
      <Actions
        {...defaultProps}
        otherCategoryList={["소설", "역사"]}
        suggestedCategories={{ yes24: "", aladin: "소설" }}
      />,
    );
    expect(document.querySelectorAll(".highlight").length).toBeGreaterThan(0);
  });

  it("구분자만 있는 서점 카테고리는 하이라이트를 만들지 않는다", () => {
    render(
      <Actions
        {...defaultProps}
        otherCategoryList={["소설", "역사"]}
        suggestedCategories={{ yes24: "///" }}
      />,
    );
    expect(document.querySelectorAll(".highlight").length).toBe(0);
  });

  it("_root 만 있는 목록은 하이라이트 대상이 없다", () => {
    render(
      <Actions
        {...defaultProps}
        otherCategoryList={["_root"]}
        suggestedCategories={{ yes24: "소설" }}
      />,
    );
    expect(document.querySelectorAll(".highlight").length).toBe(0);
  });

  it("숫자 prefix 디렉토리도 하이라이트 대상이 된다", () => {
    render(
      <Actions
        {...defaultProps}
        otherCategoryList={["4_소설", "5_역사"]}
        suggestedCategories={{ yes24: "소설" }}
      />,
    );
    expect(document.querySelectorAll(".highlight").length).toBeGreaterThan(0);
  });
});

describe("Actions renderingInfoList 폴백", () => {
  it("otherCategoryList가 없으면 카테고리 버튼을 렌더링하지 않는다", () => {
    render(<Actions {...defaultProps} otherCategoryList={null} />);
    // infoList 계산이 undefined가 되어 빈 배열로 폴백되고, 카테고리 버튼이 렌더링되지 않음
    expect(screen.queryByText("소설")).toBeNull();
    expect(screen.getByText("다음 책으로")).toBeTruthy();
  });
});

describe("Actions 버튼 비활성화 조건", () => {
  it('선택된 항목과 카테고리가 모두 없으면 "로 옮기기" 가 비활성화된다', () => {
    render(
      <Actions
        {...defaultProps}
        selectedEntryId=""
        selectedCategory=""
        isProcessing={false}
      />,
    );
    const btn = screen.getByText(
      (content, element) =>
        element.tagName === "BUTTON" &&
        element.textContent.includes("로 옮기기"),
    );
    expect(btn.disabled).toBe(true);
  });
});

