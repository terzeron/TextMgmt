import { useEffect, useRef, useState, useCallback, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";
import "./ViewEPUB.css";

const CHAPTERS_PREVIEW = 2;
const CHAPTERS_FULLVIEW_INITIAL = 1;

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

export default function ViewEPUB({ bookId, preview = false }) {
    const renditionRef = useRef(null);
    const timeoutRef = useRef(null);
    const locationRef = useRef("");
    const savedLocationRef = useRef(null);
    const backgroundEpubRef = useRef(null);
    const initialLoadDoneRef = useRef(false);
    const totalChaptersRef = useRef(0);
    const backgroundFetchDoneRef = useRef(false);
    const locationsReadyRef = useRef(false);
    const allChaptersLoadedRef = useRef(false);
    const diagStateRef = useRef("fetch");

    const [epubData, setEpubData] = useState(null);
    const [initialLoadDone, setInitialLoadDone] = useState(false);
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);
    const [loadedChapters, setLoadedChapters] = useState(0);
    const [totalChapters, setTotalChapters] = useState(0);
    const [epubKey, setEpubKey] = useState(0);

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
    const [backgroundFetchReady, setBackgroundFetchReady] = useState(false);

    const fetchChapters = useCallback((chapters, signal) => {
        const url = `${getApiUrlPrefix()}/preview/${bookId}?chapters=${chapters}`;
        return fetch(url, { signal })
            .then(async (res) => {
                if (!res.ok) {
                    const errText = await res.text();
                    throw new Error(errText || `서버 응답 오류: ${res.status}`);
                }
                const total = parseInt(res.headers.get("X-Total-Chapters") || "0", 10);
                const buf = await res.arrayBuffer();
                return { buf, total };
            });
    }, [bookId]);

    // 초기 로딩: 미리보기 2챕터, 전체보기 1챕터 (나머지는 백그라운드 페칭)
    useEffect(() => {
        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        setIsLoading(true);
        setErrorMessage(null);
        setEpubData(null);
        setLoadedChapters(0);
        setTotalChapters(0);
        totalChaptersRef.current = 0;
        setInitialLoadDone(false);
        initialLoadDoneRef.current = false;
        backgroundEpubRef.current = null;
        backgroundFetchDoneRef.current = false;
        locationsReadyRef.current = false;
        allChaptersLoadedRef.current = false;
        setPageInfo({ page: 0, total: 0 });
        setLocationsReady(false);
        setBackgroundFetchReady(false);

        // 전체보기: 읽기 위치를 별도 저장 (초기 부분 로드에는 적용하지 않음)
        if (!preview) {
            savedLocationRef.current = localStorage.getItem(`epub_location_${bookId}`) || null;
        } else {
            savedLocationRef.current = null;
        }
        locationRef.current = "";

        const initialChapters = preview ? CHAPTERS_PREVIEW : CHAPTERS_FULLVIEW_INITIAL;
        const controller = new AbortController();
        fetchChapters(initialChapters, controller.signal)
            .then(({ buf, total }) => {
                setEpubData(buf);
                setLoadedChapters(Math.min(initialChapters, total));
                setTotalChapters(total);
                totalChaptersRef.current = total;
                allChaptersLoadedRef.current = Math.min(initialChapters, total) >= total;
                setEpubKey((k) => k + 1);
            })
            .catch((err) => {
                if (err.name !== "AbortError") {
                    setErrorMessage(`EPUB 로딩 실패: ${err.message}`);
                    setIsLoading(false);
                }
            });

        return () => {
            controller.abort();
            // 이전 rendition 리소스 해제 (epubjs 내부 상태 충돌 방지)
            if (renditionRef.current) {
                try {
                    renditionRef.current.destroy();
                } catch (_) { /* 이미 파괴된 경우 무시 */ }
                renditionRef.current = null;
            }
            setEpubData(null);
        };
    }, [bookId, preview, fetchChapters]);

    // 로딩 타임아웃: 초기 로딩 시에만 적용 (15초)
    useEffect(() => {
        if (!epubData || initialLoadDone) return;
        timeoutRef.current = setTimeout(() => {
            // epub.js 내부 상태 진단 수집
            let diag = diagStateRef.current;
            const r = renditionRef.current;
            if (r) {
                try {
                    const mgr = r.manager;
                    const vis = mgr?.visible?.()?.length ?? "?";
                    const disp = mgr?.views?.displayed?.()?.length ?? "?";
                    const cw = mgr?.container?.offsetWidth ?? "?";
                    const ch = mgr?.container?.offsetHeight ?? "?";
                    diag += ` mgr=${!!mgr} vis=${vis} disp=${disp} container=${cw}x${ch}`;
                } catch (_) {}
            }
            setIsLoading(false);
            setErrorMessage(`EPUB 로딩 시간이 초과되었습니다. (book_id=${bookId}, stage=${diag})`);
        }, 15000);
        return () => clearTimeout(timeoutRef.current);
    }, [epubData, initialLoadDone, bookId]);

    // 전체보기: 초기 챕터 수신 즉시 전체 챕터 백그라운드 페칭
    useEffect(() => {
        if (preview) return;
        if (totalChapters <= CHAPTERS_FULLVIEW_INITIAL) return;
        if (backgroundFetchDoneRef.current) return;

        backgroundFetchDoneRef.current = true;
        const controller = new AbortController();
        fetchChapters(totalChapters, controller.signal)
            .then(({ buf }) => {
                backgroundEpubRef.current = buf;
                setBackgroundFetchReady(true);
            })
            .catch((err) => {
                if (err.name === "AbortError") {
                    backgroundFetchDoneRef.current = false;
                }
            });

        return () => controller.abort();
    }, [preview, totalChapters, fetchChapters]);

    // 전체보기: 초기 렌더링 완료 + 전체 챕터 준비 시 자동 교체
    useEffect(() => {
        if (preview || !initialLoadDone || !backgroundFetchReady) return;
        if (!backgroundEpubRef.current) return;

        const fullData = backgroundEpubRef.current;
        backgroundEpubRef.current = null;

        if (savedLocationRef.current) {
            locationRef.current = savedLocationRef.current;
            savedLocationRef.current = null;
        }
        allChaptersLoadedRef.current = true;
        setEpubData(fullData);
        setLoadedChapters(totalChaptersRef.current);
        setEpubKey((k) => k + 1);
    }, [preview, initialLoadDone, backgroundFetchReady]);

    // 페이지 변경 핸들러
    const handleLocationChanged = useCallback((epubcfi) => {
        locationRef.current = epubcfi;
        setIsLoading(false);
        setErrorMessage(null);
        if (timeoutRef.current) clearTimeout(timeoutRef.current);

        if (!initialLoadDoneRef.current) {
            initialLoadDoneRef.current = true;
            setInitialLoadDone(true);

            // 전체보기: 전체 챕터가 초기 로드로 충분하면 저장된 위치 즉시 복원
            if (savedLocationRef.current && totalChaptersRef.current <= CHAPTERS_FULLVIEW_INITIAL) {
                locationRef.current = savedLocationRef.current;
                savedLocationRef.current = null;
                setEpubKey((k) => k + 1);
            }
            return;
        }

        // 전체보기: 읽기 위치 저장
        if (!preview && bookId) {
            localStorage.setItem(`epub_location_${bookId}`, epubcfi);
        }
    }, [preview, bookId]);

    // 글자 크기 변경
    const handleFontSizeChange = useCallback((delta) => {
        setFontSize((prev) => {
            const next = Math.max(FONT_SIZE_MIN, Math.min(FONT_SIZE_MAX, prev + delta));
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

    const getRendition = useCallback((rendition) => {
        // 이전 rendition이 남아있으면 정리 (epubKey 변경에 의한 재마운트)
        if (renditionRef.current && renditionRef.current !== rendition) {
            try {
                renditionRef.current.destroy();
            } catch (_) { /* 이미 파괴된 경우 무시 */ }
        }
        renditionRef.current = rendition;
        locationsReadyRef.current = false;
        setLocationsReady(false);
        diagStateRef.current = "getRendition";

        const spine_get = rendition.book.spine.get.bind(rendition.book.spine);
        rendition.book.spine.get = function (target) {
            let t = spine_get(target);
            if (!t) {
                t = spine_get(undefined);
            }
            return t;
        };

        // epub.js display 에러 수신
        rendition.on("displayerror", (err) => {
            console.error("[epub.js] display error:", err);
            setErrorMessage(`EPUB 렌더링 오류: ${err?.message || String(err)}`);
            setIsLoading(false);
            if (timeoutRef.current) clearTimeout(timeoutRef.current);
        });

        // book 로드 에러 수신
        rendition.book.ready
            .then(() => { diagStateRef.current = "bookReady"; })
            .catch((err) => {
                console.error("[epub.js] book load error:", err);
                setErrorMessage(`EPUB 파싱 오류: ${err?.message || String(err)}`);
                setIsLoading(false);
                if (timeoutRef.current) clearTimeout(timeoutRef.current);
            });

        // rendition.display 실패를 적극적으로 잡기 (react-reader가 미처리)
        if (typeof rendition.display === "function") {
            const origDisplay = rendition.display.bind(rendition);
            rendition.display = function (target) {
                diagStateRef.current = `display(${String(target).slice(0, 40)})`;
                return origDisplay(target)
                    .then((result) => {
                        diagStateRef.current = "displayed";
                        return result;
                    })
                    .catch((err) => {
                        console.error("[epub.js] display() rejected:", err);
                        setErrorMessage(`EPUB 표시 실패: ${err?.message || String(err)}`);
                        setIsLoading(false);
                        if (timeoutRef.current) clearTimeout(timeoutRef.current);
                        throw err;
                    });
            };
        }

        // rendered 이벤트 추적
        rendition.on("started", () => { diagStateRef.current = "started"; });
        rendition.on("rendered", () => { diagStateRef.current = "rendered"; });

        // 전체보기 전용 초기화
        if (!preview) {
            // 책 제목 추출
            rendition.book.loaded.metadata.then((meta) => {
                if (meta && meta.title) {
                    setBookTitle(meta.title);
                }
            });

            // 저장된 글자 크기 적용
            const savedSize = localStorage.getItem("epub_fontSize");
            if (savedSize) {
                rendition.themes.fontSize(`${savedSize}%`);
            }

            // 저장된 글꼴 적용
            const savedFont = localStorage.getItem("epub_fontFamily");
            if (savedFont) {
                rendition.themes.font(savedFont);
            }

            // 전체 페이지 위치 인덱스 생성 (전체 챕터 로드 완료 시)
            if (allChaptersLoadedRef.current) {
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
                                const idx = rendition.book.locations.locationFromCfi(locationRef.current);
                                // locations.total은 0-indexed 최대값 (_locations.length - 1)
                                const totalPages = rendition.book.locations.total + 1;
                                if (idx >= 0 && totalPages > 0) {
                                    setPageInfo({ page: idx + 1, total: totalPages });
                                }
                            } catch (_) { /* ignore */ }
                        }
                    })
                    .catch((err) => console.warn("[epub.js] locations error:", err));
            }

            // 페이지 정보 이벤트 리스닝 (전체 위치 준비 시에만 전역 페이지 표시)
            rendition.on("relocated", (location) => {
                if (location && location.start && locationsReadyRef.current && location.start.cfi) {
                    try {
                        const idx = renditionRef.current.book.locations.locationFromCfi(location.start.cfi);
                        const totalPages = renditionRef.current.book.locations.total + 1;
                        if (idx >= 0 && totalPages > 0) {
                            setPageInfo({ page: idx + 1, total: totalPages });
                        }
                    } catch (_) { /* ignore */ }
                }
            });
        }
    }, [preview]);

    const containerHeight = preview ? "60vh" : "100vh";

    return (
        <div style={{ height: containerHeight, textAlign: "center", position: "relative" }}>
            {isLoading && (
                <div className="loading-container">
                    <div className="spinner"></div>
                    <span className="blinking">로딩 중...</span>
                </div>
            )}
            {errorMessage && (
                <div className="error-message">
                    {errorMessage}
                </div>
            )}

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
                {epubData && <ReactReader
                    key={epubKey}
                    location={locationRef.current || undefined}
                    locationChanged={handleLocationChanged}
                    url={epubData}
                    title={!preview ? bookTitle : undefined}
                    getRendition={getRendition}
                />}
            </Suspense>

            {/* 전체보기 전용 페이지 정보 */}
            {!preview && !isLoading && epubData && (
                <div className="epub-page-info" data-testid="epub-page-info">
                    {locationsReady && pageInfo.total > 0
                        ? `${pageInfo.page} / ${pageInfo.total}`
                        : loadedChapters < totalChapters
                            ? `전체 챕터 로딩 중... (${loadedChapters}/${totalChapters})`
                            : "페이지 계산 중..."}
                </div>
            )}
        </div>
    );
}

ViewEPUB.propTypes = {
    bookId: PropTypes.number.isRequired,
    preview: PropTypes.bool,
};
