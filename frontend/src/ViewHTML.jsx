import { useEffect, useRef, useState } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";

export default function ViewHTML({ bookId }) {
    const ref = useRef(null);
    const [iframeHeight, setIframeHeight] = useState(0);
    const [url, setUrl] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);

    useEffect(() => {
        console.log(`ViewHTML: useEffect(${bookId})`);

        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        setIframeHeight(document.body.scrollHeight);
        const iframeUrl = getApiUrlPrefix() + "/download/" + bookId;
        console.log(iframeUrl);
        setUrl(iframeUrl);
        setIsLoading(true);
        setErrorMessage(null);

        return () => {
            setUrl("");
            setIframeHeight(0);
        };
    }, [bookId]);

    return (
        <div className="html-container">
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
            <iframe
                title="html viewer"
                src={url}
                ref={ref}
                onLoad={() => setIsLoading(false)} // ✅ iframe 로드 완료 시 로딩 숨김
                style={{
                    display: isLoading || errorMessage ? "none" : "block",
                    width: "100%",
                    height: iframeHeight,
                    overflow: "visible",
                    border: "none"
                }}
            />
        </div>
    );
}

ViewHTML.propTypes = {
    bookId: PropTypes.number.isRequired,
};
