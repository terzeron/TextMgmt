import { useEffect, useState } from "react";
import { useOutletContext } from "react-router-dom";
import PropTypes from "prop-types";

import "bootstrap/dist/css/bootstrap.min.css";
import { Alert, Container } from "react-bootstrap";

import { rawJsonGetReq } from "./Common.js";
import SearchResult from "./SearchResult";
import CoverViewToggle from "./CoverViewToggle";
import { useCoverViewMode } from "./useCoverViewMode";
import { useCoverBrowse } from "./useCoverBrowse";

const LATEST_ITEM_LIMIT = 2000;

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

/** 최신 책. 서버가 최근 파일을 한 번에 보낸다. */
function LatestBookItems({ config, children }) {
  const [items, setItems] = useState([]);
  const [loading, setLoading] = useState(true);
  const [errorMessage, setErrorMessage] = useState("");

  useEffect(() => {
    setLoading(true);
    setErrorMessage("");
    rawJsonGetReq(
      `${config.apiPrefix}/latest?limit=${LATEST_ITEM_LIMIT}`,
      (data) => {
        if (data.status === "success") {
          setItems(Array.isArray(data.result) ? data.result : []);
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

  return children({ items, loading, errorMessage });
}

/**
 * 최신 만화. 서버가 작품마다 대표 한 권(대체 후보 포함)으로 묶어 최신순으로 한 페이지씩 보낸다.
 * 화면 끝에 닿으면 다음 페이지를 이어받으므로 개수 상한을 두지 않는다.
 */
function LatestComicItems({ config, children }) {
  const { items, hasMore, loading, error, loadMore } = useCoverBrowse({
    apiPrefix: config.apiPrefix,
    endpoint: "latest",
  });
  return children({
    items,
    loading,
    errorMessage: error ? config.errorMessage : "",
    hasMore,
    onLoadMore: loadMore,
  });
}

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
  const [viewMode, handleViewModeChange] = useCoverViewMode(contentType);
  const LatestItems =
    config === LATEST_CONFIG.comic ? LatestComicItems : LatestBookItems;

  return (
    <LatestItems config={config}>
      {({ items, loading, errorMessage, hasMore = false, onLoadMore }) => (
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
            hasMore={hasMore}
            loading={loading}
            onLoadMore={onLoadMore}
            coverResetKey={config.containerId}
            headerActions={
              <CoverViewToggle
                id={config.containerId}
                viewMode={viewMode}
                onChange={handleViewModeChange}
              />
            }
          />
        </Container>
      )}
    </LatestItems>
  );
}

LatestBooks.propTypes = {
  contentType: PropTypes.oneOf(["book", "comic"]),
};

const latestItemsPropTypes = {
  config: PropTypes.object.isRequired,
  children: PropTypes.func.isRequired,
};
LatestBookItems.propTypes = latestItemsPropTypes;
LatestComicItems.propTypes = latestItemsPropTypes;
