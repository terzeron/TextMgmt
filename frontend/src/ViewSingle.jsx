import { useEffect, useState, useCallback, lazy, Suspense } from "react";
import { useParams, useSearchParams } from "react-router-dom";
import PropTypes from "prop-types";
import { jsonGetReq, getApiUrlPrefix } from "./Common";

import "./ViewSingle.css";
import "bootstrap/dist/css/bootstrap.min.css";

const ViewPDF = lazy(() => import("./ViewPDF"));
const ViewEPUB = lazy(() => import("./ViewEPUB"));
import ViewDOC from "./ViewDOC";
import ViewTXT from "./ViewTXT";
import ViewHTML from "./ViewHTML";
import ViewRTF from "./ViewRTF";
import ViewImage from "./ViewImage";
import { Button, Card } from "react-bootstrap";

export default function ViewSingle(props) {
  // URL 파라미터와 쿼리 파라미터에서 값 추출
  const { entryId, fileType: paramFileType } = useParams();
  const [searchParams] = useSearchParams();
  const paramFilePath = searchParams.get("path") || "";
  const paramApiPrefix = searchParams.get("api") || "";
  const paramCategory = searchParams.get("category") || "";
  const standalone = Boolean(entryId && paramFileType);
  const [bookId, setBookId] = useState(0);
  const [filePath, setFilePath] = useState("");
  const [fileType, setFileType] = useState("");
  const [lineCount, setLineCount] = useState(0);
  const [pageCount, setPageCount] = useState(0);
  const [prevBook, setPrevBook] = useState(null);
  const [nextBook, setNextBook] = useState(null);

  useEffect(() => {
    if (entryId && paramFileType && paramFilePath) {
      // standalone page via route
      setBookId(Number(entryId));
      setFileType(paramFileType);
      setFilePath(paramFilePath);
    } else if (props.bookId) {
      // nested component
      setBookId(props.bookId);
      setFileType(props.fileType);
      setFilePath(props.filePath);
      setLineCount(props.lineCount);
      setPageCount(props.pageCount);
    }
  }, [
    props.bookId,
    props.fileType,
    props.filePath,
    props.lineCount,
    props.pageCount,
    entryId,
    paramFileType,
    paramFilePath,
  ]);

  // standalone 모드에서 body/html 스크롤 잠금 (iOS 바운스 방지)
  useEffect(() => {
    if (!standalone) return;
    const html = document.documentElement;
    const body = document.body;
    const saved = {
      htmlOverflow: html.style.overflow,
      htmlHeight: html.style.height,
      bodyOverflow: body.style.overflow,
      bodyHeight: body.style.height,
      bodyPosition: body.style.position,
      bodyWidth: body.style.width,
    };
    html.style.overflow = "hidden";
    html.style.height = "100%";
    body.style.overflow = "hidden";
    body.style.height = "100%";
    body.style.position = "fixed";
    body.style.width = "100%";
    return () => {
      html.style.overflow = saved.htmlOverflow;
      html.style.height = saved.htmlHeight;
      body.style.overflow = saved.bodyOverflow;
      body.style.height = saved.bodyHeight;
      body.style.position = saved.bodyPosition;
      body.style.width = saved.bodyWidth;
    };
  }, [standalone]);

  // standalone 모드: 카테고리 내 이전/다음 책 결정
  useEffect(() => {
    if (!standalone || !paramCategory || paramCategory === "_root") return;
    const apiUrl =
      paramApiPrefix +
      "/categories/" +
      paramCategory.split("/").map(encodeURIComponent).join("/");
    jsonGetReq(apiUrl, null, (books) => {
      books.sort((a, b) => a.title.localeCompare(b.title));
      const currentIndex = books.findIndex(
        (b) => b.book_id === Number(entryId),
      );
      setPrevBook(currentIndex > 0 ? books[currentIndex - 1] : null);
      setNextBook(
        currentIndex >= 0 && currentIndex < books.length - 1
          ? books[currentIndex + 1]
          : null,
      );
    });
  }, [standalone, paramCategory, paramApiPrefix, entryId]);

  const navigateToBook = useCallback(
    (book) => {
      const url =
        "/viewer/" +
        book.file_type +
        "/" +
        book.book_id +
        "?path=" +
        encodeURIComponent(book.file_path) +
        (paramApiPrefix ? "&api=" + encodeURIComponent(paramApiPrefix) : "") +
        "&category=" +
        encodeURIComponent(paramCategory);
      window.location.href = url;
    },
    [paramApiPrefix, paramCategory],
  );

  const preview = props.preview || false;

  // 이전/다음 책 네비게이션 (미리보기/전체보기 공통)
  const showBookNav = standalone
    ? prevBook || nextBook
    : props.onPrevBook || props.onNextBook;
  const navPrevDisabled = standalone ? !prevBook : !props.hasPrevBook;
  const navNextDisabled = standalone ? !nextBook : !props.hasNextBook;
  const navPrevClick = standalone
    ? () => prevBook && navigateToBook(prevBook)
    : props.onPrevBook;
  const navNextClick = standalone
    ? () => nextBook && navigateToBook(nextBook)
    : props.onNextBook;

  const ap = standalone ? paramApiPrefix : props.apiPrefix || "";
  const componentMap = {
    pdf: (
      <ViewPDF
        bookId={bookId}
        pageCount={pageCount}
        preview={preview}
        apiPrefix={ap}
      />
    ),
    epub: <ViewEPUB bookId={bookId} preview={preview} apiPrefix={ap} />,
    doc: <ViewDOC bookId={bookId} fileType="doc" apiPrefix={ap} />,
    docx: (
      <ViewDOC
        bookId={bookId}
        fileType="docx"
        lineCount={lineCount}
        apiPrefix={ap}
      />
    ),
    hwp: <ViewDOC bookId={bookId} fileType="hwp" apiPrefix={ap} />,
    txt: <ViewTXT bookId={bookId} lineCount={lineCount} apiPrefix={ap} />,
    html: <ViewHTML bookId={bookId} apiPrefix={ap} />,
    rtf: <ViewRTF bookId={bookId} apiPrefix={ap} />,
    jpg: <ViewImage bookId={bookId} apiPrefix={ap} />,
    gif: <ViewImage bookId={bookId} apiPrefix={ap} />,
    png: <ViewImage bookId={bookId} apiPrefix={ap} />,
  };
  const renderComponent = componentMap[fileType];

  return (
    <Card className={standalone ? "standalone-viewer" : ""}>
      {!standalone && (
        <Card.Header>
          책 보기
          <span>
            {props.viewUrl && (
              <a href={props.viewUrl} target="_blank" rel="noreferrer">
                <Button
                  variant="outline-primary"
                  disabled={!props.viewUrl}
                  size="sm"
                  className="float-end"
                >
                  전체 보기
                </Button>
              </a>
            )}
            {props.downloadUrl && (
              <a href={props.downloadUrl} target="_blank" rel="noreferrer">
                <Button
                  variant="outline-primary"
                  disabled={!props.downloadUrl}
                  size="sm"
                  className="float-end"
                >
                  다운로드
                </Button>
              </a>
            )}
            {props.role === "admin" && props.editUrl && (
              <a href={props.editUrl}>
                <Button
                  variant="outline-secondary"
                  size="sm"
                  className="float-end"
                >
                  편집
                </Button>
              </a>
            )}
          </span>
        </Card.Header>
      )}
      <Card.Body>
        {showBookNav && (
          <div className="standalone-nav">
            <button
              className="standalone-nav-btn"
              onClick={navPrevClick}
              disabled={navPrevDisabled}
            >
              ◀ 이전 책으로
            </button>
            <button
              className="standalone-nav-btn"
              onClick={navNextClick}
              disabled={navNextDisabled}
            >
              다음 책으로 ▶
            </button>
          </div>
        )}
        {bookId ? (
          <Suspense fallback={<div>로딩 중...</div>}>
            {renderComponent}
          </Suspense>
        ) : (
          <div>책이 선택되지 않았습니다.</div>
        )}
      </Card.Body>
    </Card>
  );
}

ViewSingle.propTypes = {
  bookId: PropTypes.number,
  filePath: PropTypes.string,
  fileType: PropTypes.string,
  viewUrl: PropTypes.string,
  downloadUrl: PropTypes.string,
  lineCount: PropTypes.number,
  pageCount: PropTypes.number,
  preview: PropTypes.bool,
  apiPrefix: PropTypes.string,
  editUrl: PropTypes.string,
  onNextBook: PropTypes.func,
  hasNextBook: PropTypes.bool,
  onPrevBook: PropTypes.func,
  hasPrevBook: PropTypes.bool,
  role: PropTypes.string,
};
