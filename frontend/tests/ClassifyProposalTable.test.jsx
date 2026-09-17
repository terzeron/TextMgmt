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
    expect(within(recommend1).getByText("후보 동률")).toBeTruthy();
    expect(within(recommend1).getByText("5_음악")).toBeTruthy();
    expect(within(recommend2).getByText("1_서양고전")).toBeTruthy();
  });

  it("키워드 점수가 동점인 행도 같은 배지로 동률임을 드러낸다", () => {
    const items = [
      {
        file_path: "A/d.epub",
        title: "키워드 동점 책",
        current_category: "A",
        target_category: null,
        grade: "unknown",
        confidence: null,
        source: "keyword",
        reason: "여러 카테고리가 동일 점수로 일치합니다: 3_SF, 5_음악",
        candidates: [
          { category: "3_SF", source: "keyword", detail: "키워드 'SF' 일치" },
          {
            category: "5_음악",
            source: "keyword",
            detail: "키워드 '음악' 일치",
          },
        ],
      },
    ];
    renderTable({ items });
    const row = screen.getByText("키워드 동점 책").closest("tr");
    const cells = within(row).getAllByRole("cell");
    const [, , , recommend1, recommend2] = cells;
    expect(within(recommend1).getByText("후보 동률")).toBeTruthy();
    expect(within(recommend1).getByText("3_SF")).toBeTruthy();
    expect(within(recommend2).getByText("5_음악")).toBeTruthy();
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

  it("apply_status가 moved인 행만 체크박스가 비활성이고, moving은 재시도할 수 있다", () => {
    // I4: moving(서버가 죽어 결과를 모름)은 사람이 재확인할 수 있어야 하므로 목적지가
    // 있으면 체크박스가 켜진다. moved만 재이동 대상에서 빠진다.
    const items = [
      { ...ITEMS[0], apply_status: "moving" },
      { ...ITEMS[1], apply_status: "moved" },
    ];
    renderTable({ items, selection: new Set(["A/a.epub", "A/b.epub"]) });

    expect(screen.getByLabelText("A/a.epub 선택").disabled).toBe(false);
    expect(screen.getByLabelText("A/b.epub 선택").disabled).toBe(true);

    const movingBadge = screen.getByText("이동 중(중단됨)");
    const movedBadge = screen.getByText("이동 완료");
    // 같은 상태 배지 클래스를 공유하지 않아야 moving이 moved/대기와 섞여 보이지 않는다.
    expect(movingBadge.className).not.toBe(movedBadge.className);
  });

  it("failed 행도 목적지가 있으면 체크박스가 켜져 재시도할 수 있다", () => {
    // C1: 실패한(거부/오류) 행을 다시 체크해 승인을 누를 수 있어야 재시도가 된다.
    const items = [
      { ...ITEMS[0], apply_status: "failed", apply_error: "대상 폴더 없음" },
    ];
    renderTable({ items, selection: new Set() });

    expect(screen.getByLabelText("A/a.epub 선택").disabled).toBe(false);
  });

  it("모델과 목적지가 다르면 점수 칸에 모델 카테고리를 함께 보여준다", () => {
    // I3: 키워드가 고른 목적지와 모델이 자신 있게 가리킨 카테고리가 다르면, 점수가
    // 어느 카테고리의 확신도인지 화면에서 구분돼야 한다.
    const items = [
      {
        ...ITEMS[1],
        target_category: "5_음악",
        model_category: "3_SF",
        confidence: 0.91,
      },
    ];
    renderTable({ items });

    const row = screen.getByText("애매한 책").closest("tr");
    expect(within(row).getByText("0.91")).toBeTruthy();
    expect(within(row).getByText(/모델: 3_SF/)).toBeTruthy();
  });

  it("모델과 목적지가 같으면 점수 칸에 모델 배지를 따로 보여주지 않는다", () => {
    const items = [
      {
        ...ITEMS[0],
        target_category: "3_SF",
        model_category: "3_SF",
        confidence: 0.91,
      },
    ];
    renderTable({ items });

    const row = screen.getByText("확실한 책").closest("tr");
    expect(within(row).queryByText(/모델:/)).toBeNull();
  });

  it("목적지가 categories 목록에 없어도 셀렉트 옵션으로 끼워 넣는다", () => {
    // Minor B: ES 문서가 0건이라 카테고리 목록에서 빠진 목적지도 선택된 값으로
    // 보여야 한다 — 그러지 않으면 화면은 "(선택 안 함)"인데 실제 상태는 그
    // 목적지를 들고 있어, 체크한 행이 안 보이는 곳으로 승인될 수 있다.
    const items = [{ ...ITEMS[0], target_category: "9_없는카테고리" }];
    renderTable({ items });

    const select = screen.getByLabelText("A/a.epub 목적지");
    expect(select.value).toBe("9_없는카테고리");
    expect(
      within(select).getByRole("option", { name: "9_없는카테고리" }),
    ).toBeTruthy();
  });
});
