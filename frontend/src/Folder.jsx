import "./Folder.css";

import React, { useEffect, useMemo, useRef } from "react";
import PropTypes from "prop-types";

import clsx from "clsx";
import { animated, useSpring } from "@react-spring/web";
import { styled, alpha } from "@mui/material/styles";

import { Card } from "react-bootstrap";
import Box from "@mui/material/Box";
import Collapse from "@mui/material/Collapse";
import Typography from "@mui/material/Typography";
import { RichTreeView } from "@mui/x-tree-view/RichTreeView";
import { treeItemClasses } from "@mui/x-tree-view/TreeItem";
import { useTreeItem } from "@mui/x-tree-view/useTreeItem";
import {
  TreeItemContent,
  TreeItemIconContainer,
  TreeItemLabel,
  TreeItemRoot,
} from "@mui/x-tree-view/TreeItem";
import { TreeItemIcon } from "@mui/x-tree-view/TreeItemIcon";
import { TreeItemProvider } from "@mui/x-tree-view/TreeItemProvider";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import {
  faChevronDown,
  faChevronRight,
} from "@fortawesome/free-solid-svg-icons";
import { findFolderInTree, MORE_ENTRY_FILE_TYPE } from "./folderUtils";
import { TreeNodeIcon } from "./fileTypeIcons";

function DotIcon() {
  return (
    <Box
      sx={{
        width: 6,
        height: 6,
        borderRadius: "70%",
        bgcolor: "warning.main",
        display: "inline-block",
        verticalAlign: "middle",
        zIndex: 1,
        mx: 1,
      }}
    />
  );
}

const StyledTreeItemRoot = styled(TreeItemRoot)(({ theme }) => ({
  color:
    theme.palette.mode === "light"
      ? theme.palette.grey[800]
      : theme.palette.grey[400],
  position: "relative",
  [`& .${treeItemClasses.groupTransition}`]: {
    marginLeft: theme.spacing(3.5),
  },
}));

const CustomTreeItemContent = styled(TreeItemContent)(({ theme }) => ({
  flexDirection: "row-reverse",
  borderRadius: theme.spacing(0.7),
  marginBottom: theme.spacing(0.1),
  marginTop: theme.spacing(0.1),
  padding: theme.spacing(0.1),
  paddingRight: theme.spacing(0.2),
  fontWeight: 400,
  [`& .${treeItemClasses.iconContainer}`]: {
    marginRight: theme.spacing(2),
  },
  [`&.Mui-expanded `]: {
    "&:not(.Mui-focused, .Mui-selected, .Mui-selected.Mui-focused) .labelIcon":
      {
        color:
          theme.palette.mode === "light"
            ? theme.palette.primary.main
            : theme.palette.primary.dark,
      },
    "&::before": {
      content: '""',
      display: "block",
      position: "absolute",
      left: "16px",
      top: "44px",
      height: "calc(100% - 48px)",
      width: "1.5px",
      backgroundColor:
        theme.palette.mode === "light"
          ? theme.palette.grey[300]
          : theme.palette.grey[700],
    },
  },
  "&:hover": {
    backgroundColor: alpha(theme.palette.primary.main, 0.1),
    color:
      theme.palette.mode === "light" ? theme.palette.primary.main : "white",
  },
  [`&.Mui-focused, &.Mui-selected, &.Mui-selected.Mui-focused`]: {
    backgroundColor:
      theme.palette.mode === "light"
        ? theme.palette.primary.main
        : theme.palette.primary.dark,
    color: theme.palette.primary.contrastText,
  },
}));

const AnimatedCollapse = animated(Collapse);

function TransitionComponent(props) {
  const style = useSpring({
    to: {
      // eslint-disable-next-line react/prop-types
      opacity: props.in ? 1 : 0,
      // eslint-disable-next-line react/prop-types
      transform: `translate3d(0,${props.in ? 0 : 20}px,0)`,
    },
  });

  return <AnimatedCollapse style={style} {...props} />;
}

const StyledTreeItemLabelText = styled(Typography)({
  color: "inherit",
  fontFamily: "General Sans",
  fontWeight: 500,
});

