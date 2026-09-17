import { useEffect, useState, useCallback, useRef, useMemo } from "react";
import React from "react";
import PropTypes from "prop-types";

import "bootstrap/dist/css/bootstrap.min.css";
import {
  Alert,
  Button,
  Card,
  Form,
  InputGroup,
  Badge,
  Row,
  Col,
  Spinner,
  Modal,
} from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import {
  faPlus,
  faTrash,
  faEdit,
  faRotate,
} from "@fortawesome/free-solid-svg-icons";

import { RichTreeView } from "@mui/x-tree-view/RichTreeView";
import {
  TreeItemContent,
  TreeItemIconContainer,
  TreeItemLabel,
  TreeItemRoot,
} from "@mui/x-tree-view/TreeItem";
import { TreeItemIcon } from "@mui/x-tree-view/TreeItemIcon";
import { TreeItemProvider } from "@mui/x-tree-view/TreeItemProvider";
import { useTreeItem } from "@mui/x-tree-view/useTreeItem";
import { treeItemClasses } from "@mui/x-tree-view/TreeItem";
import { styled, alpha } from "@mui/material/styles";
import { animated, useSpring } from "@react-spring/web";
import Collapse from "@mui/material/Collapse";
import Typography from "@mui/material/Typography";
import Box from "@mui/material/Box";
import clsx from "clsx";

import { jsonGetReq, jsonPostReq, jsonPutReq, jsonDeleteReq } from "./Common";
import {
  formatErrorMessage,
  getReloadRemainingCount,
} from "./categoryAdminUtils";
import { updateCachedMappings } from "./categoryMappingCache";
import {
  findCommonPrefix,
  buildFolderHierarchy,
  findFolderInTree,
  updateFolderInTree,
} from "./folderUtils";
import { TreeNodeIcon } from "./fileTypeIcons";
import ClassifyProposalTable, { isSelectable } from "./ClassifyProposalTable";
import "./Folder.css";
import "./CategoryAdmin.css";

// ── MUI TreeItem 스타일 (Folder.jsx의 CustomTreeItem 스타일 재사용) ──

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

