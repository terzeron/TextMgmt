import PropTypes from "prop-types";
import ImageUrlModal from "./ImageUrlModal";

export default function EpubIllustrationModal({ initialUrl, onSubmit, onClose }) {
  return (
    <ImageUrlModal
      idPrefix="epub-illustration"
      title="이 쪽 앞에 삽화 삽입"
      description="공개 HTTP/HTTPS 이미지 URL이나 이 기기의 파일을 쓸 수 있습니다. 지금 보는 쪽 바로 앞에 이미지 한 장짜리 쪽이 들어가며, 표지는 바뀌지 않습니다. 원본은 0_trash에 보존됩니다."
      initialUrl={initialUrl}
      onSubmit={onSubmit}
      onClose={onClose}
    />
  );
}

EpubIllustrationModal.propTypes = {
  initialUrl: PropTypes.string,
  onSubmit: PropTypes.func.isRequired,
  onClose: PropTypes.func.isRequired,
};
