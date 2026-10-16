import PropTypes from "prop-types";
import ImageUrlModal from "./ImageUrlModal";

export default function PdfInsertImageModal({
  pageNum,
  initialUrl,
  onSubmit,
  onClose,
}) {
  return (
    <ImageUrlModal
      idPrefix="pdf-insert-image"
      title={`${pageNum}쪽 앞에 이미지 삽입`}
      description={`공개 HTTP/HTTPS 이미지 URL만 사용할 수 있습니다. 서버가 이미지를 내려받아 ${pageNum}쪽 앞에 들어갈 쪽을 미리 보여 주고, 저장하면 실제로 끼웁니다.`}
      initialUrl={initialUrl}
      onSubmit={onSubmit}
      onClose={onClose}
    />
  );
}

PdfInsertImageModal.propTypes = {
  pageNum: PropTypes.number.isRequired,
  initialUrl: PropTypes.string,
  onSubmit: PropTypes.func.isRequired,
  onClose: PropTypes.func.isRequired,
};