function AdminLabel({
  fileType,
  nodeLabel,
  muted,
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
        muted={muted}
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

AdminLabel.propTypes = {
  fileType: PropTypes.string,
  nodeLabel: PropTypes.string,
  muted: PropTypes.bool,
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

// AdminTreeItem: hidden 카테고리에 opacity 적용하는 래퍼
const AdminTreeItem = React.forwardRef(function AdminTreeItem(props, ref) {
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
  const opacity = item?.isHidden ? 0.5 : 1;

  return (
    <TreeItemProvider {...getContextProviderProps()}>
      <StyledTreeItemRoot {...getRootProps(other)} style={{ opacity }}>
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
          <AdminLabel
            {...getLabelProps({
              fileType: item?.fileType,
              nodeLabel: item?.label,
              muted: Boolean(item?.isHidden),
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
});

// ── 불일치 건수 계산 ──

const ALL_RELOAD_OWNER_SESSION_KEY_PREFIX = "CategoryAdmin.allReloadOwner.";

function normalizeAllReloadOwner(owner) {
  return owner === "mismatch" || owner === "bulk" ? owner : null;
}

function getAllReloadStatusOwner(status, fallbackOwner = null) {
  return (
    normalizeAllReloadOwner(status?.reload_source) ||
    normalizeAllReloadOwner(fallbackOwner) ||
    "bulk"
  );
}

function getAllReloadOwnerSessionKey(contentType) {
  return `${ALL_RELOAD_OWNER_SESSION_KEY_PREFIX}${contentType || "book"}`;
}

function getStoredAllReloadOwner(contentType) {
  if (typeof window === "undefined") return null;

  try {
    return normalizeAllReloadOwner(
      window.sessionStorage.getItem(getAllReloadOwnerSessionKey(contentType)),
    );
  } catch {
    return null;
  }
}

function storeAllReloadOwner(contentType, owner) {
  if (typeof window === "undefined") return;

  try {
    const key = getAllReloadOwnerSessionKey(contentType);
    const normalizedOwner = normalizeAllReloadOwner(owner);
    if (normalizedOwner) {
      window.sessionStorage.setItem(key, normalizedOwner);
    } else {
      window.sessionStorage.removeItem(key);
    }
  } catch {
    // sessionStorage can be unavailable in restricted browser contexts.
  }
}

// 카테고리 하나의 이상 항목 수. 백엔드는 경로를 직접 비교해 anomaly_count를 준다.
// 건수 차이(diff)만 주던 옛 응답은 실제 이상 항목보다 작게 나오지만, 응답을 받지 못하는
// 것보다는 낫기 때문에 fallback으로 남긴다.
function getMismatchItemCount(item) {
  if (item.anomaly_count != null) return item.anomaly_count;
  if (item.diff != null) return Math.abs(item.diff);
  return item.es_count ?? item.fs_count ?? 0;
}

function buildMismatchCounts(mismatchData) {
  const counts = {};
  for (const key of ["mismatches", "es_only", "fs_only"]) {
    for (const item of mismatchData[key] || []) {
      counts[item.category] = getMismatchItemCount(item);
    }
  }
  return counts;
}

function buildMismatchStats(mismatchData) {
  const counts = buildMismatchCounts(mismatchData);
  return {
    categoryCount: Object.keys(counts).length,
    itemCount: Object.values(counts).reduce((sum, count) => sum + count, 0),
  };
}

function countLoadedMismatchEntries(folder) {
  return (folder?.children || []).reduce(
    (count, child) => count + (child.mismatchType ? 1 : 0),
    0,
  );
}

// 재적재는 선택한 카테고리 하나만 처리하므로 하위 폴더 합계(count)가 아니라
// 그 카테고리 자체의 건수를 대상으로 삼는다.
function getMismatchReloadTargetCount(folder) {
  const ownCount = Number(folder?.ownCount ?? folder?.count ?? 0);
  if (Number.isFinite(ownCount) && ownCount > 0) return ownCount;

  return countLoadedMismatchEntries(folder);
}

function encodeCategoryPath(category) {
  return category.split("/").map(encodeURIComponent).join("/");
}

function getCategoryTargetLabel(category) {
  return category === "_root" ? "최상위 디렉토리" : `카테고리 '${category}'`;
}

// ── 메인 컴포넌트 ──

export default function CategoryAdmin({
  contentType = "book",
  initialShowOnlyAbnormal = true,
}) {
  // 공통 상태
  const [loading, setLoading] = useState(false);
  const [message, setMessage] = useState("");
  const [saving, setSaving] = useState(false);
  const [showOnlyAbnormal, setShowOnlyAbnormal] = useState(
    initialShowOnlyAbnormal,
  );

  // 카테고리 트리
  const [folderData, setFolderData] = useState([]);
  const [expandedItems, setExpandedItems] = useState([]);
  const [hiddenCategories, setHiddenCategories] = useState(new Set());
  const [latestExcludedCategories, setLatestExcludedCategories] = useState(
    new Set(),
  );
  const latestExcludedRequestIdRef = useRef(0);
  const [esDocCounts, setEsDocCounts] = useState({});
  const [fsFileCounts, setFsFileCounts] = useState({}); // lazy-loaded per category
  const [mismatchStats, setMismatchStats] = useState({
    categoryCount: 0,
    itemCount: 0,
  });

  // 선택 상태
  const [selectedCategory, setSelectedCategory] = useState("");
  const [selectedMismatch, setSelectedMismatch] = useState(null);
  const [actionResult, setActionResult] = useState(null);

  // 카테고리 관리 (키워드)
  const [mappings, setMappings] = useState({});
  const [newKeyword, setNewKeyword] = useState("");
  const keywordInputRef = useRef(null);

  // rename/delete 모달
  const [showRenameModal, setShowRenameModal] = useState(false);
  const [showDeleteModal, setShowDeleteModal] = useState(false);
  const [showReloadModal, setShowReloadModal] = useState(false);
  const [showMismatchReloadModal, setShowMismatchReloadModal] = useState(false);
  const [showBulkReloadModal, setShowBulkReloadModal] = useState(false);
  const [showDeleteFileModal, setShowDeleteFileModal] = useState(false);
  const [reloading, setReloading] = useState(false);
  const [mismatchReloading, setMismatchReloading] = useState(false);
  const [bulkReloading, setBulkReloading] = useState(false);
  const [allReloadOwner, setAllReloadOwner] = useState(() =>
    getStoredAllReloadOwner(contentType),
  );
  // 일괄(전체) 재적재 락(__all__)의 잔여 건수. 이 상태는 "일괄" 버튼만 반영한다.
  const [bulkRemainingCount, setBulkRemainingCount] = useState(null);
  // 이상 항목 버튼이 시작한 작업의 잔여 건수. 선택 카테고리 전용 작업뿐 아니라
  // 미선택 상태에서 시작한 전체 이상 항목 작업도 이 상태로 표시한다.
  const [mismatchRemainingCount, setMismatchRemainingCount] = useState(null);
  // 분류 제안(propose → 검토 → 승인) 흐름 상태. 파일은 승인 전까지 옮기지 않는다.
  const [proposal, setProposal] = useState(null);
  // 행마다 목적지를 하나만 들고 있는다: 추천 체크에서 왔는지(candidate) 사용자가
  // 직접 고른 것인지(manual)를 함께 담아, 둘이 동시에 켜지는 상태를 만들지 않는다.
  const [proposalChoices, setProposalChoices] = useState({});
  const [proposalPolling, setProposalPolling] = useState(false);
  const [proposalStarting, setProposalStarting] = useState(false);
  const [showProposalApplyModal, setShowProposalApplyModal] = useState(false);
  const [showProposalClearModal, setShowProposalClearModal] = useState(false);
  const [showProposalRestartModal, setShowProposalRestartModal] =
    useState(false);
  const proposalStatusRequestIdRef = useRef(0);
  // 시작 요청(POST)이 서버에 반영되기 전에 폴링(GET)이 먼저 도착해 아직 "idle"인 상태를
  // 읽어버릴 수 있다. 그 사이에는 idle 응답을 무시하고 스피너를 유지한다. 각 작업은
  // 독립적으로 시작될 수 있으므로 각자의 ref로 관리한다.
  const bulkStartPendingRef = useRef(false);
  const mismatchStartPendingRef = useRef(false);
  // 각 폴러가 "실행 중"을 실제로 관측했는지 기록한다. 이 기록이 없으면 백엔드가 계속
  // 돌려주는 직전 작업의 done/error를 새 완료로 오인한다.
  const allReloadTrackingRef = useRef(false);
  const mismatchTrackingRef = useRef(false);
  const bulkStatusRequestIdRef = useRef(0);
  const mismatchStatusRequestIdRef = useRef(0);
  const [indexingFile, setIndexingFile] = useState(false);
  const [newCategoryName, setNewCategoryName] = useState("");

  const apiPrefix = contentType === "comic" ? "/comics" : "";
  const contentLabel = contentType === "comic" ? "만화" : "책";
  const isRootCategory = selectedCategory === "_root";
  const mismatchReloadTargetCategory =
    selectedCategory || selectedMismatch?.category || "";
  const mismatchReloadTargetFolder = mismatchReloadTargetCategory
    ? findFolderInTree(folderData, mismatchReloadTargetCategory)
    : null;
  const selectedMismatchCount = getMismatchReloadTargetCount(
    mismatchReloadTargetFolder,
  );
  const mismatchReloadTargetCount = mismatchReloadTargetCategory
    ? selectedMismatchCount
    : mismatchStats.itemCount;

  const updateAllReloadOwner = useCallback(
    (owner) => {
      const normalizedOwner = normalizeAllReloadOwner(owner);
      storeAllReloadOwner(contentType, normalizedOwner);
      setAllReloadOwner(normalizedOwner);
    },
    [contentType],
  );

  useEffect(() => {
    setAllReloadOwner(getStoredAllReloadOwner(contentType));
  }, [contentType]);

  // ── 데이터 로드 ──

  const loadData = useCallback(() => {
    setLoading(true);
    setMessage("");
    setLatestExcludedCategories(new Set());
    const latestExcludedRequestId = latestExcludedRequestIdRef.current + 1;
    latestExcludedRequestIdRef.current = latestExcludedRequestId;

    let categoriesResult = null;
    let mismatchResult = null;
    let mappingsResult = null;
    let hiddenResult = null;
    let completed = 0;
    let hasError = false;
    const total = 4;

    const tryBuild = () => {
      completed++;
      if (completed < total || hasError) return;

      // 매핑 캐시 갱신
      setMappings(mappingsResult || {});
      updateCachedMappings(contentType, mappingsResult || {});

      // 비노출 카테고리 설정
      setHiddenCategories(new Set(hiddenResult || []));

      // ES 문서 수 저장
      /* v8 ignore next -- categories endpoint success payload is always an object. */
      setEsDocCounts(categoriesResult || {});

      // 불일치 건수
      const mismatchCounts = buildMismatchCounts(mismatchResult);
      setMismatchStats(buildMismatchStats(mismatchResult));

      // 모든 카테고리 목록
      const esCategories = Object.keys(categoriesResult);
      const fsOnlyCategories = (mismatchResult.fs_only || []).map(
        (item) => item.category,
      );
      const allCategories = [
        ...new Set([...esCategories, ...fsOnlyCategories]),
      ].sort((a, b) => a.localeCompare(b));

      const commonPrefix = findCommonPrefix(allCategories);

      // 트리 빌드: 모든 카테고리 표시, 불일치 건수 포함
      const categoryCounts = {};
      for (const cat of allCategories) {
        categoryCounts[cat] = mismatchCounts[cat] || 0;
      }
      const data = buildFolderHierarchy(
        allCategories,
        commonPrefix,
        categoryCounts,
      );

      // 비노출 카테고리에 isHidden 플래그 설정 + 불일치 있는 leaf에 placeholder child
      const hiddenSet = new Set(hiddenResult || []);
      const enriched = data.map((item) => {
        const enrichItem = (node) => {
          const enriched = { ...node, isHidden: hiddenSet.has(node.id) };
          if (enriched.children) {
            enriched.children = enriched.children.map(enrichItem);
          }
          // 불일치가 있는 leaf 카테고리에 placeholder child 추가 (확장 아이콘 표시용).
          // 하위 폴더 합계가 아니라 이 카테고리 자체의 건수로 판단한다.
          if (
            enriched.ownCount > 0 &&
            !enriched.children?.length &&
            !enriched.isVirtualParent
          ) {
            enriched.children = [
              {
                id: enriched.id + "/__placeholder__",
                label: "로딩 중...",
                fileType: "placeholder",
              },
            ];
          }
          return enriched;
        };
        return enrichItem(item);
      });

      setFolderData(enriched);
      setExpandedItems([]);
      setLoading(false);
    };

    // 1) 카테고리 목록
    jsonGetReq(
      apiPrefix + "/categories",
      null,
      (result) => {
        categoriesResult = result;
        tryBuild();
      },
      (err) => {
        hasError = true;
        setMessage(`카테고리 목록을 불러올 수 없습니다. ${err}`);
        setLoading(false);
      },
    );

    // 2) 불일치 데이터
    jsonGetReq(
      apiPrefix + "/category-mismatches",
      null,
      (result) => {
        mismatchResult = result;
        tryBuild();
      },
      (err) => {
        hasError = true;
        setMessage(`불일치 데이터를 불러올 수 없습니다. ${err}`);
        setLoading(false);
      },
    );

    // 3) 키워드 매핑
    jsonGetReq(
      `/category-mappings?content_type=${contentType}`,
      null,
      (result) => {
        mappingsResult = result;
        tryBuild();
      },
      () => {
        mappingsResult = {};
        tryBuild();
      },
    );

    // 4) 비노출 카테고리
    jsonGetReq(
      `/hidden-categories?content_type=${contentType}`,
      null,
      (result) => {
        hiddenResult = result;
        tryBuild();
      },
      () => {
        hiddenResult = [];
        tryBuild();
      },
    );

    jsonGetReq(
      `/latest-excluded-categories?content_type=${contentType}`,
      null,
      (result) => {
        if (latestExcludedRequestIdRef.current === latestExcludedRequestId) {
          setLatestExcludedCategories(new Set(result || []));
        }
      },
      () => {
        if (latestExcludedRequestIdRef.current === latestExcludedRequestId) {
          setLatestExcludedCategories(new Set());
        }
      },
    );
  }, [apiPrefix, contentType]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // ── 재적재 작업 상태 반영 ──
  // 백엔드는 카테고리별 락과 전체(일괄) 락을 독립적으로 추적하므로, 두 락을 각각 폴링해
  // 서로 다른 카테고리(또는 카테고리 vs 일괄)의 진행 상태가 뒤섞이지 않게 한다.

  const applyReloadStatus = useCallback(
    (status, setRunning, setRemaining, startPendingRef, trackingRef) => {
      if (!status || status.status === "idle") {
        // 시작 요청(POST)이 서버에 반영되기 전에 이 폴링이 먼저 도착해 stale한 idle을
        // 읽은 것일 수 있다. 그 경우 스피너를 끄지 않고 유지한다.
        if (startPendingRef.current) return true;
        trackingRef.current = false;
        setRunning(false);
        setRemaining(null);
        return false;
      }
      if (status.status === "running") {
        startPendingRef.current = false;
        trackingRef.current = true;
        setRunning(true);
        setRemaining((current) => getReloadRemainingCount(status, current));
        return true;
      }

      // 백엔드는 새 작업이 시작되기 전까지 직전 작업의 done/error를 계속 돌려준다.
      // 이 컴포넌트가 그 작업을 추적하고 있지 않았다면(마운트 직후, 디렉토리 선택 변경
      // 직후의 첫 폴링) 과거 작업의 잔상이므로 완료로 처리하지 않는다. 그러지 않으면
      // 디렉토리를 클릭할 때마다 같은 완료/실패 안내와 loadData()가 되풀이된다.
      const wasTracking = startPendingRef.current || trackingRef.current;
      startPendingRef.current = false;
      trackingRef.current = false;
      setRunning(false);
      setRemaining(null);
      if (!wasTracking) return false;
      loadData();
      if (status.status !== "done") {
        setMessage(
          formatErrorMessage(
            status.error,
            "이상 항목 ES 재적재에 실패했습니다.",
          ),
        );
        setTimeout(() => setMessage(""), 5000);
      }
      return false;
    },
    [loadData],
  );

  const clearNonOwnerAllReloadState = useCallback((owner) => {
    if (owner === "mismatch") {
      setBulkReloading(false);
      setBulkRemainingCount(null);
    } else {
      setMismatchReloading(false);
      setMismatchRemainingCount(null);
    }
  }, []);

  const applyAllReloadStatus = useCallback(
    (status, fallbackOwner = allReloadOwner) => {
      const statusOwner = getAllReloadStatusOwner(status, fallbackOwner);
      const isMismatchOwner = statusOwner === "mismatch";
      clearNonOwnerAllReloadState(statusOwner);
      const isActive = applyReloadStatus(
        status,
        isMismatchOwner ? setMismatchReloading : setBulkReloading,
        isMismatchOwner ? setMismatchRemainingCount : setBulkRemainingCount,
        bulkStartPendingRef,
        allReloadTrackingRef,
      );

      updateAllReloadOwner(isActive ? statusOwner : null);
      return isActive;
    },
    [
      allReloadOwner,
      applyReloadStatus,
      clearNonOwnerAllReloadState,
      updateAllReloadOwner,
    ],
  );

  // 전체 락 상태 폴링: backend에서는 category=null 락 하나지만, UI에서는 그 작업을
  // 시작한 버튼에만 spinner와 잔여 건수를 표시한다.
  useEffect(() => {
    let cancelled = false;
    const pollStatus = () => {
      const requestId = bulkStatusRequestIdRef.current + 1;
      bulkStatusRequestIdRef.current = requestId;
      jsonGetReq(
        apiPrefix + "/category-mismatches/reload-status",
        null,
        (result) => {
          if (!cancelled && requestId === bulkStatusRequestIdRef.current) {
            applyAllReloadStatus(result);
          }
        },
        () => {},
      );
    };

    pollStatus();
    const isAllReloadRunning =
      allReloadOwner === "mismatch" ? mismatchReloading : bulkReloading;
    const intervalId = isAllReloadRunning
      ? setInterval(pollStatus, 10000)
      : null;
    return () => {
      cancelled = true;
      if (intervalId) clearInterval(intervalId);
    };
  }, [
    allReloadOwner,
    bulkReloading,
    mismatchReloading,
    apiPrefix,
    applyAllReloadStatus,
  ]);

  // 대상 카테고리가 바뀌면 추적 기록을 버린다. 이전 카테고리에서 관측한 "실행 중"을
  // 새 카테고리의 stale done에 그대로 적용하면 안 된다.
  useEffect(() => {
    mismatchTrackingRef.current = false;
  }, [mismatchReloadTargetCategory]);

  // 선택된 카테고리 전용 락 상태 폴링: 카테고리를 선택했을 때만 동작하며, 다른
  // 카테고리나 일괄 재적재와는 독립적으로 이 카테고리의 진행 상태만 추적한다.
  useEffect(() => {
    if (!mismatchReloadTargetCategory) return undefined;

    let cancelled = false;
    const pollStatus = () => {
      const requestId = mismatchStatusRequestIdRef.current + 1;
      mismatchStatusRequestIdRef.current = requestId;
      jsonGetReq(
        `${apiPrefix}/category-mismatches/reload-status?category=${encodeURIComponent(mismatchReloadTargetCategory)}`,
        null,
        (result) => {
          if (!cancelled && requestId === mismatchStatusRequestIdRef.current)
            applyReloadStatus(
              result,
              setMismatchReloading,
              setMismatchRemainingCount,
              mismatchStartPendingRef,
              mismatchTrackingRef,
            );
        },
        () => {},
      );
    };

    pollStatus();
    const intervalId = mismatchReloading
      ? setInterval(pollStatus, 10000)
      : null;
    return () => {
      cancelled = true;
      if (intervalId) clearInterval(intervalId);
    };
  }, [
    mismatchReloadTargetCategory,
    mismatchReloading,
    apiPrefix,
    applyReloadStatus,
  ]);

  // 분류 제안 상태를 반영한다. ready/done/failed(검토 가능한 종료 상태)에 새로
  // 진입할 때만 기본 선택을 다시 채운다 — 이 상태들은 폴링이 멈추는 지점이라
  // 이후 사용자가 직접 고친 체크 상태를 덮어쓸 일이 없다.
  const applyProposalStatus = useCallback((data) => {
    if (!data) return;
    setProposal(data);
    if (["ready", "done", "failed"].includes(data.status)) {
      const items = data.items || [];
      // 확실한 행만 추천 1을 미리 체크해 둔다. target_category는 candidates[0]과
      // 같다는 것이 백엔드의 불변식이라, 체크 표시와 실제 목적지가 어긋나지 않는다.
      const defaults = {};
      for (const item of items) {
        const first = (item.candidates || [])[0];
        if (
          item.grade === "certain" &&
          first?.category &&
          item.apply_status !== "moved"
        ) {
          defaults[item.file_path] = {
            source: "candidate",
            index: 0,
            category: first.category,
          };
        }
      }
      setProposalChoices(defaults);
    }
    setProposalPolling(data.status === "running" || data.status === "applying");
  }, []);

  // 마운트 시 지난 제안 상태를 한 번 복원한다. ready였다면 표를 바로 볼 수 있게
  // 그 제안을 만들었던 카테고리를 선택 상태로 되돌린다.
  useEffect(() => {
    jsonGetReq(
      apiPrefix + "/categories/classify-proposal",
      null,
      (data) => {
        applyProposalStatus(data);
        // ready뿐 아니라 도는 중(running/applying)에도 되돌린다. 그래야 페이지를
        // 떠났다 돌아왔을 때 표와 함께 버튼의 진행 표시도 다시 보인다.
        if (data?.status && data.status !== "idle" && data.source_category) {
          setSelectedCategory(data.source_category);
        }
      },
      () => {},
    );
    // 마운트 시 1회만 복원한다. apiPrefix(컨텐츠 타입)는 이 컴포넌트 수명 동안
    // 바뀌지 않으므로 의존성에서 빠져도 안전하다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 진행 중(running/applying)일 때만 3초 간격으로 폴링하고, 종료 상태가 되면 멈춘다.
  useEffect(() => {
    if (!proposalPolling) return undefined;

    let cancelled = false;
    const pollProposal = () => {
      const requestId = proposalStatusRequestIdRef.current + 1;
      proposalStatusRequestIdRef.current = requestId;
      jsonGetReq(
        apiPrefix + "/categories/classify-proposal",
        null,
        (data) => {
          if (!cancelled && requestId === proposalStatusRequestIdRef.current) {
            applyProposalStatus(data);
          }
        },
        () => {},
      );
    };

    pollProposal();
    const intervalId = setInterval(pollProposal, 3000);
    return () => {
      cancelled = true;
      clearInterval(intervalId);
    };
  }, [proposalPolling, apiPrefix, applyProposalStatus]);

  // ── 폴더 클릭 → 불일치 detail lazy-load ──

  const onFolderClick = useCallback(
    (selectedId) => {
      const selectedFolderData = findFolderInTree(folderData, selectedId);
      /* v8 ignore next 2 -- tree click events target known folder nodes. */
      if (!selectedFolderData || selectedFolderData.fileType !== "folder")
        return;
      /* v8 ignore next -- virtual parent nodes are not expandable mismatch leaves. */
      if (selectedFolderData.isVirtualParent) return;
      // 상세 조회는 이 카테고리 자체의 이상 항목만 가져온다. 하위 폴더 합계인 count로
      // 판단하면 자기 항목이 없는 부모까지 빈 조회를 날린다.
      if (!selectedFolderData.ownCount) return;
      if (selectedFolderData.booksLoaded) return;

      jsonGetReq(
        apiPrefix + "/category-mismatches/" + selectedId,
        null,
        (result) => {
          const entries = [];

          for (const item of result.es_only || []) {
            entries.push({
              id: selectedId + "/es_" + item.book_id.toString(),
              label: item.title + "." + item.file_type,
              fileType: item.file_type,
              children: [],
              mismatchType: "es_only",
              bookId: item.book_id,
              category: selectedId,
              filePath: item.file_path,
            });
          }

          for (const item of result.fs_only || []) {
            entries.push({
              id: selectedId + "/fs_" + item.file_name,
              label: item.file_name,
              fileType: "unknown",
              children: [],
              mismatchType: "fs_only",
              category: selectedId,
              filePath: item.file_path,
            });
          }

          for (const item of result.duplicates || []) {
            const ids = item.docs.map((d) => d.book_id);
            const fileName = item.file_path.split("/").pop();
            entries.push({
              id: selectedId + "/dup_" + ids.join("_"),
              label: `[중복] ${fileName} (${item.docs.length}건)`,
              fileType: item.docs[0]?.file_type || "unknown",
              children: [],
              mismatchType: "duplicate",
              dupDocs: item.docs,
              fileExists: item.file_exists,
              category: selectedId,
              filePath: item.file_path,
            });
          }

          // FS 파일 수 저장
          if (result.fs_count != null) {
            setFsFileCounts((prev) => ({
              ...prev,
              [selectedId]: result.fs_count,
            }));
          }

          const data = updateFolderInTree(folderData, selectedId, (folder) => {
            /* v8 ignore next -- mismatch folders always carry a children array. */
            const existingSubfolders = (folder.children || []).filter(
              (c) => c.fileType === "folder",
            );
            // 요약 스캔 이후 파일이 바뀌었을 수 있으므로 배지를 실제로 받아온
            // 항목 수로 맞춘다. 배지와 펼친 목록이 어긋나면 사용자가 판단할 수 없다.
            const subfolderTotal = existingSubfolders.reduce(
              (sum, c) => sum + Number(c.count || 0),
              0,
            );
            return {
              ...folder,
              booksLoaded: true,
              ownCount: entries.length,
              count: entries.length + subfolderTotal,
              children: [...existingSubfolders, ...entries],
            };
          });
          setFolderData(data);
        },
        (error) => {
          setMessage(
            formatErrorMessage(error, "불일치 상세 조회에 실패했습니다."),
          );
          setTimeout(() => setMessage(""), 5000);
        },
      );
    },
    [folderData, apiPrefix],
  );

  // ── 트리 아이템 클릭 핸들러 ──

  const handleTreeItemClick = useCallback(
    (event, selectedId) => {
      // 불일치 항목(leaf) 클릭 확인 - 재귀 검색
      const findMismatchItem = (items) => {
        for (const item of items) {
          if (item.id === selectedId && item.mismatchType) return item;
          if (item.children) {
            const found = findMismatchItem(item.children);
            if (found) return found;
          }
        }
        return null;
      };

      const foundMismatch = findMismatchItem(folderData);

      if (foundMismatch) {
        setSelectedMismatch(foundMismatch);
        setSelectedCategory("");
        setActionResult(null);
        return;
      }

      // placeholder 클릭 무시
      const foundFolder = findFolderInTree(folderData, selectedId);
      /* v8 ignore next -- tree item clicks originate from rendered tree nodes. */
      if (!foundFolder) return;
      if (foundFolder.fileType === "placeholder") return;

      // 폴더 클릭 → 카테고리 선택 + expand 토글 + 불일치 detail lazy-load
      setSelectedMismatch(null);
      setActionResult(null);
      setSelectedCategory(selectedId);
      setNewKeyword("");

      const willExpand = !expandedItems.includes(selectedId);
      setExpandedItems((prev) =>
        willExpand
          ? [...prev, selectedId]
          : prev.filter((x) => x !== selectedId),
      );
      if (willExpand) {
        onFolderClick(selectedId);
      }
    },
    [folderData, expandedItems, onFolderClick],
  );

  // ── 카테고리 관리 핸들러 ──

  const handleAddKeyword = useCallback(() => {
    if (!selectedCategory || !newKeyword.trim()) return;
    const keyword = newKeyword.trim();

    if (mappings[selectedCategory]?.includes(keyword)) {
      return;
    }

    setSaving(true);
    jsonPostReq(
      `/category-mappings/${encodeURIComponent(selectedCategory)}/keywords?content_type=${contentType}`,
      { keyword },
      () => {
        setMappings((prev) => {
          const updated = { ...prev };
          if (!updated[selectedCategory]) updated[selectedCategory] = [];
          updated[selectedCategory] = [...updated[selectedCategory], keyword];
          updateCachedMappings(contentType, updated);
          return updated;
        });
        setNewKeyword("");
        setTimeout(() => keywordInputRef.current?.focus(), 0);
      },
      (error) => {
        setMessage(
          formatErrorMessage(
            error,
            "이미 등록된 키워드이거나 추가에 실패했습니다.",
          ),
        );
        setTimeout(() => setMessage(""), 3000);
      },
      () => setSaving(false),
    );
  }, [selectedCategory, newKeyword, mappings, contentType]);

  const handleRemoveKeyword = useCallback(
    (keyword) => {
      /* v8 ignore next -- remove buttons are rendered only after category selection. */
      if (!selectedCategory) return;
      setSaving(true);
      jsonDeleteReq(
        `/category-mappings/${encodeURIComponent(selectedCategory)}/keywords/${encodeURIComponent(keyword)}?content_type=${contentType}`,
        null,
        () => {
          setMappings((prev) => {
            const updated = { ...prev };
            /* v8 ignore next 5 -- remove buttons are rendered from existing mapping rows. */
            if (updated[selectedCategory]) {
              updated[selectedCategory] = updated[selectedCategory].filter(
                (k) => k !== keyword,
              );
            }
            updateCachedMappings(contentType, updated);
            return updated;
          });
        },
        (error) => {
          setMessage(formatErrorMessage(error, "삭제에 실패했습니다."));
          setTimeout(() => setMessage(""), 3000);
        },
        () => setSaving(false),
      );
    },
    [selectedCategory, contentType],
  );

  const handleToggleHidden = useCallback(
    (category, currentlyHidden) => {
      setSaving(true);
      jsonPostReq(
        `/hidden-categories/${encodeCategoryPath(category)}?content_type=${contentType}`,
        { hidden: !currentlyHidden },
        (result) => {
          const newHidden = new Set(result || []);
          setHiddenCategories(newHidden);
          // 트리에서 isHidden 플래그 업데이트
          setFolderData((prev) => {
            const updateHidden = (items) =>
              items.map((item) => {
                const updated = { ...item, isHidden: newHidden.has(item.id) };
                if (updated.children) {
                  updated.children = updateHidden(updated.children);
                }
                return updated;
              });
            return updateHidden(prev);
          });
        },
        (error) => {
          setMessage(
            formatErrorMessage(error, "비노출 설정 변경에 실패했습니다."),
          );
          setTimeout(() => setMessage(""), 3000);
        },
        () => setSaving(false),
      );
    },
    [contentType],
  );

  const handleToggleLatestExcluded = useCallback(
    (category, currentlyExcluded) => {
      latestExcludedRequestIdRef.current += 1;
      setSaving(true);
      jsonPostReq(
        `/latest-excluded-categories/${encodeCategoryPath(category)}?content_type=${contentType}`,
        { excluded: !currentlyExcluded },
        (result) => {
          setLatestExcludedCategories(new Set(result || []));
        },
        (error) => {
          setMessage(
            formatErrorMessage(
              error,
              "최신 자료 검색 제외 설정 변경에 실패했습니다.",
            ),
          );
          setTimeout(() => setMessage(""), 3000);
        },
        () => setSaving(false),
      );
    },
    [contentType],
  );

  const handleRenameCategory = useCallback(() => {
    if (!selectedCategory || !newCategoryName.trim()) return;
    const trimmed = newCategoryName.trim();
    if (trimmed === selectedCategory) {
      return;
    }

    setSaving(true);
    jsonPutReq(
      `${apiPrefix}/categories/rename`,
      { old_category: selectedCategory, new_category: trimmed },
      () => {
        setShowRenameModal(false);
        setSelectedCategory("");
        loadData();
      },
      (error) => {
        setMessage(formatErrorMessage(error, "이름 변경에 실패했습니다."));
        setTimeout(() => setMessage(""), 5000);
      },
      () => setSaving(false),
    );
  }, [selectedCategory, newCategoryName, apiPrefix, loadData]);

  const handleDeleteCategory = useCallback(() => {
    /* v8 ignore next -- delete modal opens only after category selection. */
    if (!selectedCategory) return;
    setSaving(true);
    jsonPostReq(
      `${apiPrefix}/categories/delete`,
      { category: selectedCategory },
      () => {
        setShowDeleteModal(false);
        setSelectedCategory("");
        loadData();
      },
      (error) => {
        setMessage(formatErrorMessage(error, "삭제에 실패했습니다."));
        setTimeout(() => setMessage(""), 5000);
      },
      () => setSaving(false),
    );
  }, [selectedCategory, apiPrefix, loadData]);

  const handleReloadCategory = useCallback(() => {
    /* v8 ignore next -- reload modal opens only after category selection. */
    if (!selectedCategory) return;
    setShowReloadModal(false);
    setReloading(true);
    setSaving(true);
    jsonPostReq(
      `${apiPrefix}/category-mismatches/reload`,
      { category: selectedCategory },
      () => {},
      (error) => {
        setMessage(formatErrorMessage(error, "ES 재적재에 실패했습니다."));
        setTimeout(() => setMessage(""), 5000);
      },
      () => {
        setReloading(false);
        setSaving(false);
      },
    );
  }, [selectedCategory, apiPrefix]);

  // 분류 제안 시작. 더 이상 파일을 바로 옮기지 않는다 — 제안만 만들고, 실제
  // 이동은 관리자가 표를 검토하고 승인해야 일어난다.
  const handleStartClassifyProposal = useCallback(() => {
    if (!selectedCategory) return;
    const category = selectedCategory;
    setProposalChoices({});
    setProposalStarting(true);
    setMessage("");
    jsonPostReq(
      `${apiPrefix}/categories/classify-proposal`,
      { category },
      (result) => {
        applyProposalStatus(result);
      },
      (error) => {
        setMessage(formatErrorMessage(error, "분류 제안에 실패했습니다."));
        setTimeout(() => setMessage(""), 5000);
      },
      () => {
        setProposalStarting(false);
      },
    );
  }, [selectedCategory, apiPrefix, applyProposalStatus]);

  // 이미 만들어진 제안에 항목이 있으면, 다시 누르는 순간 관리자가 읽고 체크하고
  // 목적지를 고친 검토 결과가 통째로 사라진다(선택/오버라이드는 즉시 비워지고,
  // 서버도 새 제안을 만들며 이전 항목을 지운다). 잃을 게 있을 때만 확인을 거친다.
  const hasReviewedProposal = (proposal?.items || []).length > 0;

  // 시작 요청이 도는 동안과 분류 작업이 도는 동안 모두 버튼이 돌아야 한다.
  // 시작 요청은 백그라운드 작업을 띄우고 곧바로 끝나므로, 그것만 보면 스피너가
  // 깜빡이고 만다.
  // 제안은 한 번에 하나만 존재하고 그것이 만들어진 카테고리에 속한다. 다른
  // 디렉토리를 선택했는데도 그 표가 남아 있으면, 지금 보는 디렉토리의 책이 그렇게
  // 분류된 것으로 읽힌다 — 잘못 승인하면 엉뚱한 책이 옮겨진다.
  const proposalIsForSelection =
    Boolean(proposal?.source_category) &&
    proposal.source_category === selectedCategory;
  const proposalRunning =
    proposalStarting ||
    (proposal?.status === "running" && proposalIsForSelection);
  // 다른 카테고리의 작업이 도는 중이라 시작할 수 없을 때는, 버튼이 왜 잠겼는지
  // 알려준다. 그러지 않으면 눌러도 아무 일이 없는 것처럼 보인다.
  const proposalBlockedBy =
    proposalPolling && !proposalIsForSelection ? proposal?.source_category : null;

  const handleClickProposeButton = useCallback(() => {
    if (hasReviewedProposal) {
      setShowProposalRestartModal(true);
      return;
    }
    handleStartClassifyProposal();
  }, [hasReviewedProposal, handleStartClassifyProposal]);

  const handleConfirmRestartProposal = useCallback(() => {
    setShowProposalRestartModal(false);
    handleStartClassifyProposal();
  }, [handleStartClassifyProposal]);

  // 승인 대상: 선택된 행 중에서도 목적지가 있는 행만 최종적으로 담는다.
  // isSelectable 필터는 2차 방어다 — 표 컴포넌트가 목적지를 지울 때 선택에서
  // 빼주지만, 제출 직전에 한 번 더 걸러 목적지 없는 항목이 승인 요청에
  // 실리는 것을 막는다.
  const proposalApplyItems = useMemo(() => {
    const items = proposal?.items || [];
    return items
      .filter((item) => isSelectable(item, proposalChoices))
      .map((item) => ({
        file_path: item.file_path,
        target_category: proposalChoices[item.file_path].category,
      }));
  }, [proposal, proposalChoices]);

  const handleApplyClassifyProposal = useCallback(() => {
    setShowProposalApplyModal(false);
    setSaving(true);
    setMessage("");
    jsonPostReq(
      `${apiPrefix}/categories/classify-proposal/apply`,
      { items: proposalApplyItems },
      () => {
        // 서버가 응답 전에 이미 상태를 applying으로 선점해 두므로, 다음 폴링이
        // 실제 진행 상황(이동 상태별 항목)을 곧바로 읽어온다.
        setProposal((prev) => (prev ? { ...prev, status: "applying" } : prev));
        setProposalPolling(true);
      },
      (error) => {
        setMessage(formatErrorMessage(error, "분류 승인에 실패했습니다."));
        setTimeout(() => setMessage(""), 5000);
      },
      () => {
        setSaving(false);
      },
    );
  }, [apiPrefix, proposalApplyItems]);

  const handleClearAppliedProposalItems = useCallback(() => {
    setShowProposalClearModal(false);
    setSaving(true);
    setMessage("");
    jsonDeleteReq(
      `${apiPrefix}/categories/classify-proposal/applied`,
      null,
      () => {
        jsonGetReq(
          apiPrefix + "/categories/classify-proposal",
          null,
          applyProposalStatus,
          (error) => {
            // 삭제(DELETE) 자체는 성공했다. 이 재조회만 실패하면 표가 갱신되지
            // 않아 관리자가 삭제 여부를 알 길이 없으므로, 다른 핸들러와 같은
            // 방식으로 실패를 드러낸다.
            setMessage(
              formatErrorMessage(
                error,
                "완료 기록은 삭제했지만 표를 다시 불러오지 못했습니다.",
              ),
            );
            setTimeout(() => setMessage(""), 5000);
          },
        );
      },
      (error) => {
        setMessage(formatErrorMessage(error, "완료 기록 삭제에 실패했습니다."));
        setTimeout(() => setMessage(""), 5000);
      },
      () => {
        setSaving(false);
      },
    );
  }, [apiPrefix, applyProposalStatus]);

  // 재적재는 서버에서 백그라운드로 돈다. 여기서는 시작만 확인하고, 완료/실패 메시지는
  // 위쪽 상태 폴링(applyReloadStatus)이 처리한다. 카테고리별 락과 전체(일괄) 락은 서로
  // 독립적이지만, 일괄이 진행 중이면 모든 카테고리에 영향을 주므로 카테고리별 요청을 막고,
  // 그 반대도 마찬가지다. 이 경우 서버는 에러가 아니라 그 "다른" 작업의 상태를
  // {already_running: true, ...}로 돌려주므로, 그 결과의 category가 내가 요청한 것과
  // 다르면 내 요청은 시작되지 않은 것이니 스피너를 켜 둔 채로 두면 안 된다.
  const startAllReloadMismatches = useCallback(
    (owner, failureMessage, blockedActionLabel) => {
      const isMismatchOwner = owner === "mismatch";
      const setOwnerReloading = isMismatchOwner
        ? setMismatchReloading
        : setBulkReloading;
      const setOwnerRemainingCount = isMismatchOwner
        ? setMismatchRemainingCount
        : setBulkRemainingCount;

      updateAllReloadOwner(owner);
      bulkStatusRequestIdRef.current += 1;
      bulkStartPendingRef.current = true;
      setOwnerReloading(true);
      setOwnerRemainingCount(mismatchStats.itemCount);
      setSaving(true);
      setMessage("");
      jsonPostReq(
        `${apiPrefix}/category-mismatches/reload-all`,
        { reload_source: owner },
        (result) => {
          if (result && result.already_running && result.category) {
            bulkStartPendingRef.current = false;
            setOwnerReloading(false);
            setOwnerRemainingCount(null);
            updateAllReloadOwner(null);
            setMessage(
              `카테고리 '${result.category}' 재적재가 이미 진행 중이라 지금은 ${blockedActionLabel}를 실행할 수 없습니다. 완료 후 다시 시도하세요.`,
            );
            setTimeout(() => setMessage(""), 5000);
          } else if (result && result.already_running) {
            applyAllReloadStatus(result, owner);
          } else {
            const responseOwner =
              normalizeAllReloadOwner(result?.reload_source) || owner;
            if (responseOwner !== owner) {
              setOwnerReloading(false);
              setOwnerRemainingCount(null);
              if (responseOwner === "mismatch") {
                setMismatchReloading(true);
                setMismatchRemainingCount(mismatchStats.itemCount);
              } else {
                setBulkReloading(true);
                setBulkRemainingCount(mismatchStats.itemCount);
              }
            }
            updateAllReloadOwner(responseOwner);
          }
        },
        (error) => {
          setMessage(formatErrorMessage(error, failureMessage));
          setTimeout(() => setMessage(""), 5000);
          bulkStartPendingRef.current = false;
          setOwnerReloading(false);
          setOwnerRemainingCount(null);
          updateAllReloadOwner(null);
        },
        () => setSaving(false),
      );
    },
    [
      apiPrefix,
      mismatchStats.itemCount,
      applyAllReloadStatus,
      updateAllReloadOwner,
    ],
  );

  const handleReloadCategoryMismatches = useCallback(() => {
    setShowMismatchReloadModal(false);
    const targetCategory = mismatchReloadTargetCategory;
    if (!targetCategory) {
      startAllReloadMismatches(
        "mismatch",
        "이상 항목 ES 재적재 시작에 실패했습니다.",
        "이상 항목 재적재",
      );
      return;
    }

    mismatchStartPendingRef.current = true;
    setMismatchReloading(true);
    setMismatchRemainingCount(selectedMismatchCount);
    setSaving(true);
    setMessage("");

    jsonPostReq(
      `${apiPrefix}/category-mismatches/reload-mismatches`,
      { category: targetCategory },
      (result) => {
        const requestedCategory = targetCategory;
        if (
          result &&
          result.already_running &&
          (result.category || null) !== requestedCategory
        ) {
          mismatchStartPendingRef.current = false;
          setMismatchReloading(false);
          setMismatchRemainingCount(null);
          setMessage(
            result.category
              ? `카테고리 '${result.category}' 재적재가 이미 진행 중이라 지금은 실행할 수 없습니다. 완료 후 다시 시도하세요.`
              : "일괄 재적재가 이미 진행 중이라 지금은 실행할 수 없습니다. 완료 후 다시 시도하세요.",
          );
          setTimeout(() => setMessage(""), 5000);
        } else if (result && result.already_running) {
          applyReloadStatus(
            result,
            setMismatchReloading,
            setMismatchRemainingCount,
            mismatchStartPendingRef,
            mismatchTrackingRef,
          );
        }
      },
      (error) => {
        setMessage(
          formatErrorMessage(error, "이상 항목 ES 재적재 시작에 실패했습니다."),
        );
        setTimeout(() => setMessage(""), 5000);
        mismatchStartPendingRef.current = false;
        setMismatchReloading(false);
        setMismatchRemainingCount(null);
      },
      () => setSaving(false),
    );
  }, [
    mismatchReloadTargetCategory,
    selectedMismatchCount,
    apiPrefix,
    applyReloadStatus,
    startAllReloadMismatches,
  ]);

  const handleBulkReloadMismatches = useCallback(() => {
    setShowBulkReloadModal(false);
    startAllReloadMismatches(
      "bulk",
      "불일치 일괄 ES 재적재 시작에 실패했습니다.",
      "일괄 재적재",
    );
  }, [startAllReloadMismatches]);

  // ── 불일치 관리 핸들러 ──

  const handleDeleteEsDoc = useCallback(() => {
    /* v8 ignore next 2 -- delete ES button is rendered only for es_only mismatches. */
    if (!selectedMismatch || selectedMismatch.mismatchType !== "es_only")
      return;
    setActionResult(null);
    jsonDeleteReq(
      apiPrefix + "/books/" + selectedMismatch.bookId,
      null,
      (result) => {
        const warning = result?.warning;
        const msg = warning
          ? `${contentLabel} 정보가 삭제되었습니다. (${warning})`
          : `${contentLabel} 정보가 삭제되었습니다.`;
        setActionResult({ type: "success", message: msg });
        const data = updateFolderInTree(
          folderData,
          selectedMismatch.category,
          (folder) => ({
            ...folder,
            /* v8 ignore next -- mismatch folders always carry a children array. */
            children: (folder.children || []).filter(
              (c) => c.id !== selectedMismatch.id,
            ),
          }),
        );
        setFolderData(data);
        setSelectedMismatch(null);
      },
      (error) => {
        setActionResult({ type: "error", message: `삭제 실패: ${error}` });
      },
    );
  }, [selectedMismatch, folderData, apiPrefix, contentLabel]);

  const handleIndexFile = useCallback(() => {
    /* v8 ignore next 2 -- index button is rendered only for fs_only mismatches. */
    if (!selectedMismatch || selectedMismatch.mismatchType !== "fs_only")
      return;
    setActionResult(null);
    setIndexingFile(true);
    jsonPostReq(
      apiPrefix + "/category-mismatches/index-file",
      { file_path: selectedMismatch.filePath },
      () => {
        setActionResult({ type: "success", message: "ES에 적재되었습니다." });
        const data = updateFolderInTree(
          folderData,
          selectedMismatch.category,
          (folder) => ({
            ...folder,
            /* v8 ignore next -- mismatch folders always carry a children array. */
            children: (folder.children || []).filter(
              (c) => c.id !== selectedMismatch.id,
            ),
          }),
        );
        setFolderData(data);
        setSelectedMismatch(null);
      },
      (error) => {
        setActionResult({ type: "error", message: `ES 적재 실패: ${error}` });
      },
      () => setIndexingFile(false),
    );
  }, [selectedMismatch, folderData, apiPrefix]);

  const handleDeleteFile = useCallback(() => {
    setShowDeleteFileModal(false);
    /* v8 ignore next 2 -- delete-file button is rendered only for fs_only mismatches. */
    if (!selectedMismatch || selectedMismatch.mismatchType !== "fs_only")
      return;
    setActionResult(null);
    jsonPostReq(
      apiPrefix + "/category-mismatches/delete-file",
      { file_path: selectedMismatch.filePath },
      () => {
        setActionResult({ type: "success", message: "파일이 삭제되었습니다." });
        const data = updateFolderInTree(
          folderData,
          selectedMismatch.category,
          (folder) => ({
            ...folder,
            /* v8 ignore next -- mismatch folders always carry a children array. */
            children: (folder.children || []).filter(
              (c) => c.id !== selectedMismatch.id,
            ),
          }),
        );
        setFolderData(data);
        setSelectedMismatch(null);
      },
      (error) => {
        setActionResult({ type: "error", message: `파일 삭제 실패: ${error}` });
      },
    );
  }, [selectedMismatch, folderData, apiPrefix]);

  // ── 키 핸들러 ──

  const handleKeyDown = useCallback(
    (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleAddKeyword();
      }
    },
    [handleAddKeyword],
  );

  const handleRenameKeyDown = useCallback(
    (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleRenameCategory();
      }
    },
    [handleRenameCategory],
  );

  // ── 파생 값 ──

  const isSubcategory =
    typeof selectedCategory === "string" && selectedCategory.includes("/");
  const currentKeywords = selectedCategory
    ? mappings[selectedCategory] || []
    : [];
  // apply_category_changes는 target_category로 최상위 카테고리만 받는다. 하위
  // 카테고리를 고르게 두면 서버가 어차피 거부하므로 애초에 고를 수 없게 한다.
  const topLevelCategoryNames = Object.keys(esDocCounts).filter(
    (name) => name !== "_root" && !name.includes("/"),
  );

  const displayedFolderData = useMemo(() => {
    if (!showOnlyAbnormal) return folderData;

    const filterAbnormalItems = (items) =>
      items.flatMap((item) => {
        const children = item.children || [];
        const filteredChildren = filterAbnormalItems(children);
        const placeholderChildren = children.filter(
          (child) => child.fileType === "placeholder",
        );
        const isAbnormal =
          Boolean(item.mismatchType) || Number(item.count || 0) > 0;

        if (!isAbnormal && filteredChildren.length === 0) return [];

        return [
          {
            ...item,
            children: [...filteredChildren, ...placeholderChildren],
          },
        ];
      });

    return filterAbnormalItems(folderData);
  }, [folderData, showOnlyAbnormal]);

  const displayedTreeMeta = useMemo(() => {
    const ids = [];
    const collectIds = (items) => {
      for (const item of items) {
        ids.push(item.id);
        if (item.children?.length) {
          collectIds(item.children);
        }
      }
    };

    collectIds(displayedFolderData);

    return {
      ids: new Set(ids),
      key: `${contentType}:${showOnlyAbnormal ? "abnormal" : "all"}:${ids.join("|")}`,
    };
  }, [contentType, displayedFolderData, showOnlyAbnormal]);

  const displayedExpandedItems = useMemo(
    () => expandedItems.filter((itemId) => displayedTreeMeta.ids.has(itemId)),
    [expandedItems, displayedTreeMeta],
  );

  const treeViewStyles = useMemo(
    () => ({
      height: "fit-content",
      flexGrow: 1,
      overflowY: "auto",
    }),
    [],
  );

  // message는 이 컴포넌트 안에서 항상 문자열로만 세팅된다(setMessage 호출부 전부 문자열 리터럴/
  // 템플릿 리터럴 또는 formatErrorMessage()의 반환값).
  // 완료·안내 메시지는 더 이상 세팅하지 않는다. 상태 폴링이 직전 작업의 done을 계속
  // 돌려주는 탓에 같은 완료 배너가 되풀이 노출되어 방해가 됐다. 남은 것은 오류뿐이므로
  // 문자열로 심각도를 추측하지 않고 항상 alert-danger로 표시한다.

  // ── 렌더링 ──

  return (
    <>
      {message && <div className="alert alert-danger py-1 mb-2">{message}</div>}
      {loading ? (
        <div className="text-center p-4">
          <Spinner animation="border" />
          <p className="mt-2">로딩 중...</p>
        </div>
      ) : folderData.length === 0 ? (
        <div className="text-muted p-3">카테고리 없음</div>
      ) : (
        <Row className="g-0">
          <Col md={4} className="category-admin-directory">
            <Card className="h-100">
              <Card.Header className="py-1 d-flex flex-wrap align-items-center gap-2">
                <span className="me-auto">디렉토리</span>
                <Form.Check
                  type="switch"
                  id={`show-only-abnormal-${contentType}`}
                  label="이상 항목만"
                  aria-label="이상 항목만 보기"
                  checked={showOnlyAbnormal}
                  onChange={(event) =>
                    setShowOnlyAbnormal(event.target.checked)
                  }
                  className="m-0"
                />
                <Button
                  variant="outline-danger"
                  size="sm"
                  aria-label="일괄 재적재"
                  disabled={
                    saving ||
                    bulkReloading ||
                    mismatchReloading ||
                    mismatchStats.itemCount === 0
                  }
                  onClick={() => setShowBulkReloadModal(true)}
                  title="불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)"
                >
                  {bulkReloading ? (
                    <span className="d-flex align-items-center gap-1">
                      <Spinner animation="border" size="sm" />
                      {bulkRemainingCount !== null && (
                        <small style={{ fontSize: "0.7rem" }}>
                          잔여 {bulkRemainingCount}건
                        </small>
                      )}
                    </span>
                  ) : (
                    <span className="d-flex align-items-center gap-1">
                      일괄 <FontAwesomeIcon icon={faRotate} />
                    </span>
                  )}
                </Button>
                <Button
                  variant="outline-warning"
                  size="sm"
                  aria-label="이상 항목 재적재"
                  disabled={
                    saving ||
                    bulkReloading ||
                    mismatchReloading ||
                    mismatchReloadTargetCount === 0
                  }
                  onClick={() => setShowMismatchReloadModal(true)}
                  title={
                    mismatchReloadTargetCategory
                      ? "선택 디렉토리 이상 항목만 ES 재적재"
                      : "전체 이상 항목 ES 재적재"
                  }
                >
                  {mismatchReloading ? (
                    <span className="d-flex align-items-center gap-1">
                      <Spinner animation="border" size="sm" />
                      {mismatchRemainingCount !== null && (
                        <small style={{ fontSize: "0.7rem" }}>
                          잔여 {mismatchRemainingCount}건
                        </small>
                      )}
                    </span>
                  ) : (
                    <span className="d-flex align-items-center gap-1">
                      이상 항목 <FontAwesomeIcon icon={faRotate} />
                    </span>
                  )}
                </Button>
              </Card.Header>
              <Card.Body className="overflow-auto">
                <div id="dir_list">
                  <RichTreeView
                    key={displayedTreeMeta.key}
                    items={displayedFolderData}
                    aria-label="category admin"
                    sx={treeViewStyles}
                    slots={{ item: AdminTreeItem }}
                    expandedItems={displayedExpandedItems}
                    onSelectedItemsChange={handleTreeItemClick}
                  />
                </div>
              </Card.Body>
            </Card>
          </Col>
          <Col md={8}>
            {/* 카테고리 선택 시 */}
            {selectedCategory && !selectedMismatch && (
              <Card>
                <Card.Header className="py-1 d-flex justify-content-between align-items-center">
                  <span>
                    <strong>{selectedCategory}</strong>
                    {saving && (
                      <Spinner animation="border" size="sm" className="ms-2" />
                    )}
                  </span>
                  <span className="d-flex gap-1">
                    <Badge bg="secondary">
                      ES {esDocCounts[selectedCategory] ?? 0}건
                    </Badge>
                    {fsFileCounts[selectedCategory] != null && (
                      <Badge bg="info">
                        파일 {fsFileCounts[selectedCategory]}건
                      </Badge>
                    )}
                  </span>
                </Card.Header>
                <Card.Body>
                  <Form.Check
                    type="checkbox"
                    id={`hidden-${contentType}-${selectedCategory}`}
                    label="사용자 비노출"
                    checked={hiddenCategories.has(selectedCategory)}
                    onChange={() =>
                      handleToggleHidden(
                        selectedCategory,
                        hiddenCategories.has(selectedCategory),
                      )
                    }
                    disabled={saving}
                    className="mb-2"
                  />
                  <Form.Check
                    type="checkbox"
                    id={`latest-excluded-${contentType}-${selectedCategory}`}
                    label="최신 자료 검색 제외"
                    checked={latestExcludedCategories.has(selectedCategory)}
                    onChange={() =>
                      handleToggleLatestExcluded(
                        selectedCategory,
                        latestExcludedCategories.has(selectedCategory),
                      )
                    }
                    disabled={saving}
                    className="mb-2"
                  />
                  {!isSubcategory && (
                    <>
                      <InputGroup className="mb-2">
                        <Form.Control
                          ref={keywordInputRef}
                          type="text"
                          placeholder="새 키워드 입력"
                          value={newKeyword}
                          onChange={(e) => setNewKeyword(e.target.value)}
                          onKeyDown={handleKeyDown}
                          disabled={saving}
                        />
                        <Button
                          variant="outline-primary"
                          onClick={handleAddKeyword}
                          disabled={saving || !newKeyword.trim()}
                        >
                          <FontAwesomeIcon icon={faPlus} /> 추가
                        </Button>
                      </InputGroup>
                      <div className="d-flex flex-wrap gap-1">
                        {currentKeywords.map((keyword) => (
                          <Badge
                            key={keyword}
                            bg="info"
                            className="d-flex align-items-center gap-1"
                            style={{
                              fontSize: "0.85rem",
                              padding: "0.4rem 0.6rem",
                            }}
                          >
                            {keyword}
                            <FontAwesomeIcon
                              icon={faTrash}
                              style={{
                                cursor: saving ? "not-allowed" : "pointer",
                                marginLeft: "4px",
                              }}
                              onClick={() =>
                                !saving && handleRemoveKeyword(keyword)
                              }
                            />
                          </Badge>
                        ))}
                        {currentKeywords.length === 0 && (
                          <span className="text-muted">
                            등록된 키워드가 없습니다.
                          </span>
                        )}
                      </div>
                    </>
                  )}
                  <div className="d-flex flex-wrap gap-1 mb-2">
                    <Button
                      variant="outline-secondary"
                      size="sm"
                      disabled={saving || isRootCategory}
                      onClick={() => {
                        setNewCategoryName(selectedCategory);
                        setShowRenameModal(true);
                      }}
                      title={
                        isRootCategory
                          ? "최상위 디렉토리는 이름을 변경할 수 없습니다"
                          : "이름 변경"
                      }
                    >
                      이름 변경 <FontAwesomeIcon icon={faEdit} />
                    </Button>
                    <Button
                      variant="outline-danger"
                      size="sm"
                      disabled={saving || isRootCategory}
                      onClick={() => setShowDeleteModal(true)}
                      title={
                        isRootCategory
                          ? "최상위 디렉토리는 삭제할 수 없습니다"
                          : "카테고리 삭제"
                      }
                    >
                      삭제 <FontAwesomeIcon icon={faTrash} />
                    </Button>
                    <Button
                      variant="outline-success"
                      size="sm"
                      disabled={saving || bulkReloading || mismatchReloading}
                      onClick={() => setShowReloadModal(true)}
                      title="ES 재적재"
                    >
                      {reloading ? (
                        <Spinner animation="border" size="sm" />
                      ) : (
                        <>
                          ES 재적재 <FontAwesomeIcon icon={faRotate} />
                        </>
                      )}
                    </Button>
                    <Button
                      variant="outline-warning"
                      size="sm"
                      disabled={
                        saving ||
                        bulkReloading ||
                        mismatchReloading ||
                        mismatchReloadTargetCount === 0
                      }
                      onClick={() => setShowMismatchReloadModal(true)}
                      title="이상 항목만 ES 재적재"
                    >
                      {mismatchReloading ? (
                        <span className="d-flex align-items-center gap-1">
                          <Spinner animation="border" size="sm" />
                          {mismatchRemainingCount !== null && (
                            <small style={{ fontSize: "0.7rem" }}>
                              잔여 {mismatchRemainingCount}건
                            </small>
                          )}
                        </span>
                      ) : (
                        <span className="d-flex align-items-center gap-1">
                          이상 항목 재적재 <FontAwesomeIcon icon={faRotate} />
                        </span>
                      )}
                    </Button>
                    <Button
                      variant="outline-primary"
                      size="sm"
                      disabled={saving || proposalStarting || proposalPolling}
                      onClick={handleClickProposeButton}
                      title={
                        proposalBlockedBy
                          ? `${proposalBlockedBy} 분류 작업이 끝나야 시작할 수 있습니다`
                          : "분류 제안"
                      }
                    >
                      {/* 시작 요청이 끝나면 스피너를 내리던 예전 동작은, 정작 오래
                          걸리는 분류 자체가 도는 동안 버튼이 멈춘 것처럼 보이게 했다.
                          책 한 권에 서점 조회까지 하면 수 초가 들어 카테고리 전체로는
                          한참이다. 작업이 끝날 때까지 돌리고 진행 수를 같이 보여준다. */}
                      {proposalRunning ? (
                        <span className="d-flex align-items-center gap-1">
                          <Spinner animation="border" size="sm" />
                          {proposal?.total_count > 0 && (
                            <small style={{ fontSize: "0.7rem" }}>
                              {proposal.processed_count ?? 0}/
                              {proposal.total_count}
                            </small>
                          )}
                        </span>
                      ) : (
                        <>
                          분류 제안 <FontAwesomeIcon icon={faRotate} />
                        </>
                      )}
                    </Button>
                  </div>
                </Card.Body>
              </Card>
            )}

            {/* 분류 제안 검토 · 승인 */}
            {proposal && proposal.status !== "idle" && proposalIsForSelection && (
              <Card className="mt-2">
                <Card.Header className="py-1 d-flex justify-content-between align-items-center">
                  <strong>분류 제안: {proposal.source_category}</strong>
                  <span className="text-muted" style={{ fontSize: "0.8rem" }}>
                    {proposal.processed_count ?? 0} /{" "}
                    {proposal.total_count ?? 0}
                  </span>
                </Card.Header>
                <Card.Body>
                  {proposal.status === "failed" && (
                    // I2: error는 지금까지 상태 파일에만 쌓이고 화면 어디서도 읽지 않았다.
                    // 제안이 중간에 죽으면(예: 1,200권 중 34권) 스피너만 멈추고 아무 신호가
                    // 없어, 분류 승인이 (C1/I4가 재시도할 수 있어야 하므로) 계속 켜진 채로
                    // 관리자가 미완성 제안을 완성됐다고 착각해 승인하기 쉽다. 실패라고
                    // 막지는 않되(그러면 재시도를 못 한다), 중단됐다는 사실과 어디까지
                    // 처리됐는지는 분명히 보여준다.
                    <Alert
                      variant="danger"
                      className="py-2 px-3 mb-2"
                      style={{ fontSize: "0.85rem" }}
                    >
                      <strong>작업이 중단됐습니다.</strong>{" "}
                      {proposal.error || "원인을 알 수 없습니다."}
                      {typeof proposal.total_count === "number" &&
                        typeof proposal.processed_count === "number" &&
                        proposal.processed_count < proposal.total_count && (
                          <>
                            {" "}
                            전체 {proposal.total_count}건 중{" "}
                            {proposal.processed_count}건까지만 처리됐습니다. 이
                            제안은 아직 끝나지 않았습니다 — 승인 전에
                            확인하세요.
                          </>
                        )}
                    </Alert>
                  )}
                  <ClassifyProposalTable
                    items={proposal.items || []}
                    categories={topLevelCategoryNames}
                    choices={proposalChoices}
                    onChoicesChange={setProposalChoices}
                  />
                  <div className="d-flex gap-2 mt-2">
                    <Button
                      variant="primary"
                      size="sm"
                      disabled={
                        saving ||
                        !["ready", "done", "failed"].includes(
                          proposal.status,
                        ) ||
                        proposalApplyItems.length === 0
                      }
                      onClick={() => setShowProposalApplyModal(true)}
                    >
                      분류 승인 ({proposalApplyItems.length}건)
                    </Button>
                    <Button
                      variant="outline-secondary"
                      size="sm"
                      disabled={
                        saving ||
                        proposal.status === "running" ||
                        proposal.status === "applying" ||
                        !(proposal.items || []).some(
                          (item) => item.apply_status === "moved",
                        )
                      }
                      onClick={() => setShowProposalClearModal(true)}
                    >
                      완료 기록 삭제
                    </Button>
                  </div>
                </Card.Body>
              </Card>
            )}

            {/* 불일치 항목 선택 시 */}
            {selectedMismatch && (
              <Card>
                <Card.Header className="py-1">
                  <strong>{selectedMismatch.label}</strong>
                </Card.Header>
                <Card.Body>
                  <div
                    className="text-muted mb-2"
                    style={{ fontSize: "0.85rem" }}
                  >
                    {selectedMismatch.mismatchType === "es_only"
                      ? `${contentLabel} 정보만 존재하고 파일시스템에는 존재하지 않습니다.`
                      : selectedMismatch.mismatchType === "duplicate"
                        ? "동일한 파일 경로로 ES에 중복 문서가 존재합니다. 파일 삭제 후 재적재 시 발생할 수 있습니다."
                        : `${contentLabel} 정보는 없고 파일시스템에만 존재합니다.`}
                  </div>
                  {selectedMismatch.mismatchType === "duplicate" &&
                    selectedMismatch.dupDocs && (
                      <>
                        <table
                          className="table table-sm table-bordered mb-2"
                          style={{ fontSize: "0.8rem" }}
                        >
                          <thead>
                            <tr>
                              <th>ID</th>
                              <th>제목</th>
                              <th>저자</th>
                              <th>파일 연결</th>
                              <th>액션</th>
                            </tr>
                          </thead>
                          <tbody>
                            {selectedMismatch.dupDocs.map((doc) => (
                              <tr key={doc.book_id}>
                                <td>{doc.book_id}</td>
                                <td>{doc.title}</td>
                                <td>{doc.author}</td>
                                <td className="text-center">
                                  {selectedMismatch.fileExists ? (
                                    <Badge
                                      bg={
                                        doc.file_linked ? "success" : "warning"
                                      }
                                      style={{ fontSize: "0.7rem" }}
                                    >
                                      {doc.file_linked ? "연결됨" : "미연결"}
                                    </Badge>
                                  ) : (
                                    <Badge
                                      bg="danger"
                                      style={{ fontSize: "0.7rem" }}
                                    >
                                      파일 없음
                                    </Badge>
                                  )}
                                </td>
                                <td>
                                  <Button
                                    variant="outline-primary"
                                    size="sm"
                                    className="me-1 py-0"
                                    onClick={() =>
                                      window.open(
                                        `/${contentType === "comic" ? "comics-view" : "book-view"}/${doc.book_id}?category=${encodeURIComponent(selectedMismatch.category)}`,
                                        "_blank",
                                        "noopener",
                                      )
                                    }
                                  >
                                    조회
                                  </Button>
                                  {!doc.file_linked && (
                                    <Button
                                      variant="outline-danger"
                                      size="sm"
                                      className="py-0"
                                      onClick={() => {
                                        if (
                                          !window.confirm(
                                            `ID ${doc.book_id} (${doc.title}) ES 문서를 삭제하시겠습니까? (파일은 유지됩니다)`,
                                          )
                                        )
                                          return;
                                        jsonDeleteReq(
                                          apiPrefix +
                                            "/category-mismatches/es-doc/" +
                                            doc.book_id,
                                          null,
                                          () => {
                                            setActionResult({
                                              type: "success",
                                              message: `ID ${doc.book_id} 문서가 삭제되었습니다.`,
                                            });
                                            setSelectedMismatch(null);
                                            loadData();
                                          },
                                          (error) =>
                                            setActionResult({
                                              type: "danger",
                                              message: formatErrorMessage(
                                                error,
                                                "삭제 실패",
                                              ),
                                            }),
                                        );
                                      }}
                                    >
                                      삭제
                                    </Button>
                                  )}
                                </td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </>
                    )}
                  <div className="d-flex flex-wrap gap-1">
                    {selectedMismatch.mismatchType === "es_only" && (
                      <>
                        <Button
                          variant="outline-warning"
                          size="sm"
                          onClick={() =>
                            window.open(
                              `/${contentType === "comic" ? "comics-edit" : "book-edit"}/${selectedMismatch.bookId}?category=${encodeURIComponent(selectedMismatch.category)}`,
                              "_blank",
                              "noopener",
                            )
                          }
                        >
                          편집
                        </Button>
                        <Button
                          variant="outline-primary"
                          size="sm"
                          onClick={() =>
                            window.open(
                              `/${contentType === "comic" ? "comics-view" : "book-view"}/${selectedMismatch.bookId}?category=${encodeURIComponent(selectedMismatch.category)}`,
                              "_blank",
                              "noopener",
                            )
                          }
                        >
                          조회
                        </Button>
                        <Button
                          variant="outline-danger"
                          size="sm"
                          onClick={handleDeleteEsDoc}
                        >
                          삭제
                        </Button>
                      </>
                    )}
                    {selectedMismatch.mismatchType === "fs_only" && (
                      <>
                        <Button
                          variant="outline-success"
                          size="sm"
                          onClick={handleIndexFile}
                          disabled={indexingFile}
                        >
                          {indexingFile && (
                            <Spinner
                              animation="border"
                              size="sm"
                              className="me-1"
                            />
                          )}
                          ES 적재
                        </Button>
                        <Button
                          variant="outline-danger"
                          size="sm"
                          onClick={() => setShowDeleteFileModal(true)}
                        >
                          파일 삭제
                        </Button>
                      </>
                    )}
                  </div>
                </Card.Body>
              </Card>
            )}

            {/* 아무것도 선택 안 됨 */}
            {!selectedCategory && !selectedMismatch && !actionResult && (
              <div className="text-muted p-3">
                왼쪽에서 디렉토리를 선택하세요.
              </div>
            )}
            {actionResult && (
              <div
                className={`p-3 ${actionResult.type === "success" ? "text-success" : "text-danger"}`}
                style={{ fontSize: "0.85rem" }}
              >
                {actionResult.message}
              </div>
            )}
          </Col>
        </Row>
      )}

      {/* 이름 변경 모달 */}
      <Modal
        show={showRenameModal}
        onHide={() => setShowRenameModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>카테고리 이름 변경</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <Form.Group>
            <Form.Label>현재 이름</Form.Label>
            <Form.Control type="text" value={selectedCategory} disabled />
          </Form.Group>
          <Form.Group className="mt-3">
            <Form.Label>새 이름</Form.Label>
            <Form.Control
              type="text"
              value={newCategoryName}
              onChange={(e) => setNewCategoryName(e.target.value)}
              onKeyDown={handleRenameKeyDown}
              autoFocus
            />
          </Form.Group>
        </Modal.Body>
        <Modal.Footer>
          <Button variant="secondary" onClick={() => setShowRenameModal(false)}>
            취소
          </Button>
          <Button
            variant="primary"
            onClick={handleRenameCategory}
            disabled={
              saving ||
              !newCategoryName.trim() ||
              newCategoryName.trim() === selectedCategory
            }
          >
            {saving ? <Spinner animation="border" size="sm" /> : "변경"}
          </Button>
        </Modal.Footer>
      </Modal>

      {/* 파일 삭제 확인 모달 */}
      <Modal
        show={showDeleteFileModal}
        onHide={() => setShowDeleteFileModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>파일 삭제</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="text-danger fw-bold">
            파일 &apos;{selectedMismatch?.filePath}&apos;을(를) 디스크에서
            삭제합니다.
          </p>
          <p className="text-muted">
            원본 파일이 사라집니다. 이 작업은 되돌릴 수 없습니다.
          </p>
        </Modal.Body>
        <Modal.Footer>
          <Button
            variant="secondary"
            onClick={() => setShowDeleteFileModal(false)}
          >
            취소
          </Button>
          <Button variant="danger" onClick={handleDeleteFile}>
            삭제
          </Button>
        </Modal.Footer>
      </Modal>

      {/* 카테고리 삭제 확인 모달 */}
      <Modal
        show={showDeleteModal}
        onHide={() => setShowDeleteModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>카테고리 삭제</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="fw-bold">
            카테고리 &apos;{selectedCategory}&apos; 및 하위 카테고리의 ES 문서를
            삭제합니다.
          </p>
          <p className="text-muted">
            파일은 디스크에 그대로 남습니다. 다시 필요하면 ES 재적재로 되살릴 수
            있습니다.
          </p>
        </Modal.Body>
        <Modal.Footer>
          <Button variant="secondary" onClick={() => setShowDeleteModal(false)}>
            취소
          </Button>
          <Button
            variant="danger"
            onClick={handleDeleteCategory}
            disabled={saving}
          >
            {saving ? <Spinner animation="border" size="sm" /> : "삭제"}
          </Button>
        </Modal.Footer>
      </Modal>

      {/* 분류 제안 다시 시작 확인 모달 */}
      <Modal
        show={showProposalRestartModal}
        onHide={() => setShowProposalRestartModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>분류 제안 다시 시작</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="fw-bold">
            지금 표에 남아 있는 선택과 직접 고친 목적지가 모두 사라지고,{" "}
            {selectedCategory}로 새 제안을 처음부터 다시 만듭니다.
          </p>
          <p className="text-muted">되돌릴 수 없습니다.</p>
        </Modal.Body>
        <Modal.Footer>
          <Button
            variant="secondary"
            onClick={() => setShowProposalRestartModal(false)}
          >
            취소
          </Button>
          <Button variant="danger" onClick={handleConfirmRestartProposal}>
            다시 시작
          </Button>
        </Modal.Footer>
      </Modal>

      {/* 분류 승인 확인 모달 */}
      <Modal
        show={showProposalApplyModal}
        onHide={() => setShowProposalApplyModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>분류 승인</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="fw-bold">
            선택한 {proposalApplyItems.length}건을 실제로 이동하고 ES 문서를 새
            카테고리로 다시 적재합니다.
          </p>
          <p className="text-muted">
            목적지가 없는 항목은 선택했어도 이 요청에 포함되지 않습니다.
          </p>
        </Modal.Body>
        <Modal.Footer>
          <Button
            variant="secondary"
            onClick={() => setShowProposalApplyModal(false)}
          >
            취소
          </Button>
          <Button
            variant="primary"
            onClick={handleApplyClassifyProposal}
            disabled={saving || proposalApplyItems.length === 0}
          >
            승인
          </Button>
        </Modal.Footer>
      </Modal>

      {/* 완료 기록 삭제 확인 모달 */}
      <Modal
        show={showProposalClearModal}
        onHide={() => setShowProposalClearModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>완료 기록 삭제</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="fw-bold">이동이 끝난(완료) 행만 표에서 지웁니다.</p>
          <p className="text-muted">
            대기·실패 행은 남아 나중에 다시 승인할 수 있습니다.
          </p>
        </Modal.Body>
        <Modal.Footer>
          <Button
            variant="secondary"
            onClick={() => setShowProposalClearModal(false)}
          >
            취소
          </Button>
          <Button variant="danger" onClick={handleClearAppliedProposalItems}>
            삭제
          </Button>
        </Modal.Footer>
      </Modal>

      {/* ES 재적재 확인 모달 */}
      <Modal
        show={showReloadModal}
        onHide={() => setShowReloadModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>ES 재적재</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="fw-bold">
            {getCategoryTargetLabel(selectedCategory)}의 모든 파일을 ES에
            재적재합니다.
          </p>
          <p className="text-muted">
            {selectedCategory === "_root"
              ? "최상위 디렉토리에 바로 있는 파일만 재적재하며, 하위 카테고리는 건드리지 않습니다."
              : "하위 디렉토리를 포함하여 전체 재적재하며, 파일 수에 따라 수 분이 소요될 수 있습니다."}
          </p>
        </Modal.Body>
        <Modal.Footer>
          <Button variant="secondary" onClick={() => setShowReloadModal(false)}>
            취소
          </Button>
          <Button
            variant="success"
            onClick={handleReloadCategory}
            disabled={saving}
          >
            {reloading ? <Spinner animation="border" size="sm" /> : "재적재"}
          </Button>
        </Modal.Footer>
      </Modal>

      {/* 이상 항목 ES 재적재 확인 모달 */}
      <Modal
        show={showMismatchReloadModal}
        onHide={() => setShowMismatchReloadModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>이상 항목 재적재</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="fw-bold">
            {mismatchReloadTargetCategory ? (
              <>
                카테고리 &apos;{mismatchReloadTargetCategory}&apos;의 이상 항목{" "}
                {selectedMismatchCount}건만 ES에 재적재합니다.
              </>
            ) : (
              <>
                전체 이상 항목 {mismatchReloadTargetCount}건을 ES에
                재적재합니다.
              </>
            )}
          </p>
          <p className="mb-1 fw-semibold">
            작업 대상: {mismatchReloadTargetCount}건
          </p>
          <p className="text-muted">
            누락 파일은 적재하고 연결되지 않은 ES 문서는 정리합니다.
          </p>
        </Modal.Body>
        <Modal.Footer>
          <Button
            variant="secondary"
            onClick={() => setShowMismatchReloadModal(false)}
          >
            취소
          </Button>
          <Button
            variant="warning"
            onClick={handleReloadCategoryMismatches}
            disabled={
              saving ||
              bulkReloading ||
              mismatchReloading ||
              mismatchReloadTargetCount === 0
            }
          >
            {mismatchReloading ? (
              <Spinner animation="border" size="sm" />
            ) : (
              "이상 항목 재적재"
            )}
          </Button>
        </Modal.Footer>
      </Modal>

      {/* 불일치 일괄 ES 재적재 확인 모달 */}
      <Modal
        show={showBulkReloadModal}
        onHide={() => setShowBulkReloadModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>불일치 일괄 재적재</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="fw-bold">
            현재 불일치 카테고리 {mismatchStats.categoryCount}개의 이상 항목{" "}
            {mismatchStats.itemCount}건을 ES에 재적재합니다.
          </p>
          <p className="text-muted">
            누락 파일은 적재하고 연결되지 않은 ES 문서는 정리합니다.
          </p>
          <p className="text-danger mb-0">
            <FontAwesomeIcon icon={faRotate} className="me-1" />
            이상 항목 수가 많으면 완료까지 오래 걸릴 수 있는 작업입니다. 완료
            전까지는 페이지를 벗어나도 서버에서 계속 진행되며, 같은 작업이 중복
            실행되지 않도록 서버에서 막습니다.
          </p>
        </Modal.Body>
        <Modal.Footer>
          <Button
            variant="secondary"
            onClick={() => setShowBulkReloadModal(false)}
          >
            취소
          </Button>
          <Button
            variant="danger"
            onClick={handleBulkReloadMismatches}
            disabled={saving || bulkReloading}
          >
            {bulkReloading ? (
              <Spinner animation="border" size="sm" />
            ) : (
              "일괄 재적재"
            )}
          </Button>
        </Modal.Footer>
      </Modal>
    </>
  );
}

CategoryAdmin.propTypes = {
  contentType: PropTypes.string,
  initialShowOnlyAbnormal: PropTypes.bool,
};
