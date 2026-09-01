import { useCallback, useEffect, useState } from "react";

import "bootstrap/dist/css/bootstrap.min.css";
import {
  Alert,
  Badge,
  Button,
  ButtonGroup,
  Modal,
  Spinner,
  Table,
} from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faRotate, faTrash } from "@fortawesome/free-solid-svg-icons";
import PropTypes from "prop-types";
import { DateTime } from "luxon";

import { jsonDeleteReq, jsonGetReq } from "./Common";

const PAGE_SIZE = 50;

const STATUS_FILTERS = [
  { key: "active", label: "활성" },
  { key: "all", label: "전체" },
];

const STATUS_LABELS = {
  active: { text: "활성", variant: "success" },
  expired: { text: "만료", variant: "secondary" },
  revoked: { text: "폐기", variant: "danger" },
};

function formatEpoch(seconds) {
  if (!seconds) return "-";
  return DateTime.fromSeconds(seconds)
    .setZone("local")
    .toFormat("yyyy-MM-dd HH:mm");
}

function StatusBadge({ status }) {
  const meta = STATUS_LABELS[status] || { text: status, variant: "secondary" };
  return <Badge bg={meta.variant}>{meta.text}</Badge>;
}

StatusBadge.propTypes = {
  status: PropTypes.string.isRequired,
};

// 백엔드가 요약과 원문을 함께 내려준다(요약은 원문에서 파생되므로 둘 다 있거나 둘 다 없다).
// 표에는 요약만 두고 원문은 툴팁으로 보여준다.
function UserAgentCell({ summary, raw }) {
  if (!summary) return <>-</>;
  return (
    <span
      className="d-inline-block text-truncate"
      style={{ maxWidth: "16rem" }}
      title={raw}
    >
      {summary}
    </span>
  );
}

UserAgentCell.propTypes = {
  summary: PropTypes.string,
  raw: PropTypes.string,
};

