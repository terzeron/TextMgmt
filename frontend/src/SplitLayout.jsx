/**
 * 왼쪽 디렉토리 / 오른쪽 작업 공간 두 컬럼 레이아웃.
 * 두 pane이 따로 스크롤되고, 가운데 핸들을 끌거나 화살표 키로 왼쪽 너비를 바꾼다.
 * 768px 미만에서는 위아래로 쌓고 핸들을 렌더하지 않는다.
 */
import { Children, Fragment, useCallback, useRef, useState } from "react";
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
      {otherItems}
    </Fragment>
  );
}

SplitLayout.Left = SplitLeft;
SplitLayout.Right = SplitRight;
