import {useEffect, useState} from "react";
import PropTypes from "prop-types";
import {getApiUrlPrefix} from "./Common";

export default function ViewImage({bookId}) {
    const [url, setUrl] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);

    useEffect(() => {
        console.log(`ViewImage: useEffect(${bookId})`);

        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        const imageUrl = getApiUrlPrefix() + "/download/" + bookId;
        console.log(imageUrl);
        setUrl(imageUrl);
        setIsLoading(true);
        setErrorMessage(null);
    }, [bookId]);

    return (
        <div className="image-container">
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
            <img
                src={url}
                alt="book cover"
                onLoad={() => setIsLoading(false)}
                onError={() => {
                    setIsLoading(false);
                    setErrorMessage("❌ 이미지 파일을 불러올 수 없습니다.");
                }}
                style={{
                    display: isLoading || errorMessage ? "none" : "block",
                    maxWidth: "100%",
                    height: "auto",
                }}
            />
        </div>
    );
}

ViewImage.propTypes = {
    bookId: PropTypes.number.isRequired,
};

