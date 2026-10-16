import { useState } from "react";
import PropTypes from "prop-types";
import { Alert, Button, Form, Modal } from "react-bootstrap";

// 표지 관리의 "표지 직접 입력" 모달과 같은 모양이다. URL 을 받으면 서버가 이미지를 내려받아 미리보기를 만들고,
// 성공하면 부모가 이 모달을 닫고 뷰어에 삽입될 쪽을 보여 준다. 실제 삽입은 저장할 때 한다.
// onSubmit(url)은 실패 사유(문자열)를 돌려주거나, 성공하면 null 을 돌려준다.
export default function PdfInsertImageModal({
  pageNum,
  initialUrl,
  onSubmit,
  onClose,
}) {
  const [url, setUrl] = useState(initialUrl || "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const trimmed = url.trim();

  const handleSubmit = async (event) => {
    event.preventDefault();
    if (!trimmed || busy) return;
    setBusy(true);
    setError("");
    const message = await onSubmit(trimmed);
    // 성공하면 부모가 이 모달을 닫으므로 상태를 건드리지 않는다.
    if (message) {
      setError(message);
      setBusy(false);
    }
  };

  return (
    <Modal
      show
      onHide={() => {
        if (!busy) onClose();
      }}
      centered
      aria-labelledby="pdf-insert-image-title"
    >
      <Modal.Header closeButton={!busy}>
        <Modal.Title id="pdf-insert-image-title">
          {pageNum}쪽 앞에 이미지 삽입
        </Modal.Title>
      </Modal.Header>
      <Form onSubmit={handleSubmit}>
        <Modal.Body>
          <Form.Group controlId="pdf-insert-image-url">
            <Form.Label>이미지 URL</Form.Label>
            <Form.Control
              type="url"
              required
              autoFocus
              maxLength={4096}
              disabled={busy}
              value={url}
              onChange={(event) => setUrl(event.target.value)}
            />
            <Form.Text muted>
              공개 HTTP/HTTPS 이미지 URL만 사용할 수 있습니다. 서버가 이미지를
              내려받아 {pageNum}쪽 앞에 들어갈 쪽을 미리 보여 주고, 저장하면
              실제로 끼웁니다.
            </Form.Text>
          </Form.Group>
          {error && (
            <Alert variant="danger" className="mt-2">
              {error}
            </Alert>
          )}
        </Modal.Body>
        <Modal.Footer>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            취소
          </Button>
          <Button type="submit" disabled={busy || !trimmed}>
            {busy ? "불러오는 중…" : "삽입 표시"}
          </Button>
        </Modal.Footer>
      </Form>
    </Modal>
  );
}

PdfInsertImageModal.propTypes = {
  pageNum: PropTypes.number.isRequired,
  initialUrl: PropTypes.string,
  onSubmit: PropTypes.func.isRequired,
  onClose: PropTypes.func.isRequired,
};
