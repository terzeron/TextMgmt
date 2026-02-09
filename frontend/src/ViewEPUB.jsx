import { useEffect, useRef, useState, useCallback, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";

export default function ViewEPUB({ bookId, preview = false }) {
    const renditionRef = useRef(null);
    const timeoutRef = useRef(null);
    const [epubData, setEpubData] = useState(null);
    const [location, setLocation] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);

    useEffect(() => {
        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        const epubUrl = preview
            ? `${getApiUrlPrefix()}/preview/${bookId}?chapters=3`
            : `${getApiUrlPrefix()}/download/${bookId}`;

        setIsLoading(true);
        setErrorMessage(null);
        setEpubData(null);

        const controller = new AbortController();
        fetch(epubUrl, { signal: controller.signal })
            .then((res) => {
                if (!res.ok) throw new Error(`서버 응답 오류: ${res.status}`);
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
            setEpubData(null);
        };
    }, [bookId, preview]);

    // 로딩 타임아웃: 30초 내 로딩 미완료 시 에러 표시
    useEffect(() => {
        if (!epubData) return;
        timeoutRef.current = setTimeout(() => {
            setIsLoading(false);
            setErrorMessage("미리보기 로딩 시간이 초과되었습니다.");
        }, 30000);
        return () => clearTimeout(timeoutRef.current);
    }, [epubData]);

    const handleLocationChanged = useCallback((epubcfi) => {
        setLocation(epubcfi);
        setIsLoading(false);
        setErrorMessage(null);
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
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
                    location={location}
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
