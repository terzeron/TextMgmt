import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup, within, act } from "@testing-library/react";
import BookCoverAdmin from "../src/BookCoverAdmin";

const { request } = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("../src/coverAdminApi", () => ({ coverAdminRequest: request }));
vi.mock("../src/Folder", () => ({ default: (props) => <div>디렉토리
  <button onClick={() => props.onClickHandler("other")}>다른 디렉토리</button>
  <button onClick={() => props.onClickHandler("_root")}>최상위 선택</button>
  <button onClick={() => props.onToggle(!props.isOpen)}>디렉토리 접기</button>
  <button onClick={() => props.onExpandedItemsChange([])}>디렉토리 축소</button>
  <span>{props.isOpen ? "열림" : "닫힘"}</span>
</div> }));

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); request.mockReset(); });

function setup({ fail = false } = {}) {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 200, left: 0, right: 100, width: 100, height: 200 });
  vi.stubGlobal("URL", { ...URL, createObjectURL: () => "blob:cover", revokeObjectURL: vi.fn() });
  request.mockImplementation(async (path) => {
    if (path.startsWith("/directories")) return [];
    if (path.startsWith("/missing")) return { items: [{ file_path: "책.epub", title: "테스트 책", status: "missing", revision: "r1", book_id: 123, category: "소설/한글 & 이야기" }], next_cursor: null, scanned: 1, total: 1 };
    if (path.startsWith("/candidates/yes24")) return { status: "success", candidates: [{ candidate_id: "a".repeat(64), title: "서점 책", author: "저자", detail_url: "https://www.yes24.com/book" }] };
    if (path.startsWith("/candidates/")) return { status: "not_found", candidates: [] };
    if (path.startsWith("/candidate-image")) return new Blob(["cover"], { type: "image/jpeg" });
    if (path === "/apply") {
      if (fail) throw new Error("저장 실패");
      return { revision: "r2" };
    }
    if (path === "/apply-url") return { revision: "r2" };
    if (path === "/apply-image") return { revision: "r3" };
    if (path === "/cache/clear") return { cleared: true };
    throw new Error(path);
  });
}

function overrideRequest(handler) {
  const original = request.getMockImplementation();
  request.mockImplementation((path, options) => handler(path, options, original));
}

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const storeOrder = (container) => [...container.querySelectorAll("[data-store]")].map((node) => node.getAttribute("data-store"));
const searchedStores = () => [...new Set(request.mock.calls.map(([path]) => path).filter((path) => path.startsWith("/candidates/")).map((path) => path.split("?")[0].split("/")[2]))];
function useBookPath(filePath) {
  overrideRequest((path, options, original) => path.startsWith("/missing")
    ? { items: [{ file_path: filePath, title: "테스트 책", status: "missing", revision: "r1", book_id: 123, category: "x" }], next_cursor: null, scanned: 1, total: 1 }
    : original(path, options));
}

describe("디렉토리별 자동·수동 서점 검색", () => {
  const GENERAL = ["yes24", "aladin", "kyobo", "naver", "ridi"];
  const WEB_NOVEL = ["munpia", "naverseries", "joara"];

  it.each(["3_라이트노벨", "3_판타지", "3_무협", "3_여성향"])("%s 아래 책은 웹소설 서점을 자동 검색하고 일반 서점은 수동으로 오른쪽에 둔다", async (directory) => {
    setup();
    useBookPath(`${directory}/하위/책.epub`);
    const { container } = render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    expect(storeOrder(container)).toEqual([...WEB_NOVEL, ...GENERAL]);
    await waitFor(() => expect(searchedStores()).toEqual(expect.arrayContaining(WEB_NOVEL)));
    for (const store of GENERAL) expect(searchedStores()).not.toContain(store);
    for (const label of ["YES24", "알라딘", "교보문고", "네이버쇼핑", "RIDI"]) expect(screen.getByRole("button", { name: `테스트 책 ${label} 검색` })).toBeTruthy();
    for (const label of ["문피아", "시리즈", "조아라"]) expect(screen.queryByRole("button", { name: `테스트 책 ${label} 검색` })).toBeNull();
  });

  it("일반 책은 일반 서점을 자동 검색하고 웹소설 서점은 수동으로 오른쪽에 둔다", async () => {
    setup();
    useBookPath("소설/책.epub");
    const { container } = render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    expect(storeOrder(container)).toEqual([...GENERAL, ...WEB_NOVEL]);
    await waitFor(() => expect(searchedStores()).toEqual(expect.arrayContaining(["yes24", "aladin", "kyobo", "naver"])));
    for (const store of WEB_NOVEL) expect(searchedStores()).not.toContain(store);
    for (const label of ["문피아", "시리즈", "조아라"]) expect(screen.getByRole("button", { name: `테스트 책 ${label} 검색` })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "테스트 책 YES24 검색" })).toBeNull();
  });

  it("디렉토리 이름과 같은 이름의 최상위 파일은 서브컬쳐 책으로 보지 않는다", async () => {
    setup();
    useBookPath("3_판타지.epub");
    const { container } = render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    expect(storeOrder(container).slice(0, 4)).toEqual(["yes24", "aladin", "kyobo", "naver"]);
  });

  it("수동 서점은 검색 버튼을 눌러야 요청하고 결과를 표시한다", async () => {
    setup();
    useBookPath("3_판타지/책.epub");
    render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    await waitFor(() => expect(searchedStores()).toContain("munpia"));
    expect(searchedStores()).not.toContain("yes24");
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 YES24 검색" }));
    await screen.findByAltText("서점 책");
    expect(searchedStores()).toContain("yes24");
    expect(screen.queryByRole("button", { name: "테스트 책 YES24 검색" })).toBeNull();
  });
});

