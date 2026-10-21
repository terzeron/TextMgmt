import { useState } from "react";
import PropTypes from "prop-types";
import { Alert, Button, Form, Modal } from "react-bootstrap";

// PDF 쪽 삽입, EPUB 표지, 책 표지 관리가 함께 쓰는 이미지 입력 모달이다.
// onSubmit(source)에 URL(문자열) 또는 선택한 File을 넘긴다. 부모가 미리보기나 저장을 처리한다.
// 실패 사유(문자열)를 돌려주면 모달에 표시한다. 성공하면 부모가 모달을 닫는다.
export default function ImageUrlModal({
  idPrefix,
  title,
  description,
  initialUrl = "",
  urlLabel = "이미지 URL",
  submitLabel = "삽입 표시",
  busyLabel = "불러오는 중…",
  fileDescription = "PC의 파일이나 모바일의 사진첩·카메라에서 고를 수 있습니다. 15MB 이하이며, 고르면 바로 미리 봅니다.",
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
            <Form.Label>{urlLabel}</Form.Label>
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
              {fileDescription}
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
            {busy ? busyLabel : submitLabel}
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
  urlLabel: PropTypes.string,
  submitLabel: PropTypes.string,
  busyLabel: PropTypes.string,
  fileDescription: PropTypes.node,
  onSubmit: PropTypes.func.isRequired,
  onClose: PropTypes.func.isRequired,
};
