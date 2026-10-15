# SplitLayout 구현 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 왼쪽 디렉토리 / 오른쪽 작업 공간 페이지 5곳에 공통 `SplitLayout`을 적용해, 두 컬럼 독립 스크롤과 드래그(마우스·터치) 너비 조절을 통일한다.

**Architecture:** `src/SplitLayout.jsx` 하나가 grid(`왼쪽 | 핸들 | 오른쪽`)를 그린다. 페이지는 `Row/Col`을 `SplitLayout / SplitLayout.Left / SplitLayout.Right` 태그로 바꾸기만 한다(자식 JSX는 그대로). 핸들은 Pointer Events로 마우스와 터치를 함께 처리하고, 너비는 `localStorage`에 저장한다. 768px 미만은 기존처럼 위아래로 쌓고 핸들을 렌더하지 않는다.

**Tech Stack:** React 19, vitest 5 + @testing-library/react + jsdom, Playwright, react-bootstrap(기존 페이지 내부용). 새 의존성 없음.

**Spec:** `docs/superpowers/specs/2026-10-15-split-layout-design.md`

## Global Constraints

- 새 의존성을 추가하지 않는다(`package.json` 변경 금지).
- 프론트엔드 변경 후 `cd frontend && npm test` 통과를 확인한다. 테스트는 vitest(jest 아님).
- 왼쪽 최소 너비 200px, 최대는 컨테이너의 60%. 핸들 터치 영역 16px.
- 768px 미만은 쌓기 레이아웃이며 핸들이 없다(`useIsMobile` 기준 `window.innerWidth < 768`).
- `localStorage` 접근은 항상 try/catch로 감싼다.
- `.row/.col-*`는 `App.css`에서 `!important`로 덮이므로 `SplitLayout`은 이를 쓰지 않는다.
- 커밋 제목은 Conventional Commits, 본문은 `-` bullet 명령형 72자 이내.
- 커밋 전에 GitNexus `detect_changes({scope:"all"})`를 실행한다. `partial`/`truncated`이면 깨끗한 결과가 아니므로 다시 실행한다. 심볼을 수정하기 전에는 `impact`(upstream)를 실행하고 HIGH/CRITICAL이면 사용자에게 알린다.
- Edit/Write 후 포매터 훅이 파일 전체를 다시 포맷할 수 있다. `git diff`로 편집 영역 밖 변경을 확인한다.

## Review Focus

- 오른쪽 pane이 없고 왼쪽만 있는 경우(책 보기에서 검색 전): 왼쪽이 전체 폭으로 퍼지지 않고 저장된 너비를 유지한다. → Task 1 테스트
- 컨테이너가 저장된 너비보다 좁은 경우(창 축소, 다른 기기): 화면 너비는 CSS `min(Npx, 60%)`로 즉시 제한하고, 저장값은 첫 조작에서 다시 맞춘다. → Task 1 테스트(저장값 보정)와 Task 2 시각 확인(CSS 제한). jsdom은 CSS `min()`을 계산하지 못하므로 CSS 제한은 단위 테스트로 검증하지 못한다.
- `localStorage`가 막히거나 값이 숫자가 아닌 경우: 기본 너비로 동작한다. → Task 1 테스트
- 드래그 중 포인터가 핸들 밖으로 나가거나 `pointercancel`이 오는 경우: 드래그가 끝나고 텍스트 선택이 풀린다. → Task 1 테스트
- 책 편집에서 폴더를 접었다 펼치는 경우: 왼쪽 pane이 사라지고 핸들도 사라지며, 다시 펼치면 이전 너비로 돌아온다. → Task 2 테스트

---

## File Structure

| 파일                                                                                    | 책임                                             |
| --------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `frontend/src/SplitLayout.jsx` (신규)                                                   | 두 pane + 핸들 렌더, 드래그·키보드·더블클릭·저장 |
| `frontend/src/SplitLayout.css` (신규)                                                   | grid, 높이, 독립 스크롤, 핸들 모양               |
| `frontend/tests/SplitLayout.test.jsx` (신규)                                            | 단위 테스트                                      |
| `frontend/src/Edit.jsx`, `Edit.css`, `tests/Edit.test.jsx`                              | 책 편집·만화 편집 적용                           |
| `frontend/src/CategoryAdmin.jsx`, `CategoryAdmin.css`(삭제)                             | 카테고리 관리 적용                               |
| `frontend/src/BookCoverAdmin.jsx`, `BookCoverAdmin.css`, `e2e/book-cover-admin.spec.js` | 표지 관리 적용                                   |
| `frontend/src/View.jsx`, `View.css`                                                     | 책 보기 상단 영역 적용                           |

## 인터페이스 (모든 Task 공통)

```jsx
import SplitLayout from "./SplitLayout";

<SplitLayout
  storageKey="split:edit" // 필수. localStorage 키
  defaultLeftWidth={260} // px, 기본 280
  minLeft={200} // px, 기본 200
  maxLeftRatio={0.6} // 컨테이너 대비, 기본 0.6
  scroll={true} // false면 높이 고정과 pane 스크롤을 끈다(책 보기)
  className="" // 컨테이너에 추가
>
  <SplitLayout.Left className="">…</SplitLayout.Left>
  <SplitLayout.Right className="">…</SplitLayout.Right>
</SplitLayout>;
```

