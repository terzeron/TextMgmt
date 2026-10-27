import { useCallback, useEffect, useRef, useState } from "react";
import { Alert, Button, Spinner, Form } from "react-bootstrap";
import SplitLayout from "./SplitLayout";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faTrash, faMagnifyingGlass, faFloppyDisk } from "@fortawesome/free-solid-svg-icons";
import Folder from "./Folder";
import { updateFolderInTree } from "./folderUtils";
import { getApiUrlPrefix } from "./Common";
import { coverAdminRequest } from "./coverAdminApi";
import { createStoreQueue, selectViewportTargets } from "./coverFetchQueue";
import { rankCoverImages, formatCoverBytes } from "./coverImageRanking";
import CoverImagePreview from "./CoverImagePreview";
import ImageUrlModal from "./ImageUrlModal";
import { MAX_UPLOAD_BYTES } from "./insertPreview";
import { coverThumbProps, markTallCover } from "./coverImageFit.js";
import "./CoverGrid.css";
import "./BookCoverAdmin.css";

const STORES = [
  ["yes24", "YES24"], ["aladin", "알라딘"], ["kyobo", "교보문고"],
  ["naver", "네이버쇼핑"], ["ridi", "RIDI"], ["munpia", "문피아"],
  ["naverseries", "시리즈"], ["joara", "조아라"],
];
// 디렉토리별로 자동 검색할 서점 그룹. 나머지 서점은 "검색" 버튼을 눌러야 검색한다.
// 서브컬쳐 장르는 웹소설 서점을, 일반 책은 일반 서점을 자동 검색한다.
const SUBCULTURE_DIRECTORIES = new Set(["3_라이트노벨", "3_판타지", "3_무협", "3_여성향"]);
const AUTO_STORES = {
  subculture: new Set(["munpia", "naverseries", "joara"]),
  general: new Set(["yes24", "aladin", "kyobo", "naver", "ridi"]),
};
// 자동 검색 서점을 왼쪽에, 수동 검색 서점을 오른쪽에 둔다(각 그룹 안의 순서는 STORES 순서).
function storeLayout(filePath) {
  const [top, ...rest] = String(filePath).split("/");
  const auto = rest.length && SUBCULTURE_DIRECTORIES.has(top) ? AUTO_STORES.subculture : AUTO_STORES.general;
  return { auto, stores: [...STORES.filter(([name]) => auto.has(name)), ...STORES.filter(([name]) => !auto.has(name))] };
}
const API_ROOT = "/admin/book-covers";
const ROOT = "_root";
const initialTree = [{ id: ROOT, label: "책 디렉토리", fileType: "folder", children: [] }];