function CustomLabel({
  fileType,
  nodeLabel,
  expandable,
  expanded,
  count,
  children,
  ...other
}) {
  return (
    <TreeItemLabel
      {...other}
      sx={{
        display: "flex",
        alignItems: "center",
        width: "100%",
        overflow: "hidden",
      }}
    >
      <TreeNodeIcon
        fileType={fileType}
        label={nodeLabel}
        expandable={expandable}
      />
      <StyledTreeItemLabelText
        variant="body2"
        sx={{ flex: "1 1 0%", minWidth: 0, wordBreak: "break-word" }}
      >
        {children}
      </StyledTreeItemLabelText>
      {count > 0 && (
        <Typography
          variant="caption"
          component="span"
          sx={{
            color: "text.secondary",
            fontWeight: 400,
            fontSize: "0.45rem",
            flexShrink: 0,
            whiteSpace: "nowrap",
            ml: "auto",
            textAlign: "right",
          }}
        >
          {count}
        </Typography>
      )}
      {expandable && expanded && <DotIcon />}
    </TreeItemLabel>
  );
}

CustomLabel.propTypes = {
  fileType: PropTypes.string,
  nodeLabel: PropTypes.string,
  expandable: PropTypes.bool,
  expanded: PropTypes.bool,
  count: PropTypes.number,
  children: PropTypes.node,
};

const isExpandable = (reactChildren) => {
  if (Array.isArray(reactChildren)) {
    return reactChildren.length > 0 && reactChildren.some(isExpandable);
  }
  return Boolean(reactChildren);
};

export const CustomTreeItem = React.forwardRef(
  function CustomTreeItem(props, ref) {
    // eslint-disable-next-line react/prop-types
    const { id, itemId, label, disabled, children, ...other } = props;

    const {
      getContextProviderProps,
      getRootProps,
      getContentProps,
      getIconContainerProps,
      getLabelProps,
      getGroupTransitionProps,
      status,
      publicAPI,
    } = useTreeItem({ id, itemId, children, label, disabled, rootRef: ref });

    const item = useMemo(() => publicAPI.getItem(itemId), [publicAPI, itemId]);
    const expandable = isExpandable(children);

    return (
      <TreeItemProvider {...getContextProviderProps()}>
        <StyledTreeItemRoot
          {...getRootProps({ ...other, "data-item-id": itemId })}
        >
          <CustomTreeItemContent
            {...getContentProps({
              className: clsx("content", {
                "Mui-expanded": status.expanded,
                "Mui-selected": status.selected,
                "Mui-focused": status.focused,
                "Mui-disabled": status.disabled,
              }),
            })}
          >
            <TreeItemIconContainer {...getIconContainerProps()}>
              <TreeItemIcon status={status} />
            </TreeItemIconContainer>

            <CustomLabel
              {...getLabelProps({
                fileType: item?.fileType,
                nodeLabel: item?.label,
                expandable,
                expanded: status.expanded,
                count: item?.count,
              })}
            />
          </CustomTreeItemContent>
          {children && <TransitionComponent {...getGroupTransitionProps()} />}
        </StyledTreeItemRoot>
      </TreeItemProvider>
    );
  },
);

// URL로 지정한 항목을 스크롤 영역 높이의 이 비율 지점에 둔다.
const SCROLL_TARGET_RATIO = 0.1;
// 부모 폴더 펼침 애니메이션 동안 위치가 바뀌므로 이 시간만큼 매 프레임 보정한다.
const SCROLL_SETTLE_MS = 1000;
const USER_SCROLL_EVENTS = ["wheel", "touchstart", "pointerdown", "keydown"];

