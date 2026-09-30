import { useEffect, useState } from "react";
import PropTypes from "prop-types";
import { Button, Card, Form, InputGroup } from "react-bootstrap";
import { jsonPostReq, jsonPutReq, rawJsonGetReq } from "./Common";
import Actions from "./Actions";

export default function DirectoryEditPanel({
  directory,
  apiPrefix,
  selectedCategory,
  otherCategoryList,
  isProcessing,
  onSelectCategory,
  onMove,
  onComplete,
  onError,
}) {
  const [name, setName] = useState(directory.name);
  const [saving, setSaving] = useState(false);
  const [pdfStats, setPdfStats] = useState(null);
  const [statsError, setStatsError] = useState("");
  const [bulkPattern, setBulkPattern] = useState("");
  const [bulkReplacement, setBulkReplacement] = useState("");
  const [bulkBusy, setBulkBusy] = useState(false);
  const [bulkInfo, setBulkInfo] = useState("");
  const [bulkFailures, setBulkFailures] = useState([]);

  useEffect(() => {
    setPdfStats(null);
    setStatsError("");
    rawJsonGetReq(
      `${apiPrefix}/category-pdf-stats?category=${encodeURIComponent(directory.category)}`,
      (data) => {
        if (data.status === "success") setPdfStats(data.result);
        else setStatsError(data.error || "PDF 통계를 불러오지 못했습니다.");
      },
      () => setStatsError("PDF 통계를 불러오지 못했습니다."),
    );
  }, [apiPrefix, directory.category]);

  const submit = () => {
    const cleanName = name.trim();
    if (!cleanName || cleanName.includes("/") || cleanName === "." || cleanName === "..") {
      onError("디렉토리 이름을 입력하세요. 이름에는 '/'를 사용할 수 없습니다.");
      return;
    }
    const parent = directory.category.includes("/")
      ? directory.category.slice(0, directory.category.lastIndexOf("/"))
      : "";
    const newCategory = parent ? `${parent}/${cleanName}` : cleanName;
    if (newCategory === directory.category) return;
    setSaving(true);
    jsonPutReq(
      `${apiPrefix}/categories/rename`,
      { old_category: directory.category, new_category: newCategory },
      () => onComplete("디렉토리 이름을 변경했습니다.", { type: "rename", category: newCategory }),
      (error) => onError(`디렉토리 변경에 실패했습니다. ${error}`),
      () => setSaving(false),
    );
  };

  const deleteDirectory = () => {
    const confirmed = window.confirm(
      `"${directory.category}" 디렉토리와 하위 디렉토리의 모든 파일을 삭제할까요? 이 작업은 되돌릴 수 없습니다.`,
    );
    if (!confirmed) return;

    setSaving(true);
    jsonPostReq(
      `${apiPrefix}/categories/delete`,
      { category: directory.category },
      () => onComplete("디렉토리와 하위 파일을 삭제했습니다.", { type: "delete", category: directory.category }),
      (error) => onError(`디렉토리 삭제에 실패했습니다. ${error}`),
      () => setSaving(false),
    );
  };

  const bulkRenameFiles = () => {
    setBulkBusy(true);
    setBulkInfo("이름 변경 중...");
    setBulkFailures([]);
    jsonPostReq(
      `${apiPrefix}/categories/bulk-rename-files`,
      { category: directory.category, pattern: bulkPattern, replacement: bulkReplacement },
      (result) => {
        const failures = result.failures || [];
        setBulkFailures(failures);
        const failureText = result.failed_count ? `, 실패 ${result.failed_count}개` : "";
        setBulkInfo(`변경된 파일 ${result.changed_count || 0}개${failureText}`);
      },
      (error) => setBulkInfo(`오류: ${error || "파일 이름 변경 요청에 실패했습니다."}`),
      () => setBulkBusy(false),
    );
  };

  return (
    <>
    <Card className="mb-3">
      <Card.Header>디렉토리 편집</Card.Header>
      <Card.Body>
        <Form.Group className="mb-3">
          <Form.Label>디렉토리 이름</Form.Label>
          <InputGroup>
            <Form.Control
              value={name}
              onChange={(event) => setName(event.target.value)}
              disabled={saving}
            />
            <Button onClick={submit} disabled={saving || isProcessing || !name.trim()}>
              {saving ? "처리 중..." : "이름 변경"}
            </Button>
            <Button
              variant="outline-danger"
              onClick={deleteDirectory}
              disabled={saving || isProcessing}
            >
              삭제
            </Button>
          </InputGroup>
        </Form.Group>
        <div className="mb-3" role="status">
          {pdfStats ? (
            <>PDF {pdfStats.file_count.toLocaleString()}ea, {pdfStats.page_count.toLocaleString()}p, {Math.trunc(pdfStats.total_file_size / 1000).toLocaleString()}MB</>
          ) : statsError ? statsError : "PDF 통계 불러오는 중..."}
        </div>
        <hr />
        <Actions
          directoryMode
          selectedEntryId={directory.category}
          selectedCategory={selectedCategory}
          otherCategoryList={otherCategoryList}
          newFileName={name}
          moveToUpperButtonClicked={() => {}}
          moveToDirectoryButtonClicked={() => onMove(name.trim())}
          selectDirectoryButtonClicked={onSelectCategory}
          toNextEntryClicked={() => {}}
          toPrevEntryClicked={() => {}}
          suggestedCategories={{}}
          isProcessing={isProcessing || saving}
        />
      </Card.Body>
    </Card>
    <Card className="mb-3">
        <Card.Header>파일 이름 일괄 편집</Card.Header>
        <Card.Body>
          <Form.Group className="mb-3">
            <Form.Label>Python 정규표현식</Form.Label>
            <Form.Control
              value={bulkPattern}
              onChange={(event) => setBulkPattern(event.target.value)}
              placeholder="파일 이름에 적용할 패턴"
              disabled={bulkBusy || saving || isProcessing}
            />
          </Form.Group>
          <Form.Group className="mb-3">
            <Form.Label>바꿀 문자열</Form.Label>
            <Form.Control
              value={bulkReplacement}
              onChange={(event) => setBulkReplacement(event.target.value)}
              placeholder="정규표현식 치환 문자열"
              disabled={bulkBusy || saving || isProcessing}
            />
          </Form.Group>
          <Button onClick={bulkRenameFiles} disabled={!bulkPattern || bulkBusy || saving || isProcessing}>
            {bulkBusy ? "변경 중..." : "파일 이름 변경"}
          </Button>
          <div className="info mt-3" role="status">
            {bulkInfo}
            {bulkFailures.length > 0 && (
              <ul className="mb-0 mt-2">
                {bulkFailures.map((failure, index) => (
                  <li key={`${failure.file}-${index}`}>
                    {failure.file}: {failure.error}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </Card.Body>
      </Card>
    </>
  );
}

DirectoryEditPanel.propTypes = {
  directory: PropTypes.shape({
    category: PropTypes.string.isRequired,
    name: PropTypes.string.isRequired,
  }).isRequired,
  apiPrefix: PropTypes.string.isRequired,
  selectedCategory: PropTypes.string.isRequired,
  otherCategoryList: PropTypes.arrayOf(PropTypes.string).isRequired,
  isProcessing: PropTypes.bool.isRequired,
  onSelectCategory: PropTypes.func.isRequired,
  onMove: PropTypes.func.isRequired,
  onComplete: PropTypes.func.isRequired,
  onError: PropTypes.func.isRequired,
};
