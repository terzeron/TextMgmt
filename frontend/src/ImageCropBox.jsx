import { Fragment, useRef, useState } from "react";
import PropTypes from "prop-types";
import RotateRightIcon from "@mui/icons-material/RotateRight";
import "./ImageCropBox.css";
import {
  CROP_COLOR,
  GRID_DIVISIONS,
  SPLIT_COLORS,
  boxCenter,
  normalizeAngle,
  resizeBox,
  resizeCursor,
  rotateBox,
  snapAngle,
} from "./cropRect";

const HANDLES = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
const SNAP_STEP = 15;

const percent = (value) => `${value * 100}%`;
// 박스 안쪽을 GRID_DIVISIONS 등분하는 선의 위치(0~100). 가로·세로 모두 같은 위치를 쓴다.
const GRID_LINES = Array.from({ length: GRID_DIVISIONS - 1 }, (_, i) => ((i + 1) * 100) / GRID_DIVISIONS);

// 남길 영역(박스)을 보여 주고, 박스 안에는 기울기를 눈으로 확인할 점선 격자를 깐다. 박스 바깥은 흐리게 하지 않는다.
// PDF 쪽과 EPUB 이미지가 함께 쓴다. 부모(.pdf-page-wrap 등 이미지와 같은 크기의 상자)를 덮는 레이어를 깔고,
// 박스마다 변·모서리 8곳을 드래그해 크기를 조절한다.
// 박스는 중심을 기준으로 회전할 수 있다(회전 손잡이). 조절·회전 핸들은 모두 박스 안쪽에만 있어서
// 박스끼리 맞닿아도 서로의 핸들이 겹치지 않는다.
// boxes: [{ id, rect: { x0, y0, x1, y1, angle }, color, label? }], onChange(id, rect)
export default function ImageCropBox({ boxes, onChange }) {
  const layerRef = useRef(null);
  const dragRef = useRef(null);
  // 서로 겹친 박스는 마지막에 만진 박스를 위로 올려 핸들을 잡을 수 있게 한다.
  const [activeId, setActiveId] = useState(null);

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

  return (
    <div
      ref={layerRef}
      className="image-crop-layer"
      data-testid="image-crop-layer"
    >
      {boxes.map((box) => {
        const { x0, y0, x1, y1 } = box.rect;
        const angle = box.rect.angle ?? 0;
        const tilted = Math.abs(angle) >= 0.05;
        return (
          <div
            key={box.id}
            className="image-crop-box"
            data-testid="image-crop-box"
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
            <svg
              className="image-crop-grid"
              data-testid="image-crop-grid"
              viewBox="0 0 100 100"
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              {GRID_LINES.map((at) => (
                <Fragment key={at}>
                  <line x1={at} y1="0" x2={at} y2="100" />
                  <line x1="0" y1={at} x2="100" y2={at} />
                </Fragment>
              ))}
            </svg>
            {(box.label || tilted) && (
              <span className="image-crop-label">
                {[box.label, tilted && `${angle.toFixed(1)}°`]
                  .filter(Boolean)
                  .join(" ")}
              </span>
            )}
            {HANDLES.map((handle) => (
              <div
                key={handle}
                className={`image-crop-handle image-crop-handle-${handle}`}
                data-handle={handle}
                style={{ cursor: resizeCursor(handle, angle) }}
                onPointerDown={handleResizeDown(box, handle)}
                onPointerMove={handlePointerMove}
                onPointerUp={handlePointerEnd}
                onPointerCancel={handlePointerEnd}
              />
            ))}
            <div
              className="image-crop-rotate"
              data-testid="image-crop-rotate"
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

ImageCropBox.propTypes = {
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

// 변경 표시(mark)의 크롭·분할 박스를 그린다. 쪽 삭제 등 박스가 없는 표시면 아무것도 그리지 않는다.
// PDF 쪽과 EPUB 이미지가 같은 박스 모양(색·라벨)을 쓰게 하는 얇은 껍데기다.
export function MarkCropBox({ mark, onChange }) {
  if (mark?.kind === "crop") {
    return (
      <ImageCropBox
        boxes={[{ id: "crop", rect: mark.rect, color: CROP_COLOR }]}
        onChange={onChange}
      />
    );
  }
  if (mark?.kind === "split") {
    return (
      <ImageCropBox
        boxes={[
          { id: "first", rect: mark.rects.first, color: SPLIT_COLORS.first, label: "앞" },
          { id: "second", rect: mark.rects.second, color: SPLIT_COLORS.second, label: "뒤" },
        ]}
        onChange={onChange}
      />
    );
  }
  return null;
}

MarkCropBox.propTypes = {
  mark: PropTypes.shape({
    kind: PropTypes.string.isRequired,
    rect: PropTypes.object,
    rects: PropTypes.object,
  }),
  onChange: PropTypes.func.isRequired,
};
