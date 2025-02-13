import {useEffect, useState, Suspense} from "react";
import PropTypes from "prop-types";
import {textGetReq} from "./Common";

export default function ViewTXT({bookId, lineCount}) {
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);
    const [fileContent, setFileContent] = useState([]);

    useEffect(() => {
        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        const downloadUrl = "/download/" + bookId;
        setIsLoading(true);
        setErrorMessage(null);

        textGetReq(
            downloadUrl,
            null,
            (result) => {
                const lineList = result.split("\n").slice(0, lineCount).map((line) => line);
                setFileContent(lineList);
                setIsLoading(false);
            },
            (error) => {
                setErrorMessage(`❌ 파일을 불러올 수 없습니다: ${error}`);
                setIsLoading(false);
            }
        );

        return () => {
            setFileContent([]);
        };
    }, [bookId, lineCount]);

    return (
        <div className="txt-container">
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
                {!isLoading &&
                    fileContent.map((line, index) => (
                        <div key={index}>{line}</div>
                    ))
                }
            </Suspense>
        </div>
    );
}

ViewTXT.propTypes = {
    bookId: PropTypes.number.isRequired,
    lineCount: PropTypes.number,
};
