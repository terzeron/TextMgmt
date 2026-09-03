import { useEffect, useState, useCallback, useRef, useMemo } from "react";
import React from "react";
import PropTypes from "prop-types";

import "bootstrap/dist/css/bootstrap.min.css";
import {
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
import { updateCachedMappings } from "./categoryMappingCache";
import {
  findCommonPrefix,
  buildFolderHierarchy,
  findFolderInTree,
  updateFolderInTree,
} from "./folderUtils";
import { TreeNodeIcon } from "./fileTypeIcons";
import "./Folder.css";

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

function buildMismatchCounts(mismatchData) {
  const counts = {};
  for (const item of mismatchData.mismatches || []) {
    counts[item.category] = Math.abs(item.diff);
  }
  for (const item of mismatchData.es_only || []) {
    counts[item.category] = item.es_count;
  }
  for (const item of mismatchData.fs_only || []) {
    counts[item.category] = item.fs_count;
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

function encodeCategoryPath(category) {
  return category.split("/").map(encodeURIComponent).join("/");
}

/**
 * 에러 객체 또는 문자열에서 안전하게 에러 메시지를 추출
 */
export function formatErrorMessage(err, fallback = "오류가 발생했습니다.") {
  if (!err) return fallback;
  if (typeof err === "string") return err;
  if (err instanceof Error && err.message) return err.message;
  if (typeof err === "object") {
    if (typeof err.message === "string" && err.message) return err.message;
    if (typeof err.detail === "string" && err.detail) return err.detail;
    if (typeof err.error === "string" && err.error) return err.error;
  }
  return String(err);
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
  const [reloading, setReloading] = useState(false);
  const [mismatchReloading, setMismatchReloading] = useState(false);
  const [bulkReloading, setBulkReloading] = useState(false);
  const [reloadProgressCount, setReloadProgressCount] = useState(null);
  const [indexingFile, setIndexingFile] = useState(false);
  const [newCategoryName, setNewCategoryName] = useState("");

  const apiPrefix = contentType === "comic" ? "/comics" : "";
  const contentLabel = contentType === "comic" ? "만화" : "책";

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
          // 불일치가 있는 leaf 카테고리에 placeholder child 추가 (확장 아이콘 표시용)
          if (
            enriched.count > 0 &&
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

  // ── 재적재 작업 상태 반영 (서버가 공유하는 단일 진행 상태를 그대로 반영) ──
  // 백엔드는 콘텐츠 타입당 재적재 작업을 하나만 추적한다: 카테고리가 있으면
  // "이상 항목 재적재", 없으면 "일괄 재적재"로 간주해 두 스피너 중 하나만 켠다.

  const applyReloadStatus = useCallback(
    (status) => {
      if (!status || status.status === "idle") {
        setMismatchReloading(false);
        setBulkReloading(false);
        setReloadProgressCount(null);
        return false;
      }
      if (status.status === "running") {
        // 어느 버튼에 스피너를 켤지는 클릭한 쪽에서 이미 결정했으므로 여기서는
        // 건드리지 않는다 — category 유무로 다시 정하면 "이상 항목 재적재"를
        // 눌렀는데 "일괄 재적재" 쪽이 도는 것처럼 보이는 문제가 생긴다.
        setReloadProgressCount(
          (status.indexed_count || 0) + (status.deleted_count || 0),
        );
        return true;
      }

      setMismatchReloading(false);
      setBulkReloading(false);
      setReloadProgressCount(null);
      loadData();
      if (status.status === "done") {
        const indexed = status.indexed_count || 0;
        const deleted = status.deleted_count || 0;
        const remaining = status.after_count || 0;
        const failed = status.failed_count || 0;
        const label = status.category
          ? `카테고리 '${status.category}' 이상 항목`
          : "이상 항목 일괄";
        setMessage(
          `${label} ES 재적재 완료 (적재 ${indexed}건, ES 정리 ${deleted}건, 남은 이상 ${remaining}건${
            failed ? `, 실패 ${failed}건` : ""
          })`,
        );
      } else {
        setMessage(
          formatErrorMessage(
            status.error,
            "이상 항목 ES 재적재에 실패했습니다.",
          ),
        );
      }
      setTimeout(() => setMessage(""), 5000);
      return false;
    },
    [loadData],
  );

  // 마운트 시 한 번: 새로고침해도 이미 진행 중인 작업이 있으면 바로 붙어서 보여준다.
  // 이때는 어떤 버튼을 눌러서 시작됐는지 알 수 없으므로, category 유무로 추측해
  // 스피너를 켤 버튼을 정한다(진행 중에 다시 폴링될 때는 이 추측을 건드리지 않음).
  useEffect(() => {
    let cancelled = false;
    jsonGetReq(
      apiPrefix + "/category-mismatches/reload-status",
      null,
      (result) => {
        if (cancelled) return;
        if (result && result.status === "running") {
          setMismatchReloading(!!result.category);
          setBulkReloading(!result.category);
          setReloadProgressCount(
            (result.indexed_count || 0) + (result.deleted_count || 0),
          );
          return;
        }
        applyReloadStatus(result);
      },
      () => {},
    );
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiPrefix]);

  // 진행 중일 때만 10초 간격으로 상태 폴링
  useEffect(() => {
    if (!mismatchReloading && !bulkReloading) return undefined;

    let cancelled = false;
    const pollStatus = () => {
      jsonGetReq(
        apiPrefix + "/category-mismatches/reload-status",
        null,
        (result) => {
          if (!cancelled) applyReloadStatus(result);
        },
        () => {},
      );
    };

    pollStatus();
    const intervalId = setInterval(pollStatus, 10000);
    return () => {
      cancelled = true;
      clearInterval(intervalId);
    };
  }, [mismatchReloading, bulkReloading, apiPrefix, applyReloadStatus]);

  // ── 폴더 클릭 → 불일치 detail lazy-load ──

  const onFolderClick = useCallback(
    (selectedId) => {
      const selectedFolderData = findFolderInTree(folderData, selectedId);
      /* v8 ignore next 2 -- tree click events target known folder nodes. */
      if (!selectedFolderData || selectedFolderData.fileType !== "folder")
        return;
      /* v8 ignore next -- virtual parent nodes are not expandable mismatch leaves. */
      if (selectedFolderData.isVirtualParent) return;
      if (!selectedFolderData.count) return;
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
            return {
              ...folder,
              booksLoaded: true,
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
      setMessage("이미 등록된 키워드입니다.");
      setTimeout(() => setMessage(""), 3000);
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
      setMessage("현재 이름과 동일합니다.");
      setTimeout(() => setMessage(""), 3000);
      return;
    }

    setSaving(true);
    jsonPutReq(
      `${apiPrefix}/categories/rename`,
      { old_category: selectedCategory, new_category: trimmed },
      () => {
        setMessage(
          `카테고리 '${selectedCategory}'을(를) '${trimmed}'(으)로 변경했습니다.`,
        );
        setTimeout(() => setMessage(""), 5000);
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
      (result) => {
        setMessage(
          `카테고리 '${selectedCategory}'이(가) 삭제되었습니다. (${result.deleted_count}건)`,
        );
        setTimeout(() => setMessage(""), 5000);
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
      (result) => {
        setMessage(
          `카테고리 '${selectedCategory}' ES 재적재 완료 (${result.processed_count}건 처리)`,
        );
        setTimeout(() => setMessage(""), 5000);
      },
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

  // 재적재는 서버에서 백그라운드로 돈다 — 여기서는 시작만 확인하고, 완료/실패 메시지는
  // 위쪽 상태 폴링(applyReloadStatus)이 처리한다. 이미 진행 중이면 서버가
  // {already_running: true, ...}를 성공 응답으로 돌려주므로 에러 토스트 없이 그대로 붙는다.
  const handleReloadCategoryMismatches = useCallback(() => {
    setShowMismatchReloadModal(false);
    // 카테고리 미선택 시에도 내부적으로는 reload-all을 호출하지만, 사용자가 누른 건
    // "이상 항목 재적재" 버튼이므로 그 버튼에 스피너를 켠다("일괄 재적재"는 건드리지 않음).
    setMismatchReloading(true);
    setBulkReloading(false);
    setSaving(true);
    setMessage("");

    const url = selectedCategory
      ? `${apiPrefix}/category-mismatches/reload-mismatches`
      : `${apiPrefix}/category-mismatches/reload-all`;
    const payload = selectedCategory ? { category: selectedCategory } : null;

    jsonPostReq(
      url,
      payload,
      () => {},
      (error) => {
        setMessage(
          formatErrorMessage(error, "이상 항목 ES 재적재 시작에 실패했습니다."),
        );
        setTimeout(() => setMessage(""), 5000);
        setMismatchReloading(false);
        setBulkReloading(false);
      },
      () => setSaving(false),
    );
  }, [selectedCategory, apiPrefix]);

  const handleBulkReloadMismatches = useCallback(() => {
    setShowBulkReloadModal(false);
    setBulkReloading(true);
    setMismatchReloading(false);
    setSaving(true);
    setMessage("");
    jsonPostReq(
      `${apiPrefix}/category-mismatches/reload-all`,
      null,
      () => {},
      (error) => {
        setMessage(
          formatErrorMessage(
            error,
            "불일치 일괄 ES 재적재 시작에 실패했습니다.",
          ),
        );
        setTimeout(() => setMessage(""), 5000);
        setBulkReloading(false);
      },
      () => setSaving(false),
    );
  }, [apiPrefix]);

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
  const selectedFolder = selectedCategory
    ? findFolderInTree(folderData, selectedCategory)
    : null;
  const selectedMismatchCount = Number(selectedFolder?.count || 0);

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
  const messageText = message;

  // ── 렌더링 ──

  return (
    <>
      {messageText && (
        <div
          className={`alert ${messageText.includes("실패") || messageText.includes("오류") ? "alert-danger" : "alert-info"} py-1 mb-2`}
        >
          {messageText}
        </div>
      )}
      {loading ? (
        <div className="text-center p-4">
          <Spinner animation="border" />
          <p className="mt-2">로딩 중...</p>
        </div>
      ) : folderData.length === 0 ? (
        <div className="text-muted p-3">카테고리 없음</div>
      ) : (
        <Row className="g-0">
          <Col md={4}>
            <Card>
              <Card.Header className="py-1 d-flex justify-content-between align-items-center gap-2">
                <span>디렉토리 목록</span>
                <div className="d-flex flex-wrap justify-content-end align-items-center gap-2">
                  <Button
                    variant="outline-danger"
                    size="sm"
                    disabled={
                      saving || bulkReloading || mismatchStats.itemCount === 0
                    }
                    onClick={() => setShowBulkReloadModal(true)}
                    title="불일치 일괄 재적재 (이상 항목이 많으면 오래 걸릴 수 있음)"
                  >
                    {bulkReloading ? (
                      <span className="d-flex align-items-center gap-1">
                        <Spinner animation="border" size="sm" />
                        {reloadProgressCount !== null && (
                          <small style={{ fontSize: "0.7rem" }}>
                            처리 {reloadProgressCount}건
                          </small>
                        )}
                      </span>
                    ) : (
                      <>
                        일괄 재적재 <FontAwesomeIcon icon={faRotate} />
                      </>
                    )}
                  </Button>
                  <Button
                    variant="outline-warning"
                    size="sm"
                    disabled={
                      saving ||
                      mismatchReloading ||
                      (selectedCategory
                        ? selectedMismatchCount === 0
                        : mismatchStats.itemCount === 0)
                    }
                    onClick={() => setShowMismatchReloadModal(true)}
                    title={
                      selectedCategory
                        ? "선택 디렉토리 이상 항목만 ES 재적재"
                        : "하위 전체 이상 항목 ES 재적재"
                    }
                  >
                    {mismatchReloading ? (
                      <span className="d-flex align-items-center gap-1">
                        <Spinner animation="border" size="sm" />
                        {reloadProgressCount !== null && (
                          <small style={{ fontSize: "0.7rem" }}>
                            처리 {reloadProgressCount}건
                          </small>
                        )}
                      </span>
                    ) : (
                      <>
                        이상 항목 재적재 <FontAwesomeIcon icon={faRotate} />
                      </>
                    )}
                  </Button>
                  <Form.Check
                    type="switch"
                    id={`show-only-abnormal-${contentType}`}
                    label="이상 항목만 보기"
                    checked={showOnlyAbnormal}
                    onChange={(event) =>
                      setShowOnlyAbnormal(event.target.checked)
                    }
                    className="m-0"
                  />
                </div>
              </Card.Header>
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
                  <div className="d-flex flex-wrap gap-1 mb-2">
                    <Button
                      variant="outline-secondary"
                      size="sm"
                      disabled={saving}
                      onClick={() => {
                        setNewCategoryName(selectedCategory);
                        setShowRenameModal(true);
                      }}
                      title="이름 변경"
                    >
                      이름 변경 <FontAwesomeIcon icon={faEdit} />
                    </Button>
                    <Button
                      variant="outline-danger"
                      size="sm"
                      disabled={saving}
                      onClick={() => setShowDeleteModal(true)}
                      title="카테고리 삭제"
                    >
                      삭제 <FontAwesomeIcon icon={faTrash} />
                    </Button>
                    <Button
                      variant="outline-success"
                      size="sm"
                      disabled={saving}
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
                      disabled={saving || selectedMismatchCount === 0}
                      onClick={() => setShowMismatchReloadModal(true)}
                      title="이상 항목만 ES 재적재"
                    >
                      {mismatchReloading ? (
                        <span className="d-flex align-items-center gap-1">
                          <Spinner animation="border" size="sm" />
                          {reloadProgressCount !== null && (
                            <small style={{ fontSize: "0.7rem" }}>
                              처리 {reloadProgressCount}건
                            </small>
                          )}
                        </span>
                      ) : (
                        <>
                          이상 항목 재적재 <FontAwesomeIcon icon={faRotate} />
                        </>
                      )}
                    </Button>
                  </div>
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
                          onClick={handleDeleteFile}
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

      {/* 삭제 확인 모달 */}
      <Modal
        show={showDeleteModal}
        onHide={() => setShowDeleteModal(false)}
        centered
      >
        <Modal.Header closeButton>
          <Modal.Title>카테고리 삭제</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <p className="text-danger fw-bold">
            카테고리 &apos;{selectedCategory}&apos; 및 하위 카테고리의 모든
            문서가 삭제됩니다.
          </p>
          <p className="text-muted">이 작업은 되돌릴 수 없습니다.</p>
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
            카테고리 &apos;{selectedCategory}&apos;의 모든 파일을 ES에
            재적재합니다.
          </p>
          <p className="text-muted">
            하위 디렉토리를 포함하여 전체 재적재하며, 파일 수에 따라 수 분이
            소요될 수 있습니다.
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
            {selectedCategory ? (
              <>
                카테고리 &apos;{selectedCategory}&apos;의 이상 항목{" "}
                {selectedMismatchCount}건만 ES에 재적재합니다.
              </>
            ) : (
              <>
                현재 불일치 카테고리 {mismatchStats.categoryCount}개의 이상 항목{" "}
                {mismatchStats.itemCount}건을 ES에 재적재합니다.
              </>
            )}
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
            disabled={saving || mismatchReloading}
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
