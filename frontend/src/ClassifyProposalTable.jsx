/* eslint-disable react-refresh/only-export-components --
   resolveTarget/isSelectable 등은 부모와 테스트가 재사용하는 순수 헬퍼라
   컴포넌트와 co-located. HMR 힌트일 뿐 런타임 영향 없음. */
import { useEffect, useMemo, useRef, useState } from "react";
import PropTypes from "prop-types";
import { Table, Form, Badge } from "react-bootstrap";

// 한 행의 목적지는 하나다. 그래서 선택 상태를 "체크된 행 집합 + 목적지 맵" 두 개로
// 나누지 않고, 행마다 {어디서 왔는가, 어느 카테고리인가} 하나만 들고 있는다.
//
//   choices[file_path] = { source: "candidate", index: 0 | 1, category: "3_SF" }
//                      | { source: "manual", category: "5_에세이" }
//
// 이렇게 두면 "추천 1 체크"와 "직접 선택"이 동시에 켜지는 상태가 아예 만들어지지
// 않는다. 직접 선택을 바꾸면 source가 manual로 덮이므로 추천 체크는 저절로 꺼진다.

export function resolveTarget(item, choices) {
  return choices[item.file_path]?.category ?? "";
}

export function isSelectable(item, choices) {
  // 이미 옮긴(moved) 행만 재이동 대상에서 뺀다. moving(서버가 죽어 결과를 모름)과
  // failed(거부·실패)는 재시도할 수 있어야 한다 — apply_category_changes가 이동
  // 직전 원본 존재를 다시 확인하므로, 이미 끝난 이동을 moving으로 재시도해도
  // 이중 이동 없이 안전하게 failed로 남는다.
  if (item.apply_status === "moved") {
    return false;
  }
  return Boolean(resolveTarget(item, choices));
}

export function isCandidateChecked(item, choices, index) {
  const choice = choices[item.file_path];
  return choice?.source === "candidate" && choice.index === index;
}

// 직접 선택 셀렉트박스가 보여줄 값. 추천을 체크한 행은 비어 있어야 한다 —
// 추천 카테고리를 여기에도 채워 넣으면 사용자가 직접 지정한 것처럼 보인다.
export function manualValue(item, choices) {
  const choice = choices[item.file_path];
  return choice?.source === "manual" ? choice.category : "";
}

// 추천 체크박스를 누를 수 있는가. 후보가 있고 아직 안 옮긴 행이면 누를 수 있다.
//
// 등급은 여기서 보지 않는다. 불확실(unknown)은 "시스템이 둘 중 하나를 못 골랐다"는
// 뜻이지 "후보가 틀렸다"는 뜻이 아니다. 후보 동률 행에서 둘 다 잠가 버리면, 답이
// 바로 옆 칸에 보이는데도 2,000개가 넘는 드롭다운에서 같은 이름을 다시 찾아야 한다.
// 못 고른 판단을 사람이 대신 내리는 것이 이 체크박스의 용도다.
//
// "시스템이 못 정한 것이 저절로 승인되면 안 된다"는 원래 의도는 기본값으로 지킨다:
// 미리 체크되는 것은 확실(certain) 행의 추천 1뿐이고, 불확실 행은 사람이 직접
// 누르지 않으면 승인 대상에 들어가지 않는다.
export function canCheckCandidate(item, candidate) {
  return Boolean(candidate?.category && item.apply_status !== "moved");
}

// apply_status -> 화면 표시. moving은 백엔드가 일부러 자동 판정하지 않고 남긴,
// 사람이 직접 확인해야 하는 유일한 상태라 다른 상태와 색을 구분한다.
function ApplyStatusBadge({ item }) {
  switch (item.apply_status) {
    case "moving":
      return (
        <Badge bg="warning" text="dark">
          이동 중(중단됨)
        </Badge>
      );
    case "moved":
      return <Badge bg="success">이동 완료</Badge>;
    case "failed":
      return <Badge bg="danger">실패: {item.apply_error}</Badge>;
    case "pending":
    default:
      return <Badge bg="secondary">대기</Badge>;
  }
}

