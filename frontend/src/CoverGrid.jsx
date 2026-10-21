import "./CoverGrid.css";

import { useEffect, useRef, useState } from "react";
import PropTypes from "prop-types";

import { getApiUrlPrefix } from "./Common.js";
import { coverThumbProps, markTallCover } from "./coverImageFit.js";

/* global __COVER_VERSION__ */
// 한 번에 그리는 카드 수. 커버 요청도 이만큼씩만 나간다.
export const COVER_BATCH_SIZE = 20;
// 서버가 커버를 만들 수 있는 포맷. 나머지는 요청 없이 포맷명 박스로 그린다.
const COVER_FILE_TYPES = new Set(["epub", "pdf"]);
const COVER_VERSION =
  typeof __COVER_VERSION__ !== "undefined" ? __COVER_VERSION__ : "dev";

function CoverImage({ src, fileType }) {
  const [failed, setFailed] = useState(false);

  if (!src || failed) {
    return (
      <div className="cover-grid-placeholder">
        {(fileType || "?").toUpperCase()}
      </div>
    );
  }
  return (
    <img
      className="cover-grid-image"
      {...coverThumbProps(src)}
      alt=""
      loading="lazy"
      onLoad={markTallCover}
      onError={() => setFailed(true)}
    />
  );
}

CoverImage.propTypes = {
  src: PropTypes.string,
  fileType: PropTypes.string,
};

/**
 * @param {object}   props
 * @param {Array}    props.results - 지금까지 받은 책 목록
 * @param {boolean}  [props.hasMore] - 서버에 이어받을 페이지가 더 있는지
 * @param {function} [props.onLoadMore] - 끝이 화면에 들어오면 다음 서버 페이지를 부른다
 * @param {boolean}  [props.loading] - 서버 페이지를 받는 중이면 중복 호출을 막는다
 * @param {*}        [props.resetKey] - 바뀌면 그린 개수를 처음으로 되돌린다. 없으면 results가 바뀔 때마다 되돌린다
 */
export default function CoverGrid({
  results,
  basePath,
  hasMore = false,
  onLoadMore,
  loading = false,
  resetKey,
}) {
  const [visibleCount, setVisibleCount] = useState(COVER_BATCH_SIZE);
  const sentinelRef = useRef(null);
  const hasHiddenResults = visibleCount < results.length;
  const hasServerPage = hasMore && typeof onLoadMore === "function";
  const needsSentinel = hasHiddenResults || hasServerPage;
  // 서버 페이지가 이어붙을 때마다 results 참조가 바뀐다. 그때 그린 개수를 되돌리면 목록이 접히므로
  // 이어받는 쪽은 resetKey(디렉토리 등)로만 되돌린다.
  const resetToken = resetKey === undefined ? results : resetKey;
  const onLoadMoreRef = useRef(onLoadMore);
  onLoadMoreRef.current = onLoadMore;

  useEffect(() => {
    setVisibleCount(COVER_BATCH_SIZE);
  }, [resetToken]);

  useEffect(() => {
    if (!needsSentinel) return undefined;
    if (typeof IntersectionObserver === "undefined") {
      setVisibleCount(results.length);
      if (hasServerPage && !loading) onLoadMoreRef.current();
      return undefined;
    }
    // 묶음이 늘 때마다 다시 관찰한다. 새 묶음을 그린 뒤에도 끝이 화면 안이면 곧바로 다음 묶음을 부른다.
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        if (hasHiddenResults) {
          setVisibleCount((count) => count + COVER_BATCH_SIZE);
        } else if (hasServerPage && !loading) {
          onLoadMoreRef.current();
        }
      },
      { rootMargin: "200px" },
    );
    observer.observe(sentinelRef.current);
    return () => observer.disconnect();
  }, [
    needsSentinel,
    hasHiddenResults,
    hasServerPage,
    loading,
    visibleCount,
    results.length,
  ]);

  const apiPrefix = basePath.startsWith("/comics") ? "/comics" : "";
  const viewBasePath = basePath.replace("-edit", "-view");

  return (
    <>
      <div className="cover-grid">
        {results.slice(0, visibleCount).map((book) => {
          const filename =
            (book.file_path || "").split("/").pop() || book.title || "Unknown";
          const displayTitle = book.display_title || book.title || filename;
          const category = book.category || "_root";
          const fileType = book.file_type || "";
          // ID를 재사용해도 브라우저가 이전 책의 이미지를 쓰지 않도록 원본 경로도 구분한다.
          const coverSrc = COVER_FILE_TYPES.has(fileType)
            ? `${getApiUrlPrefix()}${apiPrefix}/cover/${book.book_id}?v=${COVER_VERSION}${
                book.updated_time
                  ? `&u=${encodeURIComponent(book.updated_time)}`
                  : ""
              }${book.file_path ? `&path=${encodeURIComponent(book.file_path)}` : ""}`
            : null;
          return (
            <a
              key={book.book_id}
              className="cover-grid-card"
              href={`${viewBasePath}/${book.book_id}?category=${encodeURIComponent(category)}`}
              target="_blank"
              rel="noopener noreferrer"
              title={displayTitle}
            >
              <div className="cover-grid-cover">
                <CoverImage src={coverSrc} fileType={fileType} />
              </div>
              <div className="cover-grid-title">{displayTitle}</div>
            </a>
          );
        })}
      </div>
      {needsSentinel && (
        <div ref={sentinelRef} className="cover-grid-sentinel" />
      )}
    </>
  );
}

CoverGrid.propTypes = {
  results: PropTypes.array.isRequired,
  basePath: PropTypes.string.isRequired,
  hasMore: PropTypes.bool,
  onLoadMore: PropTypes.func,
  loading: PropTypes.bool,
  resetKey: PropTypes.any,
};
