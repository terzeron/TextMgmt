import {useEffect, useRef, useState, useCallback} from "react";
import {flushSync} from "react-dom";
import PropTypes from "prop-types";
import {getApiUrlPrefix} from "./Common";
import * as pdfjs from "pdfjs-dist";

// unpkg CDN에서 워커 로드 (package.json의 pdfjs-dist 버전과 일치)
pdfjs.GlobalWorkerOptions.workerSrc = "https://unpkg.com/pdfjs-dist@4.10.38/build/pdf.worker.min.mjs";

const CHUNK_SIZE = 10;

export default function ViewPDF({bookId, pageCount = 0, preview = false}) {
    const [error, setError] = useState(null);
    const [totalPages, setTotalPages] = useState(0);
    const [loadedPages, setLoadedPages] = useState(0);
    const [downloadProgress, setDownloadProgress] = useState(0);
    const [isFirstPageReady, setIsFirstPageReady] = useState(false);
    const ZOOM_STEPS = [25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500];
    const [fitMode, setFitMode] = useState(!preview);
    const [zoomIndex, setZoomIndex] = useState(ZOOM_STEPS.indexOf(100));
    const zoomPercent = ZOOM_STEPS[zoomIndex];
    const zoomDown = () => setZoomIndex(prev => Math.max(0, prev - 1));
    const zoomUp = () => setZoomIndex(prev => Math.min(ZOOM_STEPS.length - 1, prev + 1));
    const nativeWidthRef = useRef(0);
    const nativeHeightRef = useRef(0);
    const containerRef = useRef(null);
    const canvasRefs = useRef({});
    const renderedPagesRef = useRef(new Set());
    const observerRef = useRef(null);

    // 청크 단위 로딩을 위한 ref
    const chunkDocsRef = useRef(new Map());   // key: "start-end", value: pdfDocument
    const fetchingRef = useRef(new Set());     // 현재 페칭 중인 범위 추적
    const pendingPagesRef = useRef(new Set()); // 청크 로드 대기 중인 페이지
    const cancelledRef = useRef(false);
    const totalPagesRef = useRef(0);

    // 청크 키에서 해당 페이지를 포함하는 청크의 pdfDoc과 로컬 페이지 번호를 반환
    const findChunkForPage = useCallback((globalPageNum) => {
        for (const [key, pdfDoc] of chunkDocsRef.current.entries()) {
            const [s, e] = key.split("-").map(Number);
            if (globalPageNum >= s && globalPageNum <= e) {
                return {pdfDoc, localPageNum: globalPageNum - s + 1};
            }
        }
        return null;
    }, []);

    // 청크 페칭 함수
    const fetchChunk = useCallback(async (start, end) => {
        const key = `${start}-${end}`;
        if (chunkDocsRef.current.has(key) || fetchingRef.current.has(key)) return;
        fetchingRef.current.add(key);

        try {
            const url = getApiUrlPrefix() + `/pdf-pages/${bookId}?start=${start}&end=${end}`;
            const response = await fetch(url);
            if (!response.ok) throw new Error(`HTTP ${response.status}`);

            const buffer = await response.arrayBuffer();
            if (cancelledRef.current) return;

            const pdfDoc = await pdfjs.getDocument({data: buffer}).promise;
            if (cancelledRef.current) {
                pdfDoc.destroy();
                return;
            }

            chunkDocsRef.current.set(key, pdfDoc);

            // 이 청크에 해당하는 대기 중인 페이지 렌더링
            for (let i = start; i <= end; i++) {
                if (pendingPagesRef.current.has(i)) {
                    pendingPagesRef.current.delete(i);
                    renderPage(i);
                }
            }
        } catch (err) {
            if (!cancelledRef.current) {
                console.error(`청크 ${key} 페칭 실패:`, err);
            }
        } finally {
            fetchingRef.current.delete(key);
        }
    }, [bookId]);

    // 글로벌 페이지 번호가 속하는 청크의 start를 계산
    const getChunkStart = useCallback((globalPageNum) => {
        // 첫 페이지는 항상 단독 청크
        if (globalPageNum === 1) return 1;
        // 2부터 시작해서 CHUNK_SIZE 단위
        return 2 + Math.floor((globalPageNum - 2) / CHUNK_SIZE) * CHUNK_SIZE;
    }, []);

    // 페이지 렌더링 함수 (청크 기반)
    const renderPage = useCallback(async (globalPageNum) => {
        if (renderedPagesRef.current.has(globalPageNum)) return;

        const chunkInfo = findChunkForPage(globalPageNum);
        if (!chunkInfo) {
            // 청크가 아직 로드되지 않음 → 대기 목록에 추가하고 청크 페칭 트리거
            pendingPagesRef.current.add(globalPageNum);
            const chunkStart = getChunkStart(globalPageNum);
            const chunkEnd = Math.min(chunkStart === 1 ? 1 : chunkStart + CHUNK_SIZE - 1, totalPagesRef.current);
            fetchChunk(chunkStart, chunkEnd);
            return;
        }

        renderedPagesRef.current.add(globalPageNum);

        try {
            const page = await chunkInfo.pdfDoc.getPage(chunkInfo.localPageNum);
            const dpr = window.devicePixelRatio || 1;
            const renderViewport = page.getViewport({scale: 1.2 * dpr});

            const canvas = canvasRefs.current[globalPageNum];
            if (!canvas) {
                renderedPagesRef.current.delete(globalPageNum);
                return;
            }

            const context = canvas.getContext("2d");
            canvas.width = renderViewport.width;
            canvas.height = renderViewport.height;

            await page.render({
                canvasContext: context,
                viewport: renderViewport,
            }).promise;

            setLoadedPages(prev => prev + 1);

            if (globalPageNum === 1) {
                setIsFirstPageReady(true);
            }
        } catch (err) {
            renderedPagesRef.current.delete(globalPageNum);
            console.error(`페이지 ${globalPageNum} 렌더링 실패:`, err);
        }
    }, [findChunkForPage, getChunkStart, fetchChunk]);

    useEffect(() => {
        if (!bookId) {
            setError("유효한 bookId가 제공되지 않았습니다.");
            return;
        }

        cancelledRef.current = false;

        const loadPdf = async () => {
            setError(null);
            setTotalPages(0);
            setLoadedPages(0);
            setDownloadProgress(0);
            setIsFirstPageReady(false);
            canvasRefs.current = {};
            renderedPagesRef.current = new Set();
            chunkDocsRef.current = new Map();
            fetchingRef.current = new Set();
            pendingPagesRef.current = new Set();
            totalPagesRef.current = 0;

            try {
                // 1단계: 첫 페이지만 페칭하여 총 페이지 수 확인 및 빠른 렌더링
                setDownloadProgress(10);
                const firstUrl = getApiUrlPrefix() + `/pdf-pages/${bookId}?start=1&end=1`;
                const firstResponse = await fetch(firstUrl);
                if (!firstResponse.ok) throw new Error(`HTTP ${firstResponse.status}`);

                const serverTotalPages = parseInt(firstResponse.headers.get("X-Total-Pages") || "0", 10);
                const firstBuffer = await firstResponse.arrayBuffer();
                if (cancelledRef.current) return;

                setDownloadProgress(30);

                const firstPdfDoc = await pdfjs.getDocument({data: firstBuffer}).promise;
                if (cancelledRef.current) {
                    firstPdfDoc.destroy();
                    return;
                }

                chunkDocsRef.current.set("1-1", firstPdfDoc);

                // 총 페이지 수 결정
                let pagesToRender;
                if (preview) {
                    pagesToRender = Math.min(10, serverTotalPages);
                } else {
                    pagesToRender = pageCount > 0 ? Math.min(serverTotalPages, pageCount) : serverTotalPages;
                }
                totalPagesRef.current = pagesToRender;

                // 첫 페이지 viewport로 모든 canvas placeholder 크기 결정
                const firstPage = await firstPdfDoc.getPage(1);
                const dpr = window.devicePixelRatio || 1;
                const firstCssViewport = firstPage.getViewport({scale: 1.2});
                const firstRenderViewport = firstPage.getViewport({scale: 1.2 * dpr});
                nativeWidthRef.current = firstCssViewport.width;
                nativeHeightRef.current = firstCssViewport.height;

                if (cancelledRef.current) return;

                // flushSync로 상태 업데이트를 동기화하여 canvas가 DOM에 생성된 후 렌더링
                flushSync(() => {
                    setTotalPages(pagesToRender);
                });

                // 모든 canvas에 추정 크기 설정 (placeholder로 레이아웃 확보)
                for (let i = 1; i <= pagesToRender; i++) {
                    const canvas = canvasRefs.current[i];
                    if (canvas) {
                        canvas.width = firstRenderViewport.width;
                        canvas.height = firstRenderViewport.height;
                    }
                }

                // 첫 페이지 렌더링
                if (cancelledRef.current) return;
                await renderPage(1);
                setDownloadProgress(50);

                if (cancelledRef.current || pagesToRender <= 1) return;

                // 2단계: 나머지 페이지 로딩
                if (preview) {
                    // 미리보기: 나머지 2~pagesToRender 한 번에 페칭
                    fetchChunk(2, pagesToRender);
                } else {
                    // 전체보기: 2~11페이지 즉시 페칭
                    const firstBatchEnd = Math.min(1 + CHUNK_SIZE, pagesToRender);
                    fetchChunk(2, firstBatchEnd);
                }

                setDownloadProgress(100);

                // IntersectionObserver로 스크롤 기반 청크 페칭 + 렌더링
                const observer = new IntersectionObserver(
                    (entries) => {
                        for (const entry of entries) {
                            if (entry.isIntersecting) {
                                const pageNum = parseInt(entry.target.dataset.page);
                                if (!pageNum) continue;

                                // 현재 페이지 렌더링
                                if (!renderedPagesRef.current.has(pageNum)) {
                                    renderPage(pageNum);
                                }

                                // 선렌더링: 현재 페이지 이후 CHUNK_SIZE 페이지
                                const prerenderEnd = Math.min(pageNum + CHUNK_SIZE, pagesToRender);
                                for (let i = pageNum + 1; i <= prerenderEnd; i++) {
                                    if (!renderedPagesRef.current.has(i)) {
                                        renderPage(i);
                                    }
                                }

                                const c = canvasRefs.current[pageNum];
                                if (c) observer.unobserve(c);
                            }
                        }
                    },
                    {rootMargin: '500px 0px'}
                );

                observerRef.current = observer;

                for (let i = 2; i <= pagesToRender; i++) {
                    const canvas = canvasRefs.current[i];
                    if (canvas) {
                        canvas.dataset.page = String(i);
                        observer.observe(canvas);
                    }
                }
            } catch (err) {
                if (cancelledRef.current) return;
                console.error("PDF 로드 실패:", err);
                setError(`PDF 렌더링 실패: ${err.message || "파일이 존재하지 않거나 올바르지 않은 형식일 수 있습니다."}`);
            }
        };

        loadPdf();

        return () => {
            cancelledRef.current = true;
            if (observerRef.current) {
                observerRef.current.disconnect();
                observerRef.current = null;
            }
            // 모든 청크 pdfDoc 정리
            for (const pdfDoc of chunkDocsRef.current.values()) {
                pdfDoc.destroy();
            }
            chunkDocsRef.current.clear();
            fetchingRef.current.clear();
            pendingPagesRef.current.clear();
        };
    }, [bookId, pageCount, preview, renderPage, fetchChunk]);

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

    // 첫 페이지도 준비 안 됨 - 로딩 표시 (다운로드 진행률 포함)
    if (!isFirstPageReady && totalPages === 0) {
        return (
            <div className="pdf-container">
                <div className="loading-container">
                    <div className="spinner"></div>
                    <span className="blinking">
                        {downloadProgress > 0
                            ? `PDF 다운로드 중... ${downloadProgress}%`
                            : 'PDF 로딩 중...'}
                    </span>
                    {downloadProgress > 0 && (
                        <div className="progress-bar-container">
                            <div className="progress-bar-fill" style={{width: `${downloadProgress}%`}} />
                        </div>
                    )}
                </div>
                <style>{pdfStyles}</style>
            </div>
        );
    }

    // 페이지 번호 배열 생성
    const pageNumbers = Array.from({length: totalPages}, (_, i) => i + 1);

    return (
        <div className="pdf-container" ref={containerRef}>
            <div className="pdf-toolbar">
                <div className="pdf-info">
                    {loadedPages < totalPages
                        ? `렌더링 중... ${loadedPages}/${totalPages}쪽`
                        : `총 ${totalPages}쪽 표시`
                    }
                </div>
                <div className="pdf-zoom-controls">
                    <button className={`pdf-zoom-btn pdf-zoom-fit-btn ${fitMode ? 'active' : ''}`} onClick={() => setFitMode(prev => !prev)}>
                        <span className="toggle-track"><span className="toggle-knob" /></span>
                        맞춤
                    </button>
                    <span className="pdf-zoom-separator" />
                    <button className="pdf-zoom-btn" onClick={zoomDown} disabled={fitMode || zoomIndex === 0}>−</button>
                    <span className="pdf-zoom-label">{fitMode ? '-' : `${zoomPercent}%`}</span>
                    <button className="pdf-zoom-btn" onClick={zoomUp} disabled={fitMode || zoomIndex === ZOOM_STEPS.length - 1}>+</button>
                </div>
            </div>
            <div className="pdf-content">
                {pageNumbers.map((pageNum) => (
                    <canvas
                        key={pageNum}
                        ref={setCanvasRef(pageNum)}
                        className="pdf-page"
                        style={fitMode
                            ? {width: '100%', height: 'auto'}
                            : {width: `${Math.round(nativeWidthRef.current * zoomPercent / 100)}px`,
                               height: `${Math.round(nativeHeightRef.current * zoomPercent / 100)}px`}
                        }
                    />
                ))}
            </div>
            <style>{pdfStyles}</style>
        </div>
    );
}

