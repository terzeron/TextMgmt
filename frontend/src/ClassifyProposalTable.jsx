/* eslint-disable react-refresh/only-export-components --
   resolveTarget/isSelectable는 부모(later task)와 테스트가 재사용하는 순수 헬퍼라
   컴포넌트와 co-located. HMR 힌트일 뿐 런타임 영향 없음. */
import PropTypes from "prop-types";
import { Table, Form, Badge } from "react-bootstrap";

// 목적지가 있어야 승인할 수 있다. 불확실 행도 사용자가 고르면 켜진다.
export function resolveTarget(item, targets) {
  return targets[item.file_path] ?? item.target_category ?? "";
}

export function isSelectable(item, targets) {
  // 이미 옮겼거나(moved) 옮기던 중 서버가 죽어 결과를 모르는(moving) 행은 다시
  // 승인 대상이 될 이유가 없다 — 사람이 moving을 직접 확인하기 전에는 재이동
  // 대상에서 빠져 있어야 한다.
  if (item.apply_status === "moved" || item.apply_status === "moving") {
    return false;
  }
  return Boolean(resolveTarget(item, targets));
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

// 추천 후보 한 칸(카테고리 + 근거). 후보가 없으면 "-".
function CandidateCell({ candidate }) {
  if (!candidate) return "-";
  return (
    <>
      <div>{candidate.category}</div>
      <small className="text-muted">{candidate.detail}</small>
    </>
  );
}

CandidateCell.propTypes = {
  candidate: PropTypes.shape({
    category: PropTypes.string,
    source: PropTypes.string,
    detail: PropTypes.string,
  }),
};

export default function ClassifyProposalTable({
  items,
  categories,
  selection,
  targets,
  onSelectionChange,
  onTargetChange,
}) {
  const selectablePaths = items
    .filter((item) => isSelectable(item, targets))
    .map((item) => item.file_path);
  const allSelected =
    selectablePaths.length > 0 &&
    selectablePaths.every((path) => selection.has(path));

  const toggleAll = () => {
    onSelectionChange(allSelected ? new Set() : new Set(selectablePaths));
  };

  const toggleOne = (filePath) => {
    const next = new Set(selection);
    if (next.has(filePath)) next.delete(filePath);
    else next.add(filePath);
    onSelectionChange(next);
  };

  // 목적지를 비우면 승인 근거가 사라진다. 이미 선택된 행이었다면 selection에서도
  // 즉시 빼야, 부모가 selection만 훑어 승인 처리할 때 근거 없는 행이 끼어들지 않는다.
  const handleTargetChange = (filePath, nextValue) => {
    onTargetChange(filePath, nextValue);
    if (!nextValue && selection.has(filePath)) {
      const next = new Set(selection);
      next.delete(filePath);
      onSelectionChange(next);
    }
  };

  return (
    <Table size="sm" bordered hover responsive className="mt-2">
      <thead>
        <tr>
          <th>
            <Form.Check
              type="checkbox"
              aria-label="전체 선택"
              checked={allSelected}
              onChange={toggleAll}
            />
          </th>
          <th>책</th>
          <th>현재</th>
          <th>추천 1</th>
          <th>추천 2</th>
          <th>직접 선택</th>
          <th>점수</th>
          <th>이동 상태</th>
        </tr>
      </thead>
      <tbody>
        {items.map((item) => {
          const target = resolveTarget(item, targets);
          const selectable = isSelectable(item, targets);
          const changed =
            target && item.target_category && target !== item.target_category;
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
              <td>
                <Form.Check
                  type="checkbox"
                  aria-label={`${item.file_path} 선택`}
                  checked={selection.has(item.file_path)}
                  disabled={!selectable}
                  onChange={() => toggleOne(item.file_path)}
                />
              </td>
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
                <CandidateCell candidate={candidates[0]} />
              </td>
              <td>
                <CandidateCell candidate={candidates[1]} />
              </td>
              <td>
                <Form.Select
                  size="sm"
                  aria-label={`${item.file_path} 목적지`}
                  value={target}
                  onChange={(event) =>
                    handleTargetChange(item.file_path, event.target.value)
                  }
                >
                  <option value="">(선택 안 함)</option>
                  {categories.map((category) => (
                    <option key={category} value={category}>
                      {category}
                    </option>
                  ))}
                </Form.Select>
                {changed && <small className="text-primary">직접 지정</small>}
              </td>
              <td>
                {item.confidence == null ? "-" : item.confidence.toFixed(2)}
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
  selection: PropTypes.object.isRequired,
  targets: PropTypes.object.isRequired,
  onSelectionChange: PropTypes.func.isRequired,
  onTargetChange: PropTypes.func.isRequired,
};
