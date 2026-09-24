import "./CoverGrid.css";

import { useEffect, useRef, useState } from "react";
import PropTypes from "prop-types";

import { getApiUrlPrefix } from "./Common.js";

// 한 번에 그리는 카드 수. 커버 요청도 이만큼씩만 나간다.
export const COVER_BATCH_SIZE = 20;
// 서버가 커버를 만들 수 있는 포맷. 나머지는 요청 없이 포맷명 박스로 그린다.
const COVER_FILE_TYPES = new Set(["epub", "pdf"]);

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
      src={src}
      alt=""
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}

CoverImage.propTypes = {
  src: PropTypes.string,
  fileType: PropTypes.string,
};

export default function CoverGrid({ results, basePath }) {
  const [visibleCount, setVisibleCount] = useState(COVER_BATCH_SIZE);
  const sentinelRef = useRef(null);
  const hasMore = visibleCount < results.length;

  useEffect(() => {
    setVisibleCount(COVER_BATCH_SIZE);
  }, [results]);

  useEffect(() => {
    if (!hasMore) return undefined;
    if (typeof IntersectionObserver === "undefined") {
      setVisibleCount(results.length);
      return undefined;
    }
    // 묶음이 늘 때마다 다시 관찰한다. 새 묶음을 그린 뒤에도 끝이 화면 안이면 곧바로 다음 묶음을 부른다.
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisibleCount((count) => count + COVER_BATCH_SIZE);
        }
      },
      { rootMargin: "200px" },
    );
    observer.observe(sentinelRef.current);
    return () => observer.disconnect();
  }, [hasMore, visibleCount, results.length]);

  const apiPrefix = basePath.startsWith("/comics") ? "/comics" : "";
  const viewBasePath = basePath.replace("-edit", "-view");

  return (
    <>
      <div className="cover-grid">
        {results.slice(0, visibleCount).map((book) => {
          const filename =
            (book.file_path || "").split("/").pop() || book.title || "Unknown";
          const displayTitle = book.title || filename;
          const category = book.category || "_root";
          const fileType = book.file_type || "";
          const coverSrc = COVER_FILE_TYPES.has(fileType)
            ? `${getApiUrlPrefix()}${apiPrefix}/cover/${book.book_id}`
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
      {hasMore && <div ref={sentinelRef} className="cover-grid-sentinel" />}
    </>
  );
}

CoverGrid.propTypes = {
  results: PropTypes.array.isRequired,
  basePath: PropTypes.string.isRequired,
};