- `Left`나 `Right`가 자식에 없거나 `false`면 해당 pane은 렌더하지 않는다.
- 둘 다 있을 때만 핸들을 렌더한다.
- 왼쪽만 있으면 `leftWidth` px 너비를 유지한다. 오른쪽만 있으면 전체 폭이다.
- DOM 클래스: 컨테이너 `.split-layout`(+`.is-stacked`, `.no-scroll`), `.split-layout-left`, `.split-layout-right`, `.split-layout-handle`(+`.is-dragging`).

---

### Task 1: SplitLayout 컴포넌트

**Files:**

- Create: `frontend/src/SplitLayout.jsx`
- Create: `frontend/src/SplitLayout.css`
- Test: `frontend/tests/SplitLayout.test.jsx`

**Interfaces:**

- Consumes: `useIsMobile(breakpoint = 768)` from `./useIsMobile` (boolean 반환).
- Produces: 위 "인터페이스" 절의 `SplitLayout`(default export), `SplitLayout.Left`, `SplitLayout.Right`.

- [ ] **Step 1: 실패하는 테스트 작성** — `frontend/tests/SplitLayout.test.jsx`

```jsx
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import SplitLayout from "../src/SplitLayout";

const KEY = "split:test";

function setInnerWidth(value) {
  Object.defineProperty(window, "innerWidth", {
    writable: true,
    configurable: true,
    value,
  });
}

function mockContainerWidth(width) {
  return vi
    .spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockReturnValue({
      width,
      height: 600,
      top: 0,
      left: 0,
      right: width,
      bottom: 600,
    });
}

function renderLayout(props = {}, { left = true, right = true } = {}) {
  return render(
    <SplitLayout storageKey={KEY} defaultLeftWidth={280} {...props}>
      {left && <SplitLayout.Left>LEFT</SplitLayout.Left>}
      {right && <SplitLayout.Right>RIGHT</SplitLayout.Right>}
    </SplitLayout>,
  );
}

const handle = () => screen.queryByRole("separator");
const widthOf = () => Number(handle().getAttribute("aria-valuenow"));

describe("SplitLayout", () => {
  beforeEach(() => {
    setInnerWidth(1024);
    window.localStorage.clear();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("두 pane과 핸들을 렌더한다", () => {
    const { container } = renderLayout();
    expect(container.querySelector(".split-layout-left").textContent).toBe(
      "LEFT",
    );
    expect(container.querySelector(".split-layout-right").textContent).toBe(
      "RIGHT",
    );
    expect(handle()).toBeTruthy();
    expect(widthOf()).toBe(280);
  });

  it("768px 미만에서는 쌓기 레이아웃이고 핸들이 없다", () => {
    setInnerWidth(500);
    const { container } = renderLayout();
    expect(container.querySelector(".split-layout.is-stacked")).toBeTruthy();
    expect(handle()).toBeNull();
  });

  it("드래그한 만큼 왼쪽 너비가 바뀐다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 1, button: 0 });
    fireEvent.pointerMove(handle(), { clientX: 340, pointerId: 1 });
    expect(widthOf()).toBe(320);
    fireEvent.pointerUp(handle(), { clientX: 340, pointerId: 1 });
    expect(widthOf()).toBe(320);
  });

  it("최소 200px와 최대 60%로 제한한다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 1, button: 0 });
    fireEvent.pointerMove(handle(), { clientX: -500, pointerId: 1 });
    expect(widthOf()).toBe(200);
    fireEvent.pointerMove(handle(), { clientX: 5000, pointerId: 1 });
    expect(widthOf()).toBe(600);
  });

  it("pointerup 뒤에는 이동해도 너비가 바뀌지 않고 pointercancel도 드래그를 끝낸다", () => {
    mockContainerWidth(1000);
    const { container } = renderLayout();
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 1, button: 0 });
    expect(handle().className).toContain("is-dragging");
    fireEvent.pointerCancel(handle(), { pointerId: 1 });
    expect(handle().className).not.toContain("is-dragging");
    fireEvent.pointerMove(handle(), { clientX: 400, pointerId: 1 });
    expect(widthOf()).toBe(280);
    expect(container.querySelector(".split-layout")).toBeTruthy();
  });

  it("마우스 보조 버튼은 드래그를 시작하지 않는다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.pointerDown(handle(), {
      clientX: 300,
      pointerId: 1,
      button: 2,
      pointerType: "mouse",
    });
    fireEvent.pointerMove(handle(), { clientX: 400, pointerId: 1 });
    expect(widthOf()).toBe(280);
  });

  it("화살표 키로 16px씩 조절한다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.keyDown(handle(), { key: "ArrowRight" });
    expect(widthOf()).toBe(296);
    fireEvent.keyDown(handle(), { key: "ArrowLeft" });
    fireEvent.keyDown(handle(), { key: "ArrowLeft" });
    expect(widthOf()).toBe(264);
  });

  it("더블클릭하면 기본 너비로 돌아온다", () => {
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.keyDown(handle(), { key: "ArrowRight" });
    fireEvent.doubleClick(handle());
    expect(widthOf()).toBe(280);
  });

  it("드래그가 끝나면 너비를 저장하고 다시 마운트하면 복원한다", () => {
    mockContainerWidth(1000);
    const first = renderLayout();
    fireEvent.pointerDown(handle(), { clientX: 300, pointerId: 1, button: 0 });
    fireEvent.pointerMove(handle(), { clientX: 360, pointerId: 1 });
    fireEvent.pointerUp(handle(), { pointerId: 1 });
    expect(window.localStorage.getItem(KEY)).toBe("340");
    first.unmount();
    renderLayout();
    expect(widthOf()).toBe(340);
  });

  it("저장된 너비가 컨테이너 최대값보다 크면 첫 조작에서 최대값 기준으로 줄어든다", () => {
    window.localStorage.setItem(KEY, "900");
    mockContainerWidth(1000);
    renderLayout();
    fireEvent.keyDown(handle(), { key: "ArrowLeft" });
    expect(widthOf()).toBe(584);
  });

  it("localStorage가 막히거나 값이 숫자가 아니어도 기본 너비로 동작한다", () => {
    window.localStorage.setItem(KEY, "abc");
    renderLayout();
    expect(widthOf()).toBe(280);
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const { unmount } = renderLayout();
    mockContainerWidth(1000);
    fireEvent.keyDown(screen.getAllByRole("separator")[1], {
      key: "ArrowRight",
    });
    unmount();
  });

  it("왼쪽만 있으면 핸들 없이 저장된 너비를 유지한다", () => {
    const { container } = renderLayout({}, { right: false });
    expect(handle()).toBeNull();
    expect(
      container.querySelector(".split-layout").getAttribute("style"),
    ).toContain("280px");
  });

  it("오른쪽만 있으면 핸들과 왼쪽 pane이 없다", () => {
    const { container } = renderLayout({}, { left: false });
    expect(handle()).toBeNull();
    expect(container.querySelector(".split-layout-left")).toBeNull();
    expect(container.querySelector(".split-layout-right")).toBeTruthy();
  });

  it("scroll={false}면 no-scroll 클래스를 단다", () => {
    const { container } = renderLayout({ scroll: false });
    expect(container.querySelector(".split-layout.no-scroll")).toBeTruthy();
  });

  it("pane과 컨테이너에 추가 className을 적용한다", () => {
    const { container } = render(
      <SplitLayout storageKey={KEY} className="extra">
        <SplitLayout.Left className="l">A</SplitLayout.Left>
        <SplitLayout.Right className="r">B</SplitLayout.Right>
      </SplitLayout>,
    );
    expect(container.querySelector(".split-layout.extra")).toBeTruthy();
    expect(container.querySelector(".split-layout-left.l")).toBeTruthy();
    expect(container.querySelector(".split-layout-right.r")).toBeTruthy();
  });
});
```

