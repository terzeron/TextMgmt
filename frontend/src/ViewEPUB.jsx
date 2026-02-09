import { useEffect, useRef, useState, useCallback, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";

const CHAPTERS_INITIAL = 1;
const CHAPTERS_STEP = 10;

export default function ViewEPUB({ bookId, preview = false }) {
    const renditionRef = useRef(null);
    const timeoutRef = useRef(null);
    const locationRef = useRef("");

    const [epubData, setEpubData] = useState(null);
    const [initialLoadDone, setInitialLoadDone] = useState(false);
    const [isLoading, setIsLoading] = useState(true);
    const [isLoadingMore, setIsLoadingMore] = useState(false);
    const [errorMessage, setErrorMessage] = useState(null);
    const [loadedChapters, setLoadedChapters] = useState(0);
    const [totalChapters, setTotalChapters] = useState(0);
    const [epubKey, setEpubKey] = useState(0);

    const fetchChapters = useCallback((chapters, signal) => {
        const url = `${getApiUrlPrefix()}/preview/${bookId}?chapters=${chapters}`;
        return fetch(url, { signal })
            .then((res) => {
                if (!res.ok) throw new Error(`서버 응답 오류: ${res.status}`);
                const total = parseInt(res.headers.get("X-Total-Chapters") || "0", 10);
                return res.arrayBuffer().then((buf) => ({ buf, total }));
            });
    }, [bookId]);

    // 초기 로딩: 1챕터로 시작
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
        setInitialLoadDone(false);
        locationRef.current = "";

        const controller = new AbortController();
        fetchChapters(CHAPTERS_INITIAL, controller.signal)
            .then(({ buf, total }) => {
                setEpubData(buf);
                setLoadedChapters(CHAPTERS_INITIAL);
                setTotalChapters(total);
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

    // 로딩 타임아웃: 15초
    useEffect(() => {
        if (!epubData) return;
        timeoutRef.current = setTimeout(() => {
            setIsLoading(false);
            setErrorMessage("미리보기 로딩 시간이 초과되었습니다.");
        }, 15000);
        return () => clearTimeout(timeoutRef.current);
    }, [epubData]);

    // 미리보기 모드: 첫 렌더링 후 자동으로 추가 챕터 로드
    useEffect(() => {
        if (!preview) return;
        if (!initialLoadDone) return;
        if (loadedChapters !== CHAPTERS_INITIAL) return;
        if (totalChapters <= CHAPTERS_INITIAL) return;

        const nextChapters = Math.min(CHAPTERS_INITIAL + CHAPTERS_STEP, totalChapters);
        const controller = new AbortController();
        setIsLoadingMore(true);

        fetchChapters(nextChapters, controller.signal)
            .then(({ buf, total }) => {
                setEpubData(buf);
                setLoadedChapters(nextChapters);
                setTotalChapters(total);
                setEpubKey((k) => k + 1);
                setIsLoadingMore(false);
            })
            .catch((err) => {
                if (err.name !== "AbortError") {
                    setIsLoadingMore(false);
                }
            });

        return () => controller.abort();
    }, [preview, initialLoadDone, loadedChapters, totalChapters, fetchChapters]);

    const handleLoadMore = useCallback(() => {
        if (isLoadingMore) return;
        const nextChapters = Math.min(loadedChapters + CHAPTERS_STEP, totalChapters);
        if (nextChapters <= loadedChapters) return;

        const controller = new AbortController();
        setIsLoadingMore(true);

        fetchChapters(nextChapters, controller.signal)
            .then(({ buf, total }) => {
                setEpubData(buf);
                setLoadedChapters(nextChapters);
                setTotalChapters(total);
                setEpubKey((k) => k + 1);
                setIsLoadingMore(false);
            })
            .catch((err) => {
                if (err.name !== "AbortError") {
                    setIsLoadingMore(false);
                }
            });
    }, [isLoadingMore, loadedChapters, totalChapters, fetchChapters]);

    const handleLocationChanged = useCallback((epubcfi) => {
        locationRef.current = epubcfi;
        setIsLoading(false);
        setErrorMessage(null);
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
        setInitialLoadDone(true);
    }, []);

    const containerHeight = preview ? "60vh" : "100vh";
    const hasMoreChapters = totalChapters > 0 && loadedChapters < totalChapters;

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
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                {epubData && <ReactReader
                    key={epubKey}
                    location={locationRef.current || undefined}
                    locationChanged={handleLocationChanged}
                    url={epubData}
                    getRendition={(rendition) => {
                        renditionRef.current = rendition;
                        const spine_get = rendition.book.spine.get.bind(rendition.book.spine);
                        rendition.book.spine.get = function (target) {
                            let t = spine_get(target);
                            if (!t) {
                                t = spine_get(undefined);
                            }
                            return t;
                        };
                    }}
                />}
            </Suspense>
            {!preview && hasMoreChapters && !isLoading && (
                <div style={{ padding: "10px", textAlign: "center" }}>
                    <button
                        onClick={handleLoadMore}
                        disabled={isLoadingMore}
                        style={{
                            padding: "8px 16px",
                            cursor: isLoadingMore ? "wait" : "pointer",
                            opacity: isLoadingMore ? 0.6 : 1,
                        }}
                    >
                        {isLoadingMore
                            ? "로딩 중..."
                            : `더 보기 (${loadedChapters}/${totalChapters} 챕터 로드됨)`}
                    </button>
                </div>
            )}
            {isLoadingMore && (
                <div style={{ padding: "5px", textAlign: "center", fontSize: "0.9em", color: "#666" }}>
                    추가 챕터 로딩 중...
                </div>
            )}
        </div>
    );
}

ViewEPUB.propTypes = {
    bookId: PropTypes.number.isRequired,
    preview: PropTypes.bool,
};