const pdfStyles = `
    .pdf-container {
        width: 100%;
        height: calc(100vh - 10px);
        overflow-y: auto;
    }
    .pdf-toolbar {
        position: sticky;
        top: 0;
        z-index: 10;
        display: flex;
        justify-content: space-between;
        align-items: center;
        padding: 4px 8px;
        background: rgba(255,255,255,0.95);
    }
    .pdf-info {
        color: #666;
        font-size: 14px;
    }
    .pdf-zoom-controls {
        display: flex;
        align-items: center;
        gap: 2px;
    }
    .pdf-zoom-btn {
        padding: 2px 8px;
        font-size: 12px;
        border: 1px solid #ccc;
        border-radius: 4px;
        background: rgba(255,255,255,0.9);
        cursor: pointer;
        color: #555;
        line-height: 1.4;
    }
    .pdf-zoom-btn:hover:not(:disabled) {
        background: #e0e0e0;
    }
    .pdf-zoom-fit-btn {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 2px 8px 2px 4px;
    }
    .pdf-zoom-fit-btn .toggle-track {
        position: relative;
        width: 28px;
        height: 16px;
        border-radius: 8px;
        background: #ccc;
        flex-shrink: 0;
        transition: background 0.2s;
    }
    .pdf-zoom-fit-btn .toggle-knob {
        position: absolute;
        left: 2px;
        top: 2px;
        width: 12px;
        height: 12px;
        border-radius: 50%;
        background: #fff;
        transition: left 0.2s;
    }
    .pdf-zoom-fit-btn.active .toggle-track {
        background: #4a90d9;
    }
    .pdf-zoom-fit-btn.active .toggle-knob {
        left: 14px;
    }
    .pdf-zoom-separator {
        width: 1px;
        height: 16px;
        background: #ccc;
        margin: 0 6px;
    }
    .pdf-zoom-btn:disabled {
        opacity: 0.3;
        cursor: default;
    }
    .pdf-zoom-label {
        font-size: 11px;
        color: #555;
        min-width: 36px;
        text-align: center;
    }
    .pdf-content {
        overflow-x: auto;
    }
    .pdf-page {
        display: block;
        margin: 0 auto;
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
    .progress-bar-container {
        width: 200px;
        height: 4px;
        background: #e0e0e0;
        border-radius: 2px;
        overflow: hidden;
    }
    .progress-bar-fill {
        height: 100%;
        background: #3498db;
        border-radius: 2px;
        transition: width 0.3s ease;
    }
`;

ViewPDF.propTypes = {
    bookId: PropTypes.number.isRequired,
    pageCount: PropTypes.number,
    preview: PropTypes.bool,
};