- [ ] **Step 2: 실패 확인**

Run: `cd frontend && npx vitest run tests/SplitLayout.test.jsx`
Expected: FAIL — `Failed to resolve import "../src/SplitLayout"`.

- [ ] **Step 3: 구현** — `frontend/src/SplitLayout.jsx`

```jsx
/**
 * 왼쪽 디렉토리 / 오른쪽 작업 공간 두 컬럼 레이아웃.
 * 두 pane이 따로 스크롤되고, 가운데 핸들을 끌거나 화살표 키로 왼쪽 너비를 바꾼다.
 * 768px 미만에서는 위아래로 쌓고 핸들을 렌더하지 않는다.
 */
import { Children, useCallback, useRef, useState } from "react";
import useIsMobile from "./useIsMobile";
import "./SplitLayout.css";

const HANDLE_WIDTH = 16;
const KEY_STEP = 16;

function readStoredWidth(key) {
  try {
    const value = Number(window.localStorage.getItem(key));
    return Number.isFinite(value) && value > 0 ? value : null;
  } catch {
    return null; // 저장소가 막힌 환경(사생활 보호 모드 등)에서는 기본값을 쓴다.
  }
}

function writeStoredWidth(key, value) {
  try {
    window.localStorage.setItem(key, String(value));
  } catch {
    // 저장 실패는 조절 동작에 영향을 주지 않는다.
  }
}

function SplitLeft() {
  return null;
}

function SplitRight() {
  return null;
}

export default function SplitLayout({
  storageKey,
  defaultLeftWidth = 280,
  minLeft = 200,
  maxLeftRatio = 0.6,
  scroll = true,
  className = "",
  children,
}) {
  const isStacked = useIsMobile(768);
  const containerRef = useRef(null);
  const drag = useRef(null);
  const widthRef = useRef(0);
  const [dragging, setDragging] = useState(false);
  const [rawWidth, setRawWidth] = useState(
    () => (storageKey && readStoredWidth(storageKey)) || defaultLeftWidth,
  );

  const clampWidth = useCallback(
    (value) => {
      const total = containerRef.current?.getBoundingClientRect().width || 0;
      const max = total ? Math.max(minLeft, total * maxLeftRatio) : Infinity;
      return Math.round(Math.min(Math.max(value, minLeft), max));
    },
    [minLeft, maxLeftRatio],
  );

  const leftWidth = clampWidth(rawWidth);
  widthRef.current = leftWidth;

  const commit = useCallback(
    (value, { persist }) => {
      const next = clampWidth(value);
      widthRef.current = next;
      setRawWidth(next);
      if (persist && storageKey) writeStoredWidth(storageKey, next);
    },
    [clampWidth, storageKey],
  );

  const items = Children.toArray(children);
  const leftItem = items.find((item) => item.type === SplitLeft);
  const rightItem = items.find((item) => item.type === SplitRight);

  const onPointerDown = (event) => {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    drag.current = { startX: event.clientX, startWidth: widthRef.current };
    setDragging(true);
  };

  const onPointerMove = (event) => {
    if (!drag.current) return;
    commit(drag.current.startWidth + event.clientX - drag.current.startX, {
      persist: false,
    });
  };

  const endDrag = (event) => {
    if (!drag.current) return;
    drag.current = null;
    setDragging(false);
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    commit(widthRef.current, { persist: true });
  };

  const onKeyDown = (event) => {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      commit(widthRef.current - KEY_STEP, { persist: true });
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      commit(widthRef.current + KEY_STEP, { persist: true });
    }
  };

  const showHandle = !isStacked && leftItem && rightItem;
  // 첫 렌더에는 컨테이너 너비를 모르므로, 최대 비율은 CSS min()으로도 건다.
  const maxWidthCss = (px) => `min(${px}px, ${maxLeftRatio * 100}%)`;
  let style;
  if (!isStacked && leftItem && rightItem) {
    style = {
      gridTemplateColumns: `${maxWidthCss(leftWidth)} ${HANDLE_WIDTH}px minmax(0, 1fr)`,
    };
  } else if (!isStacked && leftItem) {
    style = { gridTemplateColumns: maxWidthCss(leftWidth) };
  }

  const classes = [
    "split-layout",
    isStacked ? "is-stacked" : "",
    scroll ? "" : "no-scroll",
    className,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div ref={containerRef} className={classes} style={style}>
      {leftItem && (
        <div
          className={`split-layout-left ${leftItem.props.className || ""}`.trim()}
        >
          {leftItem.props.children}
        </div>
      )}
      {showHandle && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="디렉토리 너비 조절"
          aria-valuenow={leftWidth}
          aria-valuemin={minLeft}
          tabIndex={0}
          className={`split-layout-handle${dragging ? " is-dragging" : ""}`}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          onKeyDown={onKeyDown}
          onDoubleClick={() => commit(defaultLeftWidth, { persist: true })}
        />
      )}
      {rightItem && (
        <div
          className={`split-layout-right ${rightItem.props.className || ""}`.trim()}
        >
          {rightItem.props.children}
        </div>
      )}
    </div>
  );
}

SplitLayout.Left = SplitLeft;
SplitLayout.Right = SplitRight;
```

