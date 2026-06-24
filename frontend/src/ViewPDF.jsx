import { useEffect, useRef, useState, useCallback } from "react";
import { flushSync } from "react-dom";
import PropTypes from "prop-types";
import { getApiUrlPrefix } from "./Common";
import * as pdfjs from "pdfjs-dist";
// 워커를 설치된 pdfjs-dist에서 직접 번들 → API 버전과 항상 일치 (버전 drift 방지)
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

pdfjs.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

// JPEG2000(OpenJPEG)/JBIG2 등 이미지 디코딩용 wasm 위치.
// pdfjs 6.x 는 getDocument({ wasmUrl }) 로 wasm 디렉터리를 알려줘야 하며,
// 미지정 시 "null" + 파일명으로 로드에 실패해 이미지 기반 페이지가 렌더되지 않는다.
// vite pdf-wasm 플러그인이 설치된 pdfjs-dist 의 wasm 을 /pdf-wasm/ 로 서빙·번들한다.
const PDF_WASM_URL = `${window.location.origin}${import.meta.env.BASE_URL}pdf-wasm/`;

const CHUNK_SIZE = 10;
// 동시에 실행할 최대 렌더 개수. 워커 과부하를 막아 보이는 페이지에 자원을 집중시킨다.
const MAX_CONCURRENT_RENDERS = 3;

