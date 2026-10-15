import { useCallback, useEffect, useRef, useState } from "react";
import { Alert, Button, Spinner, Modal, Form } from "react-bootstrap";
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
import "./CoverGrid.css";
import "./BookCoverAdmin.css";

const STORES = [
  ["yes24", "YES24"], ["aladin", "알라딘"], ["kyobo", "교보문고"],
  ["naver", "네이버쇼핑"], ["ridi", "RIDI"], ["munpia", "문피아"],
  ["naverseries", "시리즈"], ["joara", "조아라"],
];
// 디렉토리별로 자동 검색할 서점 그룹. 나머지 서점은 "검색" 버튼을 눌러야 검색한다.
const LIGHT_NOVEL_DIRECTORIES = new Set(["3_라이트노벨", "3_판타지", "3_무협", "3_여성향"]);
const AUTO_STORES = {
  lightNovel: new Set(["yes24", "aladin", "kyobo", "naver", "ridi"]),
  general: new Set(["munpia", "naverseries", "joara"]),
};
// 자동 검색 서점을 왼쪽에, 수동 검색 서점을 오른쪽에 둔다(각 그룹 안의 순서는 STORES 순서).
function storeLayout(filePath) {
  const [top, ...rest] = String(filePath).split("/");
  const auto = rest.length && LIGHT_NOVEL_DIRECTORIES.has(top) ? AUTO_STORES.lightNovel : AUTO_STORES.general;
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
    queue.current = createStoreQueue();
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
    const rowTargets = selectViewportTargets(eligibleRows, viewport);
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
    const imageRecords = [];
    for (const item of items.filter((entry) => !entry.saving && !entry.busy && !entry.paused)) {
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
    }
    const imageTargets = selectViewportTargets(imageRecords, viewport);
    // 이미지 요청은 metadata 검색보다 먼저 처리한다.
    for (const record of imageTargets) {
      const existing = images[record.key];
      if (existing?.url || existing?.error) continue;
      tasks.unshift({ key: `image:${record.key}:${versions.get(record.path)}`, store: record.store, backgroundKey: visibleBooks.has(record.path) ? undefined : record.path, run: async (signal) => {
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
    queue.current?.setTasks(tasks);
    // 긴 목록에서도 blob을 무제한으로 보관하지 않는다.
    const needed = new Set(imageTargets.map((record) => record.key));
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

  const save = async (item, candidate) => {
    if (saving.current.has(item.file_path) || item.busy) return;
    setZoom(null);
    saving.current.add(item.file_path);
    const current = generation.current;
    const update = (changes) => {
      if (generation.current === current) setItems((previous) => previous.map((entry) => entry.file_path === item.file_path ? { ...entry, ...changes } : entry));
    };
    update({ saving: true, error: "" });
    try {
      const hasCover = item.has_cover || !!item.saved;
      let path = candidate.url ? "/apply-url" : "/apply";
      const body = { file_path: item.file_path, revision: item.saved?.revision || item.revision };
      if (candidate.url) Object.assign(body, { url: candidate.url }, hasCover ? { replace_existing: true } : {});
      else if (hasCover) {
        path = "/apply-image";
        body.image_data = await new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(reader.result.split(",")[1]);
          reader.onerror = () => reject(new Error("표지 이미지를 읽을 수 없습니다"));
          reader.readAsDataURL(images[candidate.candidate_id].blob);
        });
        if (generation.current !== current) return {};
      } else body.candidate_id = candidate.candidate_id;
      const result = await coverAdminRequest(path, { body });
      update({ saving: false, saved: result, previewHidden: false, previewReady: false, paused: true, warning: result.warning });
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
      if (!window.confirm("미리보기만 비울까요? EPUB 표지는 다른 표지를 선택할 때 교체합니다.")) return;
      setZoom(null);
      previewState(item.file_path, { previewHidden: true, previewReady: false, error: "", paused: false });
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
            setItems([]);
            setSources({});
            setPage(null);
            loadPage(directory);
          }}>새로고침</Button>
        </div>
        <p className="text-muted small">{replaceCovers ? "후보를 선택하면 EPUB의 기존 표지를 교체합니다." : "후보를 선택하면 EPUB에 표지를 저장합니다."} 저장한 표지를 확인한 뒤 클릭하면 행을 숨깁니다.</p>
        {error && <Alert variant="danger">{error}</Alert>}
        {page && <p className="small text-muted">EPUB 검사 {page.scanned} / {page.total} · 현재 목록 {items.length}개</p>}
        <div aria-live="polite">
          {items.map((item) => {
            const hasCover = item.has_cover || !!item.saved;
            const ownImage = !item.previewHidden && (item.saved?.cover_image || item.cover_image);
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
                        url: `${getApiUrlPrefix()}${API_ROOT}/preview?file_path=${encodeURIComponent(item.file_path)}&v=${encodeURIComponent(item.saved?.revision || item.revision)}&original=true`,
                      }] })}><FontAwesomeIcon icon={faMagnifyingGlass} /></Button>
                  {hasCover && !item.previewHidden ? (
                    <button type="button" className={`cover-grid-cover ${item.saved ? "book-cover-completed" : "book-cover-existing"}`} aria-label={`${item.title} ${item.saved ? "표지 저장 완료 · 행 숨기기" : "현재 표지"}`}
                      disabled={!item.previewReady || !item.saved} onClick={() => setItems((previous) => previous.filter((entry) => entry.file_path !== item.file_path))}>
                      <img className="cover-grid-image" loading="lazy" alt={`${item.title} ${item.saved ? "저장한 표지" : "현재 표지"}`}
                        src={`${getApiUrlPrefix()}${API_ROOT}/preview?file_path=${encodeURIComponent(item.file_path)}&v=${encodeURIComponent(item.saved?.revision || item.revision)}`}
                        onLoad={() => previewState(item.file_path, { previewReady: true })}
                        onError={() => previewState(item.file_path, { previewReady: false, error: "저장한 표지를 표시하지 못했습니다. 새로고침하세요" })} />
                    </button>
                  ) : <div className="cover-grid-cover"><div className="cover-grid-placeholder">표지 없음</div></div>}
                  </div>
                  <div className="book-cover-status">
                    <small className="book-cover-image-size">
                      <span>{ownImage?.width || "—"} × {ownImage?.height || "—"} px</span>
                      <span>{ownImage?.byte_size != null ? formatCoverBytes(ownImage.byte_size) : "— B"}</span>
                    </small>
                  </div>
                  <div className="book-cover-current-actions">
                    <Button size="sm" variant="outline-success" className="book-cover-save" title="저장 확인 후 행 숨기기"
                      aria-label={`${item.title} 저장 확인`} aria-busy={!!item.saving}
                      disabled={!item.saved || item.previewHidden || !item.previewReady || item.saving || item.busy}
                      onClick={() => setItems((previous) => previous.filter((entry) => entry.file_path !== item.file_path))}>
                      {item.saving ? <Spinner animation="border" size="sm" role="status" aria-label={`${item.title} 저장 중`} /> : <FontAwesomeIcon icon={faFloppyDisk} />}
                    </Button>
                    <Button size="sm" variant="outline-secondary" className="book-cover-trash" title="미리보기 지우기"
                      aria-label={`${item.title} 임시 표지 삭제`} disabled={item.saving || item.busy}
                      onClick={() => clearCandidates(item)}><FontAwesomeIcon icon={faTrash} /></Button>
                  </div>
                  {(!item.saved || item.previewHidden) && <Button size="sm" variant="outline-secondary" disabled={item.saving || item.busy || item.status === "error"}
                    onClick={() => { setZoom(null); setManual({ item, url: "", error: "", busy: false }); }}>직접 입력</Button>}
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
                                aria-label={`${label} ${candidate.title} 표지 선택`} onClick={() => save(item, candidate)}
                                ref={(node) => { if (node) candidates.current.set(candidate.candidate_id, node); else candidates.current.delete(candidate.candidate_id); }}>
                                {image?.url ? <img className="cover-grid-image" src={image.url} alt={candidate.title}
                                  onLoad={(event) => recordImageSize(candidate.candidate_id, event.currentTarget)} />
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
              {item.warning && <Alert variant="warning">{item.warning}</Alert>}
            </article>
            );
          })}
        </div>
        <div ref={scanTrigger} className="book-cover-scan-trigger" aria-hidden="true" />
        {loading && <div role="status"><Spinner size="sm" /> EPUB 검사 중…</div>}
        {!loading && page && items.length === 0 && <Alert variant="info">{replaceCovers ? "검사한 EPUB에는 표지가 있는 책이 없습니다." : "검사한 EPUB에는 표지 없는 책이 없습니다."}</Alert>}
      </SplitLayout.Right>
      <Modal show={!!manual} onHide={() => { if (!manual?.busy) setManual(null); }} centered aria-labelledby="book-cover-url-title">
        <Modal.Header closeButton={!manual?.busy}><Modal.Title id="book-cover-url-title">표지 직접 입력</Modal.Title></Modal.Header>
        <Form onSubmit={async (event) => {
          event.preventDefault();
          if (!manual || manual.busy || !manual.url.trim()) return;
          const current = generation.current;
          setManual({ ...manual, busy: true, error: "" });
          const result = await save(manual.item, { url: manual.url.trim() });
          if (generation.current !== current) return;
          if (result?.error) setManual((previous) => ({ ...previous, busy: false, error: result.error }));
          else setManual(null);
        }}>
          <Modal.Body>
            <p>{manual?.item.title}</p>
            <Form.Group controlId="book-cover-url"><Form.Label>표지 이미지 URL</Form.Label>
              <Form.Control type="url" required autoFocus maxLength={4096} disabled={manual?.busy}
                value={manual?.url || ""} onChange={(event) => setManual((previous) => ({ ...previous, url: event.target.value }))} />
            </Form.Group>
            {manual?.error && <Alert variant="danger" className="mt-2">{manual.error}</Alert>}
          </Modal.Body>
          <Modal.Footer><Button variant="secondary" disabled={manual?.busy} onClick={() => setManual(null)}>취소</Button>
            <Button type="submit" disabled={manual?.busy || !manual?.url.trim()}>{manual?.busy ? "적용 중…" : "표지 적용"}</Button></Modal.Footer>
        </Form>
      </Modal>
      {zoom && <CoverImagePreview preview={{ ...zoom.previews[zoom.index], index: zoom.index, count: zoom.previews.length }}
        onClose={closeZoom} onNavigate={(index) => setZoom((previous) => ({ ...previous, index }))}
        onImageLoad={zoom.own ? (event) => {
          const { naturalWidth: width, naturalHeight: height } = event.currentTarget;
          setZoom((previous) => previous?.own ? { ...previous, previews: [{ ...previous.previews[0], width, height }] } : previous);
        } : undefined}
        onImageError={zoom.own ? () => { setZoom(null); previewState(zoom.item.file_path, { error: "저장한 표지를 확대하지 못했습니다" }); } : undefined}
        onSelect={zoom.own ? undefined : () => { setZoom(null); save(zoom.item, zoom.previews[zoom.index].candidate); }} />}
    </SplitLayout>
  );
}