`frontend/src/SplitLayout.css`:

```css
/* 두 컬럼 공통 레이아웃. 내비바 높이는 --navbar-h로 바꿀 수 있다(기본 80px). */
.split-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
}

.split-layout:not(.is-stacked):not(.no-scroll) {
  height: calc(100vh - var(--navbar-h, 80px));
  height: calc(100dvh - var(--navbar-h, 80px));
}

.split-layout:not(.is-stacked):not(.no-scroll) > .split-layout-left,
.split-layout:not(.is-stacked):not(.no-scroll) > .split-layout-right {
  min-height: 0;
  overflow-y: auto;
}

.split-layout > .split-layout-left,
.split-layout > .split-layout-right {
  min-width: 0;
}

.split-layout-handle {
  position: relative;
  cursor: col-resize;
  touch-action: none;
  user-select: none;
  outline: none;
}

.split-layout-handle::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 50%;
  width: 2px;
  transform: translateX(-50%);
  background: var(--bs-border-color, #dee2e6);
  transition: background 0.1s;
}

.split-layout-handle:hover::before,
.split-layout-handle:focus-visible::before,
.split-layout-handle.is-dragging::before {
  width: 4px;
  background: var(--bs-primary, #0d6efd);
}
```

참고: 드래그 중 텍스트 선택은 핸들의 `user-select: none`과 pointer capture로 막는다. 필요하면 `.is-dragging` 동안 `document.body`에 `user-select:none`을 거는 처리를 Step 5에서 확인한 뒤에만 추가한다.

- [ ] **Step 4: 통과 확인**

Run: `cd frontend && npx vitest run tests/SplitLayout.test.jsx`
Expected: 15개 PASS. 실패하면 mock된 `getBoundingClientRect`가 `afterEach`에서 복원되는지, "localStorage 막힘" 테스트가 `getAllByRole("separator")`로 두 번째 핸들(새 렌더)을 집는지 먼저 확인한다.

- [ ] **Step 5: 커밋**