export default function BookCoverAdmin() {
  const [folderData, setFolderData] = useState(initialTree);
  const [expanded, setExpanded] = useState([ROOT]);
  const [folderOpen, setFolderOpen] = useState(true);
  const [directory, setDirectory] = useState(ROOT);
  const [replaceCovers, setReplaceCovers] = useState(false);
  const [items, setItems] = useState([]);
  const [sources, setSources] = useState({});
  const [images, setImages] = useState({});
  const [page, setPage] = useState(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [viewportTick, setViewportTick] = useState(0);
  const [manual, setManual] = useState(null);
  const [zoom, setZoom] = useState(null);
  const closeZoom = useCallback(() => setZoom(null), []);
  const generation = useRef(0);
  const rows = useRef(new Map());
  const candidates = useRef(new Map());
  const urls = useRef(new Map());
  const queue = useRef(null);
  const listingRequest = useRef(null);
  const scanTrigger = useRef(null);
  const loadedDirectories = useRef(new Set());
  const saving = useRef(new Set());
  const bookVersions = useRef(new Map());

  const loadDirectories = useCallback(async (id) => {
    if (loadedDirectories.current.has(id)) return;
    try {
      const children = await coverAdminRequest(`/directories?directory=${encodeURIComponent(id === ROOT ? "" : id)}`);
      setFolderData((tree) => updateFolderInTree(tree, id, (entry) => ({ ...entry, children })));
      loadedDirectories.current.add(id);
    } catch (exc) {
      setError(exc.message);
    }
  }, []);

  const loadPage = useCallback(async (id, cursor = "", append = false) => {
    listingRequest.current?.abort();
    const controller = new AbortController();
    listingRequest.current = controller;
    const current = generation.current;
    setLoading(true);
    setError("");
    try {
      const result = await coverAdminRequest(`/missing?directory=${encodeURIComponent(id === ROOT ? "" : id)}&cursor=${cursor}${replaceCovers ? "&covered=true" : ""}`, { signal: controller.signal });
      if (generation.current !== current || controller.signal.aborted) return;
      setItems((previous) => append ? [...previous, ...result.items] : result.items);
      setPage(result);
    } catch (exc) {
      if (!controller.signal.aborted && generation.current === current) setError(exc.message);
    } finally {
      if (listingRequest.current === controller) {
        listingRequest.current = null;
        setLoading(false);
      }
    }
  }, [replaceCovers]);

  useEffect(() => {
    const imageUrls = urls.current;
    generation.current += 1;
    queue.current = createStoreQueue({ backgroundInterval: 1000 });
    setItems([]);
    setSources({});
    setImages({});
    setPage(null);
    setManual(null);
    setZoom(null);
    bookVersions.current.clear();
    for (const url of imageUrls.values()) URL.revokeObjectURL(url);
    imageUrls.clear();
    loadDirectories(directory);
    loadPage(directory);
    return () => {
      generation.current += 1;
      queue.current?.dispose();
      listingRequest.current?.abort();
      for (const url of imageUrls.values()) URL.revokeObjectURL(url);
      imageUrls.clear();
    };
  }, [directory, loadDirectories, loadPage]);

  useEffect(() => {
    let frame;
    const changed = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => setViewportTick((value) => value + 1));
    };
    window.addEventListener("scroll", changed, { capture: true, passive: true });
    window.addEventListener("resize", changed);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("scroll", changed, true);
      window.removeEventListener("resize", changed);
    };
  }, []);

  useEffect(() => {
    if (!page?.next_cursor || loading || error || listingRequest.current) return;
    const rect = scanTrigger.current?.getBoundingClientRect();
    if (rect && rect.top <= window.innerHeight * 1.25) loadPage(directory, page.next_cursor, true);
  }, [directory, error, items, loading, loadPage, page, viewportTick]);

  useEffect(() => {
    const viewport = { top: 0, left: 0, bottom: window.innerHeight, right: window.innerWidth };
    const current = generation.current;
    const versions = new Map(items.map((item) => [item.file_path, bookVersions.current.get(item.file_path) || 0]));
    const isCurrent = (path) => generation.current === current && (bookVersions.current.get(path) || 0) === versions.get(path);
    const tasks = [];
    const isStoreVisible = (path, store, layout) => {
      const node = rows.current.get(path)?.querySelector(`[data-store="${store}"]`);
      if (!node) return false;
      const scroller = node.closest(".book-cover-candidates");
      // 초기 빈 칸은 검색 결과가 오면 넓어진다. 이때의 일시 노출로 뒤쪽 검색을 시작하지 않는다.
      if (!scroller.scrollLeft && layout.stores.slice(0, 4).filter(([name]) => layout.auto.has(name)).some(([name]) => !sources[path]?.[name] || sources[path][name].status === "loading")) return false;
      const rect = node.getBoundingClientRect();
      const clip = scroller.getBoundingClientRect();
      const left = Math.max(viewport.left, clip.left);
      const right = Math.min(viewport.right, clip.right);
      return right > left && rect.right > rect.left && rect.right > left && rect.left < right;
    };
    const eligibleRows = items.filter((item) => ["missing", "covered"].includes(item.status) && !item.saving && !item.busy && !item.paused)
      .flatMap((item) => {
        const node = rows.current.get(item.file_path);
        return node ? [{ key: item.file_path, item, rect: node.getBoundingClientRect() }] : [];
      });
    const updateSource = (path, store, value) => {
      if (!isCurrent(path)) return;
      const previousSource = sources[path]?.[store];
      const transfers = [];
      for (const candidate of value?.candidates || []) {
        const old = previousSource?.candidates?.find((entry) => entry.detail_url === candidate.detail_url && entry.title === candidate.title);
        if (old && old.candidate_id !== candidate.candidate_id && images[old.candidate_id]?.url) {
          const image = images[old.candidate_id];
          urls.current.delete(old.candidate_id);
          urls.current.set(candidate.candidate_id, image.url);
          transfers.push([old.candidate_id, candidate.candidate_id, image]);
        }
      }
      if (transfers.length) setImages((previous) => {
        const next = { ...previous };
        for (const [oldId, newId, image] of transfers) { delete next[oldId]; next[newId] = image; }
        return next;
      });
      setSources((previous) => ({ ...previous, [path]: { ...previous[path], [store]:
        value?.status === "loading" || value?.status === "error" ? { ...value, candidates: previous[path]?.[store]?.candidates } : value } }));
    };
    const rowTargets = selectViewportTargets(eligibleRows, viewport, { ahead: 3 });
    const visibleBooks = new Set(rowTargets.filter((record) => record.visible).map((record) => record.key));
    for (const { item, visible } of rowTargets) {
      const layout = storeLayout(item.file_path);
      for (const [index, [store]] of layout.stores.entries()) {
        const isAuto = layout.auto.has(store);
        // 수동 서점은 사용자가 검색을 누르기(source가 loading으로 생기기) 전에는 요청하지 않는다.
        if (!isAuto && !sources[item.file_path]?.[store]) continue;
        if (isAuto && index >= 4 && !isStoreVisible(item.file_path, store, layout)) continue;
        const source = sources[item.file_path]?.[store];
        if (source && source.status !== "loading") continue;
        tasks.push({ key: `search:${item.file_path}:${store}:${versions.get(item.file_path)}`, store, backgroundKey: visible ? undefined : item.file_path, run: async (signal) => {
          updateSource(item.file_path, store, { status: "loading", retrying: source?.retrying });
          try {
            const result = await coverAdminRequest(`/candidates/${store}?file_path=${encodeURIComponent(item.file_path)}${item.saved || item.has_cover ? "&include_covered=true" : ""}${source?.refresh ? "&refresh=true" : ""}`, { signal });
            if (!signal.aborted) updateSource(item.file_path, store, result);
          } catch (exc) {
            updateSource(item.file_path, store, signal.aborted ? source : { status: "error", error: exc.message });
          }
        } });
      }
    }
    const imageTargets = [];
    for (const { item, rect } of rowTargets) {
      const imageRecords = [];
      const layout = storeLayout(item.file_path);
      for (const [index, [store]] of layout.stores.entries()) {
        if (layout.auto.has(store) && index >= 4 && !isStoreVisible(item.file_path, store, layout)) continue;
        if (sources[item.file_path]?.[store]?.status === "loading") continue;
        for (const candidate of sources[item.file_path]?.[store]?.candidates || []) {
          const node = candidates.current.get(candidate.candidate_id);
          if (!node) continue;
          const clip = node.closest(".book-cover-candidates").getBoundingClientRect();
          imageRecords.push({ key: candidate.candidate_id, path: item.file_path, store, rect: node.getBoundingClientRect(), clip });
        }
      }
      // 검색한 하단 3권도 이미지까지 준비한다. 가로 스크롤의 노출 제한은 유지한다.
      imageTargets.push(...selectViewportTargets(imageRecords, { ...viewport, top: rect.top, bottom: rect.bottom }));
    }
    // 이미지 요청은 metadata 검색보다 먼저, 가까운 책과 왼쪽 후보부터 처리한다.
    const imageTasks = [];
    for (const record of imageTargets) {
      const existing = images[record.key];
      if (existing?.url || existing?.error) continue;
      imageTasks.push({ key: `image:${record.key}:${versions.get(record.path)}`, store: record.store, backgroundKey: visibleBooks.has(record.path) ? undefined : record.path, run: async (signal) => {
        if (!isCurrent(record.path)) return;
        setImages((previous) => ({ ...previous, [record.key]: { ...previous[record.key], loading: true } }));
        try {
          const blob = await coverAdminRequest(`/candidate-image/${record.key}`, { signal, image: true });
          if (signal.aborted || !isCurrent(record.path)) return;
          const url = URL.createObjectURL(blob);
          urls.current.set(record.key, url);
          setImages((previous) => ({ ...previous, [record.key]: { ...previous[record.key], url, blob, byte_size: blob.size, loading: false } }));
        } catch (exc) {
          if (isCurrent(record.path)) {
            setImages((previous) => ({ ...previous, [record.key]: signal.aborted ? undefined : { error: exc.message } }));
          }
        }
      } });
    }
    queue.current?.setTasks([...imageTasks, ...tasks]);
    // 긴 목록에서도 blob을 무제한으로 보관하지 않는다.
    const needed = new Set(imageTargets.map((record) => record.key));
    for (const item of items) if (item.selected) needed.add(`selected:${item.file_path}`);
    const evicted = [];
    if (urls.current.size > 80) {
      for (const [key, url] of urls.current) {
        if (needed.has(key)) continue;
        URL.revokeObjectURL(url);
        urls.current.delete(key);
        evicted.push(key);
        if (urls.current.size <= 80) break;
      }
    }
    if (evicted.length) setImages((previous) => {
      const next = { ...previous };
      evicted.forEach((key) => { next[key] = { ...next[key], url: undefined, blob: undefined }; });
      return next;
    });
  }, [items, sources, images, viewportTick]);

  const releaseSelection = (path) => {
    const key = `selected:${path}`;
    const url = urls.current.get(key);
    if (url) URL.revokeObjectURL(url);
    urls.current.delete(key);
  };
  const selectCover = (item, candidate) => {
    if (saving.current.has(item.file_path) || item.busy) return;
    const image = images[candidate.candidate_id];
    if (!image?.blob) return;
    releaseSelection(item.file_path);
    const url = URL.createObjectURL(image.blob);
    urls.current.set(`selected:${item.file_path}`, url);
    setZoom(null);
    previewState(item.file_path, { selected: { ...image, url, file: image.blob }, previewHidden: false, previewReady: false, error: "" });
  };
  const confirmCover = (item) => {
    if (item.selected) return save(item, { file: item.selected.file }, true);
    if (item.saved) setItems((previous) => previous.filter((entry) => entry.file_path !== item.file_path));
  };
  const save = async (item, candidate, removeRow = false) => {
    if (saving.current.has(item.file_path) || item.busy) return;
    setZoom(null);
    saving.current.add(item.file_path);
    const current = generation.current;
    const update = (changes) => {
      if (generation.current === current) setItems((previous) => previous.map((entry) => entry.file_path === item.file_path ? { ...entry, ...changes } : entry));
    };
    update({ saving: true, error: "" });
    try {
      if (candidate.file?.size > MAX_UPLOAD_BYTES) throw new Error("이미지는 15MB 이하여야 합니다");
      const hasCover = item.has_cover || !!item.saved;
      let path = candidate.url ? "/apply-url" : "/apply";
      const body = { file_path: item.file_path, revision: item.saved?.revision || item.revision };
      if (candidate.url) Object.assign(body, { url: candidate.url }, hasCover ? { replace_existing: true } : {});
      else if (candidate.file || hasCover) {
        path = "/apply-image";
        body.image_data = await new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(reader.result.split(",")[1]);
          reader.onerror = () => reject(new Error("표지 이미지를 읽을 수 없습니다"));
          reader.readAsDataURL(candidate.file || images[candidate.candidate_id].blob);
        });
        if (generation.current !== current) return {};
      }
      const result = await coverAdminRequest(path, { body });
      if (generation.current !== current) return {};
      releaseSelection(item.file_path);
      if (removeRow) {
        setItems((previous) => previous.filter((entry) => entry.file_path !== item.file_path));
        if (result.warning) setError(`${item.title}: ${result.warning}`);
      } else update({ saving: false, saved: result, selected: null, previewHidden: false, previewReady: false, paused: true, warning: result.warning });
      return {};
    } catch (exc) {
      update({ saving: false, error: exc.message });
      return { error: exc.message };
    } finally {
      saving.current.delete(item.file_path);
    }
  };

  const clearCandidates = async (item, repeat = false) => {
    if (saving.current.has(item.file_path) || item.busy) return;
    if (!repeat) {
      if (!window.confirm("선택한 표지와 미리보기를 비울까요? EPUB 파일은 변경하지 않습니다.")) return;
      setZoom(null);
      releaseSelection(item.file_path);
      previewState(item.file_path, { selected: null, previewHidden: true, previewReady: false, error: "", paused: false });
      if (item.saved || item.has_cover) setSources((previous) => ({ ...previous, [item.file_path]: Object.fromEntries(
        Object.entries(previous[item.file_path] || {}).map(([store, source]) => [store, { ...source, status: "loading" }]),
      ) }));
      return;
    }
    const current = generation.current;
    const path = item.file_path;
    setZoom(null);
    saving.current.add(path);
    bookVersions.current.set(path, (bookVersions.current.get(path) || 0) + 1);
    previewState(path, { busy: true, error: "" });
    try {
      await coverAdminRequest("/cache/clear", { body: { file_path: path, revision: item.saved?.revision || item.revision } });
      if (generation.current !== current) return;
      const ids = new Set(Object.values(sources[path] || {}).flatMap((source) => (source?.candidates || []).map((candidate) => candidate.candidate_id)));
      for (const id of ids) {
        const url = urls.current.get(id);
        if (url) URL.revokeObjectURL(url);
        urls.current.delete(id);
      }
      setImages((previous) => Object.fromEntries(Object.entries(previous).filter(([id]) => !ids.has(id))));
      setSources((previous) => ({ ...previous, [path]: Object.fromEntries(
        Object.entries(previous[path] || {}).map(([store]) => [store, { status: "loading", refresh: true, retrying: true }]),
      ) }));
      previewState(path, { busy: false, paused: !repeat });
    } catch (exc) {
      if (generation.current === current) previewState(path, { busy: false, error: exc.message });
    } finally {
      saving.current.delete(path);
    }
  };

  const previewState = (path, changes) => setItems((previous) => previous.map((item) => item.file_path === path ? { ...item, ...changes } : item));
  const searchStore = (item, store) => setSources((previous) => ({
    ...previous, [item.file_path]: { ...previous[item.file_path], [store]: { status: "loading" } },
  }));
  const retryStore = (item, store) => {
    bookVersions.current.set(item.file_path, (bookVersions.current.get(item.file_path) || 0) + 1);
    previewState(item.file_path, { paused: false });
    setSources((previous) => ({ ...previous, [item.file_path]: Object.fromEntries(
      Object.entries(previous[item.file_path] || {}).map(([name, source]) => [name, item.saved || item.has_cover || name === store
        ? { ...source, status: "loading", refresh: name === store, retrying: name === store } : source]),
    ) }));
  };
  const recordImageSize = (id, image) => {
    const width = image.naturalWidth;
    const height = image.naturalHeight;
    if (!width || !height) return;
    setImages((previous) => {
      if (previous[id]?.width === width && previous[id]?.height === height) return previous;
      return { ...previous, [id]: { ...previous[id], width, height } };
    });
  };

  return (
    <SplitLayout storageKey="split:book-cover-admin" className="book-cover-admin">
      <SplitLayout.Left className="book-cover-directory">
        <Folder folderData={folderData} expandedItems={expanded} onExpandedItemsChange={setExpanded}
          isOpen={folderOpen} onToggle={setFolderOpen} onClickHandler={(id) => {
            loadDirectories(id);
            if (id !== directory) setDirectory(id);
          }} />
      </SplitLayout.Left>
      <SplitLayout.Right className="book-cover-panel">
        <div className="book-cover-toolbar">
          <div><h2>책 표지 관리</h2><div className="book-cover-directory-mode">
            <small>{directory === ROOT ? "최상위 디렉토리" : directory}</small>
            <Form.Check type="switch" role="switch" id="book-cover-replace" label="표지 교체" className="m-0"
              checked={replaceCovers} disabled={items.some((item) => item.saving || item.busy)}
              onChange={(event) => setReplaceCovers(event.target.checked)} />
          </div></div>
          <Button size="sm" variant="outline-secondary" disabled={loading || items.some((item) => item.saving || item.busy)} onClick={() => {
            for (const item of items) releaseSelection(item.file_path);
            setItems([]);
            setSources({});
            setPage(null);
            loadPage(directory);
          }}>새로고침</Button>
        </div>
        <p className="text-muted small">서점 후보를 선택한 뒤 ‘내 책’ 표지를 클릭하면 EPUB에 저장하고 행을 숨깁니다.</p>
        {error && <Alert variant="danger">{error}</Alert>}
        {page && <p className="small text-muted">EPUB 검사 {page.scanned} / {page.total} · 현재 목록 {items.length}개</p>}
        <div aria-live="polite">
          {items.map((item) => {
            const hasCover = item.has_cover || !!item.saved || !!item.selected;
            const coverError = !item.saved && !item.selected && item.cover_error;
            const ownImage = !item.previewHidden && (item.selected || item.saved?.cover_image || item.cover_image);
            const ranked = rankCoverImages(Object.values(sources[item.file_path] || {}).flatMap((store) => store?.candidates || []), images);
            return (
            <article key={item.file_path} className={`book-cover-row${item.saved ? " is-saved" : ""}`}
              ref={(node) => { if (node) rows.current.set(item.file_path, node); else rows.current.delete(item.file_path); }}>
              <div className="book-cover-row-header">
                <h3>{item.title}</h3>
                <div className="book-cover-row-actions">
                  <Button size="sm" variant="outline-warning" disabled={!item.book_id}
                    onClick={() => window.open(`/book-edit/${item.book_id}?category=${encodeURIComponent(item.category || ROOT)}`, "_blank", "noopener")}>편집</Button>
                  <Button size="sm" variant="outline-primary" disabled={!item.book_id}
                    onClick={() => window.open(`/book-view/${item.book_id}?category=${encodeURIComponent(item.category || ROOT)}`, "_blank", "noopener")}>조회</Button>
                  <Button size="sm" variant="outline-secondary" disabled={item.saving || item.busy || item.status === "error"}
                    onClick={() => clearCandidates(item, true)}>재검색</Button>
                  <Button as="a" role="link" size="sm" variant="outline-secondary"
                    href={`https://www.google.com/search?tbm=isch&q=${encodeURIComponent(item.title)}`}
                    target="_blank" rel="noopener noreferrer">구글</Button>
                </div>
              </div>
              <div className="book-cover-filename">{item.file_path.split("/").pop()}</div>
              <div className="book-cover-row-body">
                <div className="book-cover-current">
                  <div className="book-cover-store-header"><h4>내 책</h4></div>
                  <div className="book-cover-image-frame">
                    <Button size="sm" variant="outline-secondary" className="book-cover-magnifier" title="표지 확대"
                      aria-label={`${item.title} 내 책 표지 확대`} disabled={!hasCover || item.previewHidden || !item.previewReady || item.saving || item.busy}
                      onClick={() => setZoom({ own: true, item, index: 0, previews: [{ id: `own:${item.file_path}`, title: `${item.title} 내 책`,
                        url: item.selected?.url || `${getApiUrlPrefix()}${API_ROOT}/preview?file_path=${encodeURIComponent(item.file_path)}&v=${encodeURIComponent(item.saved?.revision || item.revision)}&original=true`,
                      }] })}><FontAwesomeIcon icon={faMagnifyingGlass} /></Button>
                  {hasCover && !item.previewHidden && !coverError ? (
                    <button type="button" className={`cover-grid-cover ${item.saved || item.selected ? "book-cover-completed" : "book-cover-existing"}`} aria-label={`${item.title} ${item.selected ? "표지 저장 · 행 숨기기" : item.saved ? "표지 저장 완료 · 행 숨기기" : "현재 표지"}`}
                      disabled={!item.previewReady || (!item.saved && !item.selected) || item.saving || item.busy} onClick={() => confirmCover(item)}>
                      <img className="cover-grid-image" loading="lazy" alt={`${item.title} ${item.selected ? "선택한 표지" : item.saved ? "저장한 표지" : "현재 표지"}`}
                        {...(item.selected?.url ? { src: item.selected.url } : coverThumbProps(`${getApiUrlPrefix()}${API_ROOT}/preview?file_path=${encodeURIComponent(item.file_path)}&v=${encodeURIComponent(item.saved?.revision || item.revision)}`))}
                        onLoad={(event) => { markTallCover(event); previewState(item.file_path, { previewReady: true }); }}
                        onError={() => previewState(item.file_path, { previewReady: false, error: "저장한 표지를 표시하지 못했습니다. 새로고침하세요" })} />
                    </button>
                  ) : <div className="cover-grid-cover"><div className="cover-grid-placeholder">{coverError ? "표지 읽기 오류" : "표지 없음"}</div></div>}
                  </div>
                  <div className="book-cover-status">
                    <small className="book-cover-image-size">
                      <span>{ownImage?.width || "—"} × {ownImage?.height || "—"} px</span>
                      <span>{ownImage?.byte_size != null ? formatCoverBytes(ownImage.byte_size) : "— B"}</span>
                    </small>
                  </div>
                  <div className="book-cover-current-actions">
                    <Button size="sm" variant="outline-success" className="book-cover-save" title="표지 저장 후 행 숨기기"
                      aria-label={`${item.title} 저장 확인`} aria-busy={!!item.saving}
                      disabled={(!item.saved && !item.selected) || item.previewHidden || !item.previewReady || item.saving || item.busy}
                      onClick={() => confirmCover(item)}>
                      {item.saving ? <Spinner animation="border" size="sm" role="status" aria-label={`${item.title} 저장 중`} /> : <FontAwesomeIcon icon={faFloppyDisk} />}
                    </Button>
                    <Button size="sm" variant="outline-secondary" className="book-cover-trash" title="미리보기 지우기"
                      aria-label={`${item.title} 임시 표지 삭제`} disabled={item.saving || item.busy}
                      onClick={() => clearCandidates(item)}><FontAwesomeIcon icon={faTrash} /></Button>
                  </div>
                  {item.selected && <small>저장 전 · 내 책 표지를 클릭해 저장</small>}
                  {(!item.saved || item.previewHidden) && <Button size="sm" variant="outline-secondary" disabled={item.saving || item.busy || item.status === "error"}
                    onClick={() => { setZoom(null); setManual({ item }); }}>직접 입력</Button>}
                  {item.paused && !item.saved && <small>검색 중지</small>}
                </div>
                {item.status === "error" ? <Alert variant="warning">{item.error}</Alert> : (
                  <div className="book-cover-candidates" aria-label={`${item.title} 서점 표지 후보`}>
                    {storeLayout(item.file_path).stores.map(([store, label]) => {
                      const source = sources[item.file_path]?.[store];
                      const isAuto = storeLayout(item.file_path).auto.has(store);
                      const previews = (source?.candidates || []).filter((candidate) => images[candidate.candidate_id]?.url).map((candidate) => ({
                        id: candidate.candidate_id, title: candidate.title, candidate, url: images[candidate.candidate_id].url,
                        width: images[candidate.candidate_id].width || candidate.width,
                        height: images[candidate.candidate_id].height || candidate.height,
                      }));
                      return <section className="book-cover-store" key={store} data-store={store}>
                        <div className="book-cover-store-header"><h4>{label}</h4></div>
                        <div className="book-cover-store-images">
                          {(source?.candidates || []).map((candidate) => {
                            const image = images[candidate.candidate_id];
                            const rank = ranked[candidate.candidate_id];
                            const width = image?.width || candidate.width;
                            const height = image?.height || candidate.height;
                            const byteSize = image?.byte_size ?? candidate.byte_size;
                            return <div className="book-cover-candidate" key={candidate.candidate_id}>
                              <div className="book-cover-image-frame">
                              <Button size="sm" variant="outline-secondary" className="book-cover-magnifier" title="표지 확대"
                                aria-label={`${label} ${candidate.title} 표지 확대`} disabled={!image?.url || item.saving || item.busy}
                                onClick={() => setZoom({ item, previews, index: previews.findIndex((entry) => entry.id === candidate.candidate_id) })}><FontAwesomeIcon icon={faMagnifyingGlass} /></Button>
                              <button type="button" className="cover-grid-cover" disabled={!image?.url || item.saving || item.busy}
                                style={rank ? { backgroundColor: rank.color } : undefined}
                                title={rank ? `해상도 ${rank.rank}위 / ${rank.count}개` : "표지 크기 확인 전"}
                                aria-label={`${label} ${candidate.title} 표지 선택`} onClick={() => selectCover(item, candidate)}
                                ref={(node) => { if (node) candidates.current.set(candidate.candidate_id, node); else candidates.current.delete(candidate.candidate_id); }}>
                                {image?.url ? <img className="cover-grid-image" src={image.url} alt={candidate.title}
                                  onLoad={(event) => { markTallCover(event); recordImageSize(candidate.candidate_id, event.currentTarget); }} />
                                  : <div className="cover-grid-placeholder">{image?.loading ? <Spinner size="sm" /> : image?.error ? "이미지 오류" : "표지 후보"}</div>}
                              </button>
                              </div>
                              <small className="book-cover-image-size">
                                {width && height ? <><span>{width} × {height} px</span><span>{formatCoverBytes(byteSize)}</span></> : "크기 확인 전"}
                              </small>
                              <a className="book-cover-candidate-title" href={candidate.detail_url} target="_blank" rel="noopener noreferrer">{candidate.title}</a>
                              <small>{candidate.author}</small>
                              {image?.error && <button className="book-cover-retry" type="button" onClick={() => {
                                if (item.saved) retryStore(item, store);
                                else {
                                  bookVersions.current.set(item.file_path, (bookVersions.current.get(item.file_path) || 0) + 1);
                                  previewState(item.file_path, { paused: false });
                                }
                                setImages((previous) => ({ ...previous, [candidate.candidate_id]: undefined }));
                              }}>이미지 재시도</button>}
                            </div>;
                          })}
                        </div>
                        {((!source && isAuto) || source?.status === "loading") && <small role="status">{source ? "검색 중…" : "검색 대기"}</small>}
                        {!source && !isAuto && <button type="button" className="book-cover-retry" aria-label={`${item.title} ${label} 검색`}
                          disabled={item.saving || item.busy || item.paused} onClick={() => searchStore(item, store)}>검색</button>}
                        {source?.status === "loading" && source.retrying && <button type="button" className="book-cover-retry" disabled><Spinner size="sm" /> 재시도 중…</button>}
                        {source?.status === "not_found" && <small>후보 없음</small>}
                        {source?.status === "error" && <><small className="text-danger">{source.error}</small><button type="button" className="book-cover-retry" onClick={() => retryStore(item, store)}>재시도</button></>}
                        {source?.search_url && <a className="book-cover-search-link" href={source.search_url} target="_blank" rel="noopener noreferrer">서점 검색</a>}
                      </section>;
                    })}
                  </div>
                )}
              </div>
              {item.status !== "error" && item.error && <Alert variant="danger">{item.error}</Alert>}
              {coverError && <Alert variant="warning">{coverError}. 새 표지를 선택해 교체하세요.</Alert>}
              {item.warning && <Alert variant="warning">{item.warning}</Alert>}
            </article>
            );
          })}
        </div>
        <div ref={scanTrigger} className="book-cover-scan-trigger" aria-hidden="true" />
        {loading && <div role="status"><Spinner size="sm" /> EPUB 검사 중…</div>}
        {!loading && page && items.length === 0 && <Alert variant="info">{replaceCovers ? "검사한 EPUB에는 표지가 있는 책이 없습니다." : "검사한 EPUB에는 표지 없는 책이 없습니다."}</Alert>}
      </SplitLayout.Right>
      {manual && <ImageUrlModal
        idPrefix="book-cover"
        title="표지 직접 입력"
        description={<>{manual.item.title} · 공개 HTTP/HTTPS 이미지 URL을 입력하면 표지를 적용합니다.</>}
        urlLabel="표지 이미지 URL"
        submitLabel="표지 적용"
        busyLabel="적용 중…"
        fileDescription="15MB 이하의 이미지를 선택하면 EPUB 표지에 바로 적용합니다."
        onClose={() => setManual(null)}
        onSubmit={async (source) => {
          const current = generation.current;
          const result = await save(manual.item, typeof source === "string" ? { url: source } : { file: source });
          if (generation.current !== current) return;
          if (result?.error) return result.error;
          setManual(null);
        }}
      />}
      {zoom && <CoverImagePreview preview={{ ...zoom.previews[zoom.index], index: zoom.index, count: zoom.previews.length }}
        onClose={closeZoom} onNavigate={(index) => setZoom((previous) => ({ ...previous, index }))}
        onImageLoad={zoom.own ? (event) => {
          const { naturalWidth: width, naturalHeight: height } = event.currentTarget;
          setZoom((previous) => previous?.own ? { ...previous, previews: [{ ...previous.previews[0], width, height }] } : previous);
        } : undefined}
        onImageError={zoom.own ? () => { setZoom(null); previewState(zoom.item.file_path, { error: "저장한 표지를 확대하지 못했습니다" }); } : undefined}
        onSelect={zoom.own ? undefined : () => selectCover(zoom.item, zoom.previews[zoom.index].candidate)} />}
    </SplitLayout>
  );
}
