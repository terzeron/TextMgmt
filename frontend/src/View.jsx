import { useEffect, useState, useCallback, Suspense } from "react";
import { useParams, useSearchParams, useOutletContext } from "react-router-dom";
import PropTypes from "prop-types";

import { getApiUrlPrefix } from "./Common";

import "./View.css";
import "bootstrap/dist/css/bootstrap.min.css";
import { Container, Row, Col, Card, Alert } from "react-bootstrap";

import Folder from "./Folder.jsx";
import ViewSingle from "./ViewSingle.jsx";
import BookInfoView from "./BookInfoView.jsx";
import BookLoadError from "./BookLoadError.jsx";
import SearchResult from "./SearchResult";
import {
  parseEntryId,
  parseRouteTarget,
  findFolderInTree,
  determineNextEntryId,
  determinePrevEntryId,
  MORE_ENTRY_SUFFIX,
} from "./folderUtils";
import useIsMobile from "./useIsMobile";
import { useCategoryTree } from "./useCategoryTree";

export default function View({ basePath = "/book-view", apiPrefix = "" }) {
  const isMobile = useIsMobile();
  // get optional route params for deep link
  const params = useParams();
  const [searchParams] = useSearchParams();
  // 방법 B: /view/bookId?category=... (우선) → 하위호환: /view/category/bookId (폴백)
  const { routeCategory, routeBookId } = parseRouteTarget(
    params["*"],
    searchParams.get("category"),
  );
  const {
    searchResults,
    hasSearched,
    role,
    searchTotal,
    handleLoadMore,
    searchLoading,
  } = useOutletContext();
  const [isFolderOpen, setIsFolderOpen] = useState(true);
  const [expandedItems, setExpandedItems] = useState([]);
  const [errorMessage, setErrorMessage] = useState("");
  const [successMessage, setSuccessMessage] = useState("");
  const [selectedEntryId, setSelectedEntryId] = useState("");
  const [nextEntryId, setNextEntryId] = useState("");
  const [prevEntryId, setPrevEntryId] = useState("");
  const [bookInfo, setBookInfo] = useState({});
  const [bookLoadError, setBookLoadError] = useState("");
  const [viewUrl, setViewUrl] = useState("");
  const [downloadUrl, setDownloadUrl] = useState("");

  const { folderData, loadCategoryPage, loadBookById } = useCategoryTree({
    apiPrefix,
    role,
    onError: (error) => setErrorMessage(`can't load directory data, ${error}`),
  });

  // 화면 전환 시 이전 책의 잔여 상태를 정리한다.
  useEffect(() => {
    return () => {
      setErrorMessage("");
      setSuccessMessage("");
      setSelectedEntryId("");
      setNextEntryId("");
      setBookInfo({});
      setBookLoadError("");
      setViewUrl("");
      setDownloadUrl("");
    };
  }, [role, apiPrefix]);

  const entryClicked = useCallback(
    (selectedEntryId) => {
      // 새 항목 선택 시 이전 책 로드 실패 사유 초기화
      setBookLoadError("");
      // '더 보기' 노드는 책이 아니라 다음 페이지 요청이다.
      if (selectedEntryId.endsWith(MORE_ENTRY_SUFFIX)) {
        loadCategoryPage(selectedEntryId.slice(0, -MORE_ENTRY_SUFFIX.length));
        return;
      }
      // 2단계 트리에서 검색
      const selectedFolderData = findFolderInTree(folderData, selectedEntryId);
      if (selectedFolderData && selectedFolderData.fileType === "folder") {
        // category entry (폴더)

        // 가상 부모 클릭 시 API 호출 안 함
        if (selectedFolderData.isVirtualParent) {
          return;
        }

        // booksLoaded 플래그로 중복 로딩 방지
        if (!selectedFolderData.booksLoaded) {
          loadCategoryPage(selectedEntryId);
        }
      } else if (selectedFolderData && selectedFolderData.book) {
        // 최상위 파일 (folderData에 직접 포함된 파일)
        const book = selectedFolderData.book;
        const bookId = book["book_id"];
        setSelectedEntryId(selectedEntryId);
        setBookInfo(book);
        setViewUrl(
          "/viewer/" +
            book["file_type"] +
            "/" +
            bookId +
            "?path=" +
            encodeURIComponent(book["file_path"]) +
            (apiPrefix ? "&api=" + encodeURIComponent(apiPrefix) : "") +
            "&category=" +
            encodeURIComponent(book["category"] || "_root"),
        );
        setDownloadUrl(getApiUrlPrefix() + apiPrefix + "/download/" + bookId);
        window.history.replaceState(
          null,
          "",
          `${basePath}/${bookId}?category=${encodeURIComponent(book["category"] || "_root")}`,
        );
        setNextEntryId(determineNextEntryId(folderData, selectedEntryId));
        setPrevEntryId(determinePrevEntryId(folderData, selectedEntryId));
      } else {
        // book entry (폴더 내 파일)
        const parsed = parseEntryId(selectedEntryId);
        if (!parsed) return;
        const category = parsed.category;
        const bookId = parsed.bookId;
        const folder = findFolderInTree(folderData, category);
        const booksInCategory = folder?.children;
        setSelectedEntryId(selectedEntryId);
        if (booksInCategory) {
          const book = booksInCategory.find(
            (bookItem) => bookItem.id === selectedEntryId,
          )?.book;
          if (book) {
            setBookInfo(book);
            setViewUrl(
              "/viewer/" +
                book["file_type"] +
                "/" +
                bookId +
                "?path=" +
                encodeURIComponent(book["file_path"]) +
                (apiPrefix ? "&api=" + encodeURIComponent(apiPrefix) : "") +
                "&category=" +
                encodeURIComponent(category),
            );
            setDownloadUrl(
              getApiUrlPrefix() + apiPrefix + "/download/" + bookId,
            );
            window.history.replaceState(
              null,
              "",
              `${basePath}/${bookId}?category=${encodeURIComponent(category)}`,
            );
            setNextEntryId(determineNextEntryId(folderData, selectedEntryId));
            setPrevEntryId(determinePrevEntryId(folderData, selectedEntryId));
          } else {
            setBookLoadError(
              `선택한 책을 찾을 수 없습니다. (book_id=${bookId})`,
            );
          }
        } else {
          setBookLoadError(`선택한 카테고리를 찾을 수 없습니다. (${category})`);
        }
      }
    },
    [folderData, apiPrefix, basePath, loadCategoryPage],
  );

  // 폴더 트리와 무관하게 책 한 권을 직접 열어 화면에 반영한다.
  // (트리 목록은 상한에 걸려 잘릴 수 있으므로 목록에 의존하지 않는다.)
  const selectBookById = useCallback(
    (bookId) => {
      loadBookById(
        bookId,
        (book) => {
          setBookLoadError("");
          setBookInfo(book);
          setViewUrl(
            "/viewer/" +
              book["file_type"] +
              "/" +
              bookId +
              "?path=" +
              encodeURIComponent(book["file_path"]) +
              (apiPrefix ? "&api=" + encodeURIComponent(apiPrefix) : "") +
              "&category=" +
              encodeURIComponent(book["category"] || ""),
          );
          setDownloadUrl(getApiUrlPrefix() + apiPrefix + "/download/" + bookId);
        },
        (message) => {
          setBookLoadError(message);
        },
      );
    },
    [apiPrefix, loadBookById],
  );

  // folderData 변경 시 nextEntryId/prevEntryId 재계산
  useEffect(() => {
    if (selectedEntryId && folderData.length > 0) {
      setNextEntryId(determineNextEntryId(folderData, selectedEntryId));
      setPrevEntryId(determinePrevEntryId(folderData, selectedEntryId));
    }
  }, [folderData, selectedEntryId]);

  // if route specifies a category/bookId, auto-select after folderData loads
  useEffect(() => {
    if (routeCategory && routeBookId && folderData.length > 0) {
      // _root 카테고리는 folderData에서 /{bookId} 형식으로 저장됨
      if (routeCategory === "_root") {
        entryClicked("/" + routeBookId);
        return;
      }
      const categoryItem = findFolderInTree(folderData, routeCategory);
      if (!categoryItem) {
        // 폴더 트리에 없는 경로 (3레벨 이상) - 백엔드에서 직접 조회
        selectBookById(routeBookId);
        return;
      }
      // If category children not loaded, load them first
      if (!categoryItem.booksLoaded) {
        entryClicked(routeCategory);
        return;
      }
      // 목록에 있으면 트리 컨텍스트(이전/다음 이동)까지 갖춘 선택을 수행한다.
      const entryId = `${routeCategory}/${routeBookId}`;
      const isInLoadedList = (categoryItem.children || []).some(
        (child) => child.id === entryId,
      );
      if (isInLoadedList) {
        entryClicked(entryId);
        return;
      }
      // 목록에 없는 경우(카테고리가 10000건 상한에 걸려 잘린 경우 등)에는
      // 목록에 의존하지 않고 책 자체를 직접 조회한다. 트리 탐색만 비활성화된다.
      selectBookById(routeBookId);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- apiPrefix/hiddenCategories/role은 라우트 변경 시점에만 재평가하면 충분 (의도된 누락)
  }, [routeCategory, routeBookId, folderData, entryClicked]);

  const toNextEntryButtonClicked = useCallback(() => {
    if (nextEntryId) {
      entryClicked(nextEntryId);
    }
  }, [nextEntryId, entryClicked]);

  const toPrevEntryButtonClicked = useCallback(() => {
    if (prevEntryId) {
      entryClicked(prevEntryId);
    }
  }, [prevEntryId, entryClicked]);

  // editUrl 계산: basePath의 -view를 -edit로 변환
  const editBasePath = basePath.replace("-view", "-edit");
  const editUrl = bookInfo["book_id"]
    ? `${editBasePath}/${bookInfo["book_id"]}?category=${encodeURIComponent(bookInfo["category"] || "_root")}`
    : "";

  // 모바일에서는 directory-menu 클래스를 제거하여 고정 높이 스타일 방지
  const directoryClassName = isMobile
    ? "ps-0 pe-0"
    : "ps-0 pe-0 section directory-menu";

  return (
    <Container id="view">
      <Row fluid="true">
        {isFolderOpen && (
          <Col
            md={isMobile ? 12 : 5}
            lg={isMobile ? 12 : 4}
            className={directoryClassName}
          >
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
              <Folder
                folderData={folderData}
                expandedItems={expandedItems}
                onExpandedItemsChange={setExpandedItems}
                isOpen={true}
                onToggle={setIsFolderOpen}
                onClickHandler={entryClicked}
              />
            </Suspense>
          </Col>
        )}

        <Col
          md={isMobile ? 12 : isFolderOpen ? 7 : 12}
          lg={isMobile ? 12 : isFolderOpen ? 8 : 12}
          className={isMobile ? "ps-0 pe-0" : "section"}
        >
          {!isFolderOpen && (
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
              <Folder
                folderData={folderData}
                expandedItems={expandedItems}
                onExpandedItemsChange={setExpandedItems}
                isOpen={false}
                onToggle={setIsFolderOpen}
                onClickHandler={entryClicked}
              />
            </Suspense>
          )}
          {hasSearched && (
            <SearchResult
              results={searchResults}
              showEditButton={false}
              onLoadMore={handleLoadMore}
              hasMore={searchResults.length < searchTotal}
              loading={searchLoading}
              basePath={basePath}
            />
          )}
          {!hasSearched && !bookInfo["book_id"] && bookLoadError && (
            <Row id="top_panel">
              <Col lg="12" className="ps-0 pe-0 me-0 ">
                <BookLoadError
                  bookId={routeBookId}
                  category={routeCategory}
                  error={bookLoadError}
                  role={role}
                  apiPrefix={apiPrefix}
                />
              </Col>
            </Row>
          )}
          {bookInfo["book_id"] && (
            <>
              <Row id="top_panel">
                <Col lg="12" className="ps-0 pe-0 me-0 ">
                  <BookInfoView bookInfo={bookInfo} isEditEnabled={false} />
                </Col>

                <Card>
                  <Card.Header>실행 결과</Card.Header>
                  <Card.Body>
                    {errorMessage && (
                      <Alert variant="danger" className="mb-0">
                        {errorMessage}
                      </Alert>
                    )}
                    {successMessage && (
                      <Alert variant="success" className="mb-0">
                        {successMessage}
                      </Alert>
                    )}
                  </Card.Body>
                </Card>
              </Row>

              <Row id="bottom_panel">
                <Col id="right_panel" className="ps-0 pe-0">
                  {bookInfo["book_id"] && (
                    <ViewSingle
                      key={bookInfo["book_id"]}
                      bookId={bookInfo["book_id"]}
                      filePath={bookInfo["file_path"]}
                      fileType={bookInfo["file_type"]}
                      viewUrl={viewUrl}
                      downloadUrl={downloadUrl}
                      lineCount={100}
                      pageCount={10}
                      apiPrefix={apiPrefix}
                      editUrl={editUrl}
                      onNextBook={toNextEntryButtonClicked}
                      hasNextBook={!!nextEntryId}
                      onPrevBook={toPrevEntryButtonClicked}
                      hasPrevBook={!!prevEntryId}
                      role={role}
                    />
                  )}
                </Col>
              </Row>
            </>
          )}
        </Col>
      </Row>
    </Container>
  );
}

View.propTypes = {
  basePath: PropTypes.string,
  apiPrefix: PropTypes.string,
};
