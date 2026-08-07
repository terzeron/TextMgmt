import { useEffect, useState, useCallback, Suspense } from "react";
import { useParams, useSearchParams, useOutletContext } from "react-router-dom";
import PropTypes from "prop-types";

import { getApiUrlPrefix } from "./Common";

import "./View.css";
import "bootstrap/dist/css/bootstrap.min.css";
import { Container, Row, Col, Card, Alert } from "react-bootstrap";

import { jsonGetReq, rawJsonGetReq } from "./Common";
import Folder from "./Folder.jsx";
import ViewSingle from "./ViewSingle.jsx";
import BookInfoView from "./BookInfoView.jsx";
import BookLoadError from "./BookLoadError.jsx";
import SearchResult from "./SearchResult";
import {
  findCommonPrefix,
  buildFolderHierarchy,
  parseEntryId,
  findFolderInTree,
  updateFolderInTree,
  determineNextEntryId,
  determinePrevEntryId,
  MORE_ENTRY_FILE_TYPE,
  MORE_ENTRY_SUFFIX,
} from "./folderUtils";

// 카테고리 책 목록의 페이지 크기. 카테고리가 ES max_result_window(10000)를
// 넘어도 커서로 이어받아 전체에 도달할 수 있게 한다. 한 페이지 size 자체는
// max_result_window 이내여야 하므로 그 범위에서 크게 잡는다.
const CATEGORY_PAGE_SIZE = 5000;

