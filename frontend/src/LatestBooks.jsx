import { useEffect, useState } from "react";
import { useOutletContext } from "react-router-dom";
import PropTypes from "prop-types";

import "bootstrap/dist/css/bootstrap.min.css";
import { Alert, Container } from "react-bootstrap";

import { rawJsonGetReq } from "./Common.js";
import SearchResult from "./SearchResult";
import CoverViewToggle from "./CoverViewToggle";
import { useCoverViewMode } from "./useCoverViewMode";

const LATEST_ITEM_LIMIT = 2000;
const COMIC_TOP_LEVEL_DIRECTORY_PATTERN = /^\d+_[^/]+$/;
const FIRST_INSTALLMENT_PATTERN = /(?:^|[^\d])0*1\s*(?:권|화)(?=$|[^\d])/;

function isFirstInstallment(book) {
  const fileName =
    String(book?.file_path || "")
      .split("/")
      .pop() || "";
  return FIRST_INSTALLMENT_PATTERN.test(`${book?.title || ""} ${fileName}`);
}

function shouldReplaceComicRepresentative(currentBook, candidateBook) {
  const currentIsFirst = isFirstInstallment(currentBook);
  const candidateIsFirst = isFirstInstallment(candidateBook);
  if (currentIsFirst !== candidateIsFirst) return candidateIsFirst;

  return (
    String(candidateBook?.file_path || "").localeCompare(
      String(currentBook?.file_path || ""),
      "ko",
      { numeric: true, sensitivity: "base" },
    ) < 0
  );
}

function groupLatestComicsByBookDirectory(results) {
  if (!Array.isArray(results)) return [];

  const directoryIndexes = new Map();
  return results.reduce((groupedResults, book) => {
    const [topLevelDirectory, bookDirectory] = String(
      book?.category || "",
    ).split("/");
    if (
      !COMIC_TOP_LEVEL_DIRECTORY_PATTERN.test(topLevelDirectory) ||
      !bookDirectory
    ) {
      groupedResults.push(book);
      return groupedResults;
    }

    const directoryPath = `${topLevelDirectory}/${bookDirectory}`;
    const existingIndex = directoryIndexes.get(directoryPath);
    if (existingIndex !== undefined) {
      if (
        shouldReplaceComicRepresentative(groupedResults[existingIndex], book)
      ) {
        groupedResults[existingIndex] = {
          ...book,
          display_title: bookDirectory,
        };
      }
      return groupedResults;
    }

    directoryIndexes.set(directoryPath, groupedResults.length);
    groupedResults.push({ ...book, display_title: bookDirectory });
    return groupedResults;
  }, []);
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
  const [viewMode, handleViewModeChange] = useCoverViewMode(contentType);

  useEffect(() => {
    setLoading(true);
    setErrorMessage("");
    rawJsonGetReq(
      `${config.apiPrefix}/latest?limit=${LATEST_ITEM_LIMIT}`,
      (data) => {
        if (data.status === "success") {
          const results = data.result || [];
          setItems(
            contentType === "comic"
              ? groupLatestComicsByBookDirectory(results)
              : results,
          );
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
  }, [config, contentType]);

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
          <CoverViewToggle
            id={config.containerId}
            viewMode={viewMode}
            onChange={handleViewModeChange}
          />
        }
      />
    </Container>
  );
}

LatestBooks.propTypes = {
  contentType: PropTypes.oneOf(["book", "comic"]),
};
