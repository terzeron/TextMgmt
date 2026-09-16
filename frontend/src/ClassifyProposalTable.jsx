/* eslint-disable react-refresh/only-export-components --
   resolveTarget/isSelectable는 부모(later task)와 테스트가 재사용하는 순수 헬퍼라
   컴포넌트와 co-located. HMR 힌트일 뿐 런타임 영향 없음. */
import PropTypes from "prop-types";
import { Table, Form } from "react-bootstrap";

// 목적지가 있어야 승인할 수 있다. 불확실 행도 사용자가 고르면 켜진다.
export function resolveTarget(item, targets) {
  return targets[item.file_path] ?? item.target_category ?? "";
}

export function isSelectable(item, targets) {
  return Boolean(resolveTarget(item, targets));
}

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
          <th>제안</th>
          <th>점수</th>
          <th>근거</th>
        </tr>
      </thead>
      <tbody>
        {items.map((item) => {
          const target = resolveTarget(item, targets);
          const selectable = isSelectable(item, targets);
          const changed =
            target && item.target_category && target !== item.target_category;
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
                <Form.Select
                  size="sm"
                  aria-label={`${item.file_path} 목적지`}
                  value={target}
                  onChange={(event) =>
                    onTargetChange(item.file_path, event.target.value)
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
                <small>{item.apply_error || item.reason}</small>
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
