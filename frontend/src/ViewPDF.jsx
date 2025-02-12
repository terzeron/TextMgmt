import {useEffect, useRef, useState, Suspense} from "react";
import PropTypes from "prop-types";
import {getApiUrlPrefix} from "./Common";
import * as pdfjs from "pdfjs-dist/legacy/build/pdf";
import workerSrc from "pdfjs-dist/build/pdf.worker?url";

pdfjs.GlobalWorkerOptions.workerSrc = workerSrc;

export default function ViewPDF({bookId}) {
    const [url, setUrl] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState(null);
    const canvasRef = useRef(null);

    useEffect(() => {
        if (!bookId) {
            setError("❌ 유효한 bookId가 제공되지 않았습니다.");
            setIsLoading(false);
            return;
        }
        const pdfUrl = getApiUrlPrefix() + "/download/" + bookId;
        setUrl(pdfUrl);
    }, [bookId]);

    useEffect(() => {
        if (!url) return;

        const loadPdf = async () => {
            setIsLoading(true);
            setError(null);

            try {
                const loadingTask = pdfjs.getDocument(url);
                const pdf = await loadingTask.promise;
                const page = await pdf.getPage(1);
                const viewport = page.getViewport({scale: 1.5});

                const canvas = canvasRef.current;
                if (!canvas) {
                    throw new Error("캔버스 요소를 찾을 수 없습니다.");
                }

                const context = canvas.getContext("2d");
                canvas.width = viewport.width;
                canvas.height = viewport.height;

                // ✅ 원본 코드로 렌더링 로직 복원 (page.render()의 반환 값 수정)
                const renderTask = page.render({
                    canvasContext: context,
                    viewport: viewport,
                });
                await renderTask;

                setError(null);
            } catch (err) {
                console.error("PDF 로드 실패:", err);
                setError("❌ PDF 파일을 정상적으로 렌더링하지 못했습니다. 파일이 존재하지 않거나 올바르지 않은 형식일 수 있습니다.");
            } finally {
                setIsLoading(false);
            }
        };

        loadPdf();
    }, [url]);

    return (
        <div className="pdf-container">
            {isLoading && (
                <div className="loading-container">
                    <div className="spinner"></div>
                    <span className="blinking">로딩 중...</span>
                </div>
            )}
            {error && (
                <div className="error-message">
                    {error}
                </div>
            )}
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <canvas ref={canvasRef} style={{display: isLoading || error ? "none" : "block"}}/>
            </Suspense>

            {/* CSS 애니메이션 추가 */}
            <style>
                {`
                    .pdf-container {
                        display: flex;
                        flex-direction: column;
                        align-items: center;
                        justify-content: center;
                        padding: 20px;
                        text-align: center;
                    }
                `}
            </style>
        </div>
    );
}

ViewPDF.propTypes = {
    bookId: PropTypes.number.isRequired,
};
