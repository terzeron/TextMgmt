import {
  Fragment,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useCallback,
} from "react";
import { flushSync } from "react-dom";
import PropTypes from "prop-types";
import DeleteOutlineIcon from "@mui/icons-material/DeleteOutlined";
import SaveIcon from "@mui/icons-material/Save";
import CropFreeIcon from "@mui/icons-material/CropFree";
import VerticalSplitOutlinedIcon from "@mui/icons-material/VerticalSplitOutlined";
import AddPhotoAlternateIcon from "@mui/icons-material/AddPhotoAlternate";
import ArrowDropUpIcon from "@mui/icons-material/ArrowDropUp";
import ArrowDropDownIcon from "@mui/icons-material/ArrowDropDown";
import PdfInsertImageModal from "./PdfInsertImageModal";
import { describeHttpError } from "./httpError";
import { blobToBase64, requestInsertPreview } from "./insertPreview";
import "./viewerEditActions.css";
import PdfCropBox from "./PdfCropBox";
import { DEFAULT_CROP_RECT, DEFAULT_SPLIT_RECTS } from "./cropRect";

// 크롭 박스 색. 분할에서는 파랑이 앞쪽 페이지, 빨강이 뒤쪽 페이지가 된다.
const CROP_COLOR = "#4a90d9";
const SPLIT_COLORS = { first: "#4a90d9", second: "#d9534f" };
import { getApiUrlPrefix } from "./Common";
import {
  DISPLAY_SCALE,
  computeRenderScale,
  getFullPageImageSize,
} from "./pdfRenderScale";
import { pdfjs, PDF_DOCUMENT_OPTIONS } from "./pdfjsEnv";

// 서버는 실패 사유를 본문에 담아 보낸다(파일 없음, 스토리지 접근 오류 등).
// 쪽 크기(pt)가 비정상적으로 작은 PDF가 있다. 이미지를 0.09pt 폭으로 그려 넣은 만화 PDF가 그렇다.
// 첫 쪽 크기로 표시 폭과 자리표시 canvas 를 정하면 폭 0px, canvas 0x0 이 되어 아무것도 보이지 않는다.
// 첫 쪽의 CSS 폭이 이 값 미만이면 쪽 크기를 믿지 않고 기준 폭으로 대신한다(비율은 유지).
const MIN_PLAUSIBLE_CSS_WIDTH = 20;
const FALLBACK_NATIVE_CSS_WIDTH = 900;
// 이 경우 자리표시 canvas 는 작게 둔다. 실제 크기는 렌더할 때 화면 폭에 맞춰 정해진다.
const FALLBACK_PLACEHOLDER_PIXEL_WIDTH = 90;

const CHUNK_SIZE = 10;
// 동시에 실행할 최대 렌더 개수. 워커 과부하를 막아 보이는 페이지에 자원을 집중시킨다.
const MAX_CONCURRENT_RENDERS = 3;
// 보이는 영역(near zone) 밖에서 그려진 채로 남겨 두는 페이지 수. 가까운 순으로 이만큼만 남기고
// 나머지는 canvas 비트맵과 청크 pdfDoc 을 해제해 긴 PDF 를 넘겨도 메모리가 쌓이지 않게 한다.
const RETAIN_PAGES_OUTSIDE_VIEW = 6;

