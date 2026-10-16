import { useLayoutEffect, useRef, useState } from "react";
import PropTypes from "prop-types";
import RotateRightIcon from "@mui/icons-material/RotateRight";
import {
  boxCenter,
  dimClipPath,
  normalizeAngle,
  resizeBox,
  resizeCursor,
  rotateBox,
  snapAngle,
  unionDimRects,
} from "./cropRect";

const HANDLES = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
const SNAP_STEP = 15;

const percent = (value) => `${value * 100}%`;

// 남길 영역(박스)을 보여 주고, 박스들 바깥은 회색+블러로 덮어 잘려나갈 부분을 미리 보여 준다.
// 부모(.pdf-page-wrap)와 같은 크기의 레이어를 깔고, 박스마다 변·모서리 8곳을 드래그해 크기를 조절한다.
// 박스는 중심을 기준으로 회전할 수 있다(회전 손잡이). 조절·회전 핸들은 모두 박스 안쪽에만 있어서
// 박스끼리 맞닿아도 서로의 핸들이 겹치지 않는다.
// boxes: [{ id, rect: { x0, y0, x1, y1, angle }, color, label? }], onChange(id, rect)
export default function PdfCropBox({ boxes, onChange }) {
  const layerRef = useRef(null);
  const dragRef = useRef(null);
  // 서로 겹친 박스는 마지막에 만진 박스를 위로 올려 핸들을 잡을 수 있게 한다.
  const [activeId, setActiveId] = useState(null);
  // 기울어진 박스의 덮개와 꼭짓점 계산에는 쪽의 화면 크기가 필요하다.
  const [size, setSize] = useState({ width: 0, height: 0 });

  useLayoutEffect(() => {
    const layer = layerRef.current;
    if (!layer) return undefined;
    const measure = () => {
      const { width, height } = layer.getBoundingClientRect();
      setSize((prev) =>
        prev.width === width && prev.height === height
          ? prev
          : { width, height },
      );
    };
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(layer);
    return () => observer.disconnect();
  }, []);

  const layerPoint = (event) => {
    const bounds = layerRef.current.getBoundingClientRect();
    return {
      x: event.clientX - bounds.left,
      y: event.clientY - bounds.top,
      width: bounds.width,
      height: bounds.height,
    };
  };

  const startDrag = (box, event, drag) => {
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setActiveId(box.id);
    dragRef.current = { id: box.id, rect: box.rect, ...drag };
  };

  const handleResizeDown = (box, handle) => (event) =>
    startDrag(box, event, {
      kind: "resize",
      handle,
      startX: event.clientX,
      startY: event.clientY,
    });

  const handleRotateDown = (box) => (event) => {
    const point = layerPoint(event);
    const center = boxCenter(box.rect, point.width, point.height);
    startDrag(box, event, {
      kind: "rotate",
      startAngle: box.rect.angle ?? 0,
      startPointer: Math.atan2(point.y - center.y, point.x - center.x),
      lastAngle: box.rect.angle ?? 0,
    });
  };

  const handlePointerMove = (event) => {
    const drag = dragRef.current;
    const layer = layerRef.current;
    if (!drag || !layer) return;
    const point = layerPoint(event);
    if (!point.width || !point.height) return;
    if (drag.kind === "resize") {
      onChange(
        drag.id,
        resizeBox(
          drag.rect,
          drag.handle,
          event.clientX - drag.startX,
          event.clientY - drag.startY,
          point.width,
          point.height,
        ),
      );
      return;
    }
    const center = boxCenter(drag.rect, point.width, point.height);
    const pointer = Math.atan2(point.y - center.y, point.x - center.x);
    let target = drag.startAngle + ((pointer - drag.startPointer) * 180) / Math.PI;
    target = event.shiftKey ? snapAngle(target, SNAP_STEP) : normalizeAngle(target);
    const angle = rotateBox(
      drag.rect,
      drag.lastAngle,
      target,
      point.width,
      point.height,
    );
    drag.lastAngle = angle;
    onChange(drag.id, { ...drag.rect, angle });
  };

  const handlePointerEnd = (event) => {
    if (!dragRef.current) return;
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    dragRef.current = null;
  };

  const anyRotated = boxes.some((box) => Math.abs(box.rect.angle ?? 0) > 1e-9);
  const measured = size.width > 0 && size.height > 0;

  return (
    <div
      ref={layerRef}
      className="pdf-crop-layer"
      data-testid="pdf-crop-layer"
    >
      {anyRotated && measured ? (
        <div
          className="pdf-crop-dim pdf-crop-dim-rotated"
          data-testid="pdf-crop-dim-rotated"
          style={{
            inset: 0,
            clipPath: dimClipPath(
              boxes.map((box) => box.rect),
              size.width,
              size.height,
            ),
          }}
        />
      ) : (
        unionDimRects(boxes.map((box) => box.rect)).map((piece, index) => (
          <div
            // 조각은 박스 위치에 따라 매번 새로 계산되는 순수 파생값이라 index key 로 충분하다.
            key={index}
            className="pdf-crop-dim"
            style={{
              left: percent(piece.left),
              top: percent(piece.top),
              width: percent(piece.width),
              height: percent(piece.height),
            }}
          />
        ))
      )}
      {boxes.map((box) => {
        const { x0, y0, x1, y1 } = box.rect;
        const angle = box.rect.angle ?? 0;
        const tilted = Math.abs(angle) >= 0.05;
        return (
          <div
            key={box.id}
            className="pdf-crop-box"
            data-testid="pdf-crop-box"
            data-box={box.id}
            data-angle={angle}
            style={{
              "--crop-color": box.color,
              left: percent(x0),
              top: percent(y0),
              width: percent(x1 - x0),
              height: percent(y1 - y0),
              zIndex: activeId === box.id ? 2 : 1,
              transform: tilted ? `rotate(${angle}deg)` : undefined,
            }}
          >
            {(box.label || tilted) && (
              <span className="pdf-crop-label">
                {[box.label, tilted && `${angle.toFixed(1)}°`]
                  .filter(Boolean)
                  .join(" ")}
              </span>
            )}
            {HANDLES.map((handle) => (
              <div
                key={handle}
                className={`pdf-crop-handle pdf-crop-handle-${handle}`}
                data-handle={handle}
                style={{ cursor: resizeCursor(handle, angle) }}
                onPointerDown={handleResizeDown(box, handle)}
                onPointerMove={handlePointerMove}
                onPointerUp={handlePointerEnd}
                onPointerCancel={handlePointerEnd}
              />
            ))}
            <div
              className="pdf-crop-rotate"
              data-testid="pdf-crop-rotate"
              title="드래그해 박스를 회전 (Shift: 15° 단위)"
              onPointerDown={handleRotateDown(box)}
              onPointerMove={handlePointerMove}
              onPointerUp={handlePointerEnd}
              onPointerCancel={handlePointerEnd}
            >
              <RotateRightIcon fontSize="inherit" />
            </div>
          </div>
        );
      })}
    </div>
  );
}

PdfCropBox.propTypes = {
  boxes: PropTypes.arrayOf(
    PropTypes.shape({
      id: PropTypes.string.isRequired,
      rect: PropTypes.shape({
        x0: PropTypes.number.isRequired,
        y0: PropTypes.number.isRequired,
        x1: PropTypes.number.isRequired,
        y1: PropTypes.number.isRequired,
        angle: PropTypes.number,
      }).isRequired,
      color: PropTypes.string.isRequired,
      label: PropTypes.string,
    }),
  ).isRequired,
  onChange: PropTypes.func.isRequired,
};
