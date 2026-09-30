import { useEffect } from "react";
import PropTypes from "prop-types";
import { FONT_FAMILIES, buildFontFaceCss } from "./viewerFonts";
import "./ViewerFontSelect.css";

export default function ViewerFontSelect({ value, onChange }) {
  useEffect(() => {
    if (document.getElementById("viewer-font-face-override")) return;
    const style = document.createElement("style");
    style.id = "viewer-font-face-override";
    style.textContent = buildFontFaceCss(`${window.location.origin}/fonts/`);
    document.head.appendChild(style);
  }, []);

  return (
    <select
      className="viewer-font-select"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      aria-label="글꼴 선택"
    >
      {FONT_FAMILIES.map((font) => (
        <option key={font.value} value={font.value}>
          {font.label}
        </option>
      ))}
    </select>
  );
}

ViewerFontSelect.propTypes = {
  value: PropTypes.string.isRequired,
  onChange: PropTypes.func.isRequired,
};
