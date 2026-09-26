import { useEffect, useRef, useState, useCallback, Suspense } from "react";
import PropTypes from "prop-types";
import ePub from "epubjs";
import { getApiUrlPrefix } from "./Common";
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

const FONT_FAMILY_STYLE_ID = "epub-font-family-override";
const FONT_FACE_STYLE_ID = "epub-font-face-override";

// 나눔고딕/나눔명조 웹폰트를 콘텐츠 문서에 주입한다. iPad 등 시스템 폰트가
// 없는 환경에서도 글꼴 변경이 동작하게 한다. 파일은 frontend/public/fonts에
// 번들한 fontsource korean·latin 서브셋이다. epub.js가 콘텐츠 문서에 책 리소스
// 경로의 <base>를 넣으므로 폰트 URL은 절대 경로여야 한다. korean·latin 규칙이
// 같은 family·weight로 겹치면 나중에 선언된 쪽이 이기므로 latin을 나중에 둔다.
const FONT_FACE_FAMILIES = [
  { family: "Nanum Gothic", file: "nanum-gothic" },
  { family: "Nanum Myeongjo", file: "nanum-myeongjo" },
];
const FONT_FACE_WEIGHTS = [400, 700];
const KOREAN_UNICODE_RANGE =
  "U+AC00-D7AF, U+1100-11FF, U+3130-318F, U+A960-A97F, U+D7B0-D7FF, " +
  "U+3000-303F, U+FF00-FFEF, U+25A0-25FF, U+203B, U+327E-327F";
const LATIN_UNICODE_RANGE =
  "U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, " +
  "U+02DC, U+2000-206F, U+2074, U+20AC, U+2122, U+2191, U+2193, " +
  "U+2212, U+2215, U+FEFF, U+FFFD";

const buildFontFaceCss = () => {
  const base = `${window.location.origin}/fonts/`;
  const face = (family, file, weight, subset, range) =>
    `@font-face { font-family: '${family}'; font-style: normal; ` +
    `font-weight: ${weight}; font-display: swap; ` +
    `src: url(${base}${file}-${subset}-${weight}.woff2) format('woff2'); ` +
    `unicode-range: ${range}; }`;
  const rules = [];
  for (const { family, file } of FONT_FACE_FAMILIES) {
    for (const weight of FONT_FACE_WEIGHTS) {
      rules.push(face(family, file, weight, "korean", KOREAN_UNICODE_RANGE));
      rules.push(face(family, file, weight, "latin", LATIN_UNICODE_RANGE));
    }
  }
  return rules.join("\n");
};

// epub.js themes.font()는 iframe body에만 inline font-family를 건다. 책 자체
// CSS가 p, div 등 본문 요소에 font-family를 선언하면 상속이 깨져 글꼴 변경이
// 무시된다. 텍스트 요소 전반에 !important 규칙을 직접 주입하고, 기본 글꼴로
// 돌아가면 제거한다.
const FONT_FAMILY_SELECTOR =
  "html, body, p, div, span, li, ul, ol, h1, h2, h3, h4, h5, h6, " +
  "blockquote, td, th, a, section, article, aside, figure, figcaption, " +
  "dd, dt, pre";

const applyFontFamilyToContents = (contents, family) => {
  const doc = contents?.document;
  if (!doc?.head) return;
  // 웹폰트 @font-face는 항상 주입한다. 브라우저는 쓰일 때만 파일을 받는다.
  let faceEl = doc.getElementById(FONT_FACE_STYLE_ID);
  if (!faceEl) {
    faceEl = doc.createElement("style");
    faceEl.id = FONT_FACE_STYLE_ID;
    doc.head.appendChild(faceEl);
  }
  faceEl.textContent = buildFontFaceCss();
  let styleEl = doc.getElementById(FONT_FAMILY_STYLE_ID);
  if (!family) {
    styleEl?.remove();
    return;
  }
  if (!styleEl) {
    styleEl = doc.createElement("style");
    styleEl.id = FONT_FAMILY_STYLE_ID;
    doc.head.appendChild(styleEl);
  }
  styleEl.textContent = `${FONT_FAMILY_SELECTOR} { font-family: ${family} !important; }`;
};