```bash
cd /home/terzeron/workspace/tm_dev
# GitNexus detect_changes({scope:"all"}) 실행 후
git add frontend/src/SplitLayout.jsx frontend/src/SplitLayout.css frontend/tests/SplitLayout.test.jsx
git commit -m "feat: 두 컬럼 공통 SplitLayout 추가" -m "- 두 pane 독립 스크롤과 드래그·키보드 너비 조절 추가
- 너비를 localStorage에 저장하고 실패 시 기본값 사용"
```

---

### Task 2: 책 편집·만화 편집 적용

**Files:**

- Modify: `frontend/src/Edit.jsx` (현재 `1129-1180`, `1410-1411` 부근. 줄 번호는 편집 전 `grep -n "directoryClassName\|contentColumn\|<Row fluid\|</Row>" src/Edit.jsx`로 다시 확인)
- Modify: `frontend/src/Edit.css:48-53` (`#edit .directory-menu` 삭제)
- Test: `frontend/tests/Edit.test.jsx` (`:828-831`, `:1746-1764`, `:3077-3091`)

**Interfaces:**

- Consumes: `SplitLayout`, `SplitLayout.Left`, `SplitLayout.Right` (Task 1).
- Produces: 없음.

- [ ] **Step 1: 영향 분석** — GitNexus `impact({target:"Edit", direction:"upstream"})`. 결과(호출자, risk)를 사용자에게 보고하고 HIGH/CRITICAL이면 멈춰서 알린다. `risk: UNKNOWN`이면 `grep -rn "from \"./Edit\"\|import Edit" src`로 호출자(`BookEdit.jsx`, `ComicsEdit.jsx`)를 확인한다.

- [ ] **Step 2: 테스트 먼저 수정** — `Edit.test.jsx`의 세 곳에서 `.section.directory-menu` 단언을 바꾼다.
  - 데스크톱 단언 `expect(container.querySelector(".section.directory-menu")).toBeTruthy()` → `expect(container.querySelector(".split-layout:not(.is-stacked)")).toBeTruthy()`
  - 모바일 단언 `expect(container.querySelector(".section.directory-menu")).toBeNull()` → `expect(container.querySelector(".split-layout.is-stacked")).toBeTruthy()`
  - `3077-3091` 블록은 읽어서 같은 의미(데스크톱/모바일 전환)로 대응시킨다.
  - 폴더 접힘 테스트 추가(기존 접힘 토글 테스트가 있으면 그 안에 단언만 추가): 접으면 `.split-layout-left`와 `[role=separator]`가 없고, 펼치면 다시 생긴다.

Run: `cd frontend && npx vitest run tests/Edit.test.jsx`
Expected: FAIL (아직 SplitLayout 미적용).

- [ ] **Step 3: Edit.jsx 수정** (자식 JSX는 건드리지 않는다)
  1. `import SplitLayout from "./SplitLayout";` 추가.
  2. `directoryClassName`, `contentColumnMd`, `contentColumnLg`와 그 위 주석·`/* v8 ignore next 2 ... */` 주석을 삭제한다. `collapsedFolderPanel`은 그대로 둔다.
  3. `<Row fluid="true">` → `<SplitLayout storageKey="split:edit" defaultLeftWidth={260}>`
  4. 왼쪽 `<Col md={isMobile ? 12 : 3} lg={isMobile ? 12 : 2} className={directoryClassName}>` → `<SplitLayout.Left>`, 짝 `</Col>` → `</SplitLayout.Left>`
  5. 오른쪽 `<Col md={contentColumnMd} lg={contentColumnLg} className={isMobile ? "ps-0 pe-0" : "section"}>` → `<SplitLayout.Right>`, 짝 `</Col>`(`1410`) → `</SplitLayout.Right>`
  6. 마지막 `</Row>`(`1411`) → `</SplitLayout>`
  7. `isMobile`이 다른 곳에서 안 쓰이면(`grep -n isMobile src/Edit.jsx`) 선언과 `useIsMobile` import를 제거한다. 쓰이면 둔다.
  8. `Edit.css`의 `#edit .directory-menu { … }` 블록 삭제.

- [ ] **Step 4: 통과 확인**

Run: `cd frontend && npx vitest run tests/Edit.test.jsx tests/BookEdit.test.jsx tests/ComicsEdit.test.jsx tests/SplitLayout.test.jsx`(없는 파일은 생략)
Expected: PASS. 이어서 `npm test` 전체 실행(이 시점에 실패가 있으면 Edit을 렌더하는 `Admin.test.jsx`, `DefensiveHandling.test.jsx`, `FinalWrappers.test.jsx`를 먼저 본다).

- [ ] **Step 5: 시각 확인(개발 서버)** — `cd frontend && npm run dev` 후 책 편집 화면을 1280×800과 800×400에서 확인한다. 확인 항목: 왼쪽 트리와 오른쪽 폼이 따로 스크롤, 핸들 드래그로 너비 변화, 새로고침 뒤 너비 유지. 확인하지 못했으면 "미확인"으로 보고한다.

- [ ] **Step 6: 커밋**

```bash
git add frontend/src/Edit.jsx frontend/src/Edit.css frontend/tests/Edit.test.jsx
git commit -m "feat: 책 편집에 SplitLayout 적용" -m "- Row/Col을 SplitLayout으로 교체해 너비 조절 추가
- 오른쪽 컬럼도 독립 스크롤되도록 변경"
```

