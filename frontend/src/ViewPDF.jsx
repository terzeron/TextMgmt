import {useEffect, useRef, useState, Suspense} from "react";
import PropTypes from "prop-types";
import {getApiUrlPrefix} from "./Common";
import * as pdfjs from "pdfjs-dist";

// unpkg CDN에서 워커 로드 (package.json의 pdfjs-dist 버전과 일치)
pdfjs.GlobalWorkerOptions.workerSrc = "https://unpkg.com/pdfjs-dist@4.10.38/build/pdf.worker.min.mjs";

export default function ViewPDF({bookId, pageCount = 0}) {
    const [url, setUrl] = useState("");
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState(null);
    const [totalPages, setTotalPages] = useState(0);
    const [renderedPages, setRenderedPages] = useState([]);
    const containerRef = useRef(null);

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
            setRenderedPages([]);

            try {
                const loadingTask = pdfjs.getDocument(url);
                const pdf = await loadingTask.promise;
                setTotalPages(pdf.numPages);

                // pageCount가 0이면 전체 페이지, 아니면 제한된 페이지
                const pagesToRender = pageCount > 0 ? Math.min(pdf.numPages, pageCount) : pdf.numPages;
                const pageDataArray = [];

                for (let i = 1; i <= pagesToRender; i++) {
                    const page = await pdf.getPage(i);
                    const viewport = page.getViewport({scale: 1.5});

                    const canvas = document.createElement("canvas");
                    const context = canvas.getContext("2d");
                    canvas.width = viewport.width;
                    canvas.height = viewport.height;

                    const renderTask = page.render({
                        canvasContext: context,
                        viewport: viewport,
                    });
                    await renderTask.promise;

                    pageDataArray.push({
                        pageNum: i,
                        dataUrl: canvas.toDataURL(),
                        width: viewport.width,
                        height: viewport.height,
                    });
                }

                setRenderedPages(pageDataArray);
                setError(null);
            } catch (err) {
                console.error("PDF 로드 실패:", err);
                setError("❌ PDF 파일을 정상적으로 렌더링하지 못했습니다. 파일이 존재하지 않거나 올바르지 않은 형식일 수 있습니다.");
            } finally {
                setIsLoading(false);
            }
        };

        loadPdf();
    }, [url, pageCount]);

    if (error) {
        return (
            <div className="pdf-container">
                <div className="error-message">{error}</div>
                <style>{pdfStyles}</style>
            </div>
        );
    }

    if (isLoading) {
        return (
            <div className="pdf-container">
                <div className="loading-container">
                    <div className="spinner"></div>
                    <span className="blinking">로딩 중...</span>
                </div>
                <style>{pdfStyles}</style>
            </div>
        );
    }

    return (
        <div className="pdf-container" ref={containerRef}>
            {totalPages > 0 && (
                <div className="pdf-info">
                    총 {totalPages}페이지 중 {renderedPages.length}페이지 표시
                </div>
            )}
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <div className="pdf-pages">
                    {renderedPages.map((pageData) => (
                        <div key={pageData.pageNum} className="pdf-page">
                            <div className="page-number">페이지 {pageData.pageNum}</div>
                            <img
                                src={pageData.dataUrl}
                                alt={`페이지 ${pageData.pageNum}`}
                                style={{maxWidth: "100%", height: "auto"}}
                            />
                        </div>
                    ))}
                </div>
            </Suspense>
            <style>{pdfStyles}</style>
        </div>
    );
}

const pdfStyles = `
    .pdf-container {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        padding: 20px;
        text-align: center;
        width: 100%;
    }
    .pdf-info {
        margin-bottom: 15px;
        color: #666;
        font-size: 14px;
    }
    .pdf-pages {
        display: flex;
        flex-direction: column;
        gap: 20px;
        align-items: center;
    }
    .pdf-page {
        border: 1px solid #ddd;
        border-radius: 4px;
        padding: 10px;
        background: #f9f9f9;
        box-shadow: 0 2px 4px rgba(0,0,0,0.1);
    }
    .page-number {
        font-size: 12px;
        color: #888;
        margin-bottom: 8px;
    }
`;

ViewPDF.propTypes = {
    bookId: PropTypes.number.isRequired,
    pageCount: PropTypes.number,
};
