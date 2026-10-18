import PropTypes from "prop-types";

import { ToggleButton, ToggleButtonGroup } from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faList, faTableCellsLarge } from "@fortawesome/free-solid-svg-icons";

/** 커버 목록 카드 머리글에 두는 목록/커버 전환 버튼. */
export default function CoverViewToggle({ id, viewMode, onChange }) {
  return (
    <ToggleButtonGroup
      type="radio"
      name={`${id}-view-mode`}
      size="sm"
      value={viewMode}
      onChange={onChange}
    >
      <ToggleButton
        id={`${id}-view-list`}
        value="list"
        variant="outline-secondary"
        title="목록 보기"
      >
        <FontAwesomeIcon icon={faList} />
        <span className="visually-hidden">목록 보기</span>
      </ToggleButton>
      <ToggleButton
        id={`${id}-view-cover`}
        value="cover"
        variant="outline-secondary"
        title="커버 보기"
      >
        <FontAwesomeIcon icon={faTableCellsLarge} />
        <span className="visually-hidden">커버 보기</span>
      </ToggleButton>
    </ToggleButtonGroup>
  );
}

CoverViewToggle.propTypes = {
  id: PropTypes.string.isRequired,
  viewMode: PropTypes.oneOf(["list", "cover"]).isRequired,
  onChange: PropTypes.func.isRequired,
};
