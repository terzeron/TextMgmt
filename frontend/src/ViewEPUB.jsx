import { useEffect, useRef, useState, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";

export default function ViewEPUB({ bookId, filePath }) {
    const renditionRef = useRef(null);
    const [url, setUrl] = useState("");
    const [location, setLocation] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);

    useEffect(() => {
        if (!bookId || !filePath) {
            setErrorMessage("❌ 유효한 bookId 또는 filePath가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        const epubUrl = `${getApiUrlPrefix()}/download/${bookId}/${filePath}`;
        setUrl(epubUrl);
        setIsLoading(true);
        setErrorMessage(null);

        return () => {
            setUrl("");
        };
    }, [bookId, filePath]);

    return (
        <div style={{ height: "100vh", textAlign: "center", position: "relative" }}>
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
                    locationChanged={(epubcfi) => {
                        setLocation(epubcfi);
                        setIsLoading(false); // ✅ EPUB 로딩이 완료되면 스피너 숨김
                    }}
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
    filePath: PropTypes.string.isRequired,
};