const RENDER_TIMEOUT_MS = 30_000;
const SAVE_THROTTLE_MS = 200;
const ORIENTATION_QUERY = "(orientation: landscape)";

const isLandscapeNow = () => window.matchMedia(ORIENTATION_QUERY).matches;

const readFontSize = () => {
  const saved = localStorage.getItem("epub_fontSize");
  return saved ? parseInt(saved, 10) : 100;
};
const readFontFamily = () => localStorage.getItem("epub_fontFamily") || "";

const readSavedLocation = (bookId) => {
  try {
    return localStorage.getItem(`epub_location_${bookId}`) || null;
  } catch {
    return null;
  }
};
const writeSavedLocation = (bookId, cfi) => {
  try {
    if (cfi) localStorage.setItem(`epub_location_${bookId}`, cfi);
  } catch {
    /* 저장 공간 부족 등: 무시 */
  }
};

// 좌우 페이지 넘김(paginated)에서 본문은 다중 컬럼으로 쪼개진다. epub.js는
// 본문에 overflow-y: hidden만 걸어 CSS 규칙상 overflow-x가 auto로 계산되고,
// 모바일(사파리)에서 터치 팬이 본문 컬럼을 네이티브로 직접 밀어 버린다. 버튼은
// 컨테이너 scrollLeft를 움직이므로 두 스크롤 상태가 어긋나 빈 페이지와
// 건너뜀이 생긴다.
//
// 주의: 팬 차단을 overflow: hidden(양축)으로 하면 안 된다. 본문의 넘치는
// 컬럼은 본문 박스 밖으로 뻗는데 overflow: hidden은 본문 자체 클리핑이라
// WebKit(사파리)에서 그 컬럼들이 아예 그려지지 않는다(레이아웃은 되지만
// 픽셀이 비었다 — 실측 확인). touch-action: pan-y는 가로 터치 팬만 막고
// 그리기에는 영향이 없다.
const lockContentOverflow = (contents) => {
  const contentDocument = contents?.document;
  for (const element of [
    contentDocument?.documentElement,
    contentDocument?.body,
  ]) {
    element?.style.setProperty("touch-action", "pan-y", "important");
  }
};

// 각 섹션은 표시 순간에만 크기를 측정한다. 글자 크기 변경이나 늦은 폰트·이미지
// 로드로 본문이 다시 쪼개지면 뷰가 낡은 폭에 머물러 잘린 컬럼(건너뜀)과
// iframe 끝 너머 빈 페이지가 생긴다. 크기가 바뀌는 신호마다 다시 측정한다.
const reexpandView = (view) => {
  try {
    view.expand?.();
  } catch {
    /* ignore */
  }
};

const installReexpand = (view) => {
  const doc = view?.contents?.document;
  if (!doc?.body) return;
  reexpandView(view);
  if (typeof ResizeObserver !== "undefined") {
    const observer = new ResizeObserver(() => reexpandView(view));
    observer.observe(doc.body);
  }
  doc.fonts?.ready?.then(() => reexpandView(view));
  for (const img of doc.images || []) {
    if (!img.complete) {
      img.addEventListener("load", () => reexpandView(view), { once: true });
    }
  }
  setTimeout(() => reexpandView(view), 800);
  setTimeout(() => reexpandView(view), 2500);
};

