import { useEffect, useRef, useState } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";

export default function ViewHTML({ bookId, apiPrefix = '' }) {
    const ref = useRef(null);
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

        const iframeUrl = getApiUrlPrefix() + apiPrefix + "/preview/" + bookId;
        console.log(iframeUrl);
        setUrl(iframeUrl);
        setIsLoading(true);
        setErrorMessage(null);

        return () => {
            setUrl("");
        };
    }, [bookId, apiPrefix]);

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
                sandbox=""
                onLoad={() => setIsLoading(false)}
                style={{
                    display: isLoading || errorMessage ? "none" : "block",
                    width: "100%",
                    height: "100vh",
                    overflow: "visible",
                    border: "none"
                }}
            />
        </div>
    );
}

ViewHTML.propTypes = {
    bookId: PropTypes.number.isRequired,
    apiPrefix: PropTypes.string,
};