export default function ViewPDF({
  bookId,
  pageCount = 0,
  preview = false,
  apiPrefix = "",
}) {
  const [error, setError] = useState(null);
  const [totalPages, setTotalPages] = useState(0);
  const [loadedPages, setLoadedPages] = useState(0);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [isFirstPageReady, setIsFirstPageReady] = useState(false);
  const ZOOM_STEPS = [
    25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400,
    500,
  ];
  const [fitMode, setFitMode] = useState(!preview);
  const [zoomIndex, setZoomIndex] = useState(ZOOM_STEPS.indexOf(100));
  const zoomPercent = ZOOM_STEPS[zoomIndex];
  const zoomDown = () => setZoomIndex((prev) => Math.max(0, prev - 1));
  const zoomUp = () =>
    setZoomIndex((prev) => Math.min(ZOOM_STEPS.length - 1, prev + 1));
  const nativeWidthRef = useRef(0);
  const nativeHeightRef = useRef(0);
  const containerRef = useRef(null);
  const canvasRefs = useRef({});
  const renderedPagesRef = useRef(new Set());
  const observerRef = useRef(null);

  // 렌더 스케줄러 상태
  const renderQueueRef = useRef(new Set()); // 렌더 대기 중인 페이지 (아직 미실행)
  const activeRenderTasksRef = useRef(new Map()); // 실행 중인 페이지 → pdfjs RenderTask (슬롯 점유 = 동시성 카운트)

  // 청크 단위 로딩을 위한 ref
  const chunkDocsRef = useRef(new Map()); // key: "start-end", value: pdfDocument
  const fetchingRef = useRef(new Set()); // 현재 페칭 중인 범위 추적
  const cancelledRef = useRef(false);
  const totalPagesRef = useRef(0);

  // 청크 키에서 해당 페이지를 포함하는 청크의 pdfDoc과 로컬 페이지 번호를 반환
  const findChunkForPage = useCallback((globalPageNum) => {
    for (const [key, pdfDoc] of chunkDocsRef.current.entries()) {
      const [s, e] = key.split("-").map(Number);
      if (globalPageNum >= s && globalPageNum <= e) {
        return { pdfDoc, localPageNum: globalPageNum - s + 1 };
      }
    }
    return null;
  }, []);

  // 청크 페칭 함수
  const fetchChunk = useCallback(
    async (start, end) => {
      const key = `${start}-${end}`;
      if (chunkDocsRef.current.has(key) || fetchingRef.current.has(key)) return;
      fetchingRef.current.add(key);

      try {
        const url =
          getApiUrlPrefix() +
          apiPrefix +
          `/pdf-pages/${bookId}?start=${start}&end=${end}`;
        const response = await fetch(url, { credentials: "include" });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);

        const buffer = await response.arrayBuffer();
        if (cancelledRef.current) return;

        const pdfDoc = await pdfjs.getDocument({
          data: buffer,
          wasmUrl: PDF_WASM_URL,
        }).promise;
        if (cancelledRef.current) {
          pdfDoc.loadingTask.destroy();
          return;
        }

        chunkDocsRef.current.set(key, pdfDoc);

        // 청크 로드 완료 → 큐에 대기 중이던 페이지들을 다시 펌프 (렌더 가능해짐)
        pumpQueue();
      } catch (err) {
        if (!cancelledRef.current) {
          console.error(`청크 ${key} 페칭 실패:`, err);
        }
      } finally {
        fetchingRef.current.delete(key);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps -- apiPrefix/pumpQueue는 chunk fetch에 영향 없는 안정 참조라 의도적으로 제외
    [bookId],
  );

  // 글로벌 페이지 번호가 속하는 청크의 start를 계산
  const getChunkStart = useCallback((globalPageNum) => {
    // 첫 페이지는 항상 단독 청크
    if (globalPageNum === 1) return 1;
    // 2부터 시작해서 CHUNK_SIZE 단위
    return 2 + Math.floor((globalPageNum - 2) / CHUNK_SIZE) * CHUNK_SIZE;
  }, []);

  // 실제 렌더 실행. 스케줄러가 "청크 로드된" 페이지에 대해서만 호출한다. 취소 가능.
  const executeRender = useCallback(
    async (globalPageNum) => {
      if (renderedPagesRef.current.has(globalPageNum)) return;

      const chunkInfo = findChunkForPage(globalPageNum);
      if (!chunkInfo) {
        // 방어적 처리: 정상 흐름에선 발생하지 않지만, 청크가 아직 없으면 큐로 되돌린다
        renderQueueRef.current.add(globalPageNum);
        return;
      }

      // 렌더 시작 표시 (중복 실행 방지)
      renderedPagesRef.current.add(globalPageNum);

      try {
        const page = await chunkInfo.pdfDoc.getPage(chunkInfo.localPageNum);
        const dpr = window.devicePixelRatio || 1;
        const renderViewport = page.getViewport({ scale: 1.2 * dpr });

        const canvas = canvasRefs.current[globalPageNum];
        if (!canvas) {
          renderedPagesRef.current.delete(globalPageNum);
          return;
        }

        const context = canvas.getContext("2d");
        canvas.width = renderViewport.width;
        canvas.height = renderViewport.height;

        const task = page.render({
          canvasContext: context,
          viewport: renderViewport,
        });
        // 취소 가능하도록 슬롯 마커를 실제 RenderTask로 교체
        activeRenderTasksRef.current.set(globalPageNum, task);
        await task.promise;

        setLoadedPages((prev) => prev + 1);

        if (globalPageNum === 1) {
          setIsFirstPageReady(true);
        }
      } catch (err) {
        // 취소된 렌더는 재진입 시 다시 그릴 수 있도록 완료 표시를 해제한다
        renderedPagesRef.current.delete(globalPageNum);
        if (err?.name !== "RenderingCancelledException") {
          console.error(`페이지 ${globalPageNum} 렌더링 실패:`, err);
        }
      }
    },
    [findChunkForPage],
  );

  // 큐에서 청크가 로드되어 즉시 렌더 가능한 페이지 중 뷰포트에 가장 가까운 것을 고른다
  const pickRenderablePage = useCallback(() => {
    const container = containerRef.current;
    const center = container
      ? container.scrollTop + container.clientHeight / 2
      : 0;
    let best = null;
    let bestDist = Infinity;
    for (const pageNum of renderQueueRef.current) {
      if (!findChunkForPage(pageNum)) continue; // 청크 미로드 → 아직 렌더 불가
      const canvas = canvasRefs.current[pageNum];
      const top = canvas ? canvas.offsetTop : 0;
      const height = canvas ? canvas.offsetHeight : 0;
      const dist = Math.abs(top + height / 2 - center);
      if (dist < bestDist) {
        bestDist = dist;
        best = pageNum;
      }
    }
    return best;
  }, [findChunkForPage]);

  // 스케줄러 펌프: (1) 큐의 미로드 페이지에 대한 청크 페칭을 트리거하고,
  // (2) 동시성 한도 내에서 렌더 가능한 페이지를 우선순위 순으로 실행한다.
  // 페이지는 실제로 렌더 실행될 때까지 큐에 남으므로, 청크 로드 후 재호출되면
  // 누락 없이 재시도된다 (one-shot drain이 없어 race가 발생하지 않는다).
  const pumpQueue = useCallback(() => {
    // 큐에 있는 미로드 페이지들의 청크 페칭 트리거 (fetchChunk가 중복 제거)
    for (const pageNum of renderQueueRef.current) {
      if (findChunkForPage(pageNum)) continue;
      const chunkStart = getChunkStart(pageNum);
      const chunkEnd = Math.min(
        chunkStart === 1 ? 1 : chunkStart + CHUNK_SIZE - 1,
        totalPagesRef.current,
      );
      fetchChunk(chunkStart, chunkEnd);
    }
    // 렌더 가능한 페이지부터 실행
    while (activeRenderTasksRef.current.size < MAX_CONCURRENT_RENDERS) {
      const pageNum = pickRenderablePage();
      if (pageNum == null) break;
      renderQueueRef.current.delete(pageNum);
      // 슬롯 예약: RenderTask 생성 전까지 null 마커로 동시성 카운트를 확보
      activeRenderTasksRef.current.set(pageNum, null);
      executeRender(pageNum).finally(() => {
        activeRenderTasksRef.current.delete(pageNum);
        if (!cancelledRef.current) pumpQueue();
      });
    }
  }, [
    pickRenderablePage,
    findChunkForPage,
    getChunkStart,
    fetchChunk,
    executeRender,
  ]);

  // 페이지를 렌더 큐에 넣는다 (이미 렌더됐거나 실행 중이면 무시)
  const enqueueRender = useCallback(
    (pageNum) => {
      if (renderedPagesRef.current.has(pageNum)) return;
      if (activeRenderTasksRef.current.has(pageNum)) return;
      renderQueueRef.current.add(pageNum);
      pumpQueue();
    },
    [pumpQueue],
  );

  // 화면 밖으로 나간 페이지의 렌더를 취소/대기 해제 (보이는 페이지에 자원 양보)
  const cancelRender = useCallback((pageNum) => {
    renderQueueRef.current.delete(pageNum);
    const task = activeRenderTasksRef.current.get(pageNum);
    // null 마커(예약 상태)는 취소 대상이 아님
    if (task && typeof task.cancel === "function") task.cancel();
  }, []);

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
      renderQueueRef.current = new Set();
      activeRenderTasksRef.current = new Map();
      chunkDocsRef.current = new Map();
      fetchingRef.current = new Set();
      totalPagesRef.current = 0;

      try {
        // 1단계: 첫 페이지만 페칭하여 총 페이지 수 확인 및 빠른 렌더링
        setDownloadProgress(10);
        const firstUrl =
          getApiUrlPrefix() + apiPrefix + `/pdf-pages/${bookId}?start=1&end=1`;
        const firstResponse = await fetch(firstUrl, { credentials: "include" });
        if (!firstResponse.ok) throw new Error(`HTTP ${firstResponse.status}`);

        const serverTotalPages = parseInt(
          firstResponse.headers.get("X-Total-Pages") || "0",
          10,
        );
        const firstBuffer = await firstResponse.arrayBuffer();
        if (cancelledRef.current) return;

        setDownloadProgress(30);

        const firstPdfDoc = await pdfjs.getDocument({
          data: firstBuffer,
          wasmUrl: PDF_WASM_URL,
        }).promise;
        if (cancelledRef.current) {
          firstPdfDoc.loadingTask.destroy();
          return;
        }

        chunkDocsRef.current.set("1-1", firstPdfDoc);

        // 총 페이지 수 결정
        let pagesToRender;
        if (preview) {
          pagesToRender = Math.min(10, serverTotalPages);
        } else {
          pagesToRender =
            pageCount > 0
              ? Math.min(serverTotalPages, pageCount)
              : serverTotalPages;
        }
        totalPagesRef.current = pagesToRender;

        // 첫 페이지 viewport로 모든 canvas placeholder 크기 결정
        const firstPage = await firstPdfDoc.getPage(1);
        const dpr = window.devicePixelRatio || 1;
        const firstCssViewport = firstPage.getViewport({ scale: 1.2 });
        const firstRenderViewport = firstPage.getViewport({ scale: 1.2 * dpr });
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

        // 첫 페이지 렌더링 (스케줄러 큐에 최우선 투입)
        if (cancelledRef.current) return;
        enqueueRender(1);
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

        // IntersectionObserver: 근접 영역(near-zone) 진입 시 큐에 투입, 이탈 시 취소.
        // 아래쪽 margin을 크게 둬 순방향 스크롤에서 다음 페이지들을 미리 준비한다.
        // (선렌더 루프 제거: rootMargin 자체가 prefetch 역할을 하고, 그 페이지들은
        //  여전히 intersecting 상태라 즉시 취소되지 않는다.)
        const observer = new IntersectionObserver(
          (entries) => {
            for (const entry of entries) {
              const pageNum = parseInt(entry.target.dataset.page);
              if (!pageNum) continue;
              if (entry.isIntersecting) {
                enqueueRender(pageNum);
              } else if (
                renderQueueRef.current.has(pageNum) ||
                activeRenderTasksRef.current.has(pageNum)
              ) {
                // 근접 영역을 벗어난 미완 렌더는 취소해 보이는 페이지에 자원 양보
                cancelRender(pageNum);
              }
            }
          },
          { rootMargin: "300px 0px 1200px 0px" },
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
        setError(
          `PDF 렌더링 실패: ${err.message || "파일이 존재하지 않거나 올바르지 않은 형식일 수 있습니다."}`,
        );
      }
    };

    loadPdf();

    return () => {
      cancelledRef.current = true;
      if (observerRef.current) {
        observerRef.current.disconnect();
        observerRef.current = null;
      }
      // 진행 중인 렌더 취소 + 큐 비우기
      for (const task of activeRenderTasksRef.current.values()) {
        if (task && typeof task.cancel === "function") task.cancel();
      }
      activeRenderTasksRef.current.clear();
      renderQueueRef.current.clear();
      // 모든 청크 pdfDoc 정리 (pdfjs 6.x: PDFDocumentProxy 대신 loadingTask로 정리)
      for (const pdfDoc of chunkDocsRef.current.values()) {
        pdfDoc.loadingTask.destroy();
      }
      chunkDocsRef.current.clear();
      fetchingRef.current.clear();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- apiPrefix는 mount 시 고정이라 의도적으로 deps에서 제외
  }, [bookId, pageCount, preview, enqueueRender, cancelRender, fetchChunk]);

  // 캔버스 ref 설정 함수
  const setCanvasRef = useCallback(
    (pageNum) => (el) => {
      if (el) {
        canvasRefs.current[pageNum] = el;
      }
    },
    [],
  );

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
              : "PDF 로딩 중..."}
          </span>
          {downloadProgress > 0 && (
            <div className="progress-bar-container">
              <div
                className="progress-bar-fill"
                style={{ width: `${downloadProgress}%` }}
              />
            </div>
          )}
        </div>
        <style>{pdfStyles}</style>
      </div>
    );
  }

  // 페이지 번호 배열 생성
  const pageNumbers = Array.from({ length: totalPages }, (_, i) => i + 1);

  return (
    <div className="pdf-container" ref={containerRef}>
      <div className="pdf-toolbar">
        <div className="pdf-info">
          {loadedPages < totalPages
            ? `렌더링 중... ${loadedPages}/${totalPages}쪽`
            : `총 ${totalPages}쪽 표시`}
        </div>
        <div className="pdf-zoom-controls">
          <button
            className={`pdf-zoom-btn pdf-zoom-fit-btn ${fitMode ? "active" : ""}`}
            onClick={() => setFitMode((prev) => !prev)}
          >
            <span className="toggle-track">
              <span className="toggle-knob" />
            </span>
            맞춤
          </button>
          <span className="pdf-zoom-separator" />
          <button
            className="pdf-zoom-btn"
            onClick={zoomDown}
            disabled={fitMode || zoomIndex === 0}
          >
            −
          </button>
          <span className="pdf-zoom-label">
            {fitMode ? "-" : `${zoomPercent}%`}
          </span>
          <button
            className="pdf-zoom-btn"
            onClick={zoomUp}
            disabled={fitMode || zoomIndex === ZOOM_STEPS.length - 1}
          >
            +
          </button>
        </div>
      </div>
      <div className="pdf-content">
        {pageNumbers.map((pageNum) => (
          <canvas
            key={pageNum}
            ref={setCanvasRef(pageNum)}
            className="pdf-page"
            style={
              fitMode
                ? { width: "100%", height: "auto" }
                : {
                    width: `${Math.round((nativeWidthRef.current * zoomPercent) / 100)}px`,
                    height: "auto",
                  }
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
  apiPrefix: PropTypes.string,
};
