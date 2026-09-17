// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  cleanup,
  within,
} from "@testing-library/react";
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
    candidates: [{ category: "3_SF", source: "model", detail: "모델 판정" }],
    apply_status: "pending",
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
    candidates: [
      { category: "5_음악", source: "bookstore", detail: "서점 1곳 일치" },
    ],
    apply_status: "pending",
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
    candidates: [
      { category: "5_음악", source: "bookstore", detail: "서점 2곳 일치" },
      { category: "1_서양고전", source: "bookstore", detail: "서점 2곳 일치" },
    ],
    apply_status: "pending",
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

  it("선택된 행의 목적지를 비우면 선택도 함께 풀린다", () => {
    const props = renderTable({
      targets: { "A/c.epub": "1_서양고전" },
      selection: new Set(["A/a.epub", "A/c.epub"]),
    });
    fireEvent.change(screen.getByLabelText("A/c.epub 목적지"), {
      target: { value: "" },
    });
    expect(props.onSelectionChange).toHaveBeenCalled();
    const next = props.onSelectionChange.mock.calls[0][0];
    expect(next.has("A/c.epub")).toBe(false);
  });

  it("추천 후보 2개가 각 열에 나오고, 1개면 두 번째 열은 -", () => {
    renderTable();
    const row = screen.getByText("확실한 책").closest("tr");
    const [, , , recommend1] = within(row).getAllByRole("cell");
    expect(within(recommend1).getByText("3_SF")).toBeTruthy();
    expect(within(recommend1).getByText("모델 판정")).toBeTruthy();

    const singleCandidateRow = screen.getByText("애매한 책").closest("tr");
    const cells = within(singleCandidateRow).getAllByRole("cell");
    // 열 순서: 체크박스, 책, 현재, 추천1, 추천2, 직접 선택, 점수, 이동 상태
    expect(cells[4].textContent).toBe("-");
  });

  it("서점 판정이 갈린 행은 두 추천이 다른 답임을 배지로 드러낸다", () => {
    renderTable();
    const row = screen.getByText("불확실한 책").closest("tr");
    const cells = within(row).getAllByRole("cell");
    const [, , , recommend1, recommend2] = cells;
    expect(within(recommend1).getByText("서점 판정 갈림")).toBeTruthy();
    expect(within(recommend1).getByText("5_음악")).toBeTruthy();
    expect(within(recommend2).getByText("1_서양고전")).toBeTruthy();
  });

  it("이동 상태가 대기/이동 중(중단됨)/이동 완료/실패:사유로 나온다", () => {
    const items = [
      { ...ITEMS[0], apply_status: "pending" },
      { ...ITEMS[1], apply_status: "moving" },
      {
        ...ITEMS[2],
        target_category: "5_음악",
        apply_status: "moved",
      },
      {
        file_path: "A/d.epub",
        title: "실패한 책",
        current_category: "A",
        target_category: "3_SF",
        grade: "certain",
        confidence: 0.9,
        source: "model",
        candidates: [],
        apply_status: "failed",
        apply_error: "대상 폴더 없음",
      },
    ];
    renderTable({ items });
    expect(screen.getByText("대기")).toBeTruthy();
    expect(screen.getByText("이동 중(중단됨)")).toBeTruthy();
    expect(screen.getByText("이동 완료")).toBeTruthy();
    expect(screen.getByText("실패: 대상 폴더 없음")).toBeTruthy();
  });

  it("apply_status가 moved나 moving인 행은 체크박스가 비활성이고, moving은 구분되게 표시된다", () => {
    const items = [
      { ...ITEMS[0], apply_status: "moving" },
      { ...ITEMS[1], apply_status: "moved" },
    ];
    renderTable({ items, selection: new Set(["A/a.epub", "A/b.epub"]) });

    expect(screen.getByLabelText("A/a.epub 선택").disabled).toBe(true);
    expect(screen.getByLabelText("A/b.epub 선택").disabled).toBe(true);

    const movingBadge = screen.getByText("이동 중(중단됨)");
    const movedBadge = screen.getByText("이동 완료");
    // 같은 상태 배지 클래스를 공유하지 않아야 moving이 moved/대기와 섞여 보이지 않는다.
    expect(movingBadge.className).not.toBe(movedBadge.className);
  });
});
