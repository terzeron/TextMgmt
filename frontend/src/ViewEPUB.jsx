import { useEffect, useRef, useState, useCallback, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";

const CHAPTERS_PREVIEW = 2;
const CHAPTERS_FULLVIEW_INITIAL = 5;

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
        locationRef.current = "";
        backgroundEpubRef.current = null;
        backgroundFetchDoneRef.current = false;

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
                // cleanup에 의한 abort만 재시도 허용, 네트워크 에러는 재시도 안 함
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

        // 백그라운드 데이터 준비 완료 시 페이지 넘기기에 맞춰 바꿔치기
        if (backgroundEpubRef.current) {
            const fullData = backgroundEpubRef.current;
            backgroundEpubRef.current = null;
            setEpubData(fullData);
            setLoadedChapters(totalChaptersRef.current);
            setEpubKey((k) => k + 1);
        }
    }, []);

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
        </div>
    );
}

ViewEPUB.propTypes = {
    bookId: PropTypes.number.isRequired,
    preview: PropTypes.bool,
};