---

### Task 3: 카테고리 관리 적용

**Files:**

- Modify: `frontend/src/CategoryAdmin.jsx` (`2006-2007`, `2105-2106`, `2624-2625` 부근)
- Delete: `frontend/src/CategoryAdmin.css` (내용이 sticky 규칙 하나뿐. `grep -rn "CategoryAdmin.css" src tests`로 import 지점을 확인 후 import 줄도 삭제)
- Test: `frontend/tests/CategoryAdmin.test.jsx`

**Interfaces:**

- Consumes: Task 1의 `SplitLayout`.

- [ ] **Step 1: 영향 분석** — GitNexus `impact({target:"CategoryAdmin", direction:"upstream"})`, 보고.

- [ ] **Step 2: 실패하는 테스트 추가** — `CategoryAdmin.test.jsx`의 기존 "카테고리가 로드되면 디렉토리 표시" 계열 테스트 옆에 추가:

```jsx
it("디렉토리와 작업 패널을 SplitLayout으로 나눈다", async () => {
  /* 기존 테스트의 렌더·mock 준비를 그대로 복사한다 */
  await waitFor(() => {
    expect(container.querySelector(".split-layout-left")).toBeTruthy();
    expect(container.querySelector(".split-layout-right")).toBeTruthy();
    expect(screen.getByRole("separator")).toBeTruthy();
  });
});
```

(기존 테스트의 mock 준비 코드를 실제로 복사해 넣는다. 파일이 크므로 `grep -n "category-admin\|디렉토리" tests/CategoryAdmin.test.jsx | head`로 가장 가까운 테스트를 고른다. jsdom 기본 `innerWidth`는 1024다.)

Run: `cd frontend && npx vitest run tests/CategoryAdmin.test.jsx -t "SplitLayout"`
Expected: FAIL.

- [ ] **Step 3: CategoryAdmin.jsx 수정**
  1. `import SplitLayout from "./SplitLayout";` 추가, `CategoryAdmin.css` import 삭제.
  2. `<Row className="g-0">` → `<SplitLayout storageKey="split:category-admin" defaultLeftWidth={360}>`
  3. `<Col md={4} className="category-admin-directory">` → `<SplitLayout.Left>`, 짝 `</Col>`(`2105`) → `</SplitLayout.Left>`
  4. `<Col md={8}>`(`2106`) → `<SplitLayout.Right>`, 짝 `</Col>`(`2624`) → `</SplitLayout.Right>`
  5. 마지막 `</Row>`(`2625`) → `</SplitLayout>`
  6. 왼쪽 `<Card className="h-100">`는 pane 높이를 채우므로 유지한다. 카드 안의 트리가 길 때 Card 바깥으로 넘치면(시각 확인) `h-100`을 제거한다.
  7. `Row`/`Col` import가 더 안 쓰이면 제거한다(`grep -n "<Row\|<Col" src/CategoryAdmin.jsx`).
  8. `CategoryAdmin.css` 삭제.

- [ ] **Step 4: 통과 확인**

Run: `cd frontend && npx vitest run tests/CategoryAdmin.test.jsx tests/Admin.test.jsx`
Expected: PASS.

- [ ] **Step 5: 시각 확인** — 관리자 → 카테고리 관리를 1280×800에서 확인한다(왼쪽 트리 스크롤, 오른쪽 패널 스크롤, 핸들 드래그, 이상 항목 필터 표시 정상). 미확인이면 그렇게 보고한다.

- [ ] **Step 6: 커밋**

```bash
git add -A frontend/src/CategoryAdmin.jsx frontend/src/CategoryAdmin.css frontend/tests/CategoryAdmin.test.jsx
git commit -m "feat: 카테고리 관리에 SplitLayout 적용" -m "- 고정 sticky 규칙을 SplitLayout로 대체
- 오른쪽 패널 독립 스크롤과 너비 조절 추가"
```

---

### Task 4: 표지 관리 적용

**Files:**

- Modify: `frontend/src/BookCoverAdmin.jsx` (`import` 2줄, `338-346`, 마지막 `</Row>` `528`, 오른쪽 `</Col>`)
- Modify: `frontend/src/BookCoverAdmin.css:1-3`
- Modify: `frontend/e2e/book-cover-admin.spec.js:~243` (폭 측정 단언)
- Test: `frontend/tests/BookCoverAdmin.test.jsx`

**Interfaces:**

- Consumes: Task 1의 `SplitLayout`.

- [ ] **Step 0: 사전 확인** — `git status --short`가 깨끗한지, 다른 세션이 `BookCoverAdmin.*`를 수정 중이 아닌지 확인한다(계획 작성 시점에는 깨끗했다). 수정 중이면 멈추고 사용자에게 알린다.

- [ ] **Step 1: 영향 분석** — GitNexus `impact({target:"BookCoverAdmin", direction:"upstream"})`, 보고.

