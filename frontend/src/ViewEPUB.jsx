import { useEffect, useRef, useState, useCallback, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";

export default function ViewEPUB({ bookId, filePath, preview = false }) {
    const renditionRef = useRef(null);
    const timeoutRef = useRef(null);
    const [url, setUrl] = useState("");
    const [location, setLocation] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);

    useEffect(() => {
        if (!bookId || (!filePath && !preview)) {
            setErrorMessage("❌ 유효한 bookId 또는 filePath가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        const epubUrl = preview
            ? `${getApiUrlPrefix()}/preview/${bookId}?chapters=3`
            : `${getApiUrlPrefix()}/download/${bookId}/${filePath}`;
        setUrl(epubUrl);
        setIsLoading(true);
        setErrorMessage(null);

        return () => {
            setUrl("");
        };
    }, [bookId, filePath, preview]);

    // 로딩 타임아웃: 30초 내 로딩 미완료 시 에러 표시
    useEffect(() => {
        if (!url) return;
        timeoutRef.current = setTimeout(() => {
            setIsLoading(false);
            setErrorMessage("미리보기 로딩 시간이 초과되었습니다.");
        }, 30000);
        return () => clearTimeout(timeoutRef.current);
    }, [url]);

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
                <ReactReader
                    location={location}
                    locationChanged={handleLocationChanged}
                    url={url}
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
                />
            </Suspense>
        </div>
    );
}

ViewEPUB.propTypes = {
    bookId: PropTypes.number.isRequired,
    filePath: PropTypes.string,
    preview: PropTypes.bool,
};
