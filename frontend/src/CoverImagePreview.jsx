import { Button, Modal } from "react-bootstrap";
import PropTypes from "prop-types";

export default function CoverImagePreview({ preview, onClose, onSelect, onNavigate, onImageLoad, onImageError }) {
  return (
    <Modal show onHide={onClose} animation={false} centered className="book-cover-zoom-layer" aria-label={`${preview.title} 표지 확대`}>
      <Modal.Header closeButton closeLabel="확대 닫기"><span>원본 크기 {preview.width && preview.height ? `· ${preview.width} × ${preview.height} px` : ""}</span></Modal.Header>
      <Modal.Body className="book-cover-zoom-panel">
        {onSelect ? <button type="button" className="book-cover-zoom-select" aria-label={`${preview.title} 확대 표지 선택`} onClick={onSelect}>
          <img className="book-cover-zoom-image" src={preview.url} alt={`${preview.title} 확대 표지`} onLoad={onImageLoad} onError={onImageError} />
        </button> : <img className="book-cover-zoom-image" src={preview.url} alt={`${preview.title} 확대 표지`} onLoad={onImageLoad} onError={onImageError} />}
      </Modal.Body>
      {preview.count > 1 && <Modal.Footer>
        <Button size="sm" variant="outline-secondary" disabled={preview.index === 0} onClick={() => onNavigate(preview.index - 1)}>이전 표지</Button>
        <span>{preview.index + 1} / {preview.count}</span>
        <Button size="sm" variant="outline-secondary" disabled={preview.index === preview.count - 1} onClick={() => onNavigate(preview.index + 1)}>다음 표지</Button>
      </Modal.Footer>}
    </Modal>
  );
}

CoverImagePreview.propTypes = {
  preview: PropTypes.shape({ id: PropTypes.string.isRequired, title: PropTypes.string.isRequired, url: PropTypes.string.isRequired,
    width: PropTypes.number, height: PropTypes.number, index: PropTypes.number, count: PropTypes.number }).isRequired,
  onClose: PropTypes.func.isRequired,
  onSelect: PropTypes.func,
  onNavigate: PropTypes.func,
  onImageLoad: PropTypes.func,
  onImageError: PropTypes.func,
};
