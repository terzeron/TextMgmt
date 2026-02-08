import { useEffect, useState } from "react";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import mammoth from "mammoth";

export default function ViewDOC({ bookId, fileType, lineCount }) {
    const [content, setContent] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [errorMessage, setErrorMessage] = useState(null);

    useEffect(() => {
        console.log(`ViewDOC: useEffect()`, bookId, fileType);

        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        if (fileType === 'doc' || fileType === 'hwp') {
            // .doc/.hwp: 서버에서 HTML로 변환된 미리보기 사용
            const previewUri = getApiUrlPrefix() + "/preview/" + bookId;
            fetch(previewUri)
                .then(response => response.text())
                .then(html => {
                    setContent(html);
                    setIsLoading(false);
                })
                .catch(error => {
                    console.error("Error loading DOC file:", error);
                    setErrorMessage("❌ 문서를 불러오는 중 오류가 발생했습니다.");
                    setIsLoading(false);
                });
        } else {
            // .docx: mammoth 사용
            const uri = getApiUrlPrefix() + "/download/" + bookId;
            fetch(uri)
                .then(response => response.arrayBuffer())
                .then(buffer => mammoth.convertToHtml({ arrayBuffer: buffer }))
                .then(result => {
                    const paragraphs = result.value.split("</p>").slice(0, lineCount).join("</p>") + "</p>";
                    setContent(paragraphs);
                    setIsLoading(false);
                })
                .catch(error => {
                    console.error("Error loading DOCX file:", error);
                    setErrorMessage("❌ 문서를 불러오는 중 오류가 발생했습니다.");
                    setIsLoading(false);
                });
        }
    }, [bookId, fileType, lineCount]);

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
            <div
                className="doc-content"
                dangerouslySetInnerHTML={{ __html: content }}
                style={{ display: isLoading || errorMessage ? "none" : "block" }}
            />
        </div>
    );
}

ViewDOC.propTypes = {
    bookId: PropTypes.number.isRequired,
    fileType: PropTypes.string,
    lineCount: PropTypes.number,
};