function findTreeItemElement(container, itemId) {
  const quoted = itemId.replace(/["\\]/g, "\\$&");
  return container.querySelector(`[data-item-id="${quoted}"]`);
}

function scrollItemNearTop(container, element) {
  const offset =
    element.getBoundingClientRect().top - container.getBoundingClientRect().top;
  container.scrollTop += offset - container.clientHeight * SCROLL_TARGET_RATIO;
}

// 펼침 애니메이션이 끝날 때까지 매 프레임 위치를 맞춘다. 사용자가 조작하면 멈춘다.
function startScrollSettling(container, itemId) {
  let frameId = 0;
  const deadline = performance.now() + SCROLL_SETTLE_MS;
  const stop = () => {
    cancelAnimationFrame(frameId);
    USER_SCROLL_EVENTS.forEach((type) =>
      container.removeEventListener(type, stop),
    );
  };
  const step = () => {
    const element = findTreeItemElement(container, itemId);
    if (element) scrollItemNearTop(container, element);
    if (performance.now() < deadline) {
      frameId = requestAnimationFrame(step);
    } else {
      stop();
    }
  };
  USER_SCROLL_EVENTS.forEach((type) =>
    container.addEventListener(type, stop, { passive: true }),
  );
  step();
  return stop;
}

export default function Folder(props) {
  const bodyRef = useRef(null);
  const scrolledTargetRef = useRef(null);
  const stopScrollRef = useRef(null);
  const { scrollTargetId, isOpen } = props;

  // URL로 연 항목이 긴 트리 아래에 묻히지 않도록 상단 근처로 한 번 스크롤한다.
  // 부모를 펼쳐도 Collapse가 자식을 다음 렌더에 붙이므로 DOM 변경을 지켜본다.
  useEffect(() => {
    const container = bodyRef.current;
    // 접힌 카드에는 Card.Body가 없어 container가 null이다.
    if (!container || !scrollTargetId) return;
    if (scrolledTargetRef.current === scrollTargetId) return;
    const startWhenRendered = () => {
      if (!findTreeItemElement(container, scrollTargetId)) return false;
      scrolledTargetRef.current = scrollTargetId;
      stopScrollRef.current?.();
      stopScrollRef.current = startScrollSettling(container, scrollTargetId);
      return true;
    };
    if (startWhenRendered()) return;
    const observer = new MutationObserver(() => {
      if (startWhenRendered()) observer.disconnect();
    });
    observer.observe(container, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [scrollTargetId, isOpen]);

  useEffect(() => () => stopScrollRef.current?.(), []);

  const defaultExpandedItems = useMemo(
    () => (props.folderData || []).map((o) => o.id),
    [props.folderData],
  );
  const treeViewStyles = useMemo(
    () => ({
      height: "fit-content",
      flexGrow: 1,
      overflowY: "auto",
    }),
    [],
  );

  if (!props.isOpen) {
    return (
      <Card>
        <Card.Header
          onClick={() => props.onToggle(true)}
          style={{ cursor: "pointer", userSelect: "none" }}
          className="py-2"
        >
          <FontAwesomeIcon icon={faChevronRight} className="me-2" />
          디렉토리
        </Card.Header>
      </Card>
    );
  }

  return (
    <Card className="h-100">
      <Card.Header
        onClick={() => props.onToggle(false)}
        style={{ cursor: "pointer", userSelect: "none" }}
        className="py-2"
      >
        <FontAwesomeIcon icon={faChevronDown} className="me-2" />
        디렉토리
      </Card.Header>
      <Card.Body className="overflow-auto" ref={bodyRef}>
        <div id="dir_list">
          {props.folderData && (
            <RichTreeView
              items={props.folderData}
              aria-label="file explorer"
              sx={treeViewStyles}
              slots={{ item: CustomTreeItem }}
              defaultExpandedItems={defaultExpandedItems}
              expandedItems={props.expandedItems}
              selectedItems={props.selectedItems}
              onSelectedItemsChange={(event, selectedId) => {
                const found = findFolderInTree(props.folderData, selectedId);
                // '더 보기'는 폴더도 책도 아닌 합성 노드다. 다음 페이지만
                // 요청하고 트리 펼침 상태와 디렉토리 패널은 그대로 둔다.
                if (found && found.fileType === MORE_ENTRY_FILE_TYPE) {
                  props.onClickHandler(selectedId);
                  return;
                }
                const isFolder = found && found.fileType === "folder";
                props.onExpandedItemsChange((prevExpandedItems) => {
                  if (prevExpandedItems.includes(selectedId)) {
                    return prevExpandedItems.filter((x) => x !== selectedId);
                  } else {
                    return [...prevExpandedItems, selectedId];
                  }
                });

                props.onClickHandler(selectedId);
                if (!isFolder) {
                  props.onToggle(false);
                }
              }}
            />
          )}
        </div>
      </Card.Body>
    </Card>
  );
}

Folder.propTypes = {
  folderData: PropTypes.array.isRequired,
  selectedItems: PropTypes.array,
  expandedItems: PropTypes.array.isRequired,
  onExpandedItemsChange: PropTypes.func.isRequired,
  onClickHandler: PropTypes.func.isRequired,
  isOpen: PropTypes.bool.isRequired,
  onToggle: PropTypes.func.isRequired,
  scrollTargetId: PropTypes.string,
};