- [ ] **Step 2: 실패하는 테스트 추가** — `BookCoverAdmin.test.jsx`의 기존 데스크톱 렌더 테스트(`innerWidth` 1000 사용 구간, `:351-366` 부근)에 `.split-layout-left`, `.split-layout-right`, `[role=separator]` 존재 단언을 추가하고, `innerWidth=500`일 때 `.split-layout.is-stacked`가 있고 핸들이 없다는 단언을 추가한다.

Run: `cd frontend && npx vitest run tests/BookCoverAdmin.test.jsx`
Expected: 새 단언 FAIL.

- [ ] **Step 3: BookCoverAdmin.jsx 수정**
  1. `import SplitLayout from "./SplitLayout";` 추가. `Col, Row`가 다른 곳에서 안 쓰이면 react-bootstrap import에서 제거한다(`grep -n "<Row\|<Col" src/BookCoverAdmin.jsx`).
  2. `<Row className="g-0 book-cover-admin">` → `<SplitLayout storageKey="split:book-cover-admin" defaultLeftWidth={240} className="book-cover-admin">`
  3. `<Col md={2} className="book-cover-directory">` → `<SplitLayout.Left className="book-cover-directory">`, 짝 `</Col>` → `</SplitLayout.Left>`
  4. `<Col md={10} className="book-cover-panel">` → `<SplitLayout.Right className="book-cover-panel">`, 짝 `</Col>` → `</SplitLayout.Right>`
  5. 마지막 `</Row>` → `</SplitLayout>`
  6. `BookCoverAdmin.css`: `.book-cover-admin { padding-top: 12px; }`는 `SplitLayout` 높이(100dvh−내비바)와 겹쳐 아래가 잘릴 수 있으므로, 시각 확인 후 `padding-top`을 `margin-top`으로 바꾸고 높이에서 뺀다. `.book-cover-directory { padding-right: 12px; }`은 핸들 폭이 간격을 대신하므로 삭제한다.

- [ ] **Step 4: e2e 단언 수정** — `book-cover-admin.spec.js`의 `.book-cover-directory` 폭 측정이 "md=2 비율(≈16.7%)"을 가정하면, 새 기본 240px 기준(±2px)으로 바꾼다. 먼저 해당 줄을 읽고 정확한 단언을 확인한 뒤 수정한다.

- [ ] **Step 5: e2e 드래그 테스트 추가** — 같은 spec 파일에 기존 "표지 관리" 테스트의 `page.route` mock 설정(auth, directories, missing)을 재사용해 새 테스트를 추가한다.

```js
test("표지 관리: 핸들 드래그로 너비 조절, pane 독립 스크롤", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 700 });
  /* 기존 테스트의 route mock과 페이지 진입 코드를 그대로 복사 */
  const left = page.locator(".split-layout-left");
  const handle = page.getByRole("separator");
  const before = (await left.boundingBox()).width;
  const box = await handle.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + 40);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 60, box.y + 40, { steps: 5 });
  await page.mouse.up();
  const after = (await left.boundingBox()).width;
  expect(after - before).toBeGreaterThan(50);
  await page.reload();
  /* 같은 mock 재설정 후 */
  expect((await left.boundingBox()).width).toBeCloseTo(after, 0);

  // 터치 경로: 합성 pointer 이벤트(pointerType=touch)
  const b2 = await handle.boundingBox();
  const x = b2.x + b2.width / 2;
  await handle.dispatchEvent("pointerdown", {
    pointerType: "touch",
    pointerId: 7,
    isPrimary: true,
    clientX: x,
    clientY: b2.y + 40,
  });
  await handle.dispatchEvent("pointermove", {
    pointerType: "touch",
    pointerId: 7,
    isPrimary: true,
    clientX: x - 40,
    clientY: b2.y + 40,
  });
  await handle.dispatchEvent("pointerup", {
    pointerType: "touch",
    pointerId: 7,
    isPrimary: true,
    clientX: x - 40,
    clientY: b2.y + 40,
  });
  expect((await left.boundingBox()).width).toBeLessThan(after - 30);
});
```

Run: `cd frontend && npm run test:e2e -- e2e/book-cover-admin.spec.js`
Expected: PASS. 합성 터치 이벤트는 실제 기기 터치를 대신하지 못하므로, 보고서에는 "실기기 터치 미확인"으로 적는다.

- [ ] **Step 6: 단위·전체 확인과 커밋**

Run: `cd frontend && npx vitest run tests/BookCoverAdmin.test.jsx`
Expected: PASS.

```bash
git add frontend/src/BookCoverAdmin.jsx frontend/src/BookCoverAdmin.css frontend/tests/BookCoverAdmin.test.jsx frontend/e2e/book-cover-admin.spec.js
git commit -m "feat: 표지 관리에 SplitLayout 적용" -m "- Row/Col을 SplitLayout으로 교체해 너비 조절 추가
- e2e에 드래그와 터치 경로 검증 추가"
```

---

### Task 5: 책 보기 상단 영역 적용

**Files:**

- Modify: `frontend/src/View.jsx` (`287-299`, `300-326`)
- Modify: `frontend/src/View.css:27-45`
- Test: `frontend/tests/View.test.jsx`

**Interfaces:**

- Consumes: Task 1의 `SplitLayout`. 이 페이지는 `scroll={false}`로 쓴다(본문이 아래에 이어지므로 높이를 고정하지 않는다).

