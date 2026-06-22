import { useState } from "react";
import PropTypes from "prop-types";
import { useNavigate } from "react-router-dom";

import "bootstrap/dist/css/bootstrap.min.css";
import { Alert, Button, Card } from "react-bootstrap";

import { jsonDeleteReq } from "./Common";

/**
 * 책 정보를 불러오지 못했을 때(404, 권한 거부, stale 등) 책 영역 대신
 * 표시하는 사유 패널.
 *
 * - 모든 사용자: "책 정보를 불러오지 못했습니다." 안내
 * - 관리자(role === "admin"): 백엔드가 돌려준 실제 사유 원문과
 *   문제 수정 액션 버튼(ES 잔존 문서 삭제 / 카테고리 불일치 관리 이동)을 노출
 */
export default function BookLoadError(props) {
  const { bookId, category, error, role, apiPrefix } = props;
  const navigate = useNavigate();
  const [actionMessage, setActionMessage] = useState("");
  const [actionVariant, setActionVariant] = useState("success");
  const [deleting, setDeleting] = useState(false);

  const isAdmin = role === "admin";
  const hasBookId = bookId !== undefined && bookId !== null && bookId !== "";

  const handleDeleteEsDoc = () => {
    if (!hasBookId) {
      return;
    }
    if (!window.confirm(`ES에 남은 책 문서(book_id=${bookId})를 삭제할까요?`)) {
      return;
    }
    setDeleting(true);
    jsonDeleteReq(
      `${apiPrefix}/category-mismatches/es-doc/${bookId}`,
      null,
      () => {
        setActionVariant("success");
        setActionMessage(`ES 잔존 문서를 삭제했습니다. (book_id=${bookId})`);
        setDeleting(false);
      },
      (err) => {
        setActionVariant("danger");
        setActionMessage(`ES 문서 삭제 실패: ${err}`);
        setDeleting(false);
      },
    );
  };

  return (
    <Card data-testid="book-load-error">
      <Card.Header>책 정보</Card.Header>
      <Card.Body>
        <Alert variant="warning" className="mb-2">
          책 정보를 불러오지 못했습니다.
        </Alert>

        <div className="text-muted small mb-2">
          {hasBookId && <span>책 ID: {bookId}</span>}
          {category && <span className="ms-2">카테고리: {category}</span>}
        </div>

        {isAdmin && (
          <>
            {error && (
              <div
                className="text-muted small mb-2"
                data-testid="book-load-error-reason"
              >
                사유: {error}
              </div>
            )}

            {actionMessage && (
              <Alert variant={actionVariant} className="mb-2 py-1">
                {actionMessage}
              </Alert>
            )}

            <div className="d-flex gap-2 flex-wrap">
              <Button
                variant="outline-danger"
                size="sm"
                disabled={!hasBookId || deleting}
                onClick={handleDeleteEsDoc}
              >
                ES 잔존 문서 삭제
              </Button>
              <Button
                variant="outline-secondary"
                size="sm"
                onClick={() => navigate("/admin")}
              >
                카테고리 불일치 관리로 이동
              </Button>
            </div>
          </>
        )}
      </Card.Body>
    </Card>
  );
}

BookLoadError.propTypes = {
  bookId: PropTypes.oneOfType([PropTypes.string, PropTypes.number]),
  category: PropTypes.string,
  error: PropTypes.string,
  role: PropTypes.string,
  apiPrefix: PropTypes.string,
};