// 모바일 감지 훅
function useIsMobile(breakpoint = 768) {
  const [isMobile, setIsMobile] = useState(
    typeof window !== "undefined" ? window.innerWidth < breakpoint : false,
  );

  useEffect(() => {
    const handleResize = () => setIsMobile(window.innerWidth < breakpoint);
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [breakpoint]);

  return isMobile;
}

export default function View({ basePath = "/book-view", apiPrefix = "" }) {
  const isMobile = useIsMobile();
  // get optional route params for deep link
  const params = useParams();
  const [searchParams] = useSearchParams();
  // 방법 B: /view/bookId?category=... (우선) → 하위호환: /view/category/bookId (폴백)
  const routeWildcard = params["*"] || "";
  const qCategory = searchParams.get("category");
  const routeCategory =
    qCategory ||
    (routeWildcard ? parseEntryId(routeWildcard)?.category : undefined);
  const routeBookId = qCategory
    ? /^\d+$/.test(routeWildcard)
      ? routeWildcard
      : undefined
    : routeWildcard
      ? parseEntryId(routeWildcard)?.bookId
      : undefined;
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
  const [folderData, setFolderData] = useState([]);
  const [hiddenCategories, setHiddenCategories] = useState(new Set());
  const [bookInfo, setBookInfo] = useState({});
  const [bookLoadError, setBookLoadError] = useState("");
  const [viewUrl, setViewUrl] = useState("");
  const [downloadUrl, setDownloadUrl] = useState("");

  useEffect(() => {
    const categoryListUrl = apiPrefix + "/categories";
    jsonGetReq(
      categoryListUrl,
      null,
      (categoryCounts) => {
        // categoryCounts: {"_epub": 5, "_pdf": 3, "_root": 2, ...}
        const categoryList = Object.keys(categoryCounts);

        // _root 카테고리(최상위 파일) 분리
        const hasRootFiles = categoryList.includes("_root");
        const nonEmptyCategories = categoryList.filter((c) => c !== "_root");

        const buildAndSetFolderData = (filteredCategories, counts) => {
          const commonPrefix = findCommonPrefix(filteredCategories);

          // 2단계 계층 구조 생성
          let data = buildFolderHierarchy(
            filteredCategories.sort((a, b) => a.localeCompare(b)),
            commonPrefix,
            counts,
          );

          // 최상위 파일이 있으면 가져와서 추가
          if (hasRootFiles) {
            jsonGetReq(
              apiPrefix + "/categories/_root",
              null,
              (bookList) => {
                const rootFiles = bookList
                  .sort((a, b) => a["title"].localeCompare(b["title"]))
                  .map((book) => ({
                    id: "/" + book["book_id"].toString(),
                    label: book["title"] + "." + book["file_type"],
                    fileType: book["file_type"],
                    children: [],
                    book: book,
                  }));
                setFolderData([...data, ...rootFiles]);
              },
              () => {
                setFolderData(data);
              },
            );
          } else {
            setFolderData(data);
          }
        };

        // viewer인 경우 비노출 카테고리 필터링
        if (role === "viewer") {
          const contentType = apiPrefix === "" ? "book" : "comic";
          jsonGetReq(
            `/hidden-categories?content_type=${contentType}`,
            null,
            (hiddenList) => {
              const hiddenSet = new Set(hiddenList || []);
              setHiddenCategories(hiddenSet);
              const filteredCategories = nonEmptyCategories.filter((cat) => {
                for (const hidden of hiddenSet) {
                  if (cat === hidden || cat.startsWith(hidden + "/")) {
                    return false;
                  }
                }
                return true;
              });
              buildAndSetFolderData(filteredCategories, categoryCounts);
            },
            () => {
              // hidden 목록 로드 실패 시 전체 카테고리 표시
              buildAndSetFolderData(nonEmptyCategories, categoryCounts);
            },
          );
        } else {
          buildAndSetFolderData(nonEmptyCategories, categoryCounts);
        }
      },
      (error) => {
        setErrorMessage(`can't load directory data, ${error}`);
      },
    );

    return () => {
      setErrorMessage("");
      setSuccessMessage("");
      setSelectedEntryId("");
      setNextEntryId("");
      setFolderData([]);
      setBookInfo({});
      setBookLoadError("");
      setViewUrl("");
      setDownloadUrl("");
    };
  }, [role, apiPrefix]);

  // 카테고리의 책 목록을 커서 기반으로 한 페이지씩 이어 불러온다.
  // 서버가 표시 순서(제목)대로 정렬해 주므로 페이지를 그대로 이어붙이면 된다.
  const loadCategoryPage = useCallback(
    (category) => {
      const folder = findFolderInTree(folderData, category);
      if (!folder || folder.loadingBooks) {
        return;
      }
      const cursor = folder.booksCursor || "";
      const url =
        apiPrefix +
        "/categories/" +
        category +
        `?limit=${CATEGORY_PAGE_SIZE}` +
        (cursor ? `&cursor=${encodeURIComponent(cursor)}` : "");

      setFolderData((prev) =>
        updateFolderInTree(prev, category, (item) => ({
          ...item,
          loadingBooks: true,
        })),
      );

      const stopLoading = () =>
        setFolderData((prev) =>
          updateFolderInTree(prev, category, (item) => ({
            ...item,
            loadingBooks: false,
          })),
        );

      rawJsonGetReq(
        url,
        (data) => {
          if (!data || data["status"] !== "success") {
            stopLoading();
            return;
          }
          const bookList = data["result"] || [];
          const nextCursor = data["next_cursor"] || "";
          const total = data["total"] || 0;
          const bookEntries = bookList.map((book) => ({
            id: category + "/" + book["book_id"].toString(),
            label: book["title"] + "." + book["file_type"],
            fileType: book["file_type"],
            children: [],
            book: book,
          }));

          setFolderData((prev) =>
            updateFolderInTree(prev, category, (item) => {
              const children = item.children || [];
              // 하위 폴더는 앞에, 이미 불러온 책은 순서대로 유지하고 뒤에 이어붙인다.
              const subfolders = children.filter(
                (c) => c.fileType === "folder",
              );
              const loadedBooks = children.filter(
                (c) =>
                  c.fileType !== "folder" &&
                  c.fileType !== MORE_ENTRY_FILE_TYPE,
              );
              const books = [...loadedBooks, ...bookEntries];
              const moreEntry = nextCursor
                ? [
                    {
                      id: category + MORE_ENTRY_SUFFIX,
                      label: `더 보기 (${books.length}/${total})`,
                      fileType: MORE_ENTRY_FILE_TYPE,
                      children: [],
                    },
                  ]
                : [];
              return {
                ...item,
                booksLoaded: true,
                loadingBooks: false,
                booksCursor: nextCursor,
                children: [...subfolders, ...books, ...moreEntry],
              };
            }),
          );
        },
        stopLoading,
      );
    },
    [folderData, apiPrefix],
  );

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

  // 책 한 권을 폴더 트리와 무관하게 /books/{id}로 직접 조회한다.
  // 카테고리 목록(/categories/{category})은 ES max_result_window 때문에
  // 10000건에서 잘리므로, 목록에 의존하면 그 뒤의 책은 열 수 없다.
  const loadBookById = useCallback(
    (bookId) => {
      jsonGetReq(
        apiPrefix + "/books/" + bookId,
        null,
        (book) => {
          // viewer인 경우 hidden 카테고리 접근 차단
          if (role === "viewer" && hiddenCategories.size > 0) {
            const bookCat = book["category"] || "";
            for (const hidden of hiddenCategories) {
              if (bookCat === hidden || bookCat.startsWith(hidden + "/")) {
                setBookLoadError("접근 권한이 없는 카테고리입니다.");
                return;
              }
            }
          }
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
        (error) => {
          setBookLoadError(`${error}`);
        },
      );
    },
    [apiPrefix, role, hiddenCategories],
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
        loadBookById(routeBookId);
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
      loadBookById(routeBookId);
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
