import { useEffect, useState } from "react";
import { useOutletContext } from "react-router-dom";
import PropTypes from "prop-types";

import "bootstrap/dist/css/bootstrap.min.css";
import {
  Alert,
  Container,
  ToggleButton,
  ToggleButtonGroup,
} from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faList, faTableCellsLarge } from "@fortawesome/free-solid-svg-icons";

import { rawJsonGetReq } from "./Common.js";
import SearchResult from "./SearchResult";

const LATEST_ITEM_LIMIT = 100;
const VIEW_MODE_STORAGE_KEY_PREFIX = "tm_latest_view_mode_";

// 뷰 모드는 탭별 편의 설정이다. 저장소를 쓸 수 없으면 커버 뷰를 사용한다.
function readViewMode(contentType) {
  try {
    return localStorage.getItem(VIEW_MODE_STORAGE_KEY_PREFIX + contentType) ===
      "list"
      ? "list"
      : "cover";
  } catch {
    return "cover";
  }
}

function writeViewMode(contentType, viewMode) {
  try {
    localStorage.setItem(VIEW_MODE_STORAGE_KEY_PREFIX + contentType, viewMode);
  } catch {
    // 저장하지 못해도 현재 화면의 전환은 그대로 유지한다.
  }
}

const LATEST_CONFIG = {
  book: {
    apiPrefix: "",
    basePath: "/book-view",
    title: "최신 책",
    emptyMessage: "최신 책이 없습니다.",
    errorMessage: "최신 책 목록을 불러오지 못했습니다.",
    containerId: "latest-books",
  },
  comic: {
    apiPrefix: "/comics",
    basePath: "/comics-view",
    title: "최신 만화",
    emptyMessage: "최신 만화가 없습니다.",
    errorMessage: "최신 만화 목록을 불러오지 못했습니다.",
    containerId: "latest-comics",
  },
};

export default function LatestBooks({ contentType = "book" }) {
  const config = LATEST_CONFIG[contentType] || LATEST_CONFIG.book;
  const {
    searchResults = [],
    hasSearched = false,
    role = null,
    searchTotal = 0,
    handleLoadMore,
    searchLoading = false,
    searchCategories = [],
    selectedSearchCategory = "",
    handleSearchCategoryChange,
    searchInProgress = false,
  } = useOutletContext() || {};
  const [items, setItems] = useState([]);
  const [loading, setLoading] = useState(true);
  const [errorMessage, setErrorMessage] = useState("");
  const [viewMode, setViewMode] = useState(() => readViewMode(contentType));

  // 책↔만화 탭 이동 때 같은 컴포넌트 인스턴스가 재사용될 수 있으므로 탭별 값을 다시 읽는다.
  useEffect(() => {
    setViewMode(readViewMode(contentType));
  }, [contentType]);

  const handleViewModeChange = (nextViewMode) => {
    setViewMode(nextViewMode);
    writeViewMode(contentType, nextViewMode);
  };

  useEffect(() => {
    setLoading(true);
    setErrorMessage("");
    rawJsonGetReq(
      `${config.apiPrefix}/latest?limit=${LATEST_ITEM_LIMIT}`,
      (data) => {
        if (data.status === "success") {
          setItems(data.result || []);
        } else {
          setItems([]);
          setErrorMessage(config.errorMessage);
        }
      },
      () => {
        setItems([]);
        setErrorMessage(config.errorMessage);
      },
      () => setLoading(false),
    );
  }, [config]);

  return (
    <Container id={config.containerId} className="ps-0 pe-0">
      {errorMessage && (
        <Alert variant="danger" className="mb-2">
          {errorMessage}
        </Alert>
      )}
      {hasSearched && (
        <SearchResult
          results={searchResults}
          role={role}
          showEditButton={role === "admin"}
          onLoadMore={handleLoadMore}
          hasMore={searchResults.length < searchTotal}
          loading={searchLoading}
          basePath={config.basePath}
          categories={searchCategories}
          selectedCategory={selectedSearchCategory}
          onCategoryChange={handleSearchCategoryChange}
          categoryLoading={searchInProgress}
        />
      )}
      <SearchResult
        results={items}
        role={role}
        showEditButton={role === "admin"}
        basePath={config.basePath}
        title={config.title}
        emptyMessage={loading ? "로딩 중..." : config.emptyMessage}
        viewMode={viewMode}
        headerActions={
          <ToggleButtonGroup
            type="radio"
            name={`${config.containerId}-view-mode`}
            size="sm"
            value={viewMode}
            onChange={handleViewModeChange}
          >
            <ToggleButton
              id={`${config.containerId}-view-list`}
              value="list"
              variant="outline-secondary"
              title="목록 보기"
            >
              <FontAwesomeIcon icon={faList} />
              <span className="visually-hidden">목록 보기</span>
            </ToggleButton>
            <ToggleButton
              id={`${config.containerId}-view-cover`}
              value="cover"
              variant="outline-secondary"
              title="커버 보기"
            >
              <FontAwesomeIcon icon={faTableCellsLarge} />
              <span className="visually-hidden">커버 보기</span>
            </ToggleButton>
          </ToggleButtonGroup>
        }
      />
    </Container>
  );
}

LatestBooks.propTypes = {
  contentType: PropTypes.oneOf(["book", "comic"]),
};
