import PropTypes from "prop-types";
import ImageUrlModal from "./ImageUrlModal";

export default function EpubCoverModal({
  hasCover = false,
  initialUrl,
  onSubmit,
  onClose,
}) {
  return (
    <ImageUrlModal
      idPrefix="epub-cover"
      title={hasCover ? "표지 이미지 교체" : "표지 이미지 추가"}
      description={`공개 HTTP/HTTPS 이미지 URL만 사용할 수 있습니다. 서버가 이미지를 내려받아 첫 쪽에 들어갈 표지를 미리 보여 주고, 저장하면 실제로 넣습니다.${hasCover ? " 기존 표지는 교체되며 원본은 0_trash에 보존됩니다." : ""}`}
      initialUrl={initialUrl}
      onSubmit={onSubmit}
      onClose={onClose}
    />
  );
}

EpubCoverModal.propTypes = {
  hasCover: PropTypes.bool,
  initialUrl: PropTypes.string,
  onSubmit: PropTypes.func.isRequired,
  onClose: PropTypes.func.isRequired,
};
