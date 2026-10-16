import { useState } from "react";
import PropTypes from "prop-types";
import { Alert, Button, Form, Modal } from "react-bootstrap";

// PDF 이미지 삽입 모달(PdfInsertImageModal)과 같은 흐름이다. URL 을 받으면 서버가 이미지를 내려받아 미리보기를 만들고,
// 성공하면 부모가 이 모달을 닫고 뷰어 첫 쪽에 새 표지를 보여 준다. 실제 저장은 뷰어의 저장 버튼으로 한다.
// onSubmit(url)은 실패 사유(문자열)를 돌려주거나, 성공하면 null 을 돌려준다.
export default function EpubCoverModal({
  hasCover = false,
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
      aria-labelledby="epub-cover-title"
    >
      <Modal.Header closeButton={!busy}>
        <Modal.Title id="epub-cover-title">
          {hasCover ? "표지 이미지 교체" : "표지 이미지 추가"}
        </Modal.Title>
      </Modal.Header>
      <Form onSubmit={handleSubmit}>
        <Modal.Body>
          <Form.Group controlId="epub-cover-url">
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
              내려받아 첫 쪽에 들어갈 표지를 미리 보여 주고, 저장하면 실제로
              넣습니다.
              {hasCover && " 기존 표지는 교체되며 원본 EPUB은 백업됩니다."}
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

EpubCoverModal.propTypes = {
  hasCover: PropTypes.bool,
  initialUrl: PropTypes.string,
  onSubmit: PropTypes.func.isRequired,
  onClose: PropTypes.func.isRequired,
};
