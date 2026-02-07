import {useEffect, useRef, useState, useCallback} from "react";
import {flushSync} from "react-dom";
import PropTypes from "prop-types";
import {getApiUrlPrefix} from "./Common";
import * as pdfjs from "pdfjs-dist";

// unpkg CDN에서 워커 로드 (package.json의 pdfjs-dist 버전과 일치)
pdfjs.GlobalWorkerOptions.workerSrc = "https://unpkg.com/pdfjs-dist@4.10.38/build/pdf.worker.min.mjs";

export default function ViewPDF({bookId, pageCount = 0, preview = false}) {
    const [error, setError] = useState(null);
    const [totalPages, setTotalPages] = useState(0);
    const [loadedPages, setLoadedPages] = useState(0);
    const [isFirstPageReady, setIsFirstPageReady] = useState(false);
    const containerRef = useRef(null);
    const pdfRef = useRef(null);
    const loadingTaskRef = useRef(null);
    const canvasRefs = useRef({});

    // 개별 페이지 렌더링 함수
    const renderPage = useCallback(async (pdf, pageNum) => {
        try {
            const page = await pdf.getPage(pageNum);
            const viewport = page.getViewport({scale: 1.2}); // scale 약간 낮춤

            const canvas = canvasRefs.current[pageNum];
            if (!canvas) return;

            const context = canvas.getContext("2d");
            canvas.width = viewport.width;
            canvas.height = viewport.height;

            await page.render({
                canvasContext: context,
                viewport: viewport,
            }).promise;

            setLoadedPages(prev => prev + 1);

            // 첫 페이지가 렌더링되면 즉시 표시
            if (pageNum === 1) {
                setIsFirstPageReady(true);
            }
        } catch (err) {
            console.error(`페이지 ${pageNum} 렌더링 실패:`, err);
        }
    }, []);

    useEffect(() => {
        if (!bookId) {
            setError("❌ 유효한 bookId가 제공되지 않았습니다.");
            return;
        }

        let cancelled = false;

        const loadPdf = async () => {
            setError(null);
            setTotalPages(0);
            setLoadedPages(0);
            setIsFirstPageReady(false);
            canvasRefs.current = {};

            try {
                const pdfUrl = preview
                    ? getApiUrlPrefix() + "/preview/" + bookId + "?pages=" + (pageCount > 0 ? pageCount : 5)
                    : getApiUrlPrefix() + "/download/" + bookId;
                const loadingTask = pdfjs.getDocument(pdfUrl);
                loadingTaskRef.current = loadingTask;
                const pdf = await loadingTask.promise;
                loadingTaskRef.current = null;

                if (cancelled) {
                    pdf.destroy();
                    return;
                }

                pdfRef.current = pdf;

                const pagesToRender = preview ? pdf.numPages : (pageCount > 0 ? Math.min(pdf.numPages, pageCount) : pdf.numPages);

                // flushSync로 상태 업데이트를 동기화하여 canvas가 DOM에 생성된 후 렌더링
                flushSync(() => {
                    setTotalPages(pagesToRender);
                });

                // 첫 페이지 우선 렌더링
                if (cancelled) return;
                await renderPage(pdf, 1);

                // 나머지 페이지 병렬 렌더링 (2페이지씩 배치 처리)
                const batchSize = 2;
                for (let i = 2; i <= pagesToRender; i += batchSize) {
                    if (cancelled) return;
                    const batch = [];
                    for (let j = i; j < i + batchSize && j <= pagesToRender; j++) {
                        batch.push(renderPage(pdf, j));
                    }
                    await Promise.all(batch);
                }
            } catch (err) {
                if (cancelled) return;
                console.error("PDF 로드 실패:", err);
                setError(`❌ PDF 렌더링 실패: ${err.message || "파일이 존재하지 않거나 올바르지 않은 형식일 수 있습니다."}`);
            }
        };

        loadPdf();

        return () => {
            cancelled = true;
            if (loadingTaskRef.current) {
                loadingTaskRef.current.destroy();
                loadingTaskRef.current = null;
            }
            if (pdfRef.current) {
                pdfRef.current.destroy();
                pdfRef.current = null;
            }
        };
    }, [bookId, pageCount, preview, renderPage]);

    // 캔버스 ref 설정 함수
    const setCanvasRef = useCallback((pageNum) => (el) => {
        if (el) {
            canvasRefs.current[pageNum] = el;
        }
    }, []);

    if (error) {
        return (
            <div className="pdf-container">
                <div className="error-message">{error}</div>
                <style>{pdfStyles}</style>
            </div>
        );
    }

    // 첫 페이지도 준비 안 됨 - 로딩 표시
    if (!isFirstPageReady && totalPages === 0) {
        return (
            <div className="pdf-container">
                <div className="loading-container">
                    <div className="spinner"></div>
                    <span className="blinking">PDF 로딩 중...</span>
                </div>
                <style>{pdfStyles}</style>
            </div>
        );
    }

    // 페이지 번호 배열 생성
    const pageNumbers = Array.from({length: totalPages}, (_, i) => i + 1);

    return (
        <div className="pdf-container" ref={containerRef}>
            <div className="pdf-info">
                {loadedPages < totalPages
                    ? `렌더링 중... ${loadedPages}/${totalPages}쪽`
                    : `총 ${totalPages}쪽 표시`
                }
            </div>
            <div className="pdf-pages">
                {pageNumbers.map((pageNum) => (
                    <div key={pageNum} className="pdf-page">
                        <div className="page-number">{pageNum}쪽</div>
                        <canvas
                            ref={setCanvasRef(pageNum)}
                            style={{maxWidth: "100%", height: "auto"}}
                        />
                    </div>
                ))}
            </div>
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
    .loading-container {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 10px;
    }
    .spinner {
        width: 40px;
        height: 40px;
        border: 4px solid #f3f3f3;
        border-top: 4px solid #3498db;
        border-radius: 50%;
        animation: spin 1s linear infinite;
    }
    @keyframes spin {
        0% { transform: rotate(0deg); }
        100% { transform: rotate(360deg); }
    }
    .blinking {
        animation: blink 1s linear infinite;
    }
    @keyframes blink {
        50% { opacity: 0.5; }
    }
`;

ViewPDF.propTypes = {
    bookId: PropTypes.number.isRequired,
    pageCount: PropTypes.number,
    preview: PropTypes.bool,
};
