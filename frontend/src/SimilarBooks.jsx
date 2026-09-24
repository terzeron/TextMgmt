import "./Edit.css";
import "./SearchResult.css";
import "bootstrap/dist/css/bootstrap.min.css";

import { useEffect, useState, useCallback, useRef } from "react";
import PropTypes from "prop-types";
import { jsonDeleteReq, rawJsonGetReq } from "./Common";

import { Card, Button, Spinner } from "react-bootstrap";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import {
  faArrowsRotate,
  faChevronDown,
  faChevronRight,
  faEye,
  faPencil,
  faSpinner,
  faTrash,
} from "@fortawesome/free-solid-svg-icons";

const MIN_REFRESH_SPIN_MS = 400;

const formatFileSize = (bytes) => {
  if (bytes === null || bytes === undefined) return "";
  if (typeof bytes === "string" && bytes.trim() === "") return "";

  const size = Number(bytes);
  if (!Number.isFinite(size) || size < 0) return "";

  return Math.trunc(size).toLocaleString("en-US");
};

export default function SimilarBooks({
  bookId,
  onSelect,
  apiPrefix = "",
  basePath = "/book-edit",
  autoOpenHighScore = true,
  canEdit = true,
}) {
  const [isOpen, setIsOpen] = useState(false);
  const [similarBooks, setSimilarBooks] = useState([]);
  const [total, setTotal] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);
  const [deletingId, setDeletingId] = useState(null);
  const [refreshing, setRefreshing] = useState(false);
  const refreshTimerRef = useRef(null);

  const loadFirstPage = useCallback(
    (onLoaded, onFinish) => {
      rawJsonGetReq(
        `${apiPrefix}/similar/${bookId}?offset=0&limit=10`,
        (data) => {
          if (data.status === "success") {
            const books = data.result || [];
            setSimilarBooks(books);
            setTotal(data.total || 0);
            onLoaded(books);
          }
          if (onFinish) onFinish();
        },
        (error) => {
          console.error(error);
          if (onFinish) onFinish();
        },
      );
    },
    [bookId, apiPrefix],
  );

  useEffect(() => {
    if (bookId) {
      loadFirstPage((books) => {
        if (autoOpenHighScore && books.some((b) => b.score >= 90)) {
          setIsOpen(true);
        }
      });
    }
    return () => {
      if (refreshTimerRef.current !== null) {
        clearTimeout(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
      setSimilarBooks([]);
      setTotal(0);
      setIsOpen(false);
    };
  }, [bookId, loadFirstPage, autoOpenHighScore]);

  const handleRefresh = useCallback(() => {
    if (refreshing || !bookId) return;
    const startedAt = Date.now();
    setRefreshing(true);
    setIsOpen(true);
    loadFirstPage(
      () => {},
      () => {
        const remaining = Math.max(
          0,
          MIN_REFRESH_SPIN_MS - (Date.now() - startedAt),
        );
        refreshTimerRef.current = setTimeout(() => {
          setRefreshing(false);
          refreshTimerRef.current = null;
        }, remaining);
      },
    );
  }, [bookId, loadFirstPage, refreshing]);

  const handleLoadMore = useCallback(() => {
    /* v8 ignore next -- load-more control is disabled while a request is active. */
    if (loadingMore) return;
    setLoadingMore(true);
    const offset = similarBooks.length;
    rawJsonGetReq(
      `${apiPrefix}/similar/${bookId}?offset=${offset}&limit=10`,
      (data) => {
        if (data.status === "success" && data.result) {
          setSimilarBooks((prev) => [...prev, ...data.result]);
          setTotal(data.total || 0);
        }
        setLoadingMore(false);
      },
      (error) => {
        console.error(error);
        setLoadingMore(false);
      },
    );
  }, [bookId, similarBooks.length, loadingMore, apiPrefix]);

  const handleDelete = useCallback(
    (targetBookId, displayName) => {
      /* v8 ignore next -- delete buttons are disabled while a request is active. */
      if (deletingId !== null) return;
      if (!window.confirm(`"${displayName}"을(를) 삭제하시겠습니까?`)) return;
      setDeletingId(targetBookId);
      jsonDeleteReq(
        `${apiPrefix}/books/${targetBookId}`,
        null,
        () => {
          setSimilarBooks((prev) =>
            prev.filter((b) => b.book_id !== targetBookId),
          );
          setTotal((prev) => Math.max(prev - 1, 0));
          setDeletingId(null);
        },
        (error) => {
          console.error(error);
          window.alert(`책 삭제에 실패했습니다. ${error}`);
          setDeletingId(null);
        },
      );
    },
    [apiPrefix, deletingId],
  );

  const hasMore = similarBooks.length < total;

  return (
    <Card>
      <Card.Header
        onClick={() => setIsOpen(!isOpen)}
        style={{ cursor: "pointer", userSelect: "none" }}
        className="py-2 d-flex align-items-center"
      >
        <FontAwesomeIcon
          icon={isOpen ? faChevronDown : faChevronRight}
          className="me-2"
        />
        유사한 책 목록
        <Button
          variant="outline-secondary"
          className="btn-xs ms-auto"
          onClick={(e) => {
            e.stopPropagation();
            handleRefresh();
          }}
          disabled={refreshing}
          aria-busy={refreshing}
          aria-label="유사한 책 목록 새로고침"
          title="새로고침"
        >
          {refreshing ? (
            <Spinner animation="border" size="sm" aria-hidden="true" />
          ) : (
            <FontAwesomeIcon icon={faArrowsRotate} />
          )}
        </Button>
      </Card.Header>
      {isOpen && (
        <Card.Body>
          {similarBooks && similarBooks.length > 0 ? (
            <>
              {similarBooks.map((book) => {
                const filename = (book.file_path || '').split('/').pop() || book.title || 'Unknown';
                const category = book.category || '_root';
                const safeBasePath = basePath || '/book-edit';
                const viewBasePath = safeBasePath.replace('-edit', '-view');
                const categoryParam = encodeURIComponent(category);
                const displayName = (!category || category === '_root') ? filename : `${category}/${filename}`;
                const fileSizeLabel = formatFileSize(book.file_size);
                const isDeleting = deletingId === book.book_id;
                return (
                <div
                  key={book.book_id}
                  className={`search-result-item ${book.score >= 90 ? "highlight-secondary" : ""}`.trim()}
                >
                  <span
                    className="search-result-item-text"
                    style={{ cursor: "pointer" }}
                    onClick={() =>
                      onSelect && onSelect(`${category}/${book.book_id}`)
                    }
                  >
                    {displayName}
                  </span>
                  <div style={{ whiteSpace: "nowrap", flexShrink: 0 }}>
                    {fileSizeLabel && (
                      <span
                        style={{
                          display: "inline-block",
                          backgroundColor: "#fff",
                          color: "#000",
                          border: "1px solid #000",
                          borderRadius: "4px",
                          padding: "2px 6px",
                          fontSize: "0.6rem",
                          lineHeight: 1,
                          transform: "scale(0.75)",
                          transformOrigin: "right center",
                          marginRight: "4px",
                          verticalAlign: "middle",
                        }}
                      >
                        {fileSizeLabel}
                      </span>
                    )}
                    {book.score > 0 && (
                      <span
                        style={{
                          display: "inline-block",
                          backgroundColor: "#6c757d",
                          color: "#fff",
                          borderRadius: "4px",
                          padding: "1px 6px",
                          fontSize: "0.75rem",
                          marginRight: "4px",
                          verticalAlign: "middle",
                        }}
                      >
                        {Math.round(book.score)}
                      </span>
                    )}
                    {canEdit && (
                      <Button
                        variant="outline-warning"
                        className="btn-xs"
                        onClick={() =>
                          window.open(
                            `${safeBasePath}/${book.book_id}?category=${categoryParam}`,
                            "_blank",
                            "noopener",
                          )
                        }
                        aria-label={`${displayName} 편집`}
                        title="편집"
                        style={{ marginRight: "4px" }}
                      >
                        <FontAwesomeIcon icon={faPencil} />
                      </Button>
                    )}
                    <Button
                      variant="outline-primary"
                      className="btn-xs"
                      onClick={() =>
                        window.open(
                          `${viewBasePath}/${book.book_id}?category=${categoryParam}`,
                          "_blank",
                          "noopener",
                        )
                      }
                      aria-label={`${displayName} 조회`}
                      title="조회"
                      style={{ marginRight: "4px" }}
                    >
                      <FontAwesomeIcon icon={faEye} />
                    </Button>
                    {canEdit && (
                      <Button
                        variant="outline-danger"
                        className="btn-xs"
                        onClick={() => handleDelete(book.book_id, displayName)}
                        disabled={deletingId !== null}
                        aria-busy={isDeleting}
                        aria-label={`${displayName} 삭제`}
                        title={isDeleting ? "삭제 중..." : "삭제"}
                        style={{ marginRight: "4px" }}
                      >
                        <FontAwesomeIcon
                          icon={isDeleting ? faSpinner : faTrash}
                          spin={isDeleting}
                        />
                      </Button>
                    )}
                  </div>
                </div>
                );
              })}
              {hasMore && (
                <div className="search-result-load-more-wrapper">
                  <div
                    className={`search-result-load-more${loadingMore ? " disabled" : ""}`}
                    onClick={loadingMore ? undefined : handleLoadMore}
                    role="button"
                    tabIndex={0}
                    onKeyDown={(e) => {
                      if (!loadingMore && (e.key === "Enter" || e.key === " "))
                        handleLoadMore();
                    }}
                  >
                    {loadingMore ? "로딩 중..." : "더 보기"}
                    <FontAwesomeIcon icon={faChevronDown} size="sm" />
                  </div>
                </div>
              )}
            </>
          ) : (
            <div>유사한 책이 없습니다.</div>
          )}
        </Card.Body>
      )}
    </Card>
  );
}

SimilarBooks.propTypes = {
  bookId: PropTypes.oneOfType([PropTypes.number, PropTypes.string]),
  onSelect: PropTypes.func,
  apiPrefix: PropTypes.string,
  basePath: PropTypes.string,
  autoOpenHighScore: PropTypes.bool,
  canEdit: PropTypes.bool,
};
