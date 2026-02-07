import { useEffect, useRef, useState, Suspense } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import { ReactReader } from "react-reader";
import DOMPurify from "dompurify";

export default function ViewEPUB({ bookId, filePath, preview = false }) {
    const renditionRef = useRef(null);
    const [url, setUrl] = useState("");
    const [location, setLocation] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);
    const [previewHtml, setPreviewHtml] = useState("");

    useEffect(() => {
        if (!bookId || (!filePath && !preview)) {
            setErrorMessage("❌ 유효한 bookId 또는 filePath가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        if (preview) {
            const previewUrl = `${getApiUrlPrefix()}/preview/${bookId}?chapters=3`;
            setIsLoading(true);
            setErrorMessage(null);
            fetch(previewUrl)
                .then(res => {
                    if (!res.ok) throw new Error(`HTTP ${res.status}`);
                    return res.text();
                })
                .then(html => {
                    // DOMPurify로 sanitize하여 XSS 방지
                    setPreviewHtml(DOMPurify.sanitize(html, { ADD_TAGS: ["style"], ADD_ATTR: ["class", "id"] }));
                    setIsLoading(false);
                })
                .catch(err => {
                    console.error("EPUB preview 로드 실패:", err);
                    setErrorMessage("❌ EPUB 미리보기를 불러올 수 없습니다.");
                    setIsLoading(false);
                });

            return () => {
                setPreviewHtml("");
            };
        }

        const epubUrl = `${getApiUrlPrefix()}/download/${bookId}/${filePath}`;
        setUrl(epubUrl);
        setIsLoading(true);
        setErrorMessage(null);

        return () => {
            setUrl("");
        };
    }, [bookId, filePath, preview]);

    if (preview) {
        return (
            <div style={{ textAlign: "center", position: "relative" }}>
                {isLoading && (
                    <div className="loading-container">
                        <div className="spinner"></div>
                        <span className="blinking">EPUB 미리보기 로딩 중...</span>
                    </div>
                )}
                {errorMessage && (
                    <div className="error-message">{errorMessage}</div>
                )}
                {previewHtml && (
                    <div
                        style={{
                            maxHeight: "80vh",
                            overflow: "auto",
                            textAlign: "left",
                            padding: "20px",
                            border: "1px solid #ddd",
                            borderRadius: "4px",
                            background: "#fff",
                        }}
                        dangerouslySetInnerHTML={{ __html: previewHtml }}
                    />
                )}
            </div>
        );
    }

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
                        setIsLoading(false);
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
    filePath: PropTypes.string,
    preview: PropTypes.bool,
};