export default function ViewEPUB({
  bookId,
  preview = false,
  apiPrefix = "",
  standalone = false,
}) {
  const hostRef = useRef(null);
  const renditionRef = useRef(null);
  const bookRef = useRef(null);
  const timeoutRef = useRef(null);
  const saveTimerRef = useRef(null);
  const cfiRef = useRef("");

  const [epubData, setEpubData] = useState(null);
  const [isLoading, setIsLoading] = useState(true);
  const [errorMessage, setErrorMessage] = useState(null);
  const [pageInfo, setPageInfo] = useState({ page: 0, total: 0 });
  const [fontSize, setFontSize] = useState(() =>
    preview ? 100 : readFontSize(),
  );
  const [fontFamily, setFontFamily] = useState(() =>
    preview ? "" : readFontFamily(),
  );
  const fontFamilyRef = useRef(fontFamily);

  // 뷰어 높이는 상수로 둔다(측정·재계산 기계는 텍스트 잘림의 원인이었다).
  const containerHeight = standalone ? "100%" : preview ? "60vh" : "100dvh";

  // 책 파일 로드
  useEffect(() => {
    if (!bookId) {
      setErrorMessage("유효한 bookId가 제공되지 않았습니다.");
      setIsLoading(false);
      return;
    }
    setIsLoading(true);
    setErrorMessage(null);
    setEpubData(null);
    cfiRef.current = "";

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
      .then((buf) => setEpubData(buf))
      .catch((err) => {
        if (err.name !== "AbortError") {
          setErrorMessage(`EPUB 로딩 실패: ${err.message}`);
          setIsLoading(false);
        }
      });
    return () => controller.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- apiPrefix는 mount 시 고정이라 의도적으로 deps에서 제외
  }, [bookId, preview]);

  // 책 렌더링
  useEffect(() => {
    if (!epubData || !hostRef.current) return;

    const book = ePub(epubData);
    bookRef.current = book;
    const rendition = book.renderTo(hostRef.current, {
      flow: "paginated",
      manager: "default",
      width: "100%",
      height: "100%",
      allowScriptedContent: true,
    });
    renditionRef.current = rendition;

    // 세로모드 1열, 가로모드 2열. epub.js 기본(minSpreadWidth 800)은 가로폭
    // 800px 이상에서만 2열이라 기기별 세로·가로 기준과 어긋난다.
    // orientation 쿼리로 spread를 직접 강제한다(landscape: 항상 2열,
    // portrait: 항상 1열). rendition.spread()는 내부 updateLayout으로 기존
    // 뷰를 새 컬럼 수로 다시 쪼개고, 회전 시 epub.js 자체 resize 처리가
    // 현재 위치를 다시 표시한다.
    const orientationMq = window.matchMedia(ORIENTATION_QUERY);
    const applySpread = (landscape) => {
      rendition.spread(
        landscape ? "always" : "none",
        landscape ? 1 : undefined,
      );
    };
    const onOrientationChange = () => applySpread(orientationMq.matches);
    orientationMq.addEventListener("change", onOrientationChange);
    applySpread(isLandscapeNow());

    // 텍스트 읽기 전용 기본 스타일
    rendition.themes.default({
      "html, body": { margin: 0, padding: 0 },
      "img, svg": { "max-width": "100%", height: "auto" },
    });
    if (fontSize !== 100) rendition.themes.fontSize(`${fontSize}%`);

    timeoutRef.current = setTimeout(() => {
      setIsLoading(false);
      setErrorMessage(`EPUB 렌더링 시간이 초과되었습니다. (book_id=${bookId})`);
    }, RENDER_TIMEOUT_MS);

    // 본문 스크롤 잠금(터치 팬 차단) + 늦은 크기 변화 재측정
    rendition.hooks.content.register(lockContentOverflow);
    // 새로 로드되는 섹션에도 글꼴 규칙을 적용한다
    rendition.hooks.content.register((contents) =>
      applyFontFamilyToContents(contents, fontFamilyRef.current),
    );
    rendition.on("rendered", (section, view) => {
      lockContentOverflow(view?.contents);
      installReexpand(view);
    });

    const syncPageInfo = (location) => {
      const displayed = location?.start?.displayed;
      if (displayed?.total > 0) {
        setPageInfo({ page: displayed.page, total: displayed.total });
      }
    };

    rendition.on("displayed", () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      setIsLoading(false);
      setErrorMessage(null);
    });
    rendition.on("displayerror", (err) => {
      setErrorMessage(`EPUB 렌더링 오류: ${err?.message || String(err)}`);
      setIsLoading(false);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    });
    rendition.on("error", (err) => {
      setErrorMessage(`EPUB 파싱 오류: ${err?.message || String(err)}`);
      setIsLoading(false);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    });

    // 위치 이동(버튼·스크롤 모두)마다 페이지 정보와 읽기 위치를 갱신한다.
    rendition.on("relocated", (location) => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      setIsLoading(false);
      setErrorMessage(null);
      syncPageInfo(location);
      if (location?.start?.cfi) {
        cfiRef.current = location.start.cfi;
      }
      if (preview || !bookId) return;
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      saveTimerRef.current = setTimeout(() => {
        writeSavedLocation(bookId, cfiRef.current);
      }, SAVE_THROTTLE_MS);
    });

    (async () => {
      try {
        await book.ready;
        const saved = !preview ? readSavedLocation(bookId) : null;
        if (preview || !saved) {
          await rendition.display();
        } else {
          try {
            await rendition.display(saved);
          } catch {
            // 저장된 위치가 무효하면 지우고 책 맨 앞부터
            localStorage.removeItem(`epub_location_${bookId}`);
            await rendition.display();
          }
        }
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
      } catch {
        localStorage.removeItem(`epub_location_${bookId}`);
        try {
          await rendition.display();
        } catch (fallbackErr) {
          setErrorMessage(
            `EPUB 표시 실패: ${fallbackErr?.message || String(fallbackErr)}`,
          );
          setIsLoading(false);
          if (timeoutRef.current) clearTimeout(timeoutRef.current);
        }
      }
    })();

    return () => {
      orientationMq.removeEventListener("change", onOrientationChange);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      try {
        rendition.destroy();
      } catch {
        /* ignore */
      }
      try {
        book.destroy();
      } catch {
        /* ignore */
      }
      renditionRef.current = null;
      bookRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- fontSize/fontFamily는 별도 effect로 적용한다
  }, [epubData, bookId, preview]);

  // 글자 크기·글꼴 변경: reflow 뒤 뷰가 낡은 폭에 머물지 않게 다시 측정한다.
  // 글꼴 규칙은 이미 표시된 뷰에 즉시 적용하고, 새 섹션은 content hook이 처리한다.
  useEffect(() => {
    const rendition = renditionRef.current;
    if (!rendition) return;
    rendition.themes.fontSize(`${fontSize}%`);
    fontFamilyRef.current = fontFamily;
    const views = rendition.manager?.views?.all?.() || [];
    for (const view of views) {
      applyFontFamilyToContents(view?.contents, fontFamily);
      reexpandView(view);
    }
  }, [fontSize, fontFamily]);

  const goNext = useCallback(() => {
    renditionRef.current?.next();
  }, []);

  const goPrev = useCallback(() => {
    renditionRef.current?.prev();
  }, []);

  // 키보드 방향키: 문서와 iframe 내부(rendition keyup) 둘 다 받는다.
  useEffect(() => {
    const rendition = renditionRef.current;
    if (!rendition) return;
    const onKey = (event) => {
      if (event.key === "ArrowRight") goNext();
      else if (event.key === "ArrowLeft") goPrev();
    };
    document.addEventListener("keydown", onKey);
    rendition.on("keyup", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      rendition.off("keyup", onKey);
    };
  }, [epubData, goNext, goPrev]);

  const handleFontSizeChange = useCallback((delta) => {
    setFontSize((prev) => {
      const next = Math.max(
        FONT_SIZE_MIN,
        Math.min(FONT_SIZE_MAX, prev + delta),
      );
      localStorage.setItem("epub_fontSize", String(next));
      return next;
    });
  }, []);

  const handleFontFamilyChange = useCallback((family) => {
    setFontFamily(family);
    localStorage.setItem("epub_fontFamily", family);
  }, []);

  return (
    <div
      className={`epub-viewer${preview ? "" : " epub-viewer--framed"}`}
      style={{
        height: containerHeight,
        textAlign: "center",
        position: "relative",
        overflow: "hidden",
      }}
    >
      {isLoading && (
        <div className="loading-container">
          <div className="spinner"></div>
          <span className="blinking">로딩 중...</span>
        </div>
      )}
      {errorMessage && <div className="error-message">{errorMessage}</div>}

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
        <div
          ref={hostRef}
          data-testid="epub-host"
          style={{ height: "100%", width: "100%", textAlign: "left" }}
        />
      </Suspense>

      {!isLoading && !errorMessage && epubData && (
        <>
          <button
            type="button"
            className="epub-page-btn epub-page-prev"
            onClick={goPrev}
            aria-label="이전 페이지"
          >
            ‹
          </button>
          <button
            type="button"
            className="epub-page-btn epub-page-next"
            onClick={goNext}
            aria-label="다음 페이지"
          >
            ›
          </button>
        </>
      )}

      {!preview && !isLoading && epubData && (
        <div className="epub-page-info" data-testid="epub-page-info">
          {pageInfo.total > 0
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
  standalone: PropTypes.bool,
};