- [ ] **Step 1: 영향 분석** — GitNexus `impact({target:"View", direction:"upstream"})`, 보고.

- [ ] **Step 2: 실패하는 테스트 추가** — `View.test.jsx`에서 검색 결과가 있는 상태의 기존 테스트에 `.split-layout.no-scroll`, `.split-layout-left.view-directory-panel`, `[role=separator]` 단언을 추가한다. 검색 전(검색 결과·책 정보 없음) 상태에서는 `.split-layout-left`는 있고 `[role=separator]`와 `.split-layout-right`는 없다는 단언도 추가한다.

Run: `cd frontend && npx vitest run tests/View.test.jsx`
Expected: 새 단언 FAIL.

- [ ] **Step 3: View.jsx 수정**
  1. `import SplitLayout from "./SplitLayout";` 추가.
  2. `<Row className="view-tools-row g-0 align-items-start">` → `<SplitLayout storageKey="split:view" defaultLeftWidth={280} scroll={false} className="view-tools-row">`
  3. `<Col xs={12} md={3} className="view-directory-panel">` → `<SplitLayout.Left className="view-directory-panel">`, 짝 `</Col>`(`299`) → `</SplitLayout.Left>`
  4. `<Col xs={12} md={9} className="view-similar-books-panel">` → `<SplitLayout.Right className="view-similar-books-panel">`, 짝 `</Col>`(`326`) → `</SplitLayout.Right>`
  5. `</Row>`(`328`, 이 Row의 닫는 태그만) → `</SplitLayout>`
  6. `align-items: start`가 사라지므로 `View.css`의 `.view-tools-row`에 `align-items: start;`를 추가한다(grid에서 pane 높이가 서로 늘어나지 않게).
  7. `Row`/`Col`은 아래 `top_panel` 등에서 계속 쓰이므로 import를 유지한다.
  8. `View.css`의 `.view-directory-panel`/`.view-similar-books-panel`의 좌우 padding은 핸들 폭이 간격을 대신하므로 데스크톱에서는 0으로 줄인다(`@media (max-width: 767.98px)` 규칙은 이미 0).

- [ ] **Step 4: 통과 확인**

Run: `cd frontend && npx vitest run tests/View.test.jsx tests/Admin.test.jsx`
Expected: PASS. 이어서 `cd frontend && npm test` 전체와 `npm run test:e2e`(book-view 계열)를 실행한다. e2e는 환경상 못 돌리면 "미실행"으로 보고한다.

- [ ] **Step 5: 커밋**

```bash
git add frontend/src/View.jsx frontend/src/View.css frontend/tests/View.test.jsx
git commit -m "feat: 책 보기 상단 영역에 SplitLayout 적용" -m "- 디렉토리와 검색 결과 너비를 드래그로 조절
- 본문은 기존대로 전체 폭 유지"
```

---

### Task 6: 전체 검증

- [ ] **Step 1:** `cd frontend && npm test` 전체. Expected: 전부 PASS. 실패는 원인이 이번 변경인지 기존 실패인지 `git stash`로 대조해 구분한다(구분 없이 "기존 실패"로 단정하지 않는다).
- [ ] **Step 2:** `cd frontend && npm run test:e2e`. 못 돌리면 사유와 함께 미실행으로 보고한다.
- [ ] **Step 3:** GitNexus `detect_changes({scope:"compare", base_ref:"main"})`를 실행한다. `partial`/`truncated`이면 다시 실행한다. 변경 심볼이 계획 범위(Edit, CategoryAdmin, BookCoverAdmin, View, SplitLayout)를 벗어나지 않는지 확인한다.
- [ ] **Step 4:** 개발 서버에서 5개 페이지를 데스크톱(1280×800), 낮은 창(800×400), 폭 500px에서 확인하고 항목별 결과 표를 사용자에게 보고한다. 확인하지 못한 항목은 "미확인"으로 적는다.

## Self-Review

- **스펙 대비:** 컴포넌트·핸들·너비 규칙(Task 1), 적용 순서 1~5(Task 1~5), 검증 계획(단위 Task 1·2·3·4·5, e2e Task 4·6)을 모두 맡는 Task가 있다.
- **스펙과 달라진 점(승인 필요 없음, 구현 세부):**
  - `leftCollapsed` prop 대신 `SplitLayout.Left`가 없으면 접힘으로 본다(책 편집의 `{isFolderOpen && …}`를 그대로 쓸 수 있어 diff가 작다).
  - 768px 미만 판정에 CSS 미디어쿼리 대신 기존 `useIsMobile`을 쓴다. 숨겨진 핸들이 키보드 포커스를 받는 문제를 피하고, 기존 Edit 테스트의 `innerWidth` 방식과 맞는다.
  - 책 보기용으로 `scroll={false}` prop을 추가했다(본문이 아래에 이어지므로).
- **알려진 한계:** 폰 세로(768px 미만)의 왼쪽 트리는 여전히 독립 스크롤되지 않는다(스펙 비목표). 합성 터치 이벤트는 실기기 터치를 대신하지 못한다.