ApplyStatusBadge.propTypes = {
  item: PropTypes.object.isRequired,
};

// ── 정렬 ──
//
// 정렬과 필터는 화면 표시에만 쓴다. 승인 대상은 부모가 원본 items와 choices로
// 계산하므로, 여기서 순서를 바꾸거나 행을 숨겨도 무엇이 승인되는지는 안 변한다.

// 이동 상태는 문자열을 그대로 정렬하면 "failed, moved, moving, pending"이라는
// 뜻 없는 순서가 된다. 사람이 읽는 진행 순서(대기 → 이동 중 → 완료 → 실패)로
// 등급을 매긴다. 실패를 끝에 두는 이유는 정렬 한 번으로 손봐야 할 행을 모으기
// 위해서다.
const APPLY_STATUS_ORDER = {
  pending: 0,
  moving: 1,
  moved: 2,
  failed: 3,
};

// 정렬 키 -> 그 열이 비교할 값. 값이 없으면 null을 준다.
export const SORT_COLUMNS = {
  title: (item) => item.title || item.file_path,
  candidate0: (item) => (item.candidates || [])[0]?.category ?? null,
  candidate1: (item) => (item.candidates || [])[1]?.category ?? null,
  confidence: (item) => item.confidence ?? null,
  applyStatus: (item) => APPLY_STATUS_ORDER[item.apply_status] ?? 0,
};

function compareValues(a, b) {
  if (typeof a === "number" && typeof b === "number") return a - b;
  // 카테고리와 제목은 한글이라 코드포인트 순서로는 가나다순이 안 나온다.
  return String(a).localeCompare(String(b), "ko");
}

export function sortItems(items, sort) {
  const read = sort?.key ? SORT_COLUMNS[sort.key] : null;
  if (!read) return items;
  const sign = sort.dir === "desc" ? -1 : 1;
  return [...items].sort((left, right) => {
    const a = read(left);
    const b = read(right);
    // 빈 칸은 오름·내림 어느 쪽이든 뒤로 보낸다. 방향을 뒤집을 때 결과를 통째로
    // 뒤집으면 빈 칸이 맨 위에 몰려, 정렬을 눌러도 보려던 값이 화면 밖으로 밀린다.
    if (a == null || b == null) {
      if (a == null && b == null) return 0;
      return a == null ? 1 : -1;
    }
    return sign * compareValues(a, b);
  });
}

// ── 필터 ──
//
// 필터 값은 3단계다: null이면 거르지 않고, true면 그 열이 체크된 행만, false면
// 체크가 안 된 행만 남긴다. 체크박스 하나로 세 상태를 다 쓰려고 indeterminate를
// "거르지 않음"에 대응시킨다 — 2단계뿐이면 전체 행을 한 번에 보는 상태가 없다.

// 헤더 체크박스가 말하는 "이 열이 체크된 행"의 정의.
export const FILTER_COLUMNS = {
  candidate0: (item, choices) => isCandidateChecked(item, choices, 0),
  candidate1: (item, choices) => isCandidateChecked(item, choices, 1),
  manual: (item, choices) => Boolean(manualValue(item, choices)),
};

export function filterItems(items, choices, filters) {
  const active = Object.entries(FILTER_COLUMNS).filter(
    ([key]) => filters[key] != null,
  );
  if (active.length === 0) return items;
  return items.filter((item) =>
    active.every(
      ([key, isChecked]) => isChecked(item, choices) === filters[key],
    ),
  );
}

// 다음 필터 상태. 체크박스를 누를 때마다 전체 → 체크된 행만 → 체크 안 된 행만
// 순으로 돌고 전체로 돌아온다.
export function nextFilterValue(value) {
  if (value == null) return true;
  if (value === true) return false;
  return null;
}

