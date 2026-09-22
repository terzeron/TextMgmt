// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  cleanup,
  within,
} from "@testing-library/react";
import ClassifyProposalTable, {
  resolveTarget,
  isSelectable,
  isAlreadyInCurrentCategory,
} from "../src/ClassifyProposalTable";

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
      { category: "1_서양고전", source: "bookstore", detail: "서점 1곳 일치" },
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

// 확실한 행은 부모가 추천 1을 미리 체크해 넘긴다.
const FIRST_CHECKED = {
  "A/a.epub": { source: "candidate", index: 0, category: "3_SF" },
};

function renderTable(overrides = {}) {
  const props = {
    items: ITEMS,
    categories: CATEGORIES,
    choices: FIRST_CHECKED,
    onChoicesChange: vi.fn(),
    ...overrides,
  };
  render(<ClassifyProposalTable {...props} />);
  return props;
}

describe("ClassifyProposalTable", () => {
  it("체크박스가 추천 1·추천 2 열 안에 있고, 맨 앞 선택 열은 없다", () => {
    renderTable();
    const row = screen.getByText("확실한 책").closest("tr");
    const cells = within(row).getAllByRole("cell");
    // 열 순서: 책, 추천1, 추천2, 직접 선택, 예상 정답률, 이동 상태
    expect(cells).toHaveLength(6);
    expect(cells[0].textContent).toBe("확실한 책");
    expect(
      within(cells[1]).getByLabelText("A/a.epub 추천 1 선택"),
    ).toBeTruthy();
    // "현재" 열은 없다 — 표가 한 디렉토리로 한정돼 모든 행이 같은 값이다.
    expect(screen.queryByRole("columnheader", { name: "현재" })).toBeNull();
  });

  it("등급에 따라 추천 체크 상태가 다르다", () => {
    renderTable();
    expect(screen.getByLabelText("A/a.epub 추천 1 선택").checked).toBe(true);
    expect(screen.getByLabelText("A/b.epub 추천 1 선택").checked).toBe(false);
    expect(screen.getByLabelText("A/b.epub 추천 1 선택").disabled).toBe(false);
    // 불확실 행은 미리 체크되지 않는다 — 시스템이 못 정한 것이 저절로 승인되면 안 된다.
    expect(screen.getByLabelText("A/c.epub 추천 1 선택").checked).toBe(false);
    expect(screen.getByLabelText("A/c.epub 추천 2 선택").checked).toBe(false);
  });

  it("후보 동률 행도 추천 중 하나를 눌러 고를 수 있다", () => {
    // 동률은 "시스템이 둘 중 하나를 못 골랐다"는 뜻이지 후보가 틀렸다는 뜻이 아니다.
    // 둘 다 잠그면 답이 바로 옆 칸에 보이는데도 2,000개짜리 드롭다운에서 같은 이름을
    // 다시 찾아야 한다. 못 고른 판단을 사람이 대신 내리는 것이 이 체크박스의 용도다.
    const props = renderTable();
    expect(screen.getByLabelText("A/c.epub 추천 1 선택").disabled).toBe(false);
    expect(screen.getByLabelText("A/c.epub 추천 2 선택").disabled).toBe(false);

    fireEvent.click(screen.getByLabelText("A/c.epub 추천 2 선택"));

    const next = props.onChoicesChange.mock.calls[0][0];
    expect(next["A/c.epub"]).toEqual({
      source: "candidate",
      index: 1,
      category: "1_서양고전",
    });
    expect(isSelectable(ITEMS[2], next)).toBe(true);
  });

  it("직접 선택 목록이 가나다순으로 정렬된다", () => {
    // /categories 응답은 문서 수 내림차순이라 그대로 쓰면 이름으로 찾을 수 없다.
    renderTable({ categories: ["3_SF", "5_음악", "1_서양고전"] });
    const select = screen.getByLabelText("A/a.epub 목적지");
    const names = within(select)
      .getAllByRole("option")
      .map((option) => option.textContent)
      .filter((name) => name !== "(선택 안 함)");
    expect(names).toEqual(["1_서양고전", "3_SF", "5_음악"]);
  });

  it("추천 2를 체크하면 목적지가 후보 2가 되고 추천 1은 풀린다", () => {
    const props = renderTable({
      choices: {
        "A/b.epub": { source: "candidate", index: 0, category: "5_음악" },
      },
    });
    expect(screen.getByLabelText("A/b.epub 추천 1 선택").checked).toBe(true);

    fireEvent.click(screen.getByLabelText("A/b.epub 추천 2 선택"));

    const next = props.onChoicesChange.mock.calls[0][0];
    expect(next["A/b.epub"]).toEqual({
      source: "candidate",
      index: 1,
      category: "1_서양고전",
    });

    // 한 행의 목적지는 하나다 — 바뀐 상태를 렌더하면 추천 1은 꺼져 있어야 한다.
    cleanup();
    renderTable({ choices: next });
    expect(screen.getByLabelText("A/b.epub 추천 1 선택").checked).toBe(false);
    expect(screen.getByLabelText("A/b.epub 추천 2 선택").checked).toBe(true);
  });

  it("후보가 하나뿐인 행에는 추천 2 체크박스가 없다", () => {
    renderTable();
    expect(screen.queryByLabelText("A/a.epub 추천 2 선택")).toBeNull();
  });

  it("체크된 추천을 다시 누르면 그 행의 목적지가 사라진다", () => {
    const props = renderTable();
    fireEvent.click(screen.getByLabelText("A/a.epub 추천 1 선택"));
    const next = props.onChoicesChange.mock.calls[0][0];
    expect(next["A/a.epub"]).toBeUndefined();
  });

  it("직접 선택을 바꾸면 추천 체크가 모두 풀린다", () => {
    const props = renderTable();
    fireEvent.change(screen.getByLabelText("A/a.epub 목적지"), {
      target: { value: "5_음악" },
    });
    const next = props.onChoicesChange.mock.calls[0][0];
    expect(next["A/a.epub"]).toEqual({ source: "manual", category: "5_음악" });

    // 바뀐 상태를 그대로 렌더하면 추천 체크는 꺼져 있어야 한다.
    cleanup();
    renderTable({ choices: next });
    expect(screen.getByLabelText("A/a.epub 추천 1 선택").checked).toBe(false);
    expect(screen.getByLabelText("A/a.epub 목적지").value).toBe("5_음악");
  });

  it("추천을 체크한 행의 직접 선택 칸은 비어 있다", () => {
    // 추천 카테고리를 셀렉트에도 채우면 사용자가 직접 지정한 것처럼 보인다.
    renderTable();
    expect(screen.getByLabelText("A/a.epub 목적지").value).toBe("");
  });

  it("직접 선택을 비우면 그 행은 승인 대상에서 빠진다", () => {
    const props = renderTable({
      choices: { "A/c.epub": { source: "manual", category: "1_서양고전" } },
    });
    fireEvent.change(screen.getByLabelText("A/c.epub 목적지"), {
      target: { value: "" },
    });
    const next = props.onChoicesChange.mock.calls[0][0];
    expect(next["A/c.epub"]).toBeUndefined();
  });

  it("불확실 행도 직접 선택으로 목적지를 주면 승인 대상이 된다", () => {
    const choices = {
      "A/c.epub": { source: "manual", category: "1_서양고전" },
    };
    expect(isSelectable(ITEMS[2], choices)).toBe(true);
    expect(resolveTarget(ITEMS[2], choices)).toBe("1_서양고전");
  });

  it("추천 1 헤더 체크박스는 일괄 선택이 아니라 필터다", () => {
    // 헤더 체크박스를 눌러도 선택 상태(choices)는 안 바뀐다. 이 칸의 역할은
    // 행을 고르는 것이 아니라 보여 줄 행을 거르는 것이다.
    const props = renderTable();
    fireEvent.click(screen.getByLabelText("추천 1 필터"));
    expect(props.onChoicesChange).not.toHaveBeenCalled();
  });

  it("추천 후보 2개가 각 열에 나오고, 1개면 두 번째 열은 -", () => {
    const items = [ITEMS[0], ITEMS[2]];
    renderTable({ items });
    const row = screen.getByText("확실한 책").closest("tr");
    const cells = within(row).getAllByRole("cell");
    expect(within(cells[1]).getByText("3_SF")).toBeTruthy();
    expect(within(cells[1]).getByText("모델 판정")).toBeTruthy();
    expect(cells[2].textContent).toBe("-");

    const tieRow = screen.getByText("불확실한 책").closest("tr");
    const tieCells = within(tieRow).getAllByRole("cell");
    expect(within(tieCells[2]).getByText("1_서양고전")).toBeTruthy();
  });

  it("서점 판정이 갈린 행은 두 추천이 다른 답임을 배지로 드러낸다", () => {
    renderTable();
    const row = screen.getByText("불확실한 책").closest("tr");
    const cells = within(row).getAllByRole("cell");
    expect(within(cells[1]).getByText("후보 동률")).toBeTruthy();
    expect(within(cells[1]).getByText("5_음악")).toBeTruthy();
    expect(within(cells[2]).getByText("1_서양고전")).toBeTruthy();
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
    expect(within(cells[1]).getByText("후보 동률")).toBeTruthy();
    expect(within(cells[1]).getByText("3_SF")).toBeTruthy();
    expect(within(cells[2]).getByText("5_음악")).toBeTruthy();
  });

  it("아직 분류 안 된 행도 이름이 보이고 '분류 중'으로 구분된다", () => {
    // 제안이 시작되면 대상 파일의 이름 행이 먼저 깔리고 분류 결과가 뒤이어 채워진다.
    // 그 사이 행을 "-"로만 두면 "후보가 없다"와 구분되지 않는다.
    const items = [
      {
        file_path: "A/z.epub",
        title: "아직 분류 안 된 책",
        current_category: "A",
        target_category: null,
        grade: null,
        confidence: null,
        candidates: [],
        apply_status: "pending",
      },
    ];
    renderTable({ items, choices: {} });

    expect(screen.getByText("아직 분류 안 된 책")).toBeTruthy();
    const row = screen.getByText("아직 분류 안 된 책").closest("tr");
    const cells = within(row).getAllByRole("cell");
    expect(within(cells[1]).getByText("분류 중…")).toBeTruthy();
    expect(cells[2].textContent).toBe("-");
    expect(screen.queryByLabelText("A/z.epub 추천 1 선택")).toBeNull();
  });

  it("이동 상태가 대기/이동 중(중단됨)/이동 완료/실패:사유로 나온다", () => {
    const items = [
      { ...ITEMS[0], apply_status: "pending" },
      { ...ITEMS[1], apply_status: "moving" },
      { ...ITEMS[2], target_category: "5_음악", apply_status: "moved" },
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

  it("apply_status가 moved인 행만 잠기고, moving은 재시도할 수 있다", () => {
    // I4: moving(서버가 죽어 결과를 모름)은 사람이 재확인할 수 있어야 하므로 추천을
    // 다시 체크할 수 있다. moved만 재이동 대상에서 빠진다.
    const items = [
      { ...ITEMS[0], apply_status: "moving" },
      { ...ITEMS[1], apply_status: "moved" },
    ];
    renderTable({ items });

    expect(screen.getByLabelText("A/a.epub 추천 1 선택").disabled).toBe(false);
    expect(screen.getByLabelText("A/b.epub 추천 1 선택").disabled).toBe(true);
    expect(screen.getByLabelText("A/b.epub 목적지").disabled).toBe(true);

    const movingBadge = screen.getByText("이동 중(중단됨)");
    const movedBadge = screen.getByText("이동 완료");
    // 같은 상태 배지 클래스를 공유하지 않아야 moving이 moved/대기와 섞여 보이지 않는다.
    expect(movingBadge.className).not.toBe(movedBadge.className);
  });

  it("failed 행도 추천을 다시 체크해 재시도할 수 있다", () => {
    // C1: 실패한(거부/오류) 행을 다시 체크해 승인을 누를 수 있어야 재시도가 된다.
    const items = [
      { ...ITEMS[0], apply_status: "failed", apply_error: "대상 폴더 없음" },
    ];
    renderTable({ items, choices: {} });

    expect(screen.getByLabelText("A/a.epub 추천 1 선택").disabled).toBe(false);
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
    // 보정이 없으면 확신도를 셋째 자리까지 보여준다.
    expect(within(row).getByText("0.910")).toBeTruthy();
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

  it("직접 지정한 목적지가 categories 목록에 없어도 셀렉트 옵션으로 끼워 넣는다", () => {
    // Minor B: ES 문서가 0건이라 카테고리 목록에서 빠진 목적지도 선택된 값으로
    // 보여야 한다 — 그러지 않으면 화면은 "(선택 안 함)"인데 실제 상태는 그
    // 목적지를 들고 있어, 화면에 안 보이는 곳으로 승인될 수 있다.
    renderTable({
      choices: {
        "A/a.epub": { source: "manual", category: "9_없는카테고리" },
      },
    });

    const select = screen.getByLabelText("A/a.epub 목적지");
    expect(select.value).toBe("9_없는카테고리");
    expect(
      within(select).getByRole("option", { name: "9_없는카테고리" }),
    ).toBeTruthy();
  });

  // ── 정렬 ──

  function titleOrder() {
    return screen
      .getAllByRole("row")
      .slice(1)
      .map((row) => within(row).getAllByRole("cell")[0].textContent);
  }

  it("책·추천1·추천2·점수·이동 상태에 정렬 버튼이 있고 직접 선택에는 없다", () => {
    renderTable();
    for (const label of ["책", "추천 1", "추천 2", "예상 정답률", "이동 상태"]) {
      expect(screen.getByLabelText(`${label} 정렬`)).toBeTruthy();
    }
    expect(screen.queryByLabelText("직접 선택 정렬")).toBeNull();
  });

  it("정렬 전에는 부모가 준 순서를 그대로 쓴다", () => {
    // 제안이 도착한 순서가 곧 분류가 끝난 순서다. 안 누른 상태에서 순서를 바꾸면
    // 방금 채워진 행이 어디로 갔는지 알 수 없다.
    renderTable();
    expect(titleOrder()).toEqual(["확실한 책", "애매한 책", "불확실한 책"]);
  });

  it("책 정렬 버튼이 오름차순과 내림차순을 번갈아 적용한다", () => {
    renderTable();
    const button = screen.getByLabelText("책 정렬");

    // 가나다순은 불 → 애 → 확이다.
    fireEvent.click(button);
    expect(titleOrder()).toEqual(["불확실한 책", "애매한 책", "확실한 책"]);
    fireEvent.click(button);
    expect(titleOrder()).toEqual(["확실한 책", "애매한 책", "불확실한 책"]);
  });

  it("정렬 중인 열에만 aria-sort와 방향 화살표를 붙인다", () => {
    renderTable();
    const header = () => screen.getByRole("columnheader", { name: /^책/ });
    expect(header().getAttribute("aria-sort")).toBeNull();
    expect(screen.getByLabelText("책 정렬").textContent).toBe("↕");

    fireEvent.click(screen.getByLabelText("책 정렬"));
    expect(header().getAttribute("aria-sort")).toBe("ascending");
    expect(screen.getByLabelText("책 정렬").textContent).toBe("↑");

    fireEvent.click(screen.getByLabelText("책 정렬"));
    expect(header().getAttribute("aria-sort")).toBe("descending");
    expect(screen.getByLabelText("책 정렬").textContent).toBe("↓");
  });

  it("점수가 없는 행은 오름차순·내림차순 모두 뒤로 간다", () => {
    // 빈 칸이 맨 위에 몰리면 정렬을 눌러도 보려던 값이 화면 밖으로 밀린다.
    renderTable();
    const button = screen.getByLabelText("예상 정답률 정렬");

    fireEvent.click(button);
    expect(titleOrder()).toEqual(["애매한 책", "확실한 책", "불확실한 책"]);
    fireEvent.click(button);
    expect(titleOrder()).toEqual(["확실한 책", "애매한 책", "불확실한 책"]);
  });

  it("이동 상태는 문자열이 아니라 진행 순서로 정렬한다", () => {
    const items = [
      { ...ITEMS[0], apply_status: "failed" },
      { ...ITEMS[1], apply_status: "pending" },
      { ...ITEMS[2], apply_status: "moved" },
    ];
    renderTable({ items });

    fireEvent.click(screen.getByLabelText("이동 상태 정렬"));
    expect(titleOrder()).toEqual(["애매한 책", "불확실한 책", "확실한 책"]);
  });

  it("후보가 사라진 행의 옛 선택은 체크로 세지 않는다", () => {
    // 선택은 file_path로만 기억하는데 후보 목록은 제안이 갱신되면 바뀐다.
    // 추천 2 칸이 "-"라 체크박스도 없는 행이 추천 2 필터에 걸려 결과에 섞이던 결함.
    const noCandidate = { ...ITEMS[0], file_path: "A/x.epub", title: "후보없는책", candidates: [] };
    renderTable({
      items: [noCandidate],
      choices: { "A/x.epub": { source: "candidate", index: 1, category: "5_음악" } },
    });
    const row = screen.getByText("후보없는책").closest("tr");
    expect(within(row).getAllByRole("cell")[2].textContent).toBe("-");

    fireEvent.click(screen.getByLabelText("추천 2 필터"));
    expect(screen.getByText("필터 조건에 맞는 행이 없습니다.")).toBeTruthy();
  });

  it("후보가 줄어든 행도 같은 규칙으로 센다", () => {
    // 후보가 2개였다가 1개로 줄면, 추천 2의 옛 선택은 더 이상 체크가 아니다.
    const shrunk = { ...ITEMS[1], candidates: [ITEMS[1].candidates[0]] };
    renderTable({
      items: [shrunk],
      choices: { "A/b.epub": { source: "candidate", index: 1, category: "1_서양고전" } },
    });
    fireEvent.click(screen.getByLabelText("추천 2 필터"));
    expect(screen.getByText("필터 조건에 맞는 행이 없습니다.")).toBeTruthy();
  });

  // ── 편집 링크 ──

  it("이동 상태 칸에 연필 편집 링크가 있고 새 창으로 연다", () => {
    const open = vi.fn();
    const original = window.open;
    window.open = open;
    try {
      const items = [{ ...ITEMS[0], book_id: 4242 }];
      renderTable({ items, editBasePath: "/book-edit" });
      const row = screen.getByText("확실한 책").closest("tr");
      const cells = within(row).getAllByRole("cell");
      const link = within(cells[5]).getByLabelText("확실한 책 편집");
      // 레이블 없이 아이콘만 둔다 — 열이 좁고 상태 배지와 나란히 선다.
      expect(link.textContent).toBe("");
      fireEvent.click(link);
      expect(open).toHaveBeenCalledWith(
        "/book-edit/4242?category=A",
        "_blank",
        "noopener",
      );
    } finally {
      window.open = original;
    }
  });

  it("컨텐츠 타입에 따라 편집 경로가 바뀐다", () => {
    const open = vi.fn();
    const original = window.open;
    window.open = open;
    try {
      const items = [{ ...ITEMS[0], book_id: 7 }];
      renderTable({ items, editBasePath: "/comics-edit" });
      fireEvent.click(screen.getByLabelText("확실한 책 편집"));
      expect(open.mock.calls[0][0]).toBe("/comics-edit/7?category=A");
    } finally {
      window.open = original;
    }
  });

  it("book_id 가 없는 옛 제안 행에는 편집 링크를 그리지 않는다", () => {
    // 깨진 링크를 보여 주는 것보다 없는 편이 낫다.
    renderTable({ items: [ITEMS[0]] });
    expect(screen.queryByLabelText("확실한 책 편집")).toBeNull();
  });

  it("삭제 버튼을 누르면 그 행을 부모에게 넘긴다", () => {
    // confirm 과 실제 요청은 부모가 맡는다(CategoryAdmin.test.jsx 에서 검증).
    const onDeleteBook = vi.fn();
    renderTable({ items: [{ ...ITEMS[0], book_id: 11 }], onDeleteBook });
    fireEvent.click(screen.getByLabelText("확실한 책 삭제"));
    expect(onDeleteBook.mock.calls[0][0].file_path).toBe("A/a.epub");
  });

  it("삭제 중인 행은 스피너를 보이고 모든 삭제 버튼이 잠긴다", () => {
    const items = [
      { ...ITEMS[0], book_id: 11 },
      { ...ITEMS[1], book_id: 22 },
    ];
    renderTable({
      items,
      onDeleteBook: vi.fn(),
      deletingFilePath: "A/a.epub",
    });

    const running = screen.getByLabelText("확실한 책 삭제");
    const other = screen.getByLabelText("애매한 책 삭제");
    expect(running.getAttribute("title")).toBe("삭제 중...");
    expect(running.getAttribute("aria-busy")).toBe("true");
    // 두 건이 겹쳐 돌면 어느 행이 지워졌는지 알 수 없다.
    expect(running.disabled).toBe(true);
    expect(other.disabled).toBe(true);
    expect(other.getAttribute("title")).toBe("삭제");
  });

  it("book_id 가 없거나 onDeleteBook 이 없으면 삭제 버튼을 그리지 않는다", () => {
    renderTable({ items: [{ ...ITEMS[0], book_id: 11 }] });
    expect(screen.queryByLabelText("확실한 책 삭제")).toBeNull();

    cleanup();
    renderTable({ items: [ITEMS[0]], onDeleteBook: vi.fn() });
    expect(screen.queryByLabelText("확실한 책 삭제")).toBeNull();
  });

  it("추천 1·추천 2 열로 정렬하면 그 열의 후보 이름순이 된다", () => {
    renderTable();
    fireEvent.click(screen.getByLabelText("추천 1 정렬"));
    // 확실한 책=3_SF, 애매한 책=5_음악, 불확실한 책=5_음악
    expect(titleOrder()[0]).toBe("확실한 책");

    fireEvent.click(screen.getByLabelText("추천 2 정렬"));
    // 확실한 책은 추천 2가 없어 뒤로 밀린다.
    expect(titleOrder()[titleOrder().length - 1]).toBe("확실한 책");
  });

  it("같은 열을 다시 누르면 내림차순, 한 번 더 누르면 오름차순으로 돌아온다", () => {
    renderTable();
    const button = screen.getByLabelText("추천 1 정렬");
    fireEvent.click(button);
    const asc = titleOrder();
    fireEvent.click(button);
    expect(titleOrder()).not.toEqual(asc);
    fireEvent.click(button);
    expect(titleOrder()).toEqual(asc);
  });

  it("제목이 없는 행은 파일 경로로 보이고 그 값으로 정렬한다", () => {
    // 분류가 끝나기 전 자리표시자나 메타데이터가 빈 파일은 title 이 없을 수 있다.
    const items = [
      { ...ITEMS[0], title: "", file_path: "A/zz.epub" },
      { ...ITEMS[1], title: "", file_path: "A/aa.epub" },
    ];
    renderTable({ items, choices: {} });
    fireEvent.click(screen.getByLabelText("책 정렬"));
    expect(titleOrder()).toEqual(["A/aa.epub", "A/zz.epub"]);
  });

  it("candidates 키가 없는 행에 남은 옛 선택은 체크로 세지 않는다", () => {
    // candidates 가 [] 인 경우와 키 자체가 없는 경우를 모두 견뎌야 한다.
    const bare = {
      file_path: "A/bare.epub",
      title: "후보키없는책",
      current_category: "A",
      apply_status: "pending",
      grade: "unknown",
    };
    renderTable({
      items: [bare],
      choices: { "A/bare.epub": { source: "candidate", index: 0, category: "3_SF" } },
    });
    fireEvent.click(screen.getByLabelText("추천 1 필터"));
    expect(screen.getByText("필터 조건에 맞는 행이 없습니다.")).toBeTruthy();
  });

  it("후보 없는 행과 있는 행을 섞어 정렬하면 없는 쪽이 뒤로 간다", () => {
    // 비교 함수는 행이 둘 이상일 때만 돈다. 빈 칸은 오름·내림 어느 쪽이든 뒤로 간다.
    const bare = {
      file_path: "A/bare.epub",
      title: "후보키없는책",
      current_category: "A",
      apply_status: "pending",
      grade: "unknown",
    };
    const bare2 = { ...bare, file_path: "A/bare2.epub", title: "후보키없는책2" };
    renderTable({ items: [bare, ITEMS[0], bare2], choices: {} });

    fireEvent.click(screen.getByLabelText("추천 1 정렬"));
    expect(titleOrder()[0]).toBe("확실한 책");
    // 둘 다 빈 칸이면 원래 순서를 지킨다.
    expect(titleOrder().slice(1)).toEqual(["후보키없는책", "후보키없는책2"]);

    fireEvent.click(screen.getByLabelText("추천 1 정렬"));
    expect(titleOrder()[0]).toBe("확실한 책");
  });

  it("candidates 키가 아예 없는 행도 다루다 죽지 않는다", () => {
    // 옛 제안 레코드에는 candidates 가 없을 수 있다.
    const bare = {
      file_path: "A/bare.epub",
      title: "후보키없는책",
      current_category: "A",
      apply_status: "pending",
      grade: "unknown",
      confidence: null,
    };
    renderTable({ items: [bare], choices: {} });
    const cells = within(screen.getByText("후보키없는책").closest("tr")).getAllByRole("cell");
    expect(cells[1].textContent).toBe("-");
    expect(cells[2].textContent).toBe("-");

    fireEvent.click(screen.getByLabelText("추천 1 정렬"));
    expect(titleOrder()).toEqual(["후보키없는책"]);
  });

  it("모르는 이동 상태는 대기와 같은 자리로 정렬한다", () => {
    // 백엔드가 새 상태를 추가해도 표가 순서를 잃지 않아야 한다.
    const items = [
      { ...ITEMS[0], apply_status: "moved" },
      { ...ITEMS[1], apply_status: "처음보는상태" },
    ];
    renderTable({ items, choices: {} });
    fireEvent.click(screen.getByLabelText("이동 상태 정렬"));
    // 모르는 상태는 0(대기)으로 쳐서 moved(2)보다 앞에 온다.
    expect(titleOrder()).toEqual(["애매한 책", "확실한 책"]);
  });

  it("제목이 없는 행의 편집·삭제 버튼은 파일 경로로 이름을 짓는다", () => {
    const open = vi.fn();
    const original = window.open;
    window.open = open;
    try {
      const item = {
        ...ITEMS[0],
        title: "",
        current_category: "",
        book_id: 55,
      };
      renderTable({ items: [item], onDeleteBook: vi.fn(), choices: {} });
      fireEvent.click(screen.getByLabelText("A/a.epub 편집"));
      // current_category 가 비면 category 파라미터도 빈 값으로 나간다.
      expect(open.mock.calls[0][0]).toBe("/book-edit/55?category=");
      expect(screen.getByLabelText("A/a.epub 삭제")).toBeTruthy();
    } finally {
      window.open = original;
    }
  });

  // ── 필터 ──

  it("처음에는 세 필터가 모두 중간 상태라 전체 행이 보인다", () => {
    renderTable();
    for (const label of ["추천 1 필터", "추천 2 필터", "직접 선택 필터"]) {
      const box = screen.getByLabelText(label);
      expect(box.indeterminate).toBe(true);
      expect(box.checked).toBe(false);
    }
    expect(titleOrder()).toHaveLength(3);
  });

  it("추천 1 필터가 전체 → 체크된 행만 → 체크 안 된 행만 → 전체로 돈다", () => {
    renderTable();
    const box = screen.getByLabelText("추천 1 필터");

    fireEvent.click(box);
    expect(box.indeterminate).toBe(false);
    expect(box.checked).toBe(true);
    expect(titleOrder()).toEqual(["확실한 책"]);

    fireEvent.click(box);
    expect(box.indeterminate).toBe(false);
    expect(box.checked).toBe(false);
    expect(titleOrder()).toEqual(["애매한 책", "불확실한 책"]);

    fireEvent.click(box);
    expect(box.indeterminate).toBe(true);
    expect(titleOrder()).toEqual(["확실한 책", "애매한 책", "불확실한 책"]);
  });

  it("직접 선택 필터는 목적지를 직접 지정한 행을 가린다", () => {
    renderTable({
      choices: { "A/b.epub": { source: "manual", category: "1_서양고전" } },
    });
    const box = screen.getByLabelText("직접 선택 필터");

    fireEvent.click(box);
    expect(titleOrder()).toEqual(["애매한 책"]);
  });

  it("다른 열의 필터를 누르면 앞서 건 필터는 초기 상태로 돌아간다", () => {
    // 한 행의 목적지는 하나뿐이라 두 열을 겹쳐 걸면 대부분 빈 표가 된다.
    // 거르려다 아무것도 못 보는 상태에 빠지지 않도록 한 번에 하나만 건다.
    renderTable();
    const first = screen.getByLabelText("추천 1 필터");
    const second = screen.getByLabelText("추천 2 필터");

    fireEvent.click(first);
    expect(first.checked).toBe(true);
    expect(titleOrder()).toEqual(["확실한 책"]);

    fireEvent.click(second);
    expect(first.indeterminate).toBe(true);
    expect(first.checked).toBe(false);
    expect(second.checked).toBe(true);
    // 추천 2를 체크한 행이 없으니 추천 2 조건만 남아 빈 표가 된다.
    expect(screen.getByText("필터 조건에 맞는 행이 없습니다.")).toBeTruthy();
  });

  it("세 번째 열을 누르면 앞의 두 열이 모두 초기 상태가 된다", () => {
    renderTable({
      choices: {
        ...FIRST_CHECKED,
        "A/b.epub": { source: "manual", category: "1_서양고전" },
      },
    });
    const rec1 = screen.getByLabelText("추천 1 필터");
    const rec2 = screen.getByLabelText("추천 2 필터");
    const manual = screen.getByLabelText("직접 선택 필터");

    fireEvent.click(rec1);
    fireEvent.click(rec2);
    fireEvent.click(manual);

    expect(rec1.indeterminate).toBe(true);
    expect(rec2.indeterminate).toBe(true);
    expect(manual.checked).toBe(true);
    expect(titleOrder()).toEqual(["애매한 책"]);
  });

  it("같은 열을 계속 누르면 그 열에서만 3단계가 돈다", () => {
    renderTable();
    const rec2 = screen.getByLabelText("추천 2 필터");
    const rec1 = screen.getByLabelText("추천 1 필터");

    fireEvent.click(rec1);
    fireEvent.click(rec2); // 추천 1은 초기화, 추천 2는 true
    fireEvent.click(rec2); // 추천 2만 false 로
    expect(rec1.indeterminate).toBe(true);
    expect(rec2.checked).toBe(false);
    expect(rec2.indeterminate).toBe(false);
    expect(titleOrder()).toEqual(["확실한 책", "애매한 책", "불확실한 책"]);
  });

  it("제안이 아예 없으면 필터 안내 문구를 띄우지 않는다", () => {
    // 행이 0개인 것과 필터에 다 걸린 것은 다른 상황이다.
    renderTable({ items: [] });
    expect(screen.queryByText("필터 조건에 맞는 행이 없습니다.")).toBeNull();
  });

  it("필터와 정렬을 함께 걸면 거른 뒤 정렬한다", () => {
    renderTable();
    fireEvent.click(screen.getByLabelText("추천 1 필터"));
    fireEvent.click(screen.getByLabelText("추천 1 필터"));
    fireEvent.click(screen.getByLabelText("책 정렬"));

    expect(titleOrder()).toEqual(["불확실한 책", "애매한 책"]);
  });
});

describe("제자리 제안 표시", () => {
  const SAME = {
    file_path: "A/same.epub",
    title: "제자리 책",
    current_category: "A",
    target_category: "A",
    grade: "certain",
    confidence: 0.95,
    source: "model",
    reason: "모델 판정",
    candidates: [{ category: "A", source: "model", detail: "모델 판정" }],
    apply_status: "pending",
  };

  it("추천 1이 현재 디렉토리와 같으면 제자리 행으로 본다", () => {
    expect(isAlreadyInCurrentCategory(SAME)).toBe(true);
    expect(isAlreadyInCurrentCategory(ITEMS[0])).toBe(false);
    // 후보가 아직 없는 행은 제자리 행이 아니다 — 분류가 안 끝났을 뿐이다.
    expect(
      isAlreadyInCurrentCategory({ current_category: "A", candidates: [] }),
    ).toBe(false);
  });

  it("제자리 행에만 밝은 회색 배경을 깐다", () => {
    renderTable({ items: [...ITEMS, SAME], choices: {} });
    expect(screen.getByText("제자리 책").closest("tr").className).toContain(
      "table-secondary",
    );
    expect(screen.getByText("확실한 책").closest("tr").className).not.toContain(
      "table-secondary",
    );
  });

  it("실패 행은 제자리라도 실패 색을 유지한다", () => {
    // 손봐야 할 행을 회색에 묻으면 안 된다.
    renderTable({
      items: [{ ...SAME, apply_status: "failed", apply_error: "권한 없음" }],
      choices: {},
    });
    const row = screen.getByText("제자리 책").closest("tr");
    expect(row.className).toContain("table-danger");
    expect(row.className).not.toContain("table-secondary");
  });

  // 점수 표기 — 확신도 대신 예상 정답률
  //
  // 확신도는 79개 클래스 softmax라 실제 범위가 0.013~0.13이다. 소수 둘째 자리로
  // 자르면 거의 모든 행이 0.05·0.06·0.07 세 값으로 뭉개져 열이 무의미해진다.
  it("예상 정답률이 있으면 그 값을 소수로 보여준다", () => {
    renderTable({
      items: [
        {
          ...ITEMS[0],
          confidence: 0.076,
          expected_accuracy: 0.93,
        },
      ],
      choices: {},
    });
    const row = screen.getByText("확실한 책").closest("tr");
    const cells = within(row).getAllByRole("cell");
    expect(cells[4].textContent).toContain("0.93");
    expect(cells[4].textContent).not.toContain("%");
  });

  it("예상 정답률이 없으면 확신도를 소수 셋째 자리까지 보여준다", () => {
    // 보정이 없는 옛 모델. 0.05와 0.07을 구분할 자릿수는 남겨야 한다.
    renderTable({
      items: [
        {
          ...ITEMS[0],
          confidence: 0.076,
          expected_accuracy: null,
        },
      ],
      choices: {},
    });
    const row = screen.getByText("확실한 책").closest("tr");
    const cells = within(row).getAllByRole("cell");
    expect(cells[4].textContent).toContain("0.076");
  });

  it("점수 정렬은 예상 정답률을 기준으로 한다", () => {
    // 확신도 순서와 예상 정답률 순서가 어긋나게 둔다. 화면에 보이는 숫자로 정렬해야
    // 관리자가 본 대로 줄이 선다.
    renderTable({
      items: [
        { ...ITEMS[0], title: "낮은 정답률", confidence: 0.12, expected_accuracy: 0.6 },
        { ...ITEMS[1], title: "높은 정답률", confidence: 0.08, expected_accuracy: 0.95 },
      ],
      choices: {},
    });
    fireEvent.click(screen.getByLabelText("예상 정답률 정렬"));
    const titles = screen
      .getAllByRole("row")
      .slice(1)
      .map((row) => within(row).getAllByRole("cell")[0].textContent);
    expect(titles[0]).toBe("낮은 정답률");
  });
});
