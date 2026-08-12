import { useCallback, useEffect, useState } from "react";

import "bootstrap/dist/css/bootstrap.min.css";
import { Alert, Badge, Spinner, Table } from "react-bootstrap";
import PropTypes from "prop-types";
import { DateTime } from "luxon";

import { jsonGetReq } from "./Common";

function formatEpoch(seconds) {
  if (!seconds) return "-";
  return DateTime.fromSeconds(seconds)
    .setZone("local")
    .toFormat("yyyy-MM-dd HH:mm");
}

function HistoryTable({ label, items }) {
  if (!items || items.length === 0) {
    return (
      <div className="mb-3">
        <div className="fw-semibold small mb-1">
          {label} <Badge bg="secondary">0</Badge>
        </div>
        <p className="text-muted small mb-0">조회 이력이 없습니다.</p>
      </div>
    );
  }
  return (
    <div className="mb-3">
      <div className="fw-semibold small mb-1">
        {label} <Badge bg="secondary">{items.length}</Badge>
      </div>
      <Table striped bordered hover size="sm" responsive className="mb-0">
        <thead>
          <tr>
            <th>제목</th>
            <th>카테고리</th>
            <th>조회 시각</th>
          </tr>
        </thead>
        <tbody>
          {items.map((item) => (
            <tr key={`${item.book_id}-${item.viewed_at}`}>
              <td>{item.title}</td>
              <td>{item.category || "-"}</td>
              <td>{formatEpoch(item.viewed_at)}</td>
            </tr>
          ))}
        </tbody>
      </Table>
    </div>
  );
}

HistoryTable.propTypes = {
  label: PropTypes.string.isRequired,
  items: PropTypes.array,
};

export default function ViewHistoryAdmin() {
  const [users, setUsers] = useState([]);
  const [limit, setLimit] = useState(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(() => {
    setLoading(true);
    setError("");
    jsonGetReq(
      "/view-history",
      null,
      (result) => {
        setUsers(result?.users || []);
        setLimit(result?.limit ?? null);
      },
      () => setError("조회 목록을 불러오지 못했습니다."),
      () => setLoading(false),
    );
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <div className="mt-3">
      {limit !== null && (
        <div className="text-muted small mb-3">
          사용자 {users.length}명 · 유형별 최근 {limit}건
        </div>
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

      {!loading && users.length === 0 && !error && (
        <p className="text-muted">조회 이력이 있는 사용자가 없습니다.</p>
      )}

      {!loading &&
        users.map((user) => (
          <section key={user.email} className="mb-4">
            <h6 className="mb-1">{user.email}</h6>
            <div className="text-muted small mb-2">
              마지막 조회 {formatEpoch(user.last_viewed_at)}
            </div>
            <HistoryTable label="책" items={user.book} />
            <HistoryTable label="만화" items={user.comic} />
          </section>
        ))}
    </div>
  );
}