export default function LoginSessionAdmin() {
  const [items, setItems] = useState([]);
  const [summary, setSummary] = useState(null);
  const [pagination, setPagination] = useState(null);
  const [statusFilter, setStatusFilter] = useState("active");
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  // 폐기 확인 대상. null 이면 모달이 닫힌 상태다.
  const [revokeTarget, setRevokeTarget] = useState(null);
  const [revoking, setRevoking] = useState(false);
  const [notice, setNotice] = useState("");

  const load = useCallback(() => {
    setLoading(true);
    setError("");
    jsonGetReq(
      `/auth/sessions?page=${page}&pageSize=${PAGE_SIZE}&status=${statusFilter}`,
      null,
      (result) => {
        setItems(result?.items || []);
        setSummary(result?.summary || null);
        setPagination(result?.pagination || null);
      },
      () => setError("세션 목록을 불러오지 못했습니다."),
      () => setLoading(false),
    );
  }, [page, statusFilter]);

  useEffect(() => {
    load();
  }, [load]);

  const changeFilter = (key) => {
    setStatusFilter(key);
    setPage(1);
  };

  const confirmRevoke = () => {
    if (!revokeTarget) return;
    setRevoking(true);
    jsonDeleteReq(
      `/auth/sessions/${revokeTarget.session_id}`,
      null,
      (result) => {
        setRevokeTarget(null);
        if (result?.revoked_current) {
          // 백엔드가 인증 쿠키를 지웠으므로 기존 미인증 흐름으로 넘어간다.
          setNotice("현재 세션을 폐기했습니다. 다시 로그인해 주세요.");
          window.location.reload();
          return;
        }
        setNotice("세션을 폐기했습니다.");
        load();
      },
      () => {
        setRevokeTarget(null);
        setError("세션을 폐기하지 못했습니다.");
      },
      () => setRevoking(false),
    );
  };

  const totalPages = pagination?.totalPages || 1;

  return (
    <div className="mt-3">
      <div className="d-flex flex-wrap align-items-center gap-2 mb-3">
        <ButtonGroup aria-label="세션 상태 필터">
          {STATUS_FILTERS.map((filter) => (
            <Button
              key={filter.key}
              size="sm"
              variant={
                statusFilter === filter.key ? "primary" : "outline-primary"
              }
              aria-pressed={statusFilter === filter.key}
              onClick={() => changeFilter(filter.key)}
            >
              {filter.label}
            </Button>
          ))}
        </ButtonGroup>
        <Button
          size="sm"
          variant="outline-secondary"
          onClick={load}
          disabled={loading}
        >
          <FontAwesomeIcon icon={faRotate} /> 새로고침
        </Button>
        {summary && (
          <span className="text-muted small ms-auto">
            활성 {summary.active} / 만료 {summary.expired} / 폐기{" "}
            {summary.revoked} / 전체 {summary.total}
          </span>
        )}
      </div>

      {notice && (
        <Alert variant="success" dismissible onClose={() => setNotice("")}>
          {notice}
        </Alert>
      )}
      {error && (
        <Alert variant="danger" dismissible onClose={() => setError("")}>
          {error}
        </Alert>
      )}

      {loading && (
        <div className="text-center py-4">
          <Spinner animation="border" size="sm" role="status" />
          <span className="ms-2">불러오는 중...</span>
        </div>
      )}

      {!loading && items.length === 0 && !error && (
        <p className="text-muted">표시할 세션이 없습니다.</p>
      )}

      {!loading && items.length > 0 && (
        <Table striped bordered hover size="sm" responsive>
          <thead>
            <tr>
              <th>상태</th>
              <th>계정</th>
              <th>세션</th>
              <th>접속 IP</th>
              <th>User Agent</th>
              <th>생성 시각</th>
              <th>마지막 갱신</th>
              <th>만료 시각</th>
              <th>폐기 사유</th>
              <th>작업</th>
            </tr>
          </thead>
          <tbody>
            {items.map((session) => (
              <tr key={session.session_id}>
                <td>
                  <StatusBadge status={session.status} />
                </td>
                <td>{session.email}</td>
                <td>
                  {/* family_id 전체는 노출하지 않고 축약 라벨만 보여준다 */}
                  <code>{session.session_label}</code>
                  {session.is_current && (
                    <Badge bg="info" className="ms-2">
                      현재 세션
                    </Badge>
                  )}
                  {session.merged_family_ids?.length > 1 && (
                    <Badge
                      bg="secondary"
                      className="ms-2"
                      title="같은 기기의 브라우저 버전 업데이트로 판단해 합쳐 보여줍니다. 폐기는 최신 세션에만 적용됩니다."
                    >
                      +{session.merged_family_ids.length - 1} 이전 로그인
                    </Badge>
                  )}
                </td>
                {/* IP/UA 는 마지막 갱신 시점 값이다. 컬럼 추가 이전 세션은 비어 있다. */}
                <td className="text-nowrap">
                  <code>{session.client_ip || "-"}</code>
                </td>
                <td>
                  <UserAgentCell
                    summary={session.user_agent_summary}
                    raw={session.user_agent}
                  />
                </td>
                <td>{formatEpoch(session.created_at)}</td>
                <td>{formatEpoch(session.last_seen_at)}</td>
                <td>{formatEpoch(session.expires_at)}</td>
                <td>{session.revoke_reason || "-"}</td>
                <td>
                  <Button
                    size="sm"
                    variant="outline-danger"
                    disabled={session.status !== "active"}
                    onClick={() => setRevokeTarget(session)}
                    aria-label={`${session.session_label} 세션 폐기`}
                  >
                    <FontAwesomeIcon icon={faTrash} /> 폐기
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}

      {!loading && totalPages > 1 && (
        <div className="d-flex align-items-center gap-2">
          <Button
            size="sm"
            variant="outline-secondary"
            disabled={page <= 1}
            onClick={() => setPage((p) => Math.max(1, p - 1))}
          >
            이전
          </Button>
          <span className="small text-muted">
            {page} / {totalPages}
          </span>
          <Button
            size="sm"
            variant="outline-secondary"
            disabled={page >= totalPages}
            onClick={() => setPage((p) => p + 1)}
          >
            다음
          </Button>
        </div>
      )}

      <Modal
        show={revokeTarget !== null}
        onHide={() => setRevokeTarget(null)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>세션 폐기</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="mb-1">
            <strong>{revokeTarget?.email}</strong> 의 세션{" "}
            <code>{revokeTarget?.session_label}</code> 을 폐기합니다.
          </p>
          <p className="text-danger mb-0 small">
            되돌릴 수 없습니다. 해당 클라이언트는 다시 로그인해야 합니다.
            {revokeTarget?.is_current &&
              " 현재 사용 중인 세션이므로 즉시 로그아웃됩니다."}
          </p>
        </Modal.Body>
        <Modal.Footer>
          <Button
            variant="secondary"
            onClick={() => setRevokeTarget(null)}
            disabled={revoking}
          >
            취소
          </Button>
          <Button variant="danger" onClick={confirmRevoke} disabled={revoking}>
            {revoking ? "폐기 중..." : "폐기"}
          </Button>
        </Modal.Footer>
      </Modal>
    </div>
  );
}
