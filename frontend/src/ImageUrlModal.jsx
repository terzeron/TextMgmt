import { useState } from "react";
import PropTypes from "prop-types";
import { Alert, Button, Form, Modal } from "react-bootstrap";

// 표지 관리의 "표지 직접 입력" 모달과 같은 모양이다. URL 을 받으면 서버가 이미지를 내려받아 미리보기를 만들고,
// 성공하면 부모가 이 모달을 닫고 뷰어에 삽입될 이미지를 보여 준다. 실제 삽입은 저장할 때 한다.
// PDF 쪽 삽입(PdfInsertImageModal)과 EPUB 표지(EpubCoverModal)가 제목과 안내만 달리해 함께 쓴다.
// onSubmit(url)은 실패 사유(문자열)를 돌려주거나, 성공하면 null 을 돌려준다.
export default function ImageUrlModal({
  idPrefix,
  title,
  description,
  initialUrl = "",
  onSubmit,
  onClose,
}) {
  const [url, setUrl] = useState(initialUrl);
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
      aria-labelledby={`${idPrefix}-title`}
    >
      <Modal.Header closeButton={!busy}>
        <Modal.Title id={`${idPrefix}-title`}>{title}</Modal.Title>
      </Modal.Header>
      <Form onSubmit={handleSubmit}>
        <Modal.Body>
          <Form.Group controlId={`${idPrefix}-url`}>
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
            <Form.Text muted>{description}</Form.Text>
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

ImageUrlModal.propTypes = {
  idPrefix: PropTypes.string.isRequired,
  title: PropTypes.string.isRequired,
  description: PropTypes.node.isRequired,
  initialUrl: PropTypes.string,
  onSubmit: PropTypes.func.isRequired,
  onClose: PropTypes.func.isRequired,
};
