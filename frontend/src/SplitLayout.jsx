/**
 * 왼쪽 디렉토리 / 오른쪽 작업 공간 두 컬럼 레이아웃.
 * 두 pane이 따로 스크롤되고, 가운데 핸들을 끌거나 화살표 키로 왼쪽 너비를 바꾼다.
 * 768px 미만에서는 위아래로 쌓고 핸들을 렌더하지 않는다.
 */
import {
  Children,
  Fragment,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
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

function clearStoredWidth(key) {
  try {
    window.localStorage.removeItem(key);
  } catch {
    // 삭제 실패는 조절 동작에 영향을 주지 않는다.
  }
}

// 0.28 * 100 같은 부동소수점 오차가 CSS 값에 남지 않게 한다.
const pct = (ratio) => Number((ratio * 100).toFixed(4));

function SplitLeft() {
  return null;
}

function SplitRight() {
  return null;
}

export default function SplitLayout({
  storageKey,
  defaultLeftRatio = 0.25,
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
    () => (storageKey && readStoredWidth(storageKey)) || null,
  );

  // 컨테이너의 현재 너비와 문서 위쪽 간격. 창 크기나 위쪽 내용이 바뀌면 다시 잰다.
  const [metrics, setMetrics] = useState({ width: 0, top: null });

  const measure = useCallback(() => {
    const node = containerRef.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    const top = Math.round(rect.top + window.scrollY);
    setMetrics((prev) =>
      prev.width === rect.width && prev.top === top
        ? prev
        : { width: rect.width, top },
    );
  }, []);

  useLayoutEffect(() => {
    measure();
    window.addEventListener("resize", measure);
    let observer;
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(measure);
      observer.observe(containerRef.current);
      observer.observe(document.body);
    }
    return () => {
      window.removeEventListener("resize", measure);
      observer?.disconnect();
    };
  }, [measure]);

  const clampWidth = useCallback(
    (value) => {
      const total = metrics.width;
      const max = total ? Math.max(minLeft, total * maxLeftRatio) : Infinity;
      return Math.round(Math.min(Math.max(value, minLeft), max));
    },
    [metrics.width, minLeft, maxLeftRatio],
  );

  // 저장된 너비가 없으면(null) 컨테이너 너비의 defaultLeftRatio를 따라간다.
  const ratioWidth = metrics.width ? metrics.width * defaultLeftRatio : minLeft;
  const leftWidth = clampWidth(rawWidth ?? ratioWidth);
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
  // 모달처럼 pane이 아닌 자식은 grid 칸을 차지하지 않도록 grid 밖에 그대로 렌더한다.
  const otherItems = items.filter(
    (item) => item !== leftItem && item !== rightItem,
  );

  const onPointerDown = (event) => {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    try {
      event.currentTarget.setPointerCapture?.(event.pointerId);
    } catch {
      // 이미 끝난 포인터면 캡처만 포기하고 드래그는 이어간다.
    }
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
    try {
      event.currentTarget.releasePointerCapture?.(event.pointerId);
    } catch {
      // 캡처가 이미 풀렸으면 무시한다.
    }
    commit(widthRef.current, { persist: true });
  };

  const resetWidth = () => {
    setRawWidth(null);
    if (storageKey) clearStoredWidth(storageKey);
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
  const leftCss =
    rawWidth === null
      ? `clamp(${minLeft}px, ${pct(defaultLeftRatio)}%, ${pct(maxLeftRatio)}%)`
      : `min(${leftWidth}px, ${pct(maxLeftRatio)}%)`;
  let style = {};
  if (!isStacked && leftItem && rightItem) {
    style.gridTemplateColumns = `${leftCss} ${HANDLE_WIDTH}px minmax(0, 1fr)`;
  } else if (!isStacked && leftItem) {
    style.gridTemplateColumns = leftCss;
  }
  // 높이는 화면에서 컨테이너 위쪽 간격을 뺀 만큼. 측정 전에는 CSS 기본값을 쓴다.
  if (scroll && metrics.top !== null) style["--split-top"] = `${metrics.top}px`;

  const classes = [
    "split-layout",
    isStacked ? "is-stacked" : "",
    scroll ? "" : "no-scroll",
    className,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <Fragment>
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
            onDoubleClick={resetWidth}
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
      {otherItems}
    </Fragment>
  );
}

SplitLayout.Left = SplitLeft;
SplitLayout.Right = SplitRight;
