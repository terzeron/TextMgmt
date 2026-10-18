import { useState } from "react";
import PropTypes from "prop-types";
import { Alert, Button, Form, Modal } from "react-bootstrap";

// 표지 관리의 "표지 직접 입력" 모달과 같은 모양이다. URL 을 받으면 서버가 이미지를 내려받아 미리보기를 만들고,
// 성공하면 부모가 이 모달을 닫고 뷰어에 삽입될 이미지를 보여 준다. 실제 삽입은 저장할 때 한다.
// PDF 쪽 삽입(PdfInsertImageModal)과 EPUB 표지(EpubCoverModal)가 제목과 안내만 달리해 함께 쓴다.
// URL 대신 이 기기(PC, 모바일 사진첩·카메라)의 파일을 고를 수도 있다. 파일은 고르는 즉시 미리 본다.
// onSubmit(source)의 source 는 URL(문자열) 또는 File 이다. 실패 사유(문자열)를 돌려주거나, 성공하면 null 을 돌려준다.
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

  const submit = async (source) => {
    setBusy(true);
    setError("");
    const message = await onSubmit(source);
    // 성공하면 부모가 이 모달을 닫으므로 상태를 건드리지 않는다.
    if (message) {
      setError(message);
      setBusy(false);
    }
  };

  const handleSubmit = (event) => {
    event.preventDefault();
    if (!trimmed || busy) return;
    submit(trimmed);
  };

  const handleFile = (event) => {
    const input = event.target;
    const file = input.files?.[0];
    // 실패한 뒤 같은 파일을 다시 골라도 change 가 일어나도록 비운다.
    input.value = "";
    if (!file || busy) return;
    submit(file);
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
          <Form.Group controlId={`${idPrefix}-file`} className="mt-3">
            <Form.Label>이 기기에서 이미지 선택</Form.Label>
            <Form.Control
              type="file"
              accept="image/*"
              disabled={busy}
              onChange={handleFile}
            />
            <Form.Text muted>
              PC의 파일이나 모바일의 사진첩·카메라에서 고를 수 있습니다. 15MB
              이하이며, 고르면 바로 미리 봅니다.
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

ImageUrlModal.propTypes = {
  idPrefix: PropTypes.string.isRequired,
  title: PropTypes.string.isRequired,
  description: PropTypes.node.isRequired,
  initialUrl: PropTypes.string,
  onSubmit: PropTypes.func.isRequired,
  onClose: PropTypes.func.isRequired,
};