export default function ViewPDF({
  bookId,
  pageCount = 0,
  preview = false,
  apiPrefix = "",
  editable = false,
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
  // 편집 화면 전용: 쪽(1-based) → { kind: "page" }(쪽 전체 삭제) | { kind: "crop", rect }(남길 영역)
  // | { kind: "split", rects: { first, second } }(앞·뒤 두 쪽으로 분할) 표시와 저장 상태.
  // 저장하기 전까지 서버는 건드리지 않는다. 한 쪽에는 한 종류만 표시한다.
  const [markedPages, setMarkedPages] = useState(() => new Map());
  // 쪽(1-based) → { url, previewSrc }: 그 쪽 앞에 끼울 이미지의 URL 과, 서버가 만들어 준 미리보기(blob URL).
  // 삭제·크롭·분할 표시와는 따로 둔다(같은 쪽에 함께 있을 수 있다). blob URL 은 쓰지 않게 되면 해제한다.
  const [insertions, setInsertions] = useState(() => new Map());
  const insertionsRef = useRef(insertions);
  useEffect(() => {
    insertionsRef.current = insertions;
  }, [insertions]);
  useEffect(
    () => () => {
      for (const entry of insertionsRef.current.values()) {
        URL.revokeObjectURL(entry.previewSrc);
      }
    },
    [],
  );
  // 이미지 URL 입력 모달이 열려 있는 쪽 번호(없으면 null)
  const [insertDialog, setInsertDialog] = useState(null);
  // 쪽 순서 바꾸기: 화면에 보이는 순서대로 원본 쪽 번호를 나열한 배열(null 이면 순서를 바꾸지 않았다).
  // 표시와 삽입이 원본 쪽 번호를 키로 쓰므로 번호는 그대로 두고 보이는 순서만 바꾸며, 저장할 때 order 로 보낸다.
  const [pageOrder, setPageOrder] = useState(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState(null);
  // 저장 뒤 같은 bookId로 PDF를 다시 불러오기 위한 카운터
  const [reloadToken, setReloadToken] = useState(0);
  const [fitMode, setFitMode] = useState(!preview);
  // 1장씩 보기: CSS scroll-snap으로 뷰포트당 1쪽만 보이게 한다 (PC·모바일 공통, 기본 꺼짐)
  const [singlePage, setSinglePage] = useState(false);
  const [zoomIndex, setZoomIndex] = useState(ZOOM_STEPS.indexOf(100));
  const zoomPercent = ZOOM_STEPS[zoomIndex];
  const toggleSinglePage = () => {
    setSinglePage((prev) => !prev);
    // 1장씩 보기는 이미지를 letterbox 로 맞춰 박스 좌표와 화면이 어긋나므로 크롭·분할 표시는 버린다.
    setMarkedPages((prev) => {
      if (![...prev.values()].some((mark) => mark.kind !== "page")) return prev;
      return new Map([...prev].filter(([, mark]) => mark.kind === "page"));
    });
    // 버튼에 포커스가 남으면 Space가 토글을 다시 눌러버리므로 스크롤 컨테이너로 옮긴다.
    // 컨테이너가 포커스를 가져야 Space/PageDown이 이 컨테이너를 스크롤한다.
    containerRef.current?.focus({ preventScroll: true });
  };
  const toggleMarkedPage = (pageNum, kind) => {
    setSaveError(null);
    setMarkedPages((prev) => {
      const next = new Map(prev);
      if (next.get(pageNum)?.kind === kind) next.delete(pageNum);
      else if (kind === "crop")
        next.set(pageNum, { kind, rect: { ...DEFAULT_CROP_RECT } });
      else if (kind === "split")
        next.set(pageNum, {
          kind,
          rects: {
            first: { ...DEFAULT_SPLIT_RECTS.first },
            second: { ...DEFAULT_SPLIT_RECTS.second },
          },
        });
      else next.set(pageNum, { kind });
      return next;
    });
  };
  // pageNum 쪽을 화면에서 delta(-1: 위, +1: 아래)칸 옮겨 이웃한 쪽과 맞바꾼다. 원래 순서로 돌아오면 변경 표시를 지운다.
  const movePage = (pageNum, delta) => {
    setSaveError(null);
    setPageOrder((prev) => {
      const order = prev ? [...prev] : Array.from({ length: totalPages }, (_, i) => i + 1);
      const from = order.indexOf(pageNum);
      const to = from + delta;
      if (from < 0 || to < 0 || to >= order.length) return prev;
      [order[from], order[to]] = [order[to], order[from]];
      return order.every((n, i) => n === i + 1) ? null : order;
    });
  };
  const changeBox = (pageNum, id, rect) =>
    setMarkedPages((prev) => {
      const mark = prev.get(pageNum);
      if (!mark) return prev;
      const next =
        mark.kind === "split"
          ? { kind: "split", rects: { ...mark.rects, [id]: rect } }
          : { kind: "crop", rect };
      return new Map(prev).set(pageNum, next);
    });
  const setInsertion = (pageNum, entry) => {
    setSaveError(null);
    const previous = insertionsRef.current.get(pageNum);
    if (previous && previous.previewSrc !== entry?.previewSrc) {
      URL.revokeObjectURL(previous.previewSrc);
    }
    setInsertions((prev) => {
      const next = new Map(prev);
      if (entry) next.set(pageNum, entry);
      else next.delete(pageNum);
      return next;
    });
  };
  const clearInsertions = () => {
    for (const entry of insertionsRef.current.values()) {
      URL.revokeObjectURL(entry.previewSrc);
    }
    setInsertions(new Map());
  };
  // 서버가 이미지(URL 이면 내려받고, 이 기기의 파일이면 받아서)를 정규화한 미리보기를 받아 온다. 저장할 때 삽입되는 것과 같은 이미지다.
  // 실패 사유(문자열)를 돌려주고, 성공하면 삽입을 표시한 뒤 null 을 돌려준다.
  const previewInsertion = async (pageNum, source) => {
    try {
      const blob = await requestInsertPreview(apiPrefix, bookId, source);
      /* v8 ignore next -- the viewer unmounts only while this request is in flight. */
      if (cancelledRef.current) return null;
      setInsertion(pageNum, {
        ...(typeof source === "string"
          ? { url: source }
          : { label: source.name }),
        blob,
        previewSrc: URL.createObjectURL(blob),
      });
      return null;
    } catch (err) {
      return err?.message || "이미지를 가져오지 못했습니다";
    }
  };
  const saveMarkedPages = async () => {
    if (
      saving ||
      (markedPages.size === 0 && insertions.size === 0 && pageOrder === null)
    )
      return;
    const entries = [...markedPages].sort(([a], [b]) => a - b);
    const pages = entries
      .filter(([, mark]) => mark.kind === "page")
      .map(([pageNum]) => pageNum);
    const crops = entries
      .filter(([, mark]) => mark.kind === "crop")
      .map(([pageNum, mark]) => ({ page: pageNum, ...mark.rect }));
    const splits = entries
      .filter(([, mark]) => mark.kind === "split")
      .map(([pageNum, mark]) => ({ page: pageNum, ...mark.rects }));
    const inserts = [...insertions]
      .sort(([a], [b]) => a - b)
      .map(([pageNum, entry]) => ({ page: pageNum, entry }));
    const reordered = pageOrder
      ? pageOrder.filter((n, i) => n !== i + 1).sort((a, b) => a - b)
      : [];
    const summary = [
      inserts.length > 0 &&
        `${inserts.map((i) => i.page).join(", ")}쪽 앞에 이미지 삽입`,
      pages.length > 0 && `${pages.join(", ")}쪽 삭제`,
      crops.length > 0 && `${crops.map((c) => c.page).join(", ")}쪽 크롭`,
      splits.length > 0 && `${splits.map((c) => c.page).join(", ")}쪽 분할`,
      reordered.length > 0 && `${reordered.join(", ")}쪽 순서 변경`,
    ]
      .filter(Boolean)
      .join(", ");
    if (
      !window.confirm(
        `${summary}를 저장합니다. 원본은 0_trash에 보존됩니다. 저장할까요?`,
      )
    ) {
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      // URL 로 고른 이미지는 서버가 다시 내려받고, 이 기기에서 고른 이미지는 미리 본 JPEG 를 그대로 보낸다.
      const insertBodies = await Promise.all(
        inserts.map(async ({ page, entry }) =>
          entry.url
            ? { page, url: entry.url }
            : { page, image_data: await blobToBase64(entry.blob) },
        ),
      );
      const response = await fetch(
        getApiUrlPrefix() + apiPrefix + `/pdf-pages/${bookId}/delete`,
        {
          method: "POST",
          credentials: "include",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            pages,
            crops,
            splits,
            inserts: insertBodies,
            ...(pageOrder && { order: pageOrder }),
          }),
        },
      );
      if (!response.ok) throw new Error(await describeHttpError(response));
      const body = await response.json();
      if (body.status !== "success") {
        throw new Error(body.error || "저장에 실패했습니다.");
      }
      setMarkedPages(new Map());
      setPageOrder(null);
      clearInsertions();
      setReloadToken((prev) => prev + 1);
    } catch (err) {
      setSaveError(err?.message || "저장에 실패했습니다.");
    } finally {
      setSaving(false);
    }
  };
  const zoomDown = () => setZoomIndex((prev) => Math.max(0, prev - 1));
  const zoomUp = () =>
    setZoomIndex((prev) => Math.min(ZOOM_STEPS.length - 1, prev + 1));
  const nativeWidthRef = useRef(0);
  const nativeHeightRef = useRef(0);
  const containerRef = useRef(null);
  const canvasRefs = useRef({});
  const renderedPagesRef = useRef(new Set());
  const everRenderedRef = useRef(new Set()); // 한 번이라도 그려진 페이지 (진행률 표시용, 해제해도 유지)
  const nearPagesRef = useRef(new Set()); // 지금 near zone 에 있는 페이지 (해제 대상에서 제외)
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
        if (!response.ok) throw new Error(await describeHttpError(response));

        const buffer = await response.arrayBuffer();
        if (cancelledRef.current) return;

        const pdfDoc = await pdfjs.getDocument({
          ...PDF_DOCUMENT_OPTIONS,
          data: buffer,
        }).promise;
        if (cancelledRef.current) {
          pdfDoc.loadingTask.destroy();
          return;
        }

        chunkDocsRef.current.set(key, pdfDoc);

        // 청크 로드 완료 → 큐에 대기 중이던 페이지들을 다시 펌프 (렌더 가능해짐)
        pumpQueue();
      } catch (err) {
        /* v8 ignore next 3 -- chunk fetch errors after unmount are intentionally silent. */
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
    /* v8 ignore next -- startup fetch handles page 1 before scroll scheduling. */
    if (globalPageNum === 1) return 1;
    // 2부터 시작해서 CHUNK_SIZE 단위
    return 2 + Math.floor((globalPageNum - 2) / CHUNK_SIZE) * CHUNK_SIZE;
  }, []);

  // 그려 둔 페이지의 canvas 비트맵을 해제한다. 1x1 로 줄이되 aspect-ratio 로 레이아웃 높이는 유지해
  // 스크롤 위치가 밀리지 않게 한다. 다시 보이면 executeRender 가 aspect-ratio 를 풀고 새로 그린다.
  const evictPage = useCallback((pageNum) => {
    renderedPagesRef.current.delete(pageNum);
    const canvas = canvasRefs.current[pageNum];
    if (!canvas || !canvas.width || !canvas.height) return;
    canvas.style.aspectRatio = `${canvas.width} / ${canvas.height}`;
    canvas.width = 1;
    canvas.height = 1;
  }, []);

  // pageNums 쪽을 포함한 청크 중 그려진·렌더 중·대기 중인 쪽이 하나도 없는 청크의 pdfDoc 을 해제한다.
  // (해제한 청크는 그 쪽이 다시 필요해질 때 pumpQueue 가 다시 받아 온다.)
  const releaseIdleChunks = useCallback((pageNums) => {
    for (const [key, pdfDoc] of [...chunkDocsRef.current.entries()]) {
      const [s, e] = key.split("-").map(Number);
      if (!pageNums.some((n) => n >= s && n <= e)) continue;
      let busy = false;
      for (let n = s; n <= e && !busy; n++) {
        busy =
          renderedPagesRef.current.has(n) ||
          activeRenderTasksRef.current.has(n) ||
          renderQueueRef.current.has(n);
      }
      if (busy) continue;
      chunkDocsRef.current.delete(key);
      pdfDoc.loadingTask.destroy();
    }
  }, []);

  // near zone 밖에서 그려진 페이지 중 near zone 에서 먼 것부터 해제해 RETAIN_PAGES_OUTSIDE_VIEW 개만 남긴다.
  // 렌더가 끝날 때만 호출하면 충분하다: 그려진 페이지 수는 렌더로만 늘어난다.
  const trimFarPages = useCallback(() => {
    const near = nearPagesRef.current;
    // 관찰자가 아직 위치를 알려주지 않았으면 어느 쪽이 보이는지 모르므로 해제하지 않는다
    if (near.size === 0) return;
    const nearMin = Math.min(...near);
    const nearMax = Math.max(...near);
    const far = [];
    for (const pageNum of renderedPagesRef.current) {
      if (near.has(pageNum) || activeRenderTasksRef.current.has(pageNum)) continue;
      far.push({
        pageNum,
        dist: Math.max(nearMin - pageNum, pageNum - nearMax, 0),
      });
    }
    if (far.length <= RETAIN_PAGES_OUTSIDE_VIEW) return;
    far.sort((a, b) => a.dist - b.dist);
    const evicted = far.slice(RETAIN_PAGES_OUTSIDE_VIEW).map((f) => f.pageNum);
    for (const pageNum of evicted) evictPage(pageNum);
    releaseIdleChunks(evicted);
  }, [evictPage, releaseIdleChunks]);

  // 실제 렌더 실행. 스케줄러가 "청크 로드된" 페이지에 대해서만 호출한다. 취소 가능.
  const executeRender = useCallback(
    async (globalPageNum) => {
      /* v8 ignore next -- scheduler filters already-rendered pages before execution. */
      if (renderedPagesRef.current.has(globalPageNum)) return;

      const chunkInfo = findChunkForPage(globalPageNum);
      /* v8 ignore next 5 -- pumpQueue calls executeRender only after a chunk is available. */
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
        const canvas = canvasRefs.current[globalPageNum];
        if (!canvas) {
          renderedPagesRef.current.delete(globalPageNum);
          return;
        }

        const renderViewport = page.getViewport({
          scale: computeRenderScale(page, canvas, dpr),
        });

        const context = canvas.getContext("2d");
        canvas.style.aspectRatio = "";
        canvas.width = renderViewport.width;
        canvas.height = renderViewport.height;

        const task = page.render({
          canvasContext: context,
          viewport: renderViewport,
        });
        // 취소 가능하도록 슬롯 마커를 실제 RenderTask로 교체
        activeRenderTasksRef.current.set(globalPageNum, task);
        await task.promise;
        // 그린 뒤에는 pdfjs 가 페이지에 붙들고 있는 operator list 등을 놓아 준다
        page.cleanup?.();

        everRenderedRef.current.add(globalPageNum);
        setLoadedPages(everRenderedRef.current.size);

        if (globalPageNum === 1) {
          setIsFirstPageReady(true);
        }
        trimFarPages();
      } catch (err) {
        // 취소된 렌더는 재진입 시 다시 그릴 수 있도록 완료 표시를 해제한다
        renderedPagesRef.current.delete(globalPageNum);
        if (err?.name !== "RenderingCancelledException") {
          console.error(`페이지 ${globalPageNum} 렌더링 실패:`, err);
        }
      }
    },
    [findChunkForPage, trimFarPages],
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
        /* v8 ignore next -- page 1 is loaded by the initial single-page chunk. */
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
        /* v8 ignore next -- queue pumping after unmount is intentionally skipped. */
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
      /* v8 ignore next -- observer/scheduler avoids enqueueing rendered pages. */
      if (renderedPagesRef.current.has(pageNum)) return;
      /* v8 ignore next -- observer/scheduler avoids enqueueing active pages. */
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
    /* v8 ignore next -- cancellation is called only for active RenderTask objects. */
    if (task && typeof task.cancel === "function") task.cancel();
  }, []);

  // 1장씩 보기: 뷰어 위의 카드 헤더·nav가 차지하는 높이만큼 viewport가 줄어든다.
  // 컨테이너를 viewport 맨 위에 맞춘 뒤 실제로 보이는 높이를 CSS 변수로 넘기면,
  // 쪽 높이 = 보이는 높이가 되어 한 쪽이 잘리지 않는다. 스크롤과 snap은 CSS가 맡는다.
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!singlePage || !el) return undefined;
    const measure = () => {
      const top = Math.max(0, el.getBoundingClientRect().top);
      el.style.setProperty(
        "--pdf-avail-h",
        `${Math.max(window.innerHeight - top, 200)}px`,
      );
    };
    el.scrollIntoView?.({ block: "start" });
    measure();
    window.addEventListener("resize", measure);
    return () => {
      window.removeEventListener("resize", measure);
      el.style.removeProperty("--pdf-avail-h");
    };
  }, [singlePage]);

  useEffect(() => {
    if (!bookId) {
      setError("유효한 bookId가 제공되지 않았습니다.");
      return;
    }

    cancelledRef.current = false;

    const loadPdf = async () => {
      setError(null);
      setMarkedPages(new Map());
      for (const entry of insertionsRef.current.values()) {
        URL.revokeObjectURL(entry.previewSrc);
      }
      setInsertions(new Map());
      setInsertDialog(null);
      setTotalPages(0);
      setLoadedPages(0);
      setDownloadProgress(0);
      setIsFirstPageReady(false);
      canvasRefs.current = {};
      renderedPagesRef.current = new Set();
      everRenderedRef.current = new Set();
      nearPagesRef.current = new Set();
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
        if (!firstResponse.ok)
          throw new Error(await describeHttpError(firstResponse));

        const serverTotalPages = parseInt(
          firstResponse.headers.get("X-Total-Pages") || "0",
          10,
        );
        const firstBuffer = await firstResponse.arrayBuffer();
        if (cancelledRef.current) return;

        setDownloadProgress(30);

        const firstPdfDoc = await pdfjs.getDocument({
          ...PDF_DOCUMENT_OPTIONS,
          data: firstBuffer,
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
        const firstCssViewport = firstPage.getViewport({
          scale: DISPLAY_SCALE,
        });
        const firstRenderViewport = firstPage.getViewport({
          scale: DISPLAY_SCALE * dpr,
        });
        const implausiblySmall = !(
          firstCssViewport.width >= MIN_PLAUSIBLE_CSS_WIDTH
        );
        const aspect =
          firstCssViewport.width > 0 && firstCssViewport.height > 0
            ? firstCssViewport.height / firstCssViewport.width
            : 1;
        // 쪽이 이미지 한 장이면 쪽 크기(pt) 대신 이미지 픽셀 크기를 원래 크기(100%)로 삼는다.
        const imageSize = await getFullPageImageSize(
          firstPage,
          pdfjs.OPS?.paintImageXObject,
        );
        if (imageSize) {
          nativeWidthRef.current = imageSize.width;
          nativeHeightRef.current = imageSize.height;
        } else {
          nativeWidthRef.current = implausiblySmall
            ? FALLBACK_NATIVE_CSS_WIDTH
            : firstCssViewport.width;
          nativeHeightRef.current = implausiblySmall
            ? FALLBACK_NATIVE_CSS_WIDTH * aspect
            : firstCssViewport.height;
        }

        if (cancelledRef.current) return;

        // flushSync로 상태 업데이트를 동기화하여 canvas가 DOM에 생성된 후 렌더링
        flushSync(() => {
          setTotalPages(pagesToRender);
        });

        // 모든 canvas에 추정 크기 설정 (placeholder로 레이아웃 확보)
        for (let i = 1; i <= pagesToRender; i++) {
          const canvas = canvasRefs.current[i];
          /* v8 ignore next 4 -- flushSync creates all page canvases before sizing. */
          if (canvas) {
            canvas.width = implausiblySmall
              ? FALLBACK_PLACEHOLDER_PIXEL_WIDTH
              : firstRenderViewport.width;
            canvas.height = implausiblySmall
              ? Math.max(1, Math.round(FALLBACK_PLACEHOLDER_PIXEL_WIDTH * aspect))
              : firstRenderViewport.height;
          }
        }

        // 첫 페이지 렌더링 (스케줄러 큐에 최우선 투입)
        /* v8 ignore next -- cancellation here requires unmount in the same async gap. */
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
              nearPagesRef.current.delete(pageNum);
              if (entry.isIntersecting) {
                nearPagesRef.current.add(pageNum);
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

        // 1쪽도 관찰한다: 해제된 뒤 다시 보이면 이 관찰자가 되살려야 한다.
        for (let i = 1; i <= pagesToRender; i++) {
          const canvas = canvasRefs.current[i];
          /* v8 ignore next 4 -- flushSync creates all observed page canvases before observer setup. */
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
  }, [
    bookId,
    pageCount,
    preview,
    reloadToken,
    enqueueRender,
    cancelRender,
    fetchChunk,
  ]);

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
  const displayOrder = pageOrder ?? pageNumbers;

  return (
    <div
      className={`pdf-container${singlePage ? " pdf-single" : ""}`}
      ref={containerRef}
      tabIndex={-1}
    >
      <div className="pdf-toolbar">
        <div className="pdf-info">
          {loadedPages < totalPages
            ? `렌더링 중... ${loadedPages}/${totalPages}쪽`
            : `총 ${totalPages}쪽 표시`}
          {editable &&
            markedPages.size + insertions.size > 0 &&
            ` · 변경 표시 ${new Set([...markedPages.keys(), ...insertions.keys()]).size}쪽`}
          {saving && " · 저장 중..."}
          {saveError && (
            <span className="viewer-edit-error" role="alert">
              {" · "}
              {saveError}
            </span>
          )}
        </div>
        <div className="pdf-zoom-controls">
          <button
            className={`pdf-zoom-btn pdf-zoom-fit-btn pdf-single-btn ${singlePage ? "active" : ""}`}
            onClick={toggleSinglePage}
          >
            <span className="toggle-track">
              <span className="toggle-knob" />
            </span>
            1장씩
          </button>
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
            disabled={fitMode || singlePage || zoomIndex === 0}
          >
            −
          </button>
          <span className="pdf-zoom-label">
            {fitMode || singlePage ? "-" : `${zoomPercent}%`}
          </span>
          <button
            className="pdf-zoom-btn"
            onClick={zoomUp}
            disabled={
              fitMode || singlePage || zoomIndex === ZOOM_STEPS.length - 1
            }
          >
            +
          </button>
        </div>
      </div>
      <div className="pdf-content">
        {displayOrder.map((pageNum, position) => {
          const canvasStyle = fitMode
            ? { width: "100%", height: "auto" }
            : {
                width: `${Math.round((nativeWidthRef.current * zoomPercent) / 100)}px`,
                height: "auto",
                // 원래 크기(100%)에서 이미지가 컨테이너보다 넓으면 컨테이너 폭에 맞춘다.
                // 사용자가 확대(+)한 경우에는 제한을 풀어 가로 스크롤로 볼 수 있게 한다.
                maxWidth: zoomPercent <= 100 ? "100%" : "none",
              };
          if (!editable) {
            return (
              <canvas
                key={pageNum}
                ref={setCanvasRef(pageNum)}
                className="pdf-page"
                style={canvasStyle}
              />
            );
          }
          const mark = markedPages.get(pageNum);
          const insertion = insertions.get(pageNum);
          // 순서를 바꾼 쪽은 원래 자리(pageNum)와 현재 자리(position + 1)가 다르다.
          const moved = position + 1 !== pageNum;
          // 저장 버튼은 이 쪽에 무엇이든 변경 표시가 있을 때 나온다.
          const marked =
            mark !== undefined || insertion !== undefined || moved;
          return (
            <Fragment key={pageNum}>
              {insertion && (
                <div
                  className="pdf-page pdf-insert-preview viewer-edit-pending"
                  data-testid={`pdf-insert-preview-${pageNum}`}
                  style={canvasStyle}
                >
                  <img
                    className="pdf-insert-image"
                    src={insertion.previewSrc}
                    alt={`${pageNum}쪽 앞에 삽입될 이미지`}
                  />
                  <div className="viewer-edit-tag" title={insertion.url ?? insertion.label}>
                    새 쪽 · 저장 전 ({pageNum}쪽 앞에 삽입)
                  </div>
                  <div className="viewer-edit-actions">
                    <button
                      type="button"
                      className="viewer-edit-btn"
                      aria-label="저장"
                      title="표시한 변경을 저장"
                      disabled={saving}
                      onClick={saveMarkedPages}
                    >
                      <SaveIcon fontSize="small" />
                    </button>
                    <button
                      type="button"
                      className="viewer-edit-btn"
                      aria-label={`${pageNum}쪽 앞 이미지 삽입 취소`}
                      title="삽입 취소"
                      disabled={saving}
                      onClick={() => setInsertion(pageNum, null)}
                    >
                      ×
                    </button>
                  </div>
                </div>
              )}
            <div
              className="pdf-page pdf-page-wrap"
              style={canvasStyle}
              data-marked={marked ? "true" : "false"}
            >
              <canvas ref={setCanvasRef(pageNum)} className="pdf-page-canvas" />
              {moved && (
                <div className="viewer-edit-tag">
                  순서 변경 · 저장 전 ({pageNum}쪽 → {position + 1}번째)
                </div>
              )}
              {mark?.kind === "page" && (
                <div
                  className="viewer-edit-delete-overlay"
                  data-testid={`pdf-delete-overlay-${pageNum}`}
                  data-kind="page"
                />
              )}
              {mark?.kind === "crop" && (
                <PdfCropBox
                  boxes={[{ id: "crop", rect: mark.rect, color: CROP_COLOR }]}
                  onChange={(id, rect) => changeBox(pageNum, id, rect)}
                />
              )}
              {mark?.kind === "split" && (
                <PdfCropBox
                  boxes={[
                    {
                      id: "first",
                      rect: mark.rects.first,
                      color: SPLIT_COLORS.first,
                      label: "앞",
                    },
                    {
                      id: "second",
                      rect: mark.rects.second,
                      color: SPLIT_COLORS.second,
                      label: "뒤",
                    },
                  ]}
                  onChange={(id, rect) => changeBox(pageNum, id, rect)}
                />
              )}
              <div className="viewer-edit-actions">
                {marked && (
                  <button
                    type="button"
                    className="viewer-edit-btn"
                    aria-label="저장"
                    title="표시한 변경을 저장"
                    disabled={saving}
                    onClick={saveMarkedPages}
                  >
                    <SaveIcon fontSize="small" />
                  </button>
                )}
                <button
                  type="button"
                  className="viewer-edit-btn"
                  aria-label={`${pageNum}쪽을 위로 옮기기`}
                  title="이전 쪽과 순서 바꾸기"
                  disabled={saving || position === 0}
                  onClick={() => movePage(pageNum, -1)}
                >
                  <ArrowDropUpIcon fontSize="small" />
                </button>
                <button
                  type="button"
                  className="viewer-edit-btn"
                  aria-label={`${pageNum}쪽을 아래로 옮기기`}
                  title="다음 쪽과 순서 바꾸기"
                  disabled={saving || position === displayOrder.length - 1}
                  onClick={() => movePage(pageNum, 1)}
                >
                  <ArrowDropDownIcon fontSize="small" />
                </button>
                <button
                  type="button"
                  className={`viewer-edit-btn${insertion ? " active" : ""}`}
                  aria-label={`${pageNum}쪽 앞에 이미지 삽입`}
                  aria-pressed={Boolean(insertion)}
                  title={
                    insertion
                      ? "삽입할 이미지 URL 수정"
                      : "이 쪽 앞에 이미지 URL로 새 쪽 삽입"
                  }
                  disabled={saving}
                  onClick={() => setInsertDialog(pageNum)}
                >
                  <AddPhotoAlternateIcon fontSize="small" />
                </button>
                <button
                  type="button"
                  className={`viewer-edit-btn${mark?.kind === "split" ? " active" : ""}`}
                  aria-label={`${pageNum}쪽 분할`}
                  aria-pressed={mark?.kind === "split"}
                  title={
                    singlePage
                      ? "1장씩 보기에서는 분할할 수 없습니다"
                      : mark?.kind === "split"
                        ? "분할 취소"
                        : "파란 영역은 앞쪽, 빨간 영역은 뒤쪽 페이지로 분할"
                  }
                  disabled={saving || singlePage}
                  onClick={() => toggleMarkedPage(pageNum, "split")}
                >
                  <VerticalSplitOutlinedIcon fontSize="small" />
                </button>
                <button
                  type="button"
                  className={`viewer-edit-btn${mark?.kind === "crop" ? " active" : ""}`}
                  aria-label={`${pageNum}쪽 크롭`}
                  aria-pressed={mark?.kind === "crop"}
                  title={
                    singlePage
                      ? "1장씩 보기에서는 크롭할 수 없습니다"
                      : mark?.kind === "crop"
                        ? "크롭 취소"
                        : "남길 영역을 지정해 크롭"
                  }
                  disabled={saving || singlePage}
                  onClick={() => toggleMarkedPage(pageNum, "crop")}
                >
                  <CropFreeIcon fontSize="small" />
                </button>
                <button
                  type="button"
                  className={`viewer-edit-btn${mark?.kind === "page" ? " active" : ""}`}
                  aria-label={`${pageNum}쪽 삭제 표시`}
                  aria-pressed={mark?.kind === "page"}
                  title={
                    mark?.kind === "page"
                      ? "삭제 표시 해제"
                      : "이 쪽을 삭제 표시"
                  }
                  disabled={saving}
                  onClick={() => toggleMarkedPage(pageNum, "page")}
                >
                  <DeleteOutlineIcon fontSize="small" />
                </button>
              </div>
            </div>
            </Fragment>
          );
        })}
      </div>
      {insertDialog !== null && (
        <PdfInsertImageModal
          pageNum={insertDialog}
          initialUrl={insertions.get(insertDialog)?.url}
          onSubmit={async (url) => {
            const message = await previewInsertion(insertDialog, url);
            if (!message) setInsertDialog(null);
            return message;
          }}
          onClose={() => setInsertDialog(null)}
        />
      )}
      <style>{pdfStyles}</style>
    </div>
  );
}

const pdfStyles = `
    .pdf-container {
        --pdf-toolbar-h: 32px;
        width: 100%;
        height: calc(100vh - 10px);
        overflow-y: auto;
        outline: none;
    }
    .pdf-toolbar {
        box-sizing: border-box;
        height: var(--pdf-toolbar-h);
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
    /* 1장씩 보기: 각 쪽을 뷰포트 높이에 맞춰 snap 지점으로 만든다.
       PC는 키 스크롤(Space/PageDown), 모바일은 터치 스와이프가 한 쪽씩 넘어간다.
       scroll-snap-stop: always가 빠른 스와이프로 여러 쪽을 건너뛰는 것을 막는다. */
    .pdf-container.pdf-single {
        height: var(--pdf-avail-h, calc(100vh - 10px));
        scroll-snap-type: y mandatory;
        scroll-padding-top: var(--pdf-toolbar-h);
    }
    /* overflow-x:auto는 overflow-y도 auto로 만들어 snap 컨테이너를 가로채므로 푼다 */
    .pdf-container.pdf-single .pdf-content {
        overflow: visible;
    }
    .pdf-container.pdf-single .pdf-page {
        width: 100% !important;
        height: calc(var(--pdf-avail-h, calc(100vh - 10px)) - var(--pdf-toolbar-h)) !important;
        object-fit: contain;
        scroll-snap-align: start;
        scroll-snap-stop: always;
    }
    .pdf-page {
        display: block;
        margin: 0 auto;
    }
    /* 편집 화면: 캔버스를 감싸는 래퍼. 버튼·회색 레이어를 캔버스 위에 겹친다. */
    .pdf-page-wrap {
        position: relative;
    }
    .pdf-page-wrap .pdf-page-canvas {
        display: block;
        width: 100%;
        height: 100%;
    }
    .pdf-container.pdf-single .pdf-page-wrap .pdf-page-canvas {
        object-fit: contain;
    }
    /* 이미지 삽입 미리보기: 저장 전에도 삽입될 쪽을 다른 쪽처럼 보여 준다. 점선 테두리와 라벨로 아직 저장 전임을 알린다. */
    .pdf-insert-preview {
        position: relative;
    }
    .pdf-insert-image {
        display: block;
        width: 100%;
        height: auto;
    }
    .pdf-container.pdf-single .pdf-insert-image {
        height: 100%;
        object-fit: contain;
    }
    /* 크롭: 박스 안은 선명하게 두고 바깥 네 구역만 회색+블러로 덮는다. 레이어 자체는 클릭을 통과시키고 핸들만 받는다. */
    .pdf-crop-layer {
        position: absolute;
        inset: 0;
        z-index: 4;
        pointer-events: none;
    }
    .pdf-crop-dim {
        position: absolute;
        background: rgba(90, 90, 90, 0.6);
        -webkit-backdrop-filter: blur(3px);
        backdrop-filter: blur(3px);
    }
    .pdf-crop-box {
        --crop-color: #4a90d9;
        position: absolute;
        box-sizing: border-box;
        border: 2px solid var(--crop-color);
    }
    .pdf-crop-label {
        position: absolute;
        top: 20px;
        left: 4px;
        padding: 0 6px;
        border-radius: 3px;
        background: var(--crop-color);
        color: #fff;
        font-size: 12px;
        line-height: 18px;
        pointer-events: none;
    }
    /* 조절 핸들은 모두 박스 안쪽(테두리 안)에만 둔다. 박스끼리 맞닿아도 서로의 핸들 영역이 겹치지 않는다.
       변 핸들은 두 모서리 핸들 사이 전체를 따라 잡히고, 가운데에 눈에 보이는 손잡이를 그려 드래그할 수 있음을 알린다. */
    .pdf-crop-handle {
        position: absolute;
        pointer-events: auto;
        touch-action: none;
    }
    .pdf-crop-handle-n, .pdf-crop-handle-s { left: 16px; right: 16px; height: 16px; cursor: ns-resize; }
    .pdf-crop-handle-n { top: 0; }
    .pdf-crop-handle-s { bottom: 0; }
    .pdf-crop-handle-e, .pdf-crop-handle-w { top: 16px; bottom: 16px; width: 16px; cursor: ew-resize; }
    .pdf-crop-handle-w { left: 0; }
    .pdf-crop-handle-e { right: 0; }
    .pdf-crop-handle-n::after, .pdf-crop-handle-s::after,
    .pdf-crop-handle-e::after, .pdf-crop-handle-w::after {
        content: "";
        position: absolute;
        left: 50%;
        top: 50%;
        transform: translate(-50%, -50%);
        box-sizing: border-box;
        background: #fff;
        border: 2px solid var(--crop-color);
        border-radius: 3px;
    }
    .pdf-crop-handle-n::after, .pdf-crop-handle-s::after { width: 28px; height: 8px; }
    .pdf-crop-handle-e::after, .pdf-crop-handle-w::after { width: 8px; height: 28px; }
    /* 회전 손잡이: 박스 안쪽(위쪽 변 손잡이 바로 아래)에 둔다. 박스와 함께 돌고 박스 밖으로 나가지 않는다. */
    .pdf-crop-rotate {
        position: absolute;
        top: 22px;
        left: 50%;
        transform: translateX(-50%);
        display: flex;
        align-items: center;
        justify-content: center;
        width: 24px;
        height: 24px;
        box-sizing: border-box;
        border: 2px solid var(--crop-color);
        border-radius: 50%;
        background: #fff;
        color: var(--crop-color);
        font-size: 16px;
        cursor: grab;
        pointer-events: auto;
        touch-action: none;
    }
    .pdf-crop-rotate:active {
        cursor: grabbing;
    }
    .pdf-crop-handle-nw, .pdf-crop-handle-ne, .pdf-crop-handle-sw, .pdf-crop-handle-se {
        width: 16px;
        height: 16px;
        box-sizing: border-box;
        background: #fff;
        border: 2px solid var(--crop-color);
    }
    .pdf-crop-handle-nw { top: 0; left: 0; cursor: nwse-resize; }
    .pdf-crop-handle-se { bottom: 0; right: 0; cursor: nwse-resize; }
    .pdf-crop-handle-ne { top: 0; right: 0; cursor: nesw-resize; }
    .pdf-crop-handle-sw { bottom: 0; left: 0; cursor: nesw-resize; }
    @media (pointer: coarse) {
        .pdf-crop-handle-n, .pdf-crop-handle-s { left: 28px; right: 28px; height: 28px; }
        .pdf-crop-handle-e, .pdf-crop-handle-w { top: 28px; bottom: 28px; width: 28px; }
        .pdf-crop-handle-nw, .pdf-crop-handle-ne, .pdf-crop-handle-sw, .pdf-crop-handle-se { width: 28px; height: 28px; }
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
  editable: PropTypes.bool,
};
