import { useEffect, useRef, useState, useCallback, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";
import "./ViewEPUB.css";

const CHAPTERS_PREVIEW = 10;

const FONT_SIZE_MIN = 80;
const FONT_SIZE_MAX = 160;
const FONT_SIZE_STEP = 20;

const FONT_FAMILIES = [
  { label: "기본", value: "" },
  { label: "나눔고딕", value: "'Nanum Gothic', sans-serif" },
  { label: "나눔명조", value: "'Nanum Myeongjo', serif" },
  { label: "Serif", value: "serif" },
  { label: "Sans-serif", value: "sans-serif" },
];

export default function ViewEPUB({ bookId, preview = false, apiPrefix = "" }) {
  const renditionRef = useRef(null);
  const timeoutRef = useRef(null);
  const locationRef = useRef("");
  const savedLocationRef = useRef(null);
  const firstRenderRef = useRef(false);
  const locationsReadyRef = useRef(false);

  const [epubData, setEpubData] = useState(null);
  const [isLoading, setIsLoading] = useState(true);
  const [errorMessage, setErrorMessage] = useState(null);

  // 전체보기 전용 상태
  const [bookTitle, setBookTitle] = useState("");
  const [fontSize, setFontSize] = useState(() => {
    if (preview) return 100;
    const saved = localStorage.getItem("epub_fontSize");
    return saved ? parseInt(saved, 10) : 100;
  });
  const [fontFamily, setFontFamily] = useState(() => {
    if (preview) return "";
    return localStorage.getItem("epub_fontFamily") || "";
  });
  const [pageInfo, setPageInfo] = useState({ page: 0, total: 0 });
  const [locationsReady, setLocationsReady] = useState(false);

  // EPUB 로딩: 전체보기는 원본 파일(chapters=0), 미리보기는 부분 챕터
  useEffect(() => {
    if (!bookId) {
      setErrorMessage("유효한 bookId가 제공되지 않았습니다.");
      setIsLoading(false);
      return;
    }

    setIsLoading(true);
    setErrorMessage(null);
    setEpubData(null);
    setPageInfo({ page: 0, total: 0 });
    setLocationsReady(false);
    locationsReadyRef.current = false;
    firstRenderRef.current = false;

    // 저장된 읽기 위치를 별도 보관 (초기 렌더링 후 복원)
    // 초기 location prop으로 전달하면 epub.js가 무효 CFI에서 조용히 실패하므로
    // 첫 렌더링은 항상 처음부터 시작하고, 성공 후 저장된 위치로 이동
    savedLocationRef.current = !preview
      ? localStorage.getItem(`epub_location_${bookId}`) || null
      : null;
    locationRef.current = "";

    const chapters = preview ? CHAPTERS_PREVIEW : 0;
    const controller = new AbortController();
    const url = `${getApiUrlPrefix()}${apiPrefix}/preview/${bookId}?chapters=${chapters}`;

    fetch(url, { signal: controller.signal, credentials: "include" })
      .then(async (res) => {
        if (!res.ok) {
          const errText = await res.text();
          throw new Error(errText || `서버 응답 오류: ${res.status}`);
        }
        return res.arrayBuffer();
      })
      .then((buf) => {
        setEpubData(buf);
      })
      .catch((err) => {
        if (err.name !== "AbortError") {
          setErrorMessage(`EPUB 로딩 실패: ${err.message}`);
          setIsLoading(false);
        }
      });

    return () => {
      controller.abort();
      if (renditionRef.current) {
        try {
          renditionRef.current.destroy();
        } catch (_) {
          /* ignore */
        }
        renditionRef.current = null;
      }
      setEpubData(null);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- apiPrefix는 mount 시 고정이라 의도적으로 deps에서 제외
  }, [bookId, preview]);

  // 렌더링 타임아웃 (30초)
  useEffect(() => {
    if (!epubData || !isLoading) return;
    timeoutRef.current = setTimeout(() => {
      setIsLoading(false);
      setErrorMessage(`EPUB 렌더링 시간이 초과되었습니다. (book_id=${bookId})`);
    }, 30000);
    return () => clearTimeout(timeoutRef.current);
  }, [epubData, isLoading, bookId]);

  // 페이지 변경 핸들러
  const handleLocationChanged = useCallback(
    (epubcfi) => {
      locationRef.current = epubcfi;
      setIsLoading(false);
      setErrorMessage(null);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);

      // 첫 렌더링 성공 후 저장된 위치로 이동 시도
      if (!firstRenderRef.current) {
        firstRenderRef.current = true;
        if (savedLocationRef.current && renditionRef.current) {
          const loc = savedLocationRef.current;
          savedLocationRef.current = null;
          renditionRef.current.display(loc).catch(() => {
            console.warn("[epub.js] 저장된 위치 복원 실패, 현재 위치 유지");
            if (bookId) localStorage.removeItem(`epub_location_${bookId}`);
          });
          return;
        }
      }

      // 전체보기: 읽기 위치 저장
      if (!preview && bookId) {
        localStorage.setItem(`epub_location_${bookId}`, epubcfi);
      }
    },
    [preview, bookId],
  );

  // 글자 크기 변경
  const handleFontSizeChange = useCallback((delta) => {
    setFontSize((prev) => {
      const next = Math.max(
        FONT_SIZE_MIN,
        Math.min(FONT_SIZE_MAX, prev + delta),
      );
      localStorage.setItem("epub_fontSize", String(next));
      if (renditionRef.current) {
        renditionRef.current.themes.fontSize(`${next}%`);
      }
      return next;
    });
  }, []);

  // 글꼴 변경
  const handleFontFamilyChange = useCallback((family) => {
    setFontFamily(family);
    localStorage.setItem("epub_fontFamily", family);
    if (renditionRef.current) {
      renditionRef.current.themes.font(family);
    }
  }, []);

  const getRendition = useCallback(
    (rendition) => {
      if (renditionRef.current && renditionRef.current !== rendition) {
        try {
          renditionRef.current.destroy();
        } catch (_) {
          /* ignore */
        }
      }
      renditionRef.current = rendition;
      locationsReadyRef.current = false;
      setLocationsReady(false);

      // spine.get() 폴백: 누락된 항목 접근 시 첫 챕터로 이동
      const spine_get = rendition.book.spine.get.bind(rendition.book.spine);
      rendition.book.spine.get = function (target) {
        let t = spine_get(target);
        if (!t) t = spine_get(undefined);
        return t;
      };

      // epub.js display 에러 핸들링
      rendition.on("displayerror", (err) => {
        console.error("[epub.js] display error:", err);
        setErrorMessage(`EPUB 렌더링 오류: ${err?.message || String(err)}`);
        setIsLoading(false);
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
      });

      rendition.book.ready.catch((err) => {
        console.error("[epub.js] book load error:", err);
        setErrorMessage(`EPUB 파싱 오류: ${err?.message || String(err)}`);
        setIsLoading(false);
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
      });

      // display() 실패 시 저장된 위치 삭제 후 첫 페이지로 폴백
      if (typeof rendition.display === "function") {
        const origDisplay = rendition.display.bind(rendition);
        rendition.display = function (target) {
          return origDisplay(target).catch((err) => {
            console.error("[epub.js] display() rejected:", err);
            if (target) {
              console.warn("[epub.js] 저장된 위치 이동 실패, 첫 페이지로 이동");
              if (bookId) localStorage.removeItem(`epub_location_${bookId}`);
              locationRef.current = "";
              return origDisplay().catch((fallbackErr) => {
                setErrorMessage(
                  `EPUB 표시 실패: ${fallbackErr?.message || String(fallbackErr)}`,
                );
                setIsLoading(false);
                if (timeoutRef.current) clearTimeout(timeoutRef.current);
                throw fallbackErr;
              });
            }
            setErrorMessage(`EPUB 표시 실패: ${err?.message || String(err)}`);
            setIsLoading(false);
            if (timeoutRef.current) clearTimeout(timeoutRef.current);
            throw err;
          });
        };
      }

      // 전체보기 전용 초기화
      if (!preview) {
        rendition.book.loaded.metadata.then((meta) => {
          if (meta?.title) setBookTitle(meta.title);
        });

        const savedSize = localStorage.getItem("epub_fontSize");
        if (savedSize) rendition.themes.fontSize(`${savedSize}%`);

        const savedFont = localStorage.getItem("epub_fontFamily");
        if (savedFont) rendition.themes.font(savedFont);

        // 페이지 위치 인덱스 생성
        const currentRendition = rendition;
        rendition.book.ready
          .then(() => {
            if (renditionRef.current !== currentRendition) return null;
            return rendition.book.locations.generate(1024);
          })
          .then((locations) => {
            if (!locations || renditionRef.current !== currentRendition) return;
            locationsReadyRef.current = true;
            setLocationsReady(true);
            if (locationRef.current) {
              try {
                const idx = rendition.book.locations.locationFromCfi(
                  locationRef.current,
                );
                const totalPages = rendition.book.locations.total + 1;
                if (idx >= 0 && totalPages > 0) {
                  setPageInfo({ page: idx + 1, total: totalPages });
                }
              } catch (_) {
                /* ignore */
              }
            }
          })
          .catch((err) => console.warn("[epub.js] locations error:", err));

        // 페이지 이동 시 페이지 번호 업데이트
        rendition.on("relocated", (location) => {
          if (location?.start?.cfi && locationsReadyRef.current) {
            try {
              const idx = renditionRef.current.book.locations.locationFromCfi(
                location.start.cfi,
              );
              const totalPages = renditionRef.current.book.locations.total + 1;
              if (idx >= 0 && totalPages > 0) {
                setPageInfo({ page: idx + 1, total: totalPages });
              }
            } catch (_) {
              /* ignore */
            }
          }
        });
      }
    },
    [preview, bookId],
  );

  const containerHeight = preview ? "60vh" : "100dvh";
  const readerKey = `${bookId}-${preview ? "preview" : "full"}`;

  return (
    <div
      style={{
        height: containerHeight,
        textAlign: "center",
        position: "relative",
        overflow: "hidden",
        overscrollBehavior: "none",
      }}
    >
      {isLoading && (
        <div className="loading-container">
          <div className="spinner"></div>
          <span className="blinking">로딩 중...</span>
        </div>
      )}
      {errorMessage && <div className="error-message">{errorMessage}</div>}

      {/* 전체보기 전용 툴바 */}
      {!preview && !isLoading && epubData && (
        <div className="epub-toolbar" data-testid="epub-toolbar">
          <button
            onClick={() => handleFontSizeChange(-FONT_SIZE_STEP)}
            disabled={fontSize <= FONT_SIZE_MIN}
            aria-label="글자 크기 줄이기"
          >
            A−
          </button>
          <button
            onClick={() => handleFontSizeChange(FONT_SIZE_STEP)}
            disabled={fontSize >= FONT_SIZE_MAX}
            aria-label="글자 크기 늘리기"
          >
            A+
          </button>
          <select
            value={fontFamily}
            onChange={(e) => handleFontFamilyChange(e.target.value)}
            aria-label="글꼴 선택"
          >
            {FONT_FAMILIES.map((f) => (
              <option key={f.value} value={f.value}>
                {f.label}
              </option>
            ))}
          </select>
        </div>
      )}

      <Suspense fallback={<div className="loading">로딩 중...</div>}>
        {epubData && (
          <ReactReader
            key={readerKey}
            locationChanged={handleLocationChanged}
            location={preview ? 0 : undefined}
            url={epubData}
            title={!preview ? bookTitle : undefined}
            getRendition={getRendition}
          />
        )}
      </Suspense>

      {/* 전체보기 전용 페이지 정보 */}
      {!preview && !isLoading && epubData && (
        <div className="epub-page-info" data-testid="epub-page-info">
          {locationsReady && pageInfo.total > 0
            ? `${pageInfo.page} / ${pageInfo.total}`
            : "페이지 계산 중..."}
        </div>
      )}
    </div>
  );
}

ViewEPUB.propTypes = {
  bookId: PropTypes.number.isRequired,
  preview: PropTypes.bool,
  apiPrefix: PropTypes.string,
};