// 헤더의 정렬 버튼. 아직 이 열로 정렬하지 않았으면 위아래 화살표를 같이 보여
// 눌러서 정렬할 수 있다는 것만 알리고, 정렬한 뒤에는 방향 하나만 남긴다.
function SortButton({ label, active, dir, onClick }) {
  const arrow = !active ? "↕" : dir === "asc" ? "↑" : "↓";
  return (
    <button
      type="button"
      className="btn btn-link btn-sm p-0 text-decoration-none"
      aria-label={`${label} 정렬`}
      onClick={onClick}
    >
      <span aria-hidden="true">{arrow}</span>
    </button>
  );
}

SortButton.propTypes = {
  label: PropTypes.string.isRequired,
  active: PropTypes.bool.isRequired,
  dir: PropTypes.string.isRequired,
  onClick: PropTypes.func.isRequired,
};

// 헤더의 필터 체크박스. indeterminate는 DOM 속성이라 JSX로는 못 주고 ref로만
// 설정할 수 있다 — Form.Check는 ref를 감싸는 div가 아니라 input에 그대로 넘긴다.
function FilterCheckbox({ label, value, onCycle }) {
  const inputRef = useRef(null);
  useEffect(() => {
    if (inputRef.current) inputRef.current.indeterminate = value == null;
  }, [value]);
  return (
    <Form.Check
      ref={inputRef}
      type="checkbox"
      aria-label={label}
      checked={value === true}
      onChange={onCycle}
    />
  );
}

FilterCheckbox.propTypes = {
  label: PropTypes.string.isRequired,
  value: PropTypes.bool,
  onCycle: PropTypes.func.isRequired,
};

// 추천 후보 한 칸: 체크박스 + 카테고리 + 근거. 후보가 없으면 "-".
function CandidateCell({ item, candidate, index, checked, onToggle }) {
  // 제안이 시작되면 대상 파일의 이름만 담긴 행이 먼저 깔리고, 분류가 끝나는 대로
  // 채워진다. 아직 안 채워진 행을 "-"로 두면 "후보가 없다"와 구분되지 않는다.
  if (!candidate) {
    if (index === 0 && item.grade == null) {
      return <small className="text-muted">분류 중…</small>;
    }
    return "-";
  }
  return (
    <div className="d-flex gap-2">
      <Form.Check
        type="checkbox"
        aria-label={`${item.file_path} 추천 ${index + 1} 선택`}
        checked={checked}
        disabled={!canCheckCandidate(item, candidate)}
        onChange={onToggle}
      />
      <div>
        <div>{candidate.category}</div>
        <small className="text-muted">{candidate.detail}</small>
      </div>
    </div>
  );
}

CandidateCell.propTypes = {
  item: PropTypes.object.isRequired,
  candidate: PropTypes.shape({
    category: PropTypes.string,
    source: PropTypes.string,
    detail: PropTypes.string,
  }),
  index: PropTypes.number.isRequired,
  checked: PropTypes.bool.isRequired,
  onToggle: PropTypes.func.isRequired,
};

