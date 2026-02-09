import { useEffect, useRef, useState, useCallback, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";
import "./ViewEPUB.css";

const CHAPTERS_PREVIEW = 2;
const CHAPTERS_FULLVIEW_INITIAL = 5;

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
    const backgroundEpubRef = useRef(null);
    const initialLoadDoneRef = useRef(false);
    const totalChaptersRef = useRef(0);
    const backgroundFetchDoneRef = useRef(false);

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

    const fetchChapters = useCallback((chapters, signal) => {
        const url = `${getApiUrlPrefix()}/preview/${bookId}?chapters=${chapters}`;
        return fetch(url, { signal })
            .then((res) => {
                if (!res.ok) throw new Error(`서버 응답 오류: ${res.status}`);
                const total = parseInt(res.headers.get("X-Total-Chapters") || "0", 10);
                return res.arrayBuffer().then((buf) => ({ buf, total }));
            });
    }, [bookId]);

    // 초기 로딩: 미리보기 1챕터, 전체보기 5챕터
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
        setPageInfo({ page: 0, total: 0 });

        // 전체보기: localStorage에서 읽기 위치 복원
        if (!preview) {
            const savedLocation = localStorage.getItem(`epub_location_${bookId}`);
            locationRef.current = savedLocation || "";
        } else {
            locationRef.current = "";
        }

        const initialChapters = preview ? CHAPTERS_PREVIEW : CHAPTERS_FULLVIEW_INITIAL;
        const controller = new AbortController();
        fetchChapters(initialChapters, controller.signal)
            .then(({ buf, total }) => {
                setEpubData(buf);
                setLoadedChapters(Math.min(initialChapters, total));
                setTotalChapters(total);
                totalChaptersRef.current = total;
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
            setEpubData(null);
        };
    }, [bookId, preview, fetchChapters]);

    // 로딩 타임아웃: 초기 로딩 시에만 적용 (15초)
    useEffect(() => {
        if (!epubData || initialLoadDone) return;
        timeoutRef.current = setTimeout(() => {
            setIsLoading(false);
            setErrorMessage("미리보기 로딩 시간이 초과되었습니다.");
        }, 15000);
        return () => clearTimeout(timeoutRef.current);
    }, [epubData, initialLoadDone]);

    // 전체보기: 초기 렌더링 후 백그라운드에서 전체 챕터 페칭 (렌더링하지 않음)
    useEffect(() => {
        if (!initialLoadDone) return;
        if (preview) return;
        if (totalChapters <= CHAPTERS_FULLVIEW_INITIAL) return;
        if (backgroundFetchDoneRef.current) return;

        backgroundFetchDoneRef.current = true;
        const controller = new AbortController();
        fetchChapters(totalChapters, controller.signal)
            .then(({ buf }) => {
                backgroundEpubRef.current = buf;
            })
            .catch((err) => {
                if (err.name === "AbortError") {
                    backgroundFetchDoneRef.current = false;
                }
            });

        return () => controller.abort();
    }, [initialLoadDone, preview, totalChapters, fetchChapters]);

    // 페이지 변경 핸들러
    const handleLocationChanged = useCallback((epubcfi) => {
        locationRef.current = epubcfi;
        setIsLoading(false);
        setErrorMessage(null);
        if (timeoutRef.current) clearTimeout(timeoutRef.current);

        if (!initialLoadDoneRef.current) {
            initialLoadDoneRef.current = true;
            setInitialLoadDone(true);
            return;
        }

        // 전체보기: 읽기 위치 저장
        if (!preview && bookId) {
            localStorage.setItem(`epub_location_${bookId}`, epubcfi);
        }

        // 백그라운드 데이터 준비 완료 시 페이지 넘기기에 맞춰 바꿔치기
        if (backgroundEpubRef.current) {
            const fullData = backgroundEpubRef.current;
            backgroundEpubRef.current = null;
            setEpubData(fullData);
            setLoadedChapters(totalChaptersRef.current);
            setEpubKey((k) => k + 1);
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
        renditionRef.current = rendition;
        const spine_get = rendition.book.spine.get.bind(rendition.book.spine);
        rendition.book.spine.get = function (target) {
            let t = spine_get(target);
            if (!t) {
                t = spine_get(undefined);
            }
            return t;
        };

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

            // 페이지 정보 이벤트 리스닝
            rendition.on("relocated", (location) => {
                if (location && location.start && location.start.displayed) {
                    setPageInfo({
                        page: location.start.displayed.page,
                        total: location.start.displayed.total,
                    });
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
            {!preview && pageInfo.total > 0 && (
                <div className="epub-page-info" data-testid="epub-page-info">
                    {pageInfo.page} / {pageInfo.total}
                </div>
            )}
        </div>
    );
}

ViewEPUB.propTypes = {
    bookId: PropTypes.number.isRequired,
    preview: PropTypes.bool,
};
