import { useEffect, Suspense, useState, useRef } from "react";
import PropTypes from "prop-types";
import { RTFJS } from "rtf.js";
import { textGetReq } from "./Common";
import DOMPurify from "dompurify";

export default function ViewRTF({ bookId, apiPrefix = '' }) {
    const parentRef = useRef(null);
    const [errorMessage, setErrorMessage] = useState(null);
    const [isLoading, setIsLoading] = useState(true);

    const stringToArrayBuffer = (string) => {
        const buffer = new ArrayBuffer(string.length);
        const bufferView = new Uint8Array(buffer);
        for (let i = 0; i < string.length; i++) {
            bufferView[i] = string.charCodeAt(i);
        }
        return buffer;
    };

    useEffect(() => {
        console.log(`ViewRTF: useEffect()`, bookId);

        if (!bookId) {
            setErrorMessage("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }

        const downloadUrl = `${apiPrefix}/download/${bookId}`;
        console.log("Downloading RTF from:", downloadUrl);

        RTFJS.loggingEnabled(false);

        textGetReq(
            downloadUrl,
            null,
            (result) => {
                const doc = new RTFJS.Document(stringToArrayBuffer(result), null);

                doc.render()
                    .then((htmlElements) => {
                        if (parentRef.current) {
                            const tempContainer = document.createElement("div");
                            htmlElements.forEach((element) => {
                                tempContainer.appendChild(element);
                            });
                            const blockSvgDataUri = (node) => {
                                if (node.tagName && node.tagName.toLowerCase() === "img") {
                                    const src = node.getAttribute("src") || "";
                                    if (/^data:image\/svg\+xml/i.test(src)) {
                                        node.removeAttribute("src");
                                    }
                                }
                            };
                            DOMPurify.addHook("afterSanitizeAttributes", blockSvgDataUri);
                            try {
                                const safeHtml = DOMPurify.sanitize(tempContainer.innerHTML);
                                parentRef.current.innerHTML = safeHtml;
                            } finally {
                                DOMPurify.removeHook("afterSanitizeAttributes");
                            }
                        }
                        setErrorMessage(null); // ✅ 에러 초기화
                    })
                    .catch((error) => {
                        console.error("RTF 렌더링 실패:", error);
                        setErrorMessage("❌ RTF 렌더링에 실패했습니다.");
                    })
                    .finally(() => {
                        setIsLoading(false);
                    });
            },
            (error) => {
                console.error("RTF 로드 실패:", error);
                setErrorMessage(`❌ 파일을 불러올 수 없습니다: ${error}`);
                setIsLoading(false);
            }
        );
    }, [bookId, apiPrefix]);

    return (
        <div>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
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
                <div ref={parentRef} style={{ display: isLoading || errorMessage ? "none" : "block" }} />
            </Suspense>
        </div>
    );
}

ViewRTF.propTypes = {
    bookId: PropTypes.number.isRequired,
    apiPrefix: PropTypes.string,
};
