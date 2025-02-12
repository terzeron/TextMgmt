import { useEffect, useRef, useState } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";

export default function ViewDOC({ bookId }) {
    const [url, setUrl] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);
    const [iframeHeight, setIframeHeight] = useState(0);
    const ref = useRef(null);

    useEffect(() => {
        console.log(`ViewDOC: useEffect()`, bookId);

        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        setIframeHeight(document.body.scrollHeight);

        const uri = getApiUrlPrefix() + "/download/" + bookId;
        const wordViewerUrlPrefix = "https://view.officeapps.live.com/op/embed.aspx?src=";
        const fullUrl = wordViewerUrlPrefix + encodeURIComponent(uri);

        setUrl(fullUrl);
        setIsLoading(true);
        setErrorMessage(null);
        console.log(fullUrl);
    }, [bookId]);

    return (
        <div className="doc-container">
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
                title="doc viewer"
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

ViewDOC.propTypes = {
    bookId: PropTypes.number.isRequired
};
