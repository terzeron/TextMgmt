import PropTypes from "prop-types";

import { Alert, Button } from "react-bootstrap";

import SearchResult from "./SearchResult";
import CoverViewToggle from "./CoverViewToggle";
import { useCoverViewMode } from "./useCoverViewMode";
import { useCoverBrowse } from "./useCoverBrowse";

/**
 * 책·만화 탭 오른쪽 컬럼의 기본 화면. 최신순 목록을 스크롤에 맞춰 이어받아 그린다.
 * 최신 탭과 같은 카드(SearchResult)와 보기 전환을 쓰므로 어느 화면에서나 모양과 동작이 같다.
 * category가 없으면 루트, 있으면 그 디렉토리를 기준으로 한다.
 */
export default function CoverBrowser({
  apiPrefix,
  basePath,
  role,
  category,
  onClearCategory,
}) {
  const contentType = apiPrefix === "/comics" ? "comic" : "book";
  const [viewMode, changeViewMode] = useCoverViewMode(contentType);
  const { items, total, hasMore, loading, error, loadMore, reload } =
    useCoverBrowse({ apiPrefix, category });
  const allLabel = contentType === "comic" ? "전체 만화" : "전체 책";
  const containerId = `browse-${contentType === "comic" ? "comics" : "books"}`;

  return (
    <div id={containerId}>
      {error && (
        <Alert variant="danger" className="d-flex align-items-center mb-2">
          <span>{error}</span>
          <Button
            variant="outline-danger"
            size="sm"
            className="ms-auto"
            onClick={reload}
          >
            다시 시도
          </Button>
        </Alert>
      )}
      <SearchResult
        results={items}
        role={role}
        showEditButton={role === "admin"}
        basePath={basePath}
        title={
          <>
            <span>{category || allLabel}</span>
            {total !== null && (
              <span className="ms-2 text-muted">
                {total.toLocaleString()}건
              </span>
            )}
            <span className="ms-2 text-muted">(최신순)</span>
          </>
        }
        emptyMessage={loading ? "로딩 중..." : error ? "" : "책이 없습니다."}
        viewMode={viewMode}
        hasMore={hasMore}
        loading={loading}
        onLoadMore={loadMore}
        coverResetKey={category}
        headerActions={
          <>
            {category && (
              <Button
                variant="outline-secondary"
                size="sm"
                className="me-2"
                onClick={onClearCategory}
              >
                {allLabel} 보기
              </Button>
            )}
            <CoverViewToggle
              id={containerId}
              viewMode={viewMode}
              onChange={changeViewMode}
            />
          </>
        }
      />
    </div>
  );
}

CoverBrowser.propTypes = {
  apiPrefix: PropTypes.string.isRequired,
  basePath: PropTypes.string.isRequired,
  role: PropTypes.string,
  category: PropTypes.string,
  onClearCategory: PropTypes.func.isRequired,
};

CoverBrowser.defaultProps = {
  role: null,
  category: "",
};
