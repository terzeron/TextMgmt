/* eslint-disable react-refresh/only-export-components --
   resolveTarget/isSelectable 등은 부모와 테스트가 재사용하는 순수 헬퍼라
   컴포넌트와 co-located. HMR 힌트일 뿐 런타임 영향 없음. */
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

// 추천 체크박스를 누를 수 있는가. 불확실(unknown)은 시스템이 목적지를 고르지
// 못했다는 뜻이라 추천을 그대로 승인할 수 없다 — 직접 선택으로만 목적지를 준다.
export function canCheckCandidate(item, candidate) {
  return Boolean(
    candidate?.category &&
    item.grade !== "unknown" &&
    item.apply_status !== "moved",
  );
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

  // 헤더의 일괄 체크는 추천 1만 건드린다. 직접 선택으로 이미 목적지를 정한 행은
  // 사용자가 손으로 한 판단이라 일괄 조작으로 덮지 않는다.
  const bulkTargets = items.filter(
    (item) =>
      canCheckCandidate(item, (item.candidates || [])[0]) &&
      choices[item.file_path]?.source !== "manual",
  );
  const allFirstChecked =
    bulkTargets.length > 0 &&
    bulkTargets.every((item) => isCandidateChecked(item, choices, 0));

  const toggleAllFirst = () => {
    const next = { ...choices };
    for (const item of bulkTargets) {
      if (allFirstChecked) delete next[item.file_path];
      else
        next[item.file_path] = {
          source: "candidate",
          index: 0,
          category: item.candidates[0].category,
        };
    }
    onChoicesChange(next);
  };

  return (
    <Table size="sm" bordered hover responsive className="mt-2">
      <thead>
        <tr>
          <th>책</th>
          <th>현재</th>
          <th>
            <div className="d-flex gap-2 align-items-center">
              <Form.Check
                type="checkbox"
                aria-label="추천 1 전체 선택"
                checked={allFirstChecked}
                onChange={toggleAllFirst}
              />
              추천 1
            </div>
          </th>
          <th>추천 2</th>
          <th>직접 선택</th>
          <th>점수</th>
          <th>이동 상태</th>
        </tr>
      </thead>
      <tbody>
        {items.map((item) => {
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
              <td>{item.current_category}</td>
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
                  {manual && !categories.includes(manual) && (
                    <option value={manual}>{manual}</option>
                  )}
                  {categories.map((category) => (
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