describe("책 표지 관리", () => {
  it.each([false, true])("빈 페이지를 지나 160권 이후의 마지막 책까지 자동 검사한다: 교체 %s", async (covered) => {
    setup();
    const cursors = [];
    overrideRequest((path, options, original) => {
      if (!path.startsWith("/missing")) return original(path, options);
      const params = new URLSearchParams(path.split("?")[1]);
      if ((params.get("covered") === "true") !== covered) return { items: [], scanned: 0, total: 0 };
      const cursor = params.get("cursor");
      cursors.push(cursor);
      const scanned = Math.min(Number(cursor || 0) + 60, 241);
      return { items: scanned === 241 ? [{ file_path: "last.epub", title: "마지막 책", status: covered ? "covered" : "missing", has_cover: covered, revision: "r1" }] : [],
        scanned, total: 241, next_cursor: scanned < 241 ? String(scanned) : null };
    });
    render(<BookCoverAdmin />);
    if (covered) fireEvent.click(screen.getByRole("switch", { name: "표지 교체" }));
    await screen.findByRole("heading", { name: "마지막 책" });
    expect(cursors).toEqual(["", "60", "120", "180", "240"]);
    expect(screen.getByText("EPUB 검사 241 / 241 · 현재 목록 1개")).toBeTruthy();
    fireEvent.scroll(window);
    fireEvent.resize(window);
    await act(async () => new Promise((resolve) => requestAnimationFrame(resolve)));
    expect(cursors).toHaveLength(5);
  });

  it("추가 검사 중 디렉토리를 바꾸면 요청을 취소하고 늦은 실패를 무시한다", async () => {
    setup();
    const pending = deferred();
    let pendingSignal;
    overrideRequest(async (path, options, original) => {
      if (path.startsWith("/missing?directory=other")) return { items: [{ file_path: "other/new.epub", title: "새 디렉토리 책", status: "missing", revision: "r1" }], scanned: 1, total: 1 };
      if (path.startsWith("/missing") && path.includes("cursor=60")) { pendingSignal = options.signal; return pending.promise; }
      const result = await original(path, options);
      return path.startsWith("/missing") ? { ...result, next_cursor: "60", total: 180 } : result;
    });
    render(<BookCoverAdmin />);
    await waitFor(() => expect(pendingSignal).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "다른 디렉토리" }));
    await screen.findByRole("heading", { name: "새 디렉토리 책" });
    expect(pendingSignal.aborted).toBe(true);
    await act(async () => pending.reject(new Error("이전 디렉토리 검사 실패")));
    expect(screen.queryByText("이전 디렉토리 검사 실패")).toBeNull();
    expect(screen.queryByRole("heading", { name: "테스트 책" })).toBeNull();
    expect(screen.getByText("EPUB 검사 1 / 1 · 현재 목록 1개")).toBeTruthy();
  });

  it("카테고리가 없으면 편집과 조회 링크에 최상위 카테고리를 사용한다", async () => {
    setup();
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    overrideRequest(async (path, options, original) => {
      const result = await original(path, options);
      return path.startsWith("/missing") ? { ...result, items: result.items.map((item) => ({ ...item, category: undefined })) } : result;
    });
    render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    fireEvent.click(screen.getByRole("button", { name: "편집" }));
    fireEvent.click(screen.getByRole("button", { name: "조회" }));
    expect(open).toHaveBeenNthCalledWith(1, "/book-edit/123?category=_root", "_blank", "noopener");
    expect(open).toHaveBeenNthCalledWith(2, "/book-view/123?category=_root", "_blank", "noopener");
  });

  it("이미지 재시도가 연속 실패해도 다시 요청해서 표지를 표시한다", async () => {
    setup();
    let attempts = 0;
    overrideRequest((path, options, original) => {
      if (path.startsWith("/candidate-image") && ++attempts <= 2) return Promise.reject(new Error("이미지 다운로드 실패"));
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    fireEvent.click(await screen.findByRole("button", { name: "이미지 재시도" }));
    await waitFor(() => expect(attempts).toBe(2));
    fireEvent.click(await screen.findByRole("button", { name: "이미지 재시도" }));
    await screen.findByAltText("서점 책");
    expect(attempts).toBe(3);
    expect(screen.queryByRole("button", { name: "이미지 재시도" })).toBeNull();
    expect(request.mock.calls.some(([path]) => path === "/apply")).toBe(false);
  });

  it("디렉토리와 표지 패널을 SplitLayout으로 나누고 핸들을 제공한다", async () => {
    setup();
    const { container } = render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    expect(container.querySelector(".split-layout-left.book-cover-directory")).toBeTruthy();
    expect(container.querySelector(".split-layout-right.book-cover-panel")).toBeTruthy();
    expect(screen.getByRole("separator")).toBeTruthy();
  });

  it("좁은 화면에서는 SplitLayout이 쌓이고 핸들이 없다", async () => {
    setup();
    const initialWidth = window.innerWidth;
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 500 });
    try {
      const { container } = render(<BookCoverAdmin />);
      await screen.findByRole("heading", { name: "테스트 책" });
      expect(container.querySelector(".split-layout.is-stacked")).toBeTruthy();
      expect(screen.queryByRole("separator")).toBeNull();
    } finally {
      Object.defineProperty(window, "innerWidth", { configurable: true, value: initialWidth });
    }
  });

  it("빈 표지도 크기 자리와 저장 아이콘을 항상 표시하고 휴지통 왼쪽에 배치한다", async () => {
    setup();
    render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    const save = screen.getByRole("button", { name: "테스트 책 저장 확인" });
    const trash = screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" });
    const own = save.closest(".book-cover-current");
    expect(within(own).getByText("— × — px")).toBeTruthy();
    expect(within(own).getByText("— B")).toBeTruthy();
    expect(screen.queryByText("서점 표지를 선택하세요")).toBeNull();
    expect(save.disabled).toBe(true);
    expect(save.nextElementSibling).toBe(trash);
    expect(save.querySelector('svg[data-icon="floppy-disk"]')).toBeTruthy();
    expect(save.querySelector(".spinner-border")).toBeNull();
    fireEvent.click(save);
    expect(screen.getByRole("heading", { name: "테스트 책" })).toBeTruthy();
  });

  it("교체 모드에서 기존 표지의 실제 크기를 저장 전부터 표시한다", async () => {
    setup();
    overrideRequest((path, options, original) => path.startsWith("/missing") ? {
      items: [{ file_path: "책.epub", title: "테스트 책", status: "covered", has_cover: true, revision: "r1", cover_image: { width: 1200, height: 1800, byte_size: 20480 } }], scanned: 1, total: 1,
    } : original(path, options));
    render(<BookCoverAdmin />);
    const image = await screen.findByAltText("테스트 책 현재 표지");
    const own = image.closest(".book-cover-current");
    expect(within(own).getByText("1200 × 1800 px")).toBeTruthy();
    expect(within(own).getByText("20.0 KB")).toBeTruthy();
    fireEvent.load(image);
    expect(within(own).getByRole("button", { name: "테스트 책 저장 확인" }).disabled).toBe(true);
  });

  it("파일명 제목을 헤딩과 구글 검색어에 동일하게 사용한다", async () => {
    setup();
    const filePath = "3_SF그리폰북스/[랜들 개릿] 다아시 경의 모험.epub";
    const title = "랜들 개릿 다아시 경의 모험";
    overrideRequest(async (path, options, original) => {
      const result = await original(path, options);
      return path.startsWith("/missing") ? { ...result, items: result.items.map((item) => ({ ...item, file_path: filePath, title })) } : result;
    });
    render(<BookCoverAdmin />);
    expect(await screen.findByRole("heading", { name: title })).toBeTruthy();
    const query = new URLSearchParams(screen.getByRole("link", { name: "구글" }).getAttribute("href").split("?")[1]);
    expect(query.get("q")).toBe(title);
    expect(screen.getByText("[랜들 개릿] 다아시 경의 모험.epub")).toBeTruthy();
  });

  it("저장한 이미지의 실제 크기를 표시하고 저장 확인 아이콘으로 행을 숨긴다", async () => {
    setup();
    const pending = deferred();
    overrideRequest((path, options, original) => path === "/apply" ? pending.promise : original(path, options));
    render(<BookCoverAdmin />);
    const choice = await screen.findByRole("button", { name: "YES24 서점 책 표지 선택" });
    await waitFor(() => expect(choice.disabled).toBe(false));
    fireEvent.click(choice);
    const status = await screen.findByRole("status", { name: "테스트 책 저장 중" });
    expect(status.closest("button").disabled).toBe(true);
    expect(status.classList.contains("spinner-border")).toBe(true);
    await act(async () => pending.resolve({ revision: "r2", cover_image: { width: 600, height: 900, byte_size: 12345 } }));
    const own = screen.getByAltText("테스트 책 저장한 표지").closest(".book-cover-current");
    expect(within(own).getByText("600 × 900 px")).toBeTruthy();
    expect(within(own).getByText("12.1 KB")).toBeTruthy();
    expect(screen.queryByText("저장 완료 · 표지를 눌러 숨기기")).toBeNull();
    const confirm = within(own).getByRole("button", { name: "테스트 책 저장 확인" });
    expect(confirm.disabled).toBe(true);
    fireEvent.load(screen.getByAltText("테스트 책 저장한 표지"));
    expect(confirm.disabled).toBe(false);
    expect(within(own).queryByRole("status")).toBeNull();
    expect(confirm.querySelector('svg[data-icon="floppy-disk"]')).toBeTruthy();
    expect(confirm.parentElement).toBe(within(own).getByRole("button", { name: "테스트 책 임시 표지 삭제" }).parentElement);
    fireEvent.click(confirm);
    expect(screen.queryByRole("heading", { name: "테스트 책" })).toBeNull();
    expect(request.mock.calls.filter(([path]) => path === "/apply")).toHaveLength(1);
  });

  it("저장한 미리보기를 지우면 크기를 비우고 저장 버튼을 비활성화하며 후보를 유지한다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    overrideRequest((path, options, original) => path === "/apply" ? { revision: "r2", cover_image: { width: 600, height: 900, byte_size: 512 } } : original(path, options));
    render(<BookCoverAdmin />);
    const choice = await screen.findByRole("button", { name: "YES24 서점 책 표지 선택" });
    await waitFor(() => expect(choice.disabled).toBe(false));
    fireEvent.click(choice);
    await screen.findByText("600 × 900 px");
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" }));
    expect(screen.queryByText("600 × 900 px")).toBeNull();
    expect(screen.getByRole("button", { name: "테스트 책 저장 확인" }).disabled).toBe(true);
    expect(screen.getByText("— × — px")).toBeTruthy();
    expect(screen.queryByText("서점 표지를 선택하세요")).toBeNull();
    expect(screen.getByRole("button", { name: "YES24 서점 책 표지 선택" })).toBeTruthy();
    expect(request.mock.calls.some(([path]) => path === "/cache/clear")).toBe(false);
  });

  it("재검색 오른쪽 구글 버튼은 제목의 한글과 특수문자를 보존해 이미지 검색을 새 탭에서 연다", async () => {
    setup();
    const title = "구운몽 & 꿈 #1 + 이야기?";
    overrideRequest(async (path, options, original) => {
      const result = await original(path, options);
      return path.startsWith("/missing") ? { ...result, items: result.items.map((item) => ({ ...item, title })) } : result;
    });
    render(<BookCoverAdmin />);
    const link = await screen.findByRole("link", { name: "구글" });
    expect(screen.getByRole("button", { name: "재검색" }).nextElementSibling).toBe(link);
    const href = link.getAttribute("href");
    expect(href.startsWith("https://www.google.com/search?")).toBe(true);
    const query = new URLSearchParams(href.split("?")[1]);
    expect(query.get("tbm")).toBe("isch");
    expect(query.get("q")).toBe(title);
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
  });

  it("카테고리 옆 토글로 기존 표지를 조회하고 후보로 교체한 뒤 빈 표지 모드로 돌아간다", async () => {
    setup();
    const pending = deferred();
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing")) return { items: path.includes("covered=true") ? [{ file_path: "책.epub", title: "기존 표지 책", status: "covered", has_cover: true, revision: "r1" }] : [], total: 1, scanned: 1 };
      if (path === "/apply-image") return pending.promise;
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await screen.findByText("검사한 EPUB에는 표지 없는 책이 없습니다.");
    const toggle = screen.getByRole("switch", { name: "표지 교체" });
    expect(toggle.checked).toBe(false);
    expect(within(toggle.closest(".book-cover-directory-mode")).getByText("최상위 디렉토리")).toBeTruthy();
    fireEvent.click(toggle);
    const current = await screen.findByAltText("기존 표지 책 현재 표지");
    fireEvent.load(current);
    expect(toggle.checked).toBe(true);
    expect(screen.queryByRole("button", { name: /표지 저장 완료/ })).toBeNull();
    expect(screen.getByRole("button", { name: "기존 표지 책 내 책 표지 확대" }).disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "기존 표지 책 내 책 표지 확대" }));
    expect(screen.getByAltText("기존 표지 책 내 책 확대 표지").getAttribute("src")).toContain("v=r1&original=true");
    fireEvent.click(screen.getByRole("button", { name: "확대 닫기" }));
    const candidate = await screen.findByRole("button", { name: "YES24 서점 책 표지 선택" });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    expect(request.mock.calls.some(([path]) => path.startsWith("/candidates/yes24") && path.includes("include_covered=true"))).toBe(true);
    fireEvent.click(candidate);
    await waitFor(() => expect(request).toHaveBeenCalledWith("/apply-image", { body: { file_path: "책.epub", revision: "r1", image_data: "Y292ZXI=" } }));
    expect(toggle.disabled).toBe(true);
    await act(async () => pending.resolve({ revision: "r2" }));
    const completed = await screen.findByAltText("기존 표지 책 저장한 표지");
    expect(completed.getAttribute("src")).toContain("v=r2");
    expect(toggle.disabled).toBe(false);
    fireEvent.click(toggle);
    await screen.findByText("검사한 EPUB에는 표지 없는 책이 없습니다.");
    expect(screen.queryByRole("heading", { name: "기존 표지 책" })).toBeNull();
  });

  it("기존 표지 모드에서도 미리보기만 지우고 직접 URL로 표지를 교체한다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    overrideRequest((path, options, original) => path.startsWith("/missing")
      ? { items: path.includes("covered=true") ? [{ file_path: "책.epub", title: "기존 표지 책", status: "covered", has_cover: true, revision: "r1" }] : [], scanned: 1, total: 1 }
      : original(path, options));
    render(<BookCoverAdmin />);
    fireEvent.click(screen.getByRole("switch", { name: "표지 교체" }));
    await screen.findByAltText("기존 표지 책 현재 표지");
    await screen.findByAltText("서점 책");
    fireEvent.click(screen.getByRole("button", { name: "기존 표지 책 임시 표지 삭제" }));
    expect(screen.queryByAltText("기존 표지 책 현재 표지")).toBeNull();
    expect(request.mock.calls.some(([path]) => path === "/cache/clear")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "직접 입력" }));
    fireEvent.change(screen.getByLabelText("표지 이미지 URL"), { target: { value: "https://example.org/new.jpg" } });
    fireEvent.click(screen.getByRole("button", { name: "표지 적용" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("/apply-url", { body: { file_path: "책.epub", revision: "r1", url: "https://example.org/new.jpg", replace_existing: true } }));
  });

  it("표지 모드를 바꾸면 이전 검사 요청을 취소하고 늦은 결과를 무시한다", async () => {
    setup();
    const pending = deferred();
    let previousSignal;
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing")) {
        if (path.includes("covered=true")) return { items: [], scanned: 0, total: 0 };
        previousSignal = options.signal;
        return pending.promise;
      }
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await waitFor(() => expect(previousSignal).toBeTruthy());
    fireEvent.click(screen.getByRole("switch", { name: "표지 교체" }));
    await screen.findByText("검사한 EPUB에는 표지가 있는 책이 없습니다.");
    expect(previousSignal.aborted).toBe(true);
    await act(async () => pending.resolve({ items: [{ file_path: "old.epub", title: "지난 목록", status: "missing" }], scanned: 1, total: 1 }));
    expect(screen.queryByRole("heading", { name: "지난 목록" })).toBeNull();
  });

  it("교체 모드의 자동 다음 검사와 디렉토리 전환 및 새로고침에도 조회 조건을 유지한다", async () => {
    setup();
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing")) {
        if (!path.includes("covered=true")) return { items: [], scanned: 0, total: 0 };
        if (path.includes("directory=other")) return { items: [], scanned: 1, total: 1 };
        if (!path.includes("cursor=60")) return { items: [], scanned: 60, total: 61, next_cursor: "60" };
        return { items: [{ file_path: "책.epub", title: "뒤 페이지 책", status: "covered", has_cover: true, revision: "r1" }], scanned: 61, total: 61 };
      }
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    fireEvent.click(screen.getByRole("switch", { name: "표지 교체" }));
    await screen.findByAltText("뒤 페이지 책 현재 표지");
    expect(request.mock.calls.some(([path]) => path.startsWith("/missing") && path.includes("cursor=60") && path.includes("covered=true"))).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "다른 디렉토리" }));
    await screen.findByText("검사한 EPUB에는 표지가 있는 책이 없습니다.");
    expect(screen.getByRole("switch", { name: "표지 교체" }).checked).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "새로고침" }));
    await waitFor(() => expect(request.mock.calls.filter(([path]) => path.startsWith("/missing?directory=other") && path.includes("covered=true"))).toHaveLength(2));
  });

  it("기존 표지 교체에 실패하면 현재 표지와 revision을 유지한다", async () => {
    setup();
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing")) return { items: path.includes("covered=true") ? [{ file_path: "책.epub", title: "기존 표지 책", status: "covered", has_cover: true, revision: "r1" }] : [], scanned: 1, total: 1 };
      if (path === "/apply-image") throw new Error("EPUB이 변경되었습니다");
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    fireEvent.click(screen.getByRole("switch", { name: "표지 교체" }));
    await screen.findByAltText("서점 책");
    const current = screen.getByAltText("기존 표지 책 현재 표지");
    fireEvent.load(current);
    fireEvent.click(screen.getByRole("button", { name: "YES24 서점 책 표지 선택" }));
    await screen.findByText("EPUB이 변경되었습니다");
    expect(screen.getByAltText("기존 표지 책 현재 표지").getAttribute("src")).toContain("v=r1");
    expect(screen.queryByRole("button", { name: /표지 저장 완료/ })).toBeNull();
    expect(screen.getByRole("switch", { name: "표지 교체" }).disabled).toBe(false);
  });

  it("각 후보 이미지 안의 돋보기는 그 후보를 확대하고 EPUB 저장을 실행하지 않는다", async () => {
    setup();
    const second = { candidate_id: "b".repeat(64), title: "두 번째 표지" };
    overrideRequest(async (path, options, original) => {
      const result = await original(path, options);
      return path.startsWith("/candidates/yes24") ? { ...result, candidates: [...result.candidates, second] } : result;
    });
    render(<BookCoverAdmin />);
    await screen.findByAltText("두 번째 표지");
    const magnifier = screen.getByRole("button", { name: "YES24 두 번째 표지 표지 확대" });
    const frame = magnifier.closest(".book-cover-image-frame");
    expect(frame).not.toBeNull();
    expect(within(frame).getByRole("button", { name: "YES24 두 번째 표지 표지 선택" })).toBeTruthy();
    expect(magnifier.closest(".book-cover-store-header")).toBeNull();
    expect(magnifier.closest("button")).toBe(magnifier);
    const requests = request.mock.calls.filter(([path]) => path.startsWith("/candidate-image")).length;
    fireEvent.click(magnifier);
    expect(screen.getByRole("dialog", { name: "두 번째 표지 표지 확대" })).toBeTruthy();
    expect(request.mock.calls.some(([path]) => path === "/apply")).toBe(false);
    expect(request.mock.calls.filter(([path]) => path.startsWith("/candidate-image"))).toHaveLength(requests);
    fireEvent.click(screen.getByRole("button", { name: "두 번째 표지 확대 표지 선택" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("/apply", { body: { file_path: "책.epub", revision: "r1", candidate_id: second.candidate_id } }));
  });

  it("초기 빈 서점 칸이 잠깐 노출돼도 앞쪽 검색으로 레이아웃이 정해지기 전에는 뒤쪽을 검색하지 않는다", async () => {
    setup();
    const pending = deferred();
    const lateStores = ["ridi"];
    let expanded = false;
    let exposed = false;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function () {
      const store = this.getAttribute("data-store") || this.closest(".book-cover-store")?.getAttribute("data-store");
      const left = lateStores.includes(store) && expanded && !exposed ? 600 : 20;
      return { top: 0, bottom: 200, left, right: left + (this.classList.contains("book-cover-candidates") ? 400 : 100), width: 100, height: 200 };
    });
    overrideRequest(async (path, options, original) => {
      if (["yes24", "aladin", "kyobo", "naver"].some((store) => path.startsWith(`/candidates/${store}?`))) await pending.promise;
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await waitFor(() => expect(request.mock.calls.filter(([path]) => path.startsWith("/candidates/")).length).toBeGreaterThanOrEqual(4));
    const lateSearches = () => request.mock.calls.filter(([path]) => lateStores.some((store) => path.startsWith(`/candidates/${store}?`)));
    expect(lateSearches()).toHaveLength(0);
    expanded = true;
    await act(async () => pending.resolve());
    await screen.findByAltText("서점 책");
    expect(lateSearches()).toHaveLength(0);
    exposed = true;
    fireEvent.scroll(screen.getByLabelText("테스트 책 서점 표지 후보"));
    await waitFor(() => expect(lateSearches()).toHaveLength(1));
  });

  it("앞쪽 검색이 대기 중이어도 사용자가 가로로 스크롤해 노출한 뒤쪽 서점은 검색한다", async () => {
    setup();
    const pending = deferred();
    overrideRequest((path, options, original) => path.startsWith("/candidates/yes24?") ? pending.promise : original(path, options));
    render(<BookCoverAdmin />);
    await waitFor(() => expect(request.mock.calls.some(([path]) => path.startsWith("/candidates/yes24?"))).toBe(true));
    const scrollArea = screen.getByLabelText("테스트 책 서점 표지 후보");
    Object.defineProperty(scrollArea, "scrollLeft", { configurable: true, value: 100 });
    fireEvent.scroll(scrollArea);
    await waitFor(() => expect(request.mock.calls.some(([path]) => path.startsWith("/candidates/ridi?"))).toBe(true));
    await act(async () => pending.resolve({ status: "not_found", candidates: [] }));
  });

  it("오른쪽 서점은 가로로 노출될 때 검색과 이미지를 받고 재노출 때 재사용한다", async () => {
    setup();
    let exposed = "";
    const lateStores = ["ridi"];
    const ids = Object.fromEntries(lateStores.map((store, index) => [store, String(index + 1).repeat(64)]));
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function () {
      const store = this.getAttribute("data-store") || this.closest(".book-cover-store")?.getAttribute("data-store");
      const left = lateStores.includes(store) && store !== exposed ? 600 : 20;
      return { top: 0, bottom: 200, left, right: left + (this.classList.contains("book-cover-candidates") ? 400 : 100), width: 100, height: 200 };
    });
    overrideRequest((path, options, original) => {
      const store = path.split("/candidates/")[1]?.split("?")[0];
      return lateStores.includes(store) ? { status: "success", candidates: [{ candidate_id: ids[store], title: `${store} 후보` }] } : original(path, options);
    });
    render(<BookCoverAdmin />);
    await screen.findByAltText("서점 책");
    const searches = (store) => request.mock.calls.filter(([path]) => path.startsWith(`/candidates/${store}?`));
    expect(["yes24", "aladin", "kyobo", "naver"].map((store) => searches(store).length)).toEqual([1, 1, 1, 1]);
    expect(lateStores.map((store) => searches(store).length)).toEqual([0]);
    const scrollArea = screen.getByLabelText("테스트 책 서점 표지 후보");
    for (const store of lateStores) {
      exposed = store;
      fireEvent.scroll(scrollArea);
      await screen.findByAltText(`${store} 후보`);
      expect(searches(store)).toHaveLength(1);
      expect(request.mock.calls.filter(([path]) => path === `/candidate-image/${ids[store]}`)).toHaveLength(1);
    }
    exposed = "ridi";
    fireEvent.scroll(scrollArea);
    await act(async () => new Promise((resolve) => requestAnimationFrame(resolve)));
    expect(lateStores.map((store) => searches(store).length)).toEqual([1]);
    exposed = "";
    fireEvent.scroll(scrollArea);
    fireEvent.click(screen.getByRole("button", { name: "재검색" }));
    await waitFor(() => expect(searches("yes24")).toHaveLength(2));
    expect(lateStores.map((store) => searches(store).length)).toEqual([1]);
    exposed = "ridi";
    fireEvent.scroll(scrollArea);
    await waitFor(() => expect(searches("ridi")).toHaveLength(2));
  });

  it("스크롤 영역 안이어도 브라우저 오른쪽 밖에 있는 서점은 resize로 노출될 때 검색한다", async () => {
    setup();
    const initialWidth = window.innerWidth;
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 500 });
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function () {
      const store = this.getAttribute("data-store") || this.closest(".book-cover-store")?.getAttribute("data-store");
      const left = ["ridi", "munpia", "naverseries", "joara"].includes(store) ? 600 : 0;
      return { top: 0, bottom: 200, left, right: this.classList.contains("book-cover-candidates") ? 900 : left + 100, width: 100, height: 200 };
    });
    try {
      render(<BookCoverAdmin />);
      await screen.findByAltText("서점 책");
      expect(request.mock.calls.some(([path]) => path.startsWith("/candidates/ridi"))).toBe(false);
      Object.defineProperty(window, "innerWidth", { configurable: true, value: 1000 });
      fireEvent.resize(window);
      await waitFor(() => expect(request.mock.calls.some(([path]) => path.startsWith("/candidates/ridi"))).toBe(true));
    } finally {
      Object.defineProperty(window, "innerWidth", { configurable: true, value: initialWidth });
    }
  });

  it("내 책 확대에 실패하면 모달을 닫고 저장한 표지와 행을 유지한다", async () => {
    setup();
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    fireEvent.load(screen.getByAltText("테스트 책 저장한 표지"));
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 내 책 표지 확대" }));
    fireEvent.error(screen.getByAltText("테스트 책 내 책 확대 표지"));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByText("저장한 표지를 확대하지 못했습니다")).toBeTruthy();
    expect(screen.getByAltText("테스트 책 저장한 표지")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "테스트 책" })).toBeTruthy();
  });
  it("내 책 레이블과 돋보기는 저장한 EPUB 표지를 확대하고 재저장하지 않는다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<BookCoverAdmin />);
    expect(await screen.findByRole("heading", { name: "내 책" })).toBeTruthy();
    const magnifier = screen.getByRole("button", { name: "테스트 책 내 책 표지 확대" });
    expect(magnifier.disabled).toBe(true);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    fireEvent.load(screen.getByAltText("테스트 책 저장한 표지"));
    expect(magnifier.disabled).toBe(false);
    fireEvent.click(magnifier);
    const modal = screen.getByRole("dialog", { name: "테스트 책 내 책 표지 확대" });
    const enlarged = within(modal).getByAltText("테스트 책 내 책 확대 표지");
    expect(enlarged.getAttribute("src")).toContain("original=true");
    Object.defineProperties(enlarged, { naturalWidth: { value: 600 }, naturalHeight: { value: 900 } });
    fireEvent.load(enlarged);
    expect(within(modal).getByText("원본 크기 · 600 × 900 px")).toBeTruthy();
    expect(within(modal).queryByRole("button", { name: /확대 표지 선택/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "확대 닫기" }));
    expect(request.mock.calls.filter(([path]) => path === "/apply")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" }));
    expect(magnifier.disabled).toBe(true);
  });
  it("재검색 전의 늦은 서점 결과로 후보를 되돌리거나 이미지를 요청하지 않는다", async () => {
    setup();
    const pending = deferred();
    const freshId = "b".repeat(64);
    overrideRequest((path, options, original) => {
      if (path.startsWith("/candidates/yes24")) return path.includes("refresh=true")
        ? { status: "success", candidates: [{ candidate_id: freshId, title: "새로운 후보", detail_url: "https://www.yes24.com/new" }] }
        : pending.promise;
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await waitFor(() => expect(request.mock.calls.some(([path]) => path.startsWith("/candidates/yes24"))).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "재검색" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("/cache/clear", { body: { file_path: "책.epub", revision: "r1" } }));
    await act(async () => pending.resolve({ status: "success", candidates: [{ candidate_id: "a".repeat(64), title: "지난 후보" }] }));
    await screen.findByAltText("새로운 후보");
    expect(screen.queryByText("지난 후보")).toBeNull();
    expect(request.mock.calls.filter(([path]) => path.startsWith("/candidate-image/")).map(([path]) => path)).toEqual([`/candidate-image/${freshId}`]);
  });

  it("휴지통 확인을 취소하면 저장 revision과 후보를 그대로 사용해 교체한다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" }));
    expect(screen.getByAltText("테스트 책 저장한 표지").getAttribute("src")).toContain("v=r2");
    fireEvent.click(candidate);
    await waitFor(() => expect(request).toHaveBeenCalledWith("/apply-image", { body: { file_path: "책.epub", revision: "r2", image_data: "Y292ZXI=" } }));
  });

  it("교체 후 직접 URL을 적용하면 최신 revision으로 다시 교체한다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    fireEvent.click(candidate);
    await waitFor(() => expect(screen.getByAltText("테스트 책 저장한 표지").getAttribute("src")).toContain("v=r3"));
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" }));
    fireEvent.click(screen.getByRole("button", { name: "직접 입력" }));
    fireEvent.change(screen.getByLabelText("표지 이미지 URL"), { target: { value: "https://example.org/new.jpg" } });
    fireEvent.click(screen.getByRole("button", { name: "표지 적용" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("/apply-url", { body: {
      file_path: "책.epub", revision: "r3", url: "https://example.org/new.jpg", replace_existing: true,
    } }));
  });

  it("빠른 표지 선택 반복은 교체 이미지를 읽는 중 중복 요청을 만들지 않는다", async () => {
    setup();
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    const readers = [];
    vi.stubGlobal("FileReader", class { readAsDataURL() { readers.push(this); } });
    fireEvent.click(candidate);
    fireEvent.click(candidate);
    expect(readers).toHaveLength(1);
    expect(screen.getByRole("button", { name: "재검색" }).disabled).toBe(true);
    await act(async () => { readers[0].result = "data:image/jpeg;base64,Y292ZXI="; readers[0].onload(); });
    await waitFor(() => expect(request.mock.calls.filter(([path]) => path === "/apply-image")).toHaveLength(1));
  });
  it("저장 완료 이후 이미지 재시도도 검색을 재개하고 이미지를 표시한다", async () => {
    setup();
    let available = false;
    overrideRequest((path, options, original) => path.startsWith("/candidate-image") && !available ? Promise.reject(new Error("이미지 실패")) : original(path, options));
    render(<BookCoverAdmin />);
    await screen.findByText("이미지 재시도");
    fireEvent.click(screen.getByRole("button", { name: "직접 입력" }));
    fireEvent.change(screen.getByLabelText("표지 이미지 URL"), { target: { value: "https://example.org/a" } });
    fireEvent.click(screen.getByRole("button", { name: "표지 적용" }));
    await screen.findByAltText("테스트 책 저장한 표지");
    available = true;
    fireEvent.click(screen.getByText("이미지 재시도"));
    await screen.findByAltText("서점 책");
    expect(screen.getByRole("button", { name: /YES24.*서점 책.*표지 선택/ }).disabled).toBe(false);
  });
  it("저장 이후 재검색한 후보 ID가 바뀌어도 받은 이미지를 유지한다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    overrideRequest(async (path, options, original) => {
      const result = await original(path, options);
      return path.startsWith("/candidates/yes24") && path.includes("include_covered=true")
        ? { ...result, candidates: result.candidates.map((candidate) => ({ ...candidate, candidate_id: "b".repeat(64) })) }
        : result;
    });
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    const before = request.mock.calls.filter(([path]) => path.startsWith("/candidate-image")).length;
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" }));
    await waitFor(() => expect(document.querySelector('[aria-label="YES24 서점 책 표지 선택"]')).not.toBe(candidate));
    expect(screen.getByAltText("서점 책").getAttribute("src")).toBe("blob:cover");
    expect(request.mock.calls.filter(([path]) => path.startsWith("/candidate-image"))).toHaveLength(before);
    expect(URL.revokeObjectURL).not.toHaveBeenCalled();
  });

  it.each(["reader", "server"])("표지 교체의 %s 실패는 기존 저장 상태를 유지한다", async (failure) => {
    setup();
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    if (failure === "reader") vi.stubGlobal("FileReader", class { readAsDataURL() { this.onerror(); } });
    else overrideRequest((path, options, original) => path === "/apply-image" ? Promise.reject(new Error("교체 실패")) : original(path, options));
    fireEvent.click(candidate);
    await screen.findByText(failure === "reader" ? "표지 이미지를 읽을 수 없습니다" : "교체 실패");
    expect(screen.getByAltText("테스트 책 저장한 표지").getAttribute("src")).toContain("v=r2");
  });

  it("교체 이미지를 읽는 중 디렉토리가 바뀌면 이전 EPUB을 갱신하지 않는다", async () => {
    setup();
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    let reader;
    vi.stubGlobal("FileReader", class { readAsDataURL() { reader = this; } });
    fireEvent.click(candidate);
    fireEvent.click(screen.getByRole("button", { name: "다른 디렉토리" }));
    await act(async () => { reader.result = "data:image/jpeg;base64,Y292ZXI="; reader.onload(); });
    expect(request.mock.calls.some(([path]) => path === "/apply-image")).toBe(false);
  });
  it("서점 재시도는 오류 캐시를 건너뛰고 진행 상태와 재시도 실패를 표시한다", async () => {
    setup();
    const pending = deferred();
    overrideRequest((path, options, original) => path.startsWith("/candidates/yes24")
      ? path.includes("refresh=true") ? pending.promise : { status: "error", error: "서점 접근 실패" }
      : original(path, options));
    render(<BookCoverAdmin />);
    fireEvent.click(await screen.findByRole("button", { name: "재시도", exact: true }));
    expect(screen.getByRole("button", { name: "재시도 중…" }).disabled).toBe(true);
    await waitFor(() => expect(request.mock.calls.some(([path]) => path.includes("refresh=true"))).toBe(true));
    await act(async () => pending.reject(new Error("재시도 요청 실패")));
    expect(screen.getByText("재시도 요청 실패")).toBeTruthy();
    expect(screen.getByRole("button", { name: "재시도", exact: true }).disabled).toBe(false);
  });
  it("휴지통은 미리보기만 비우며 후보를 유지하고 다른 서점 표지로 교체한다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    overrideRequest((path, options, original) => path.startsWith("/candidates/aladin")
      ? { status: "success", candidates: [{ candidate_id: "b".repeat(64), title: "다른 서점 표지", detail_url: "https://aladin.co.kr/b" }] }
      : original(path, options));
    render(<BookCoverAdmin />);
    const first = await screen.findByRole("button", { name: "YES24 서점 책 표지 선택" });
    const other = await screen.findByRole("button", { name: "알라딘 다른 서점 표지 표지 선택" });
    await waitFor(() => expect(first.disabled || other.disabled).toBe(false));
    fireEvent.click(first);
    await screen.findByRole("button", { name: /테스트 책.*완료/ });
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" }));
    expect(screen.queryByAltText("테스트 책 저장한 표지")).toBeNull();
    expect(screen.getByAltText("다른 서점 표지").getAttribute("src")).toBe("blob:cover");
    expect(screen.getByRole("button", { name: "재검색" }).disabled).toBe(false);
    expect(request.mock.calls.some(([path]) => path === "/cache/clear")).toBe(false);
    fireEvent.click(other);
    await waitFor(() => expect(request).toHaveBeenCalledWith("/apply-image", { body: { file_path: "책.epub", revision: "r2", image_data: "Y292ZXI=" } }));
    await screen.findByAltText("테스트 책 저장한 표지");
    expect(screen.getByAltText("테스트 책 저장한 표지").getAttribute("src")).toContain("v=r3");
  });
  it("돋보기 클릭으로만 추가 fetch 없이 확대하고 확대 이미지에서 적용한다", async () => {
    setup();
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    const requests = request.mock.calls.filter(([path]) => path.startsWith("/candidate-image")).length;
    fireEvent.mouseEnter(candidate);
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "YES24 서점 책 표지 확대" }));
    const panel = screen.getByRole("dialog", { name: "서점 책 표지 확대" });
    expect(screen.getByAltText("서점 책 확대 표지").getAttribute("src")).toBe("blob:cover");
    fireEvent.mouseLeave(panel, { relatedTarget: document.body });
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(request.mock.calls.filter(([path]) => path.startsWith("/candidate-image"))).toHaveLength(requests);
    fireEvent.click(screen.getByRole("button", { name: "서점 책 확대 표지 선택" }));
    await screen.findByRole("button", { name: /테스트 책.*완료/ });
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.mouseEnter(candidate);
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.mouseLeave(candidate);
  });

  it("미확보 이미지의 돋보기는 비활성화하고 모달은 닫기 버튼으로 닫는다", async () => {
    setup();
    const pending = deferred();
    overrideRequest((path, options, original) => path.startsWith("/candidate-image") ? pending.promise : original(path, options));
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    expect(screen.getByRole("button", { name: "YES24 서점 책 표지 확대" }).disabled).toBe(true);
    fireEvent.mouseEnter(candidate);
    expect(screen.queryByRole("dialog")).toBeNull();
    await act(async () => pending.resolve(new Blob(["cover"])));
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "YES24 서점 책 표지 확대" }));
    expect(screen.getByRole("dialog")).toBeTruthy();
    fireEvent.mouseLeave(candidate, { relatedTarget: document.body });
    expect(screen.getByRole("dialog")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "확대 닫기" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("서점의 확보한 후보를 모달에서 이동하고 선택한 후보를 적용한다", async () => {
    setup();
    const second = { candidate_id: "b".repeat(64), title: "두 번째 표지" };
    overrideRequest(async (path, options, original) => {
      const result = await original(path, options);
      return path.startsWith("/candidates/yes24") ? { ...result, candidates: [...result.candidates, second] } : result;
    });
    render(<BookCoverAdmin />);
    await waitFor(() => expect(screen.getByRole("button", { name: "YES24 두 번째 표지 표지 선택" }).disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "YES24 서점 책 표지 확대" }));
    expect(screen.getByRole("button", { name: "이전 표지" }).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "다음 표지" }));
    expect(screen.getByAltText("두 번째 표지 확대 표지")).toBeTruthy();
    expect(screen.getByRole("button", { name: "다음 표지" }).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "이전 표지" }));
    expect(screen.getByAltText("서점 책 확대 표지")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "다음 표지" }));
    fireEvent.click(screen.getByRole("button", { name: "두 번째 표지 확대 표지 선택" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("/apply", { body: { file_path: "책.epub", revision: "r1", candidate_id: second.candidate_id } }));
  });

  it.each(["search", "image"])("미리 검색 범위를 벗어난 %s 요청 취소를 오류로 표시하지 않고 돌아오면 재요청한다", async (type) => {
    setup();
    let visible = true;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(() => ({ top: visible ? 0 : -5000, bottom: visible ? 200 : -4800, left: 0, right: 100, width: 100, height: 200 }));
    const pending = deferred();
    let signal, started = false;
    const prefix = type === "search" ? "/candidates/yes24" : "/candidate-image";
    overrideRequest((path, options, original) => {
      if (path.startsWith(prefix) && !started) { started = true; signal = options.signal; return pending.promise; }
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await waitFor(() => expect(started).toBe(true));
    visible = false;
    fireEvent.scroll(window);
    await waitFor(() => expect(signal.aborted).toBe(true));
    await act(async () => pending.reject(new DOMException("취소", "AbortError")));
    expect(screen.queryByText("이미지 오류")).toBeNull();
    expect(screen.queryByText("취소")).toBeNull();
    visible = true;
    fireEvent.scroll(window);
    await waitFor(() => expect(screen.getByRole("button", { name: /YES24.*서점 책.*표지 선택/ }).disabled).toBe(false));
  });

  it.each(["search", "image"])("이전 디렉토리의 %s 실패가 새 목록에 나타나지 않는다", async (type) => {
    setup();
    const pending = deferred();
    let started = false;
    const prefix = type === "search" ? "/candidates/yes24" : "/candidate-image";
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing?directory=other")) return { items: [], total: 0, scanned: 0 };
      if (path.startsWith(prefix) && !started) { started = true; return pending.promise; }
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await waitFor(() => expect(started).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "다른 디렉토리" }));
    await screen.findByText("검사한 EPUB에는 표지 없는 책이 없습니다.");
    await act(async () => pending.reject(new Error("늦은 오류")));
    expect(screen.queryByText("늦은 오류")).toBeNull();
  });

  it.each(["cache", "url"])("%s 처리 중 디렉토리가 바뀌면 늦은 결과를 무시한다", async (operation) => {
    setup();
    const pending = deferred();
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing?directory=other")) return { items: [], scanned: 0, total: 0 };
      if (path === (operation === "cache" ? "/cache/clear" : "/apply-url")) return pending.promise;
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    if (operation === "cache") fireEvent.click(screen.getByRole("button", { name: "재검색" }));
    else {
      fireEvent.click(screen.getByRole("button", { name: "직접 입력" }));
      fireEvent.change(screen.getByLabelText("표지 이미지 URL"), { target: { value: "https://example.org/a" } });
      fireEvent.submit(screen.getByRole("dialog").querySelector("form"));
    }
    fireEvent.click(screen.getByRole("button", { name: "다른 디렉토리" }));
    await screen.findByText("검사한 EPUB에는 표지 없는 책이 없습니다.");
    await act(async () => pending.resolve({ revision: "r2" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("heading", { name: "테스트 책" })).toBeNull();
  });

  it("현재 책의 임시 삭제와 저장은 다른 책의 이미지와 행을 유지한다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    overrideRequest(async (path, options, original) => {
      const result = await original(path, options);
      if (path.startsWith("/missing")) return { ...result, items: [...result.items, { ...result.items[0], file_path: "두번째.epub", title: "두 번째 책", book_id: 124, category: "" }] };
      if (path.startsWith("/candidates/yes24") && path.includes(encodeURIComponent("두번째.epub"))) return { ...result, candidates: result.candidates.map((item) => ({ ...item, candidate_id: "b".repeat(64), title: "두 번째 후보" })) };
      return result;
    });
    render(<BookCoverAdmin />);
    const first = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    const second = await screen.findByRole("button", { name: /YES24.*두 번째 후보.*표지 선택/ });
    await waitFor(() => expect(first.disabled || second.disabled).toBe(false));
    const secondRow = screen.getByRole("heading", { name: "두 번째 책" }).closest("article");
    fireEvent.click(within(secondRow).getByRole("button", { name: "편집" }));
    expect(open).toHaveBeenCalledWith("/book-edit/124?category=_root", "_blank", "noopener");
    fireEvent.click(first);
    await screen.findByRole("button", { name: /테스트 책.*완료/ });
    fireEvent.load(screen.getByAltText("테스트 책 저장한 표지"));
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" }));
    expect(request.mock.calls.some(([path]) => path === "/cache/clear")).toBe(false);
    expect(screen.getByAltText("두 번째 후보")).toBeTruthy();
    fireEvent.click(first);
    const completed = await screen.findByRole("button", { name: /테스트 책.*완료/ });
    fireEvent.load(screen.getByAltText("테스트 책 저장한 표지"));
    fireEvent.click(completed);
    expect(screen.getByRole("heading", { name: "두 번째 책" })).toBeTruthy();
  });

  it("긴 목록에서도 노출 영역 밖의 Blob을 회수하고 이미지 크기는 유지한다", async () => {
    setup();
    let currentRow = 0;
    const fetched = new Set();
    const count = 24;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function () {
      const article = this.closest("article");
      const index = article ? Number(article.querySelector("h3").textContent.match(/\d+/)[0]) : count;
      const top = (index - currentRow) * 250;
      return { top, bottom: top + 200, left: 0, right: 100, width: 100, height: 200 };
    });
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing")) return { items: Array.from({ length: count }, (_, index) => ({ file_path: `책${index}.epub`, title: `책 ${index}`, revision: "r1", status: "missing" })), next_cursor: null, scanned: count, total: count };
      if (path.startsWith("/candidates/yes24")) {
        const index = Number(new URLSearchParams(path.split("?")[1]).get("file_path").match(/책(\d+)/)[1]);
        return { status: "success", candidates: Array.from({ length: 4 }, (_, candidate) => ({ candidate_id: (index * 4 + candidate).toString(16).padStart(64, "0"), title: `후보 ${index}-${candidate}`, width: 120, height: 180, byte_size: 5 })) };
      }
      if (path.startsWith("/candidate-image")) { fetched.add(path.split("/").pop()); return new Blob(["cover"]); }
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    for (currentRow = 0; currentRow < count; currentRow += 3) {
      fireEvent.scroll(window);
      const last = Math.min(count - 1, currentRow + 2);
      await waitFor(() => {
        for (let row = currentRow; row <= last; row += 1) {
          for (let image = 0; image < 4; image += 1) {
            const button = document.querySelector(`[aria-label="YES24 후보 ${row}-${image} 표지 선택"]`);
            expect(button?.disabled, `${row}-${image}`).toBe(false);
          }
        }
      }, { timeout: 5000 });
    }
    await waitFor(() => expect(URL.revokeObjectURL).toHaveBeenCalled());
    const buttons = () => [...document.querySelectorAll('[aria-label^="YES24 후보 "][aria-label$="표지 선택"]')];
    await waitFor(() => expect(buttons().filter((button) => !button.disabled).length).toBeLessThanOrEqual(80));
    const evicted = buttons().find((button) => button.disabled);
    expect(evicted).toBeTruthy();
    expect(evicted.title).toBe("해상도 1위 / 4개");
    expect(evicted.style.backgroundColor).toBe("rgb(0, 255, 0)");
    expect(fetched.size).toBeGreaterThan(80);
  }, 15000);

  it("빈 목록과 디렉토리 열림 상태를 표시하고 이미 조회한 디렉토리를 재요청하지 않는다", async () => {
    setup();
    overrideRequest((path, options, original) => path.startsWith("/missing") ? { items: [], total: 0, scanned: 0, next_cursor: null } : original(path, options));
    render(<BookCoverAdmin />);
    await screen.findByText("검사한 EPUB에는 표지 없는 책이 없습니다.");
    fireEvent.click(screen.getByRole("button", { name: "디렉토리 접기" }));
    expect(screen.getByText("닫힘")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "디렉토리 축소" }));
    fireEvent.click(screen.getByRole("button", { name: "최상위 선택" }));
    expect(request.mock.calls.filter(([path]) => path.startsWith("/directories"))).toHaveLength(1);
  });

  it("디렉토리 오류와 EPUB 검사 오류를 표시한다", async () => {
    setup();
    overrideRequest((path, options, original) => {
      if (path.startsWith("/directories")) return Promise.reject(new Error("디렉토리 오류"));
      if (path.startsWith("/missing")) return { items: [{ file_path: "bad.epub", title: "손상된 책", status: "error", error: "EPUB 손상" }], scanned: 1, total: 1 };
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await screen.findByText("디렉토리 오류");
    expect(screen.getByText("EPUB 손상")).toBeTruthy();
    expect(screen.getByRole("button", { name: "직접 입력" }).disabled).toBe(true);
    expect(screen.getByRole("button", { name: "재검색" }).disabled).toBe(true);
  });

  it("새로고침은 현재 목록을 다시 조회한다", async () => {
    setup();
    render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    fireEvent.click(screen.getByRole("button", { name: "새로고침" }));
    await waitFor(() => expect(request.mock.calls.filter(([path]) => path.startsWith("/missing"))).toHaveLength(2));
    await screen.findByRole("heading", { name: "테스트 책" });
  });

  it("직접 입력을 취소하면 URL을 요청하지 않는다", async () => {
    setup();
    render(<BookCoverAdmin />);
    fireEvent.click(await screen.findByRole("button", { name: "직접 입력" }));
    expect(screen.getByRole("button", { name: "표지 적용" }).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "취소" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(request.mock.calls.some(([path]) => path === "/apply-url")).toBe(false);
  });

  it("모달 닫기와 공백 URL은 저장을 실행하지 않는다", async () => {
    setup();
    render(<BookCoverAdmin />);
    fireEvent.click(await screen.findByRole("button", { name: "직접 입력" }));
    fireEvent.change(screen.getByLabelText("표지 이미지 URL"), { target: { value: "   " } });
    fireEvent.submit(screen.getByRole("dialog").querySelector("form"));
    expect(request.mock.calls.some(([path]) => path === "/apply-url")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("직접 URL 실패는 모달을 유지하고 재입력을 허용한다", async () => {
    setup();
    overrideRequest((path, options, original) => path === "/apply-url" ? Promise.reject(new Error("이미지 다운로드 실패")) : original(path, options));
    render(<BookCoverAdmin />);
    fireEvent.click(await screen.findByRole("button", { name: "직접 입력" }));
    fireEvent.change(screen.getByLabelText("표지 이미지 URL"), { target: { value: "https://example.org/a" } });
    fireEvent.click(screen.getByRole("button", { name: "표지 적용" }));
    await within(screen.getByRole("dialog")).findByText("이미지 다운로드 실패");
    expect(screen.getByRole("button", { name: "표지 적용" }).disabled).toBe(false);
    expect(screen.getByRole("heading", { name: "테스트 책" })).toBeTruthy();
  });

  it("URL 저장 중 이중 제출과 모달 닫기를 막는다", async () => {
    setup();
    const pending = deferred();
    overrideRequest((path, options, original) => path === "/apply-url" ? pending.promise : original(path, options));
    render(<BookCoverAdmin />);
    fireEvent.click(await screen.findByRole("button", { name: "직접 입력" }));
    fireEvent.change(screen.getByLabelText("표지 이미지 URL"), { target: { value: "https://example.org/a" } });
    fireEvent.submit(screen.getByRole("dialog").querySelector("form"));
    fireEvent.submit(screen.getByRole("dialog").querySelector("form"));
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape", code: "Escape" });
    expect(screen.getByRole("button", { name: "취소" }).disabled).toBe(true);
    expect(request.mock.calls.filter(([path]) => path === "/apply-url")).toHaveLength(1);
    await act(async () => pending.resolve({ revision: "r2" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("캐시 삭제 실패 시 후보를 유지하고 오류를 표시한다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    overrideRequest((path, options, original) => path === "/cache/clear" ? Promise.reject(new Error("캐시 삭제 실패")) : original(path, options));
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "재검색" }));
    await screen.findByText("캐시 삭제 실패");
    expect(screen.getByRole("button", { name: /YES24.*서점 책.*표지 선택/ }).disabled).toBe(false);
  });

  it("완료 문구 오른쪽 휴지통은 미리보기만 비우고 재검색은 최신 revision을 보낸다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByRole("button", { name: /테스트 책.*완료/ });
    fireEvent.load(screen.getByAltText("테스트 책 저장한 표지"));
    fireEvent.click(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" }));
    expect(screen.queryByAltText("테스트 책 저장한 표지")).toBeNull();
    expect(request.mock.calls.some(([path]) => path === "/cache/clear")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "재검색" }));
    await waitFor(() => expect(request).toHaveBeenCalledWith("/cache/clear", { body: { file_path: "책.epub", revision: "r2" } }));
    expect(screen.getByRole("button", { name: "재검색" }).disabled).toBe(false);
    expect(screen.getByRole("button", { name: "직접 입력" })).toBeTruthy();
  });

  it("저장 후 preview 오류와 warning을 표시하고 완료 확인을 비활성화한다", async () => {
    setup();
    overrideRequest((path, options, original) => path === "/apply" ? { revision: "r2", warning: "캐시 정리 경고" } : original(path, options));
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByText("캐시 정리 경고");
    fireEvent.error(screen.getByAltText("테스트 책 저장한 표지"));
    expect(screen.getByText("저장한 표지를 표시하지 못했습니다. 새로고침하세요")).toBeTruthy();
    expect(screen.getByRole("button", { name: /테스트 책.*완료/ }).disabled).toBe(true);
  });

  it("서점 검색 오류와 이미지 오류를 각각 재시도한다", async () => {
    setup();
    let searched = false, downloaded = false;
    overrideRequest((path, options, original) => {
      if (path.startsWith("/candidates/yes24") && !searched) { searched = true; return Promise.reject(new Error("서점 오류")); }
      if (path.startsWith("/candidate-image") && !downloaded) { downloaded = true; return Promise.reject(new Error("이미지 오류 상세")); }
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    await screen.findByText("서점 오류");
    fireEvent.click(screen.getByRole("button", { name: "재시도", exact: true }));
    fireEvent.click(await screen.findByRole("button", { name: "이미지 재시도" }));
    await waitFor(() => expect(screen.getByRole("button", { name: /YES24.*서점 책.*표지 선택/ }).disabled).toBe(false));
  });

  it("서점 검색 링크와 서버 캐시의 이미지 크기를 표시한다", async () => {
    setup();
    overrideRequest(async (path, options, original) => {
      const result = await original(path, options);
      if (path.startsWith("/candidates/yes24")) return { ...result, search_url: "https://www.yes24.com/search", candidates: result.candidates.map((item) => ({ ...item, width: 120, height: 180, byte_size: 2048 })) };
      return result;
    });
    render(<BookCoverAdmin />);
    expect((await screen.findByRole("link", { name: "서점 검색" })).href).toBe("https://www.yes24.com/search");
    expect(screen.getByText("120 × 180 px")).toBeTruthy();
    const image = await screen.findByAltText("서점 책");
    fireEvent.load(image);
    Object.defineProperties(image, { naturalWidth: { value: 120 }, naturalHeight: { value: 180 } });
    fireEvent.load(image);
    fireEvent.load(image);
    expect(screen.getByText("120 × 180 px")).toBeTruthy();
  });

  it("디렉토리 전환 시 이전 목록과 늦은 저장 응답을 무시한다", async () => {
    setup();
    const pending = deferred();
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing?directory=other")) return { items: [], scanned: 0, total: 0 };
      if (path === "/apply") return pending.promise;
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    fireEvent.click(screen.getByRole("button", { name: "다른 디렉토리" }));
    await screen.findByText("검사한 EPUB에는 표지 없는 책이 없습니다.");
    await act(async () => pending.resolve({ revision: "r2" }));
    expect(screen.queryByRole("button", { name: /테스트 책.*완료/ })).toBeNull();
  });

  it("이전 목록 요청을 취소하고 늦은 응답으로 새 디렉토리를 바꾸지 않는다", async () => {
    setup();
    const pending = deferred();
    let signal;
    overrideRequest((path, options, original) => {
      if (path.startsWith("/missing?directory=&")) { signal = options.signal; return pending.promise; }
      return original(path, options);
    });
    render(<BookCoverAdmin />);
    fireEvent.click(screen.getByRole("button", { name: "다른 디렉토리" }));
    await screen.findByRole("heading", { name: "테스트 책" });
    expect(signal.aborted).toBe(true);
    await act(async () => pending.resolve({ items: [], scanned: 0, total: 0 }));
    expect(screen.getByRole("heading", { name: "테스트 책" })).toBeTruthy();
  });

  it("재검색 전에 시작한 이미지 응답은 늦게 와도 최신 이미지를 덮어쓰지 않는다", async () => {
    setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const pending = deferred();
    let attempts = 0;
    const createUrl = vi.spyOn(URL, "createObjectURL");
    overrideRequest((path, options, original) => path.startsWith("/candidate-image") && ++attempts === 1 ? pending.promise : original(path, options));
    render(<BookCoverAdmin />);
    await waitFor(() => expect(request.mock.calls.some(([path]) => path.startsWith("/candidate-image"))).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "재검색" }));
    await act(async () => pending.resolve(new Blob(["late"])));
    await screen.findByAltText("서점 책");
    expect(screen.getByAltText("서점 책")).toBeTruthy();
    expect(createUrl).toHaveBeenCalledTimes(1);
  });

  it("직접 입력 모달의 URL로 표지를 적용하고 완료 표지를 확인한다", async () => {
    setup();
    render(<BookCoverAdmin />);
    fireEvent.click(await screen.findByRole("button", { name: "직접 입력" }));
    fireEvent.change(screen.getByLabelText("표지 이미지 URL"), { target: { value: "https://example.org/cover.jpg" } });
    fireEvent.click(screen.getByRole("button", { name: "표지 적용" }));
    await screen.findByRole("button", { name: /테스트 책.*완료/ });
    expect(request).toHaveBeenCalledWith("/apply-url", { body: { file_path: "책.epub", revision: "r1", url: "https://example.org/cover.jpg" } });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" })).toBeTruthy();
  });

  it("휴지통 confirm 취소는 미리보기를 유지하고 확인은 후보를 유지한다", async () => {
    setup();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByAltText("테스트 책 저장한 표지");
    const trash = screen.getByRole("button", { name: "테스트 책 임시 표지 삭제" });
    fireEvent.click(trash);
    expect(screen.getByAltText("테스트 책 저장한 표지")).toBeTruthy();
    expect(request.mock.calls.some(([path]) => path === "/cache/clear")).toBe(false);
    confirm.mockReturnValue(true);
    fireEvent.click(trash);
    expect(screen.queryByAltText("테스트 책 저장한 표지")).toBeNull();
    expect(screen.getByRole("button", { name: /YES24.*서점 책.*표지 선택/ })).toBeTruthy();
    expect(URL.revokeObjectURL).not.toHaveBeenCalled();
  });

  it("재검색은 서버 캐시를 비운 뒤 서점 검색을 다시 실행한다", async () => {
    setup();
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    const searches = () => request.mock.calls.filter(([path]) => path.startsWith("/candidates/yes24"));
    expect(searches()).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "재검색" }));
    await waitFor(() => expect(searches()).toHaveLength(2));
    expect(request).toHaveBeenCalledWith("/cache/clear", { body: { file_path: "책.epub", revision: "r1" } });
  });

  it("제목 옆에서 기존 편집과 조회 화면을 새 창으로 연다", async () => {
    setup();
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    fireEvent.click(screen.getByRole("button", { name: "편집" }));
    fireEvent.click(screen.getByRole("button", { name: "조회" }));
    const category = encodeURIComponent("소설/한글 & 이야기");
    expect(open).toHaveBeenNthCalledWith(1, `/book-edit/123?category=${category}`, "_blank", "noopener");
    expect(open).toHaveBeenNthCalledWith(2, `/book-view/123?category=${category}`, "_blank", "noopener");
  });

  it("표지 없는 책이 없는 페이지도 자동 검사하고 다음 목록을 덧붙인다", async () => {
    setup();
    const original = request.getMockImplementation();
    const cursors = [];
    request.mockImplementation(async (path, options) => {
      if (!path.startsWith("/missing")) return original(path, options);
      const cursor = new URLSearchParams(path.split("?")[1]).get("cursor");
      cursors.push(cursor);
      return {
        items: cursor ? [{ file_path: `${cursor}.epub`, title: `추가 책 ${cursor}`, status: "missing" }] : [],
        next_cursor: cursor === "120" ? null : cursor === "60" ? "120" : "60",
        scanned: cursor === "120" ? 180 : cursor === "60" ? 120 : 60, total: 180,
      };
    });
    render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "추가 책 120" });
    expect(screen.getByRole("heading", { name: "추가 책 60" })).toBeTruthy();
    expect(cursors).toEqual(["", "60", "120"]);
    expect(screen.queryByRole("button", { name: "다음 EPUB 검사" })).toBeNull();
  });

  it("끝부분이 화면 아래 25% 범위에 오면 중복 요청 없이 다음 페이지를 검사한다", async () => {
    setup();
    let nearEnd = false;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function () {
      const top = this.classList.contains("book-cover-scan-trigger") ? window.innerHeight * (nearEnd ? 1.2 : 2) : 0;
      return { top, bottom: top + 200, left: 0, right: 100, width: 100, height: 200 };
    });
    const original = request.getMockImplementation();
    let finish;
    const continuation = new Promise((resolve) => { finish = resolve; });
    request.mockImplementation(async (path, options) => {
      if (!path.startsWith("/missing")) return original(path, options);
      if (path.endsWith("cursor=60")) return continuation;
      return { ...(await original(path, options)), next_cursor: "60", total: 120 };
    });
    render(<BookCoverAdmin />);
    await screen.findByRole("heading", { name: "테스트 책" });
    const listings = () => request.mock.calls.filter(([path]) => path.startsWith("/missing"));
    expect(listings()).toHaveLength(1);
    nearEnd = true;
    fireEvent.scroll(window);
    await waitFor(() => expect(listings()).toHaveLength(2));
    fireEvent.scroll(window);
    fireEvent.resize(window);
    await new Promise((resolve) => requestAnimationFrame(resolve));
    expect(listings()).toHaveLength(2);
    expect(listings()[1][1].signal.aborted).toBe(false);
    finish({ items: [{ file_path: "추가.epub", title: "추가 책", status: "missing" }], next_cursor: null, scanned: 120, total: 120 });
    await screen.findByRole("heading", { name: "추가 책" });
    expect(screen.getByRole("heading", { name: "테스트 책" })).toBeTruthy();
  });

  it("추가 검사 실패 시 기존 목록을 유지하고 자동 재요청을 중단한다", async () => {
    setup();
    const original = request.getMockImplementation();
    request.mockImplementation(async (path, options) => {
      if (!path.startsWith("/missing")) return original(path, options);
      if (path.endsWith("cursor=60")) throw new Error("추가 검사 실패");
      return { ...(await original(path, options)), next_cursor: "60", total: 120 };
    });
    render(<BookCoverAdmin />);
    await screen.findByText("추가 검사 실패");
    fireEvent.scroll(window);
    await new Promise((resolve) => requestAnimationFrame(resolve));
    expect(request.mock.calls.filter(([path]) => path.startsWith("/missing"))).toHaveLength(2);
    expect(screen.getByRole("heading", { name: "테스트 책" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "새로고침" }).disabled).toBe(false);
  });

  it("저장한 표지를 확인한 뒤 클릭해야 행을 숨긴다", async () => {
    setup();
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    const completed = await screen.findByRole("button", { name: /테스트 책.*완료/ });
    expect(screen.getByText("테스트 책")).toBeTruthy();
    expect(completed.disabled).toBe(true);
    fireEvent.load(screen.getByAltText("테스트 책 저장한 표지"));
    fireEvent.click(completed);
    expect(screen.queryByText("테스트 책")).toBeNull();
  });

  it("저장 실패 시 행을 유지한다", async () => {
    setup({ fail: true });
    render(<BookCoverAdmin />);
    const candidate = await screen.findByRole("button", { name: /YES24.*서점 책.*표지 선택/ });
    await waitFor(() => expect(candidate.disabled).toBe(false));
    fireEvent.click(candidate);
    await screen.findByText("저장 실패");
    expect(screen.getByText("테스트 책")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /테스트 책.*완료/ })).toBeNull();
  });

  it("이미지의 픽셀 크기와 파일 크기를 표시하고 큰 후보의 패딩을 초록색으로 칠한다", async () => {
    setup();
    render(<BookCoverAdmin />);
    const image = await screen.findByAltText("서점 책");
    Object.defineProperties(image, { naturalWidth: { value: 600 }, naturalHeight: { value: 900 } });
    fireEvent.load(image);
    expect(await screen.findByText("600 × 900 px")).toBeTruthy();
    expect(screen.getByText("5 B")).toBeTruthy();
    expect(screen.getByRole("button", { name: /YES24.*서점 책.*표지 선택/ }).style.backgroundColor).toBe("rgb(0, 255, 0)");
  });
});
