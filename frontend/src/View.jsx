import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useCallback,
  Suspense,
} from "react";
import { useParams, useSearchParams, useOutletContext } from "react-router-dom";
import PropTypes from "prop-types";

import { getApiUrlPrefix, recordBookView } from "./Common";

import "./View.css";
import "bootstrap/dist/css/bootstrap.min.css";
import { Container, Alert, Button } from "react-bootstrap";

import Folder from "./Folder.jsx";
import SplitLayout from "./SplitLayout";
import ViewSingle from "./ViewSingle.jsx";
import BookInfoView from "./BookInfoView.jsx";
import SimilarBooks from "./SimilarBooks.jsx";
import BookLoadError from "./BookLoadError.jsx";
import SearchResult from "./SearchResult";
import CoverBrowser from "./CoverBrowser";
import {
  parseEntryId,
  parseRouteTarget,
  findFolderInTree,
  determineNextEntryId,
  determinePrevEntryId,
  MORE_ENTRY_SUFFIX,
} from "./folderUtils";
import { useCategoryTree } from "./useCategoryTree";

export default function View({
  basePath = "/book-view",
  apiPrefix = "",
  defaultDirectory = "",
}) {
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
    searchCategories,
    selectedSearchCategory,
    handleSearchCategoryChange,
    searchInProgress,
  } = useOutletContext();
  const [isFolderOpen, setIsFolderOpen] = useState(!routeBookId);
  const [expandedItems, setExpandedItems] = useState([]);
  const [errorMessage, setErrorMessage] = useState("");
  const [selectedEntryId, setSelectedEntryId] = useState("");
  const [nextEntryId, setNextEntryId] = useState("");
  const [prevEntryId, setPrevEntryId] = useState("");
  const [bookInfo, setBookInfo] = useState({});
  const [bookLoadError, setBookLoadError] = useState("");
  const [viewUrl, setViewUrl] = useState("");
  const [downloadUrl, setDownloadUrl] = useState("");
  // 오른쪽 컬럼 커버 목록이 보여 줄 디렉토리. 비우면 루트(전체). 책 딥링크가 우선이라 그때는 쓰지 않는다.
  // ?directory= 가 있으면(빈 값은 사용자가 고른 루트) 그것을, 없으면 탭의 기본 디렉토리를 쓴다.
  const [browseCategory, setBrowseCategory] = useState(() => {
    if (routeBookId) return "";
    const directoryParam = searchParams.get("directory");
    return directoryParam !== null ? directoryParam : defaultDirectory;
  });
  // 커버 목록으로 돌아온 라우트 책 id. 같은 딥링크가 목록 뒤에 다시 열리지 않게 한다.
  const [dismissedRouteBookId, setDismissedRouteBookId] = useState("");
  const [coverGridMounted, setCoverGridMounted] = useState(false);
  const bookIdRef = useRef("");
  const deepLinkPendingRef = useRef(false);
  const coverScrollRef = useRef(0);
  const rightContentRef = useRef(null);

  // 열람 이력 기록. setBookInfo 호출 지점이 여러 곳이라 각 지점에 심으면 누락·중복이
  // 생기므로, 선택된 책이 바뀌는 것만 한 곳에서 감지한다.
  const viewedBookId = bookInfo?.book_id;
  useEffect(() => {
    if (!viewedBookId) return;
    recordBookView(apiPrefix, viewedBookId);
  }, [apiPrefix, viewedBookId]);

  const { folderData, hiddenCategories, loadCategoryPage, loadBookById } =
    useCategoryTree({
      apiPrefix,
      role,
      onError: (error) =>
        setErrorMessage(`can't load directory data, ${error}`),
    });

  // 화면 전환 시 이전 책의 잔여 상태를 정리한다.
  useEffect(() => {
    return () => {
      setErrorMessage("");
      setSelectedEntryId("");
      setNextEntryId("");
      setBookInfo({});
      setBookLoadError("");
      setViewUrl("");
      setDownloadUrl("");
    };
  }, [role, apiPrefix]);

  // 커버 목록의 기준 디렉토리를 주소에 남겨 새로고침과 공유에서도 같은 목록이 열리게 한다.
  const syncDirectoryUrl = useCallback(
    (category) => {
      // 기본 디렉토리가 있는 탭에서는 빈 값도 남겨야 "루트를 골랐다"는 뜻이 새로고침 뒤에도 유지된다.
      window.history.replaceState(
        null,
        "",
        category || defaultDirectory
          ? `${basePath}?directory=${encodeURIComponent(category)}`
          : basePath,
      );
    },
    [basePath, defaultDirectory],
  );

  // 선택된 책의 상태만 비운다. URL과 디렉토리 패널은 호출하는 쪽이 정한다.
  const clearBookSelection = useCallback(() => {
    setBookInfo({});
    setSelectedEntryId("");
    setNextEntryId("");
    setPrevEntryId("");
    setViewUrl("");
    setDownloadUrl("");
    setBookLoadError("");
  }, []);

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

        // 오른쪽 커버 목록을 이 디렉토리의 책으로 채운다. 열려 있던 책 보기는 닫아 목록으로 돌아간다.
        setBrowseCategory(selectedEntryId);
        if (bookIdRef.current) clearBookSelection();
        // 딥링크 책을 여는 중에도 이 함수가 카테고리를 불러오려고 호출된다. 그때 주소를 바꾸면 딥링크가 사라진다.
        if (!deepLinkPendingRef.current) syncDirectoryUrl(selectedEntryId);

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
                /* v8 ignore next -- comic API prefix is covered through direct-load path. */
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
    [
      folderData,
      apiPrefix,
      basePath,
      loadCategoryPage,
      clearBookSelection,
      syncDirectoryUrl,
    ],
  );

  const showCoverList = useCallback(() => {
    setDismissedRouteBookId(routeBookId || "");
    clearBookSelection();
    setIsFolderOpen(true);
    syncDirectoryUrl(browseCategory);
  }, [routeBookId, clearBookSelection, syncDirectoryUrl, browseCategory]);

  // viewer에게 비노출인 디렉토리가 기본값이면 목록을 못 열므로 루트로 대체한다.
  useEffect(() => {
    if (!browseCategory || role !== "viewer") return;
    const isHidden = Array.from(hiddenCategories).some(
      (hidden) =>
        browseCategory === hidden || browseCategory.startsWith(hidden + "/"),
    );
    if (isHidden) setBrowseCategory("");
  }, [browseCategory, role, hiddenCategories]);

  const clearBrowseCategory = useCallback(() => {
    setBrowseCategory("");
    syncDirectoryUrl("");
  }, [syncDirectoryUrl]);

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
              /* v8 ignore next -- comic API prefix is covered through tree-selection path. */
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
    if (routeBookId && routeBookId === dismissedRouteBookId) return;
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
      /* v8 ignore next -- loaded category nodes always expose a children array. */
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
    /* v8 ignore start -- next button is disabled when no next entry exists. */
    if (nextEntryId) {
      entryClicked(nextEntryId);
    }
    /* v8 ignore stop */
  }, [nextEntryId, entryClicked]);

  const toPrevEntryButtonClicked = useCallback(() => {
    /* v8 ignore start -- previous button is disabled when no previous entry exists. */
    if (prevEntryId) {
      entryClicked(prevEntryId);
    }
    /* v8 ignore stop */
  }, [prevEntryId, entryClicked]);

  // 딥링크로 책이 열리는 중이면 첫 화면에서 쓰지 않을 커버 목록을 요청하지 않는다.
  const deepLinkPending =
    Boolean(routeBookId) &&
    routeBookId !== dismissedRouteBookId &&
    !bookInfo["book_id"] &&
    !bookLoadError;
  const showCoverGrid =
    !hasSearched && !bookInfo["book_id"] && !deepLinkPending;
  bookIdRef.current = bookInfo["book_id"] || "";
  deepLinkPendingRef.current = deepLinkPending;

  useEffect(() => {
    if (showCoverGrid) setCoverGridMounted(true);
  }, [showCoverGrid]);

  // 오른쪽 컬럼이 따로 스크롤된다. 좁은 화면에서 컬럼이 위아래로 쌓이면 문서 전체가 스크롤된다.
  const getScroller = useCallback(() => {
    const pane = rightContentRef.current?.closest(".split-layout-right");
    const layout = pane?.closest(".split-layout");
    return pane && layout && !layout.classList.contains("is-stacked")
      ? pane
      : null;
  }, []);

  // 책을 보는 동안 커버 목록은 숨기기만 하고 유지한다. 같은 스크롤 영역을 책 보기와 나눠 쓰므로,
  // 목록이 보이는 동안의 위치를 기억했다가 다시 보일 때 되돌린다. 구독 해제가 화면 갱신보다
  // 먼저 일어나야 밀린 위치가 기억을 덮어쓰지 않으므로 layout effect를 쓴다.
  useLayoutEffect(() => {
    if (!showCoverGrid) return undefined;
    const scroller = getScroller();
    if (coverScrollRef.current) {
      if (scroller) scroller.scrollTop = coverScrollRef.current;
      else window.scrollTo(0, coverScrollRef.current);
    }
    const target = scroller || window;
    const remember = () => {
      coverScrollRef.current = scroller ? scroller.scrollTop : window.scrollY;
    };
    target.addEventListener("scroll", remember, { passive: true });
    return () => target.removeEventListener("scroll", remember);
  }, [showCoverGrid, getScroller]);

  // 책을 열면 위에서부터 보여 준다.
  const openedBookId = bookInfo["book_id"];
  useLayoutEffect(() => {
    if (!openedBookId) return;
    const scroller = getScroller();
    if (scroller) scroller.scrollTop = 0;
    else window.scrollTo(0, 0);
  }, [openedBookId, getScroller]);

  // editUrl 계산: basePath의 -view를 -edit로 변환
  const editBasePath = basePath.replace("-view", "-edit");
  const editUrl = bookInfo["book_id"]
    ? `${editBasePath}/${bookInfo["book_id"]}?category=${encodeURIComponent(bookInfo["category"] || "_root")}`
    : "";
  const directoryEditUrl =
    bookInfo["book_id"] && bookInfo["category"]
      ? `${editBasePath}?directory=${encodeURIComponent(bookInfo["category"])}`
      : "";

  const folderPanel = (isOpen) => (
    <Suspense fallback={<div className="loading">로딩 중...</div>}>
      <Folder
        folderData={folderData}
        expandedItems={expandedItems}
        onExpandedItemsChange={setExpandedItems}
        isOpen={isOpen}
        onToggle={setIsFolderOpen}
        onClickHandler={entryClicked}
      />
    </Suspense>
  );

  // 디렉토리가 있는 화면은 모두 왼쪽(디렉토리)과 오른쪽(내용) 컬럼이 따로 스크롤되고 폭을 조절할 수 있다.
  // 디렉토리를 접으면 왼쪽 컬럼을 없애고 접힌 디렉토리를 오른쪽 컬럼 위에 둔다(편집 화면과 같다).
  return (
    <Container id="view">
      <SplitLayout storageKey="split:view">
        {isFolderOpen && (
          <SplitLayout.Left className="view-directory-panel">
            {folderPanel(true)}
          </SplitLayout.Left>
        )}
        <SplitLayout.Right className="view-main-panel">
          <div ref={rightContentRef}>
            {!isFolderOpen && folderPanel(false)}
            {hasSearched && (
              <SearchResult
                results={searchResults}
                role={role}
                onLoadMore={handleLoadMore}
                hasMore={searchResults.length < searchTotal}
                loading={searchLoading}
                basePath={basePath}
                categories={searchCategories}
                selectedCategory={selectedSearchCategory}
                onCategoryChange={handleSearchCategoryChange}
                categoryLoading={searchInProgress}
              />
            )}
            {!hasSearched && !bookInfo["book_id"] && bookLoadError && (
              <BookLoadError
                bookId={routeBookId}
                category={routeCategory}
                error={bookLoadError}
                role={role}
                apiPrefix={apiPrefix}
              />
            )}
            {bookInfo["book_id"] && (
              <>
                <div>
                  <Button
                    variant="outline-secondary"
                    size="sm"
                    onClick={showCoverList}
                  >
                    ← 커버 목록
                  </Button>
                </div>
                <BookInfoView
                  bookInfo={bookInfo}
                  isEditEnabled={false}
                  showDetails={false}
                />
                {errorMessage && (
                  <Alert variant="danger" className="mb-0">
                    {errorMessage}
                  </Alert>
                )}
                <SimilarBooks
                  bookId={bookInfo["book_id"]}
                  onSelect={entryClicked}
                  apiPrefix={apiPrefix}
                  basePath={basePath.replace("-view", "-edit")}
                  autoOpenHighScore={false}
                  canEdit={role === "admin"}
                />
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
                  directoryEditUrl={directoryEditUrl}
                  onNextBook={toNextEntryButtonClicked}
                  hasNextBook={!!nextEntryId}
                  onPrevBook={toPrevEntryButtonClicked}
                  hasPrevBook={!!prevEntryId}
                  role={role}
                />
              </>
            )}
            {(showCoverGrid || coverGridMounted) && (
              <div hidden={!showCoverGrid}>
                <CoverBrowser
                  apiPrefix={apiPrefix}
                  basePath={basePath}
                  role={role}
                  category={browseCategory}
                  onClearCategory={clearBrowseCategory}
                />
              </div>
            )}
          </div>
        </SplitLayout.Right>
      </SplitLayout>
    </Container>
  );
}

View.propTypes = {
  basePath: PropTypes.string,
  apiPrefix: PropTypes.string,
  defaultDirectory: PropTypes.string,
};
