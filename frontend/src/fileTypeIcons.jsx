/**
 * 트리 노드 아이콘 공유 모듈.
 *
 * 디렉토리·가상 노드는 MUI 아이콘(react-file-icon은 폴더 글리프가 없다),
 * 실제 파일은 react-file-icon으로 포맷별 확장자 라벨과 색상을 함께 보여준다.
 * Folder.jsx(일반 탭)와 CategoryAdmin.jsx(관리 탭)가 이 모듈을 공유한다.
 */

import React from "react";
import PropTypes from "prop-types";

import Box from "@mui/material/Box";
import DeleteIcon from "@mui/icons-material/Delete";
import FolderOpenIcon from "@mui/icons-material/FolderOpen";
import FolderRounded from "@mui/icons-material/FolderRounded";
import MoreHorizIcon from "@mui/icons-material/MoreHoriz";
import { FileIcon, defaultStyles } from "react-file-icon";

import { MORE_ENTRY_FILE_TYPE } from "./folderUtils";

const FOLDER_COLOR = "#ffc107";
const MUTED_COLOR = "#9e9e9e";
const ICON_SIZE = "1.2rem";

/**
 * react-file-icon의 defaultStyles에 없는 장서 포맷을 보강한다.
 * (epub, hwp, webp, avif는 1.6.0 기준 defaultStyles에 없음)
 */
const FILE_STYLES = {
  ...defaultStyles,
  epub: {
    color: "#8E44AD",
    foldColor: "#7A3B95",
    glyphColor: "rgba(255,255,255,0.4)",
    labelColor: "#8E44AD",
    labelUppercase: true,
    type: "document",
  },
  hwp: {
    color: "#1B8CD4",
    foldColor: "#1677B4",
    glyphColor: "rgba(255,255,255,0.4)",
    labelColor: "#1B8CD4",
    labelUppercase: true,
    type: "document",
  },
  webp: defaultStyles.png,
  avif: defaultStyles.png,
};

/** fileType 값이 확장자가 아닌 경우의 대응 확장자. */
const FILE_TYPE_ALIASES = {
  image: "png",
  video: "mp4",
  htm: "html",
};

/** 파일이 아닌 노드(디렉토리, 휴지통, 로딩 placeholder 등)의 MUI 아이콘. */
const NODE_GLYPHS = {
  folder: { Icon: FolderRounded, color: FOLDER_COLOR },
  pinned: { Icon: FolderOpenIcon, color: FOLDER_COLOR },
  trash: { Icon: DeleteIcon, color: MUTED_COLOR },
  placeholder: { Icon: MoreHorizIcon, color: MUTED_COLOR },
  [MORE_ENTRY_FILE_TYPE]: { Icon: MoreHorizIcon, color: MUTED_COLOR },
};

/**
 * react-file-icon에 넘길 확장자를 결정한다.
 * fileType이 없거나 unknown이면 라벨(파일명)의 확장자로 대체한다.
 * @param {string} [fileType] - 트리 노드의 fileType
 * @param {string} [label] - 트리 노드의 라벨(파일명)
 * @returns {string} 소문자 확장자 (판별 불가 시 빈 문자열)
 */
function resolveExtension(fileType, label) {
  const type = (fileType || "").toLowerCase();
  if (type && type !== "unknown") {
    return Object.hasOwn(FILE_TYPE_ALIASES, type)
      ? FILE_TYPE_ALIASES[type]
      : type;
  }
  const name = (label || "").toLowerCase();
  const ext = name.split(".").pop();
  return ext && ext !== name ? ext : "";
}

/**
 * 트리 노드 아이콘.
 * @param {object} props
 * @param {string} [props.fileType] - 노드의 fileType
 * @param {string} [props.label] - 노드 라벨 (fileType이 unknown일 때 확장자 추출용)
 * @param {boolean} [props.expandable] - 하위 노드를 가진 노드는 디렉토리로 취급
 * @param {boolean} [props.muted] - 비노출 카테고리 등 흐리게 표시할 노드
 */
export const TreeNodeIcon = React.memo(function TreeNodeIcon({
  fileType,
  label,
  expandable = false,
  muted = false,
}) {
  const type = (fileType || "").toLowerCase();
  const glyph = expandable
    ? NODE_GLYPHS.folder
    : Object.hasOwn(NODE_GLYPHS, type)
      ? NODE_GLYPHS[type]
      : null;

  if (glyph) {
    return (
      <Box
        component={glyph.Icon}
        className="labelIcon"
        sx={{
          mr: 1,
          fontSize: ICON_SIZE,
          flexShrink: 0,
          color: muted ? MUTED_COLOR : glyph.color,
        }}
      />
    );
  }

  const extension = resolveExtension(fileType, label);
  const style = Object.hasOwn(FILE_STYLES, extension)
    ? FILE_STYLES[extension]
    : undefined;

  return (
    <Box
      className="labelIcon"
      sx={{
        mr: 1,
        width: ICON_SIZE,
        flexShrink: 0,
        display: "flex",
        opacity: muted ? 0.5 : 1,
      }}
    >
      <FileIcon extension={extension} {...style} />
    </Box>
  );
});

TreeNodeIcon.propTypes = {
  fileType: PropTypes.string,
  label: PropTypes.string,
  expandable: PropTypes.bool,
  muted: PropTypes.bool,
};
