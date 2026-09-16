// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import ClassifyProposalTable from "../src/ClassifyProposalTable";

// vitest globals(afterEach)가 꺼져 있어 testing-library 자동 cleanup이 동작하지
// 않는다(CategoryAdmin.test.jsx와 동일 관례). render()가 남긴 DOM을 직접 치운다.
afterEach(() => {
  cleanup();
});

const ITEMS = [
  {
    file_path: "A/a.epub",
    title: "확실한 책",
    current_category: "A",
    target_category: "3_SF",
    grade: "certain",
    confidence: 0.91,
    source: "model",
    reason: "모델 판정",
  },
  {
    file_path: "A/b.epub",
    title: "애매한 책",
    current_category: "A",
    target_category: "5_음악",
    grade: "unsure",
    confidence: 0.2,
    source: "bookstore_single",
    reason: "서점 한 곳",
  },
  {
    file_path: "A/c.epub",
    title: "불확실한 책",
    current_category: "A",
    target_category: null,
    grade: "unknown",
    confidence: null,
    source: "conflict",
    reason: "서점 판정이 갈림",
  },
];
const CATEGORIES = ["3_SF", "5_음악", "1_서양고전"];

function renderTable(overrides = {}) {
  const props = {
    items: ITEMS,
    categories: CATEGORIES,
    selection: new Set(["A/a.epub"]),
    targets: {},
    onSelectionChange: vi.fn(),
    onTargetChange: vi.fn(),
    ...overrides,
  };
  render(<ClassifyProposalTable {...props} />);
  return props;
}

describe("ClassifyProposalTable", () => {
  it("등급에 따라 체크박스 초기 상태가 다르다", () => {
    renderTable();
    expect(screen.getByLabelText("A/a.epub 선택").checked).toBe(true);
    expect(screen.getByLabelText("A/b.epub 선택").checked).toBe(false);
    expect(screen.getByLabelText("A/c.epub 선택").disabled).toBe(true);
  });

  it("불확실 행도 목적지를 고르면 체크할 수 있다", () => {
    const props = renderTable({ targets: { "A/c.epub": "1_서양고전" } });
    expect(screen.getByLabelText("A/c.epub 선택").disabled).toBe(false);
    fireEvent.click(screen.getByLabelText("A/c.epub 선택"));
    expect(props.onSelectionChange).toHaveBeenCalled();
  });

  it("전체 선택은 비활성 행을 건너뛴다", () => {
    const props = renderTable();
    fireEvent.click(screen.getByLabelText("전체 선택"));
    const next = props.onSelectionChange.mock.calls[0][0];
    expect(next.has("A/c.epub")).toBe(false);
    expect(next.has("A/b.epub")).toBe(true);
  });

  it("목적지를 바꾸면 알린다", () => {
    const props = renderTable();
    fireEvent.change(screen.getByLabelText("A/b.epub 목적지"), {
      target: { value: "1_서양고전" },
    });
    expect(props.onTargetChange).toHaveBeenCalledWith("A/b.epub", "1_서양고전");
  });
});