export default function ClassifyProposalTable({
  items,
  categories,
  choices,
  onChoicesChange,
}) {
  // /categories 응답은 문서 수 내림차순이라 그대로 쓰면 드롭다운이 "3_판타지,
  // 3_무협, 3_여성향..." 순으로 뜬다. 2,000개가 넘는 목록에서 이름으로 찾으려면
  // 가나다순이어야 한다. 부르는 쪽 순서에 기대지 않도록 여기서 정렬한다.
  const sortedCategories = useMemo(
    () => [...categories].sort((a, b) => a.localeCompare(b, "ko")),
    [categories],
  );

  const setChoice = (filePath, choice) => {
    const next = { ...choices };
    if (choice) next[filePath] = choice;
    else delete next[filePath];
    onChoicesChange(next);
  };

  const toggleCandidate = (item, index) => {
    if (isCandidateChecked(item, choices, index)) {
      setChoice(item.file_path, null);
      return;
    }
    const candidate = (item.candidates || [])[index];
    setChoice(item.file_path, {
      source: "candidate",
      index,
      category: candidate.category,
    });
  };

  const changeManual = (item, value) => {
    setChoice(
      item.file_path,
      value ? { source: "manual", category: value } : null,
    );
  };

  // 정렬을 누르기 전에는 부모가 준 순서를 그대로 쓴다. 제안이 도착한 순서가
  // 곧 분류가 끝난 순서라, 아무 열도 안 누른 상태에서 순서를 바꾸면 방금 채워진
  // 행이 어디로 갔는지 알 수 없다.
  const [sort, setSort] = useState({ key: null, dir: "asc" });
  const [filters, setFilters] = useState({
    candidate0: null,
    candidate1: null,
    manual: null,
  });

  const toggleSort = (key) => {
    setSort((prev) =>
      prev.key === key
        ? { key, dir: prev.dir === "asc" ? "desc" : "asc" }
        : { key, dir: "asc" },
    );
  };

  const cycleFilter = (key) => {
    setFilters((prev) => ({ ...prev, [key]: nextFilterValue(prev[key]) }));
  };

  const visibleItems = useMemo(
    () => sortItems(filterItems(items, choices, filters), sort),
    [items, choices, filters, sort],
  );

  // aria-sort는 지금 정렬 중인 열에만 방향을 붙인다. 모든 열에 "none"을 달면
  // 화면 낭독기가 정렬 안 된 열까지 정렬 상태로 읽는다.
  const ariaSort = (key) =>
    sort.key === key
      ? sort.dir === "asc"
        ? "ascending"
        : "descending"
      : undefined;

  return (
    <Table size="sm" bordered hover responsive className="mt-2">
      <thead>
        <tr>
          {/* "현재" 열은 두지 않는다. 표가 선택한 디렉토리 하나로 한정돼 있어
              모든 행이 같은 값이고, 그 디렉토리 이름은 카드 머리에 이미 있다. */}
          <th aria-sort={ariaSort("title")}>
            <div className="d-flex gap-2 align-items-center">
              책
              <SortButton
                label="책"
                active={sort.key === "title"}
                dir={sort.dir}
                onClick={() => toggleSort("title")}
              />
            </div>
          </th>
          <th aria-sort={ariaSort("candidate0")}>
            <div className="d-flex gap-2 align-items-center">
              <FilterCheckbox
                label="추천 1 필터"
                value={filters.candidate0}
                onCycle={() => cycleFilter("candidate0")}
              />
              추천 1
              <SortButton
                label="추천 1"
                active={sort.key === "candidate0"}
                dir={sort.dir}
                onClick={() => toggleSort("candidate0")}
              />
            </div>
          </th>
          <th aria-sort={ariaSort("candidate1")}>
            <div className="d-flex gap-2 align-items-center">
              <FilterCheckbox
                label="추천 2 필터"
                value={filters.candidate1}
                onCycle={() => cycleFilter("candidate1")}
              />
              추천 2
              <SortButton
                label="추천 2"
                active={sort.key === "candidate1"}
                dir={sort.dir}
                onClick={() => toggleSort("candidate1")}
              />
            </div>
          </th>
          {/* 직접 선택은 정렬 대상이 아니다 — 주문한 정렬 열에 없다. */}
          <th>
            <div className="d-flex gap-2 align-items-center">
              <FilterCheckbox
                label="직접 선택 필터"
                value={filters.manual}
                onCycle={() => cycleFilter("manual")}
              />
              직접 선택
            </div>
          </th>
          <th aria-sort={ariaSort("confidence")}>
            <div className="d-flex gap-2 align-items-center">
              점수
              <SortButton
                label="점수"
                active={sort.key === "confidence"}
                dir={sort.dir}
                onClick={() => toggleSort("confidence")}
              />
            </div>
          </th>
          <th aria-sort={ariaSort("applyStatus")}>
            <div className="d-flex gap-2 align-items-center">
              이동 상태
              <SortButton
                label="이동 상태"
                active={sort.key === "applyStatus"}
                dir={sort.dir}
                onClick={() => toggleSort("applyStatus")}
              />
            </div>
          </th>
        </tr>
      </thead>
      <tbody>
        {/* 필터를 걸어 남은 행이 없으면 빈 표만 보인다. 제안이 없는 것인지
            필터에 걸린 것인지 구분되게 한 줄로 알린다. */}
        {items.length > 0 && visibleItems.length === 0 && (
          <tr>
            <td colSpan={6} className="text-center text-muted">
              필터 조건에 맞는 행이 없습니다.
            </td>
          </tr>
        )}
        {visibleItems.map((item) => {
          const manual = manualValue(item, choices);
          const candidates = item.candidates || [];
          // grade가 unknown이면서 후보가 2개면, 시스템이 둘 중 하나를 고르지
          // 못하고 동점으로 남겨 뒀다는 뜻이다 — 키워드 점수 동점(source: "keyword",
          // 근거: "여러 카테고리가 동일 점수로 일치합니다")과 서점 표 갈림(source:
          // "conflict", 근거: "서점 판정이 갈림")이 여기 해당한다. 후보 칸에 득표/점수만
          // 나열하면 "동률이라 못 골랐다"는 사실 자체가 안 보이므로 배지로 명시한다.
          // "서점"으로 못 박지 않는 이유는 두 원인 모두를 가리켜야 하기 때문이다.
          const isTie = item.grade === "unknown" && candidates.length === 2;
          return (
            <tr
              key={item.file_path}
              className={
                item.apply_status === "failed" ? "table-danger" : undefined
              }
            >
              <td>{item.title || item.file_path}</td>
              <td>
                {isTie && (
                  <Badge
                    bg="info"
                    className="d-block mb-1"
                    style={{ width: "fit-content" }}
                  >
                    후보 동률
                  </Badge>
                )}
                <CandidateCell
                  item={item}
                  candidate={candidates[0]}
                  index={0}
                  checked={isCandidateChecked(item, choices, 0)}
                  onToggle={() => toggleCandidate(item, 0)}
                />
              </td>
              <td>
                <CandidateCell
                  item={item}
                  candidate={candidates[1]}
                  index={1}
                  checked={isCandidateChecked(item, choices, 1)}
                  onToggle={() => toggleCandidate(item, 1)}
                />
              </td>
              <td>
                <Form.Select
                  size="sm"
                  aria-label={`${item.file_path} 목적지`}
                  value={manual}
                  disabled={item.apply_status === "moved"}
                  onChange={(event) => changeManual(item, event.target.value)}
                >
                  <option value="">(선택 안 함)</option>
                  {/* 직접 지정한 목적지가 ES 문서 0건 카테고리라 categories(전체 목록)에
                      없을 수 있다. 그 경우 빠뜨리면 셀렉트가 "(선택 안 함)"으로 보이면서도
                      실제 상태값은 여전히 그 목적지를 들고 있어, 화면에 안 보인 곳으로
                      승인될 수 있다 — 항상 옵션으로 끼워 넣는다. */}
                  {manual && !sortedCategories.includes(manual) && (
                    <option value={manual}>{manual}</option>
                  )}
                  {sortedCategories.map((category) => (
                    <option key={category} value={category}>
                      {category}
                    </option>
                  ))}
                </Form.Select>
                {manual && manual !== item.target_category && (
                  <small className="text-primary">직접 지정</small>
                )}
              </td>
              <td>
                {item.confidence == null ? "-" : item.confidence.toFixed(2)}
                {/* 키워드가 목적지를 정했는데 모델이 다른 카테고리를 자신 있게 가리키면,
                    이 점수는 모델의 확신도이지 위 추천1(키워드 목적지)의 점수가 아니다.
                    구분 없이 보여주면 "0.91"이 화면에 보이는 카테고리의 확신도로
                    잘못 읽힌다. */}
                {item.model_category &&
                  item.model_category !== item.target_category && (
                    <div>
                      <Badge bg="info" style={{ fontSize: "0.65rem" }}>
                        모델: {item.model_category}
                      </Badge>
                    </div>
                  )}
              </td>
              <td>
                <ApplyStatusBadge item={item} />
              </td>
            </tr>
          );
        })}
      </tbody>
    </Table>
  );
}

ClassifyProposalTable.propTypes = {
  items: PropTypes.array.isRequired,
  categories: PropTypes.array.isRequired,
  choices: PropTypes.object.isRequired,
  onChoicesChange: PropTypes.func.isRequired,
};
