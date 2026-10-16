// 좌표는 모두 쪽 크기 대비 0~1 비율이고 원점은 좌상단이다. 서버도 같은 좌표를 받는다.
// angle 은 박스 중심 기준 시계 방향 회전각(도)이다.
export const DEFAULT_CROP_RECT = Object.freeze({
  x0: 0.1,
  y0: 0.1,
  x1: 0.9,
  y1: 0.9,
  angle: 0,
});
// 한 변이 이보다 작아지지 않게 한다. 서버도 8픽셀 미만은 거절한다.
export const MIN_CROP_FRACTION = 0.05;

const clamp = (value, low, high) => Math.min(high, Math.max(low, value));

// handle(n/s/e/w 조합)에 해당하는 변만 dx, dy(비율)만큼 옮긴 새 사각형을 돌려준다.
// 쪽 밖으로 나가거나 반대편 변을 넘지 않게 자른다.
export function resizeRect(rect, handle, dx, dy, min = MIN_CROP_FRACTION) {
  let { x0, y0, x1, y1 } = rect;
  if (handle.includes("w")) x0 = clamp(rect.x0 + dx, 0, rect.x1 - min);
  if (handle.includes("e")) x1 = clamp(rect.x1 + dx, rect.x0 + min, 1);
  if (handle.includes("n")) y0 = clamp(rect.y0 + dy, 0, rect.y1 - min);
  if (handle.includes("s")) y1 = clamp(rect.y1 + dy, rect.y0 + min, 1);
  return { ...rect, x0, y0, x1, y1 };
}

// 분할: 앞쪽(first)이 될 영역과 뒤쪽(second)이 될 영역의 기본값. 쪽의 네 모서리에 맞춰 정확히 가운데(x=0.5)에서
// 나뉘는 서로소 두 박스다: first 가 왼쪽 절반 전체, second 가 오른쪽 절반 전체를 차지한다.
export const DEFAULT_SPLIT_RECTS = Object.freeze({
  first: Object.freeze({ x0: 0, y0: 0, x1: 0.5, y1: 1, angle: 0 }),
  second: Object.freeze({ x0: 0.5, y0: 0, x1: 1, y1: 1, angle: 0 }),
});

// 박스들의 합집합 바깥을 {left, top, width, height}(0~1 비율) 직사각형 조각으로 나눠 돌려준다.
// 박스마다 따로 덮으면 서로 겹친 영역이 다시 어두워지므로, 세로 띠로 자른 뒤 띠마다 비는 구간만 덮는다.
export function unionDimRects(rects) {
  const xs = [
    ...new Set([0, 1, ...rects.flatMap((rect) => [rect.x0, rect.x1])]),
  ].sort((a, b) => a - b);
  const pieces = [];
  for (let i = 0; i < xs.length - 1; i += 1) {
    const left = xs[i];
    const width = xs[i + 1] - left;
    if (width <= 1e-9) continue;
    const middle = left + width / 2;
    const spans = rects
      .filter((rect) => rect.x0 <= middle && middle <= rect.x1)
      .map((rect) => [rect.y0, rect.y1])
      .sort((a, b) => a[0] - b[0]);
    let cursor = 0;
    for (const [y0, y1] of spans) {
      if (y0 > cursor) {
        pieces.push({ left, top: cursor, width, height: y0 - cursor });
      }
      cursor = Math.max(cursor, y1);
    }
    if (cursor < 1) {
      pieces.push({ left, top: cursor, width, height: 1 - cursor });
    }
  }
  return pieces;
}

const DEG = Math.PI / 180;
// 기울어진 박스의 네 꼭짓점이 쪽 밖으로 나가는 것을 허용하는 오차(px). 서버는 이미지 픽셀 1.5까지 허용한다.
const PAGE_TOLERANCE_PX = 0.5;

// 각도를 (-180, 180] 로 맞춘다.
export function normalizeAngle(angle) {
  const wrapped = ((((angle + 180) % 360) + 360) % 360) - 180;
  return wrapped === -180 ? 180 : wrapped;
}

// step 도 단위로 반올림한다(Shift 로 회전할 때).
export function snapAngle(angle, step = 15) {
  return normalizeAngle(Math.round(angle / step) * step);
}

// 박스의 중심(px). rect 는 쪽 크기(width × height px) 대비 0~1 비율이다.
export function boxCenter(rect, width, height) {
  return {
    x: ((rect.x0 + rect.x1) / 2) * width,
    y: ((rect.y0 + rect.y1) / 2) * height,
  };
}

// 회전한 박스의 네 꼭짓점(px), 위쪽 왼쪽부터 시계 방향.
export function boxCorners(rect, width, height) {
  const { x, y } = boxCenter(rect, width, height);
  const halfW = ((rect.x1 - rect.x0) * width) / 2;
  const halfH = ((rect.y1 - rect.y0) * height) / 2;
  const cos = Math.cos((rect.angle ?? 0) * DEG);
  const sin = Math.sin((rect.angle ?? 0) * DEG);
  return [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ].map(([sx, sy]) => [
    x + cos * sx * halfW - sin * sy * halfH,
    y + sin * sx * halfW + cos * sy * halfH,
  ]);
}

// 네 꼭짓점이 모두 쪽 안에 있는가. 서버도 같은 조건을 요구한다.
export function insidePage(rect, width, height) {
  return boxCorners(rect, width, height).every(
    ([x, y]) =>
      x >= -PAGE_TOLERANCE_PX &&
      x <= width + PAGE_TOLERANCE_PX &&
      y >= -PAGE_TOLERANCE_PX &&
      y <= height + PAGE_TOLERANCE_PX,
  );
}

// 0(가능)~1(불가능) 사이에서 attempt(t) 가 쪽 안에 머무는 가장 큰 t 를 이분 탐색한다.
function largestFeasible(attempt, feasible) {
  if (feasible(attempt(1))) return attempt(1);
  let low = 0;
  let high = 1;
  for (let i = 0; i < 20; i += 1) {
    const mid = (low + high) / 2;
    if (feasible(attempt(mid))) low = mid;
    else high = mid;
  }
  return attempt(low);
}

// handle 에 해당하는 변·모서리를 포인터 이동량(dx, dy: px)만큼 옮긴다. 박스의 기울어진 축을 따라 움직이고,
// 반대편 변은 화면에서 고정된다.
function applyRotatedResize(rect, handle, dx, dy, width, height, min) {
  const cos = Math.cos((rect.angle ?? 0) * DEG);
  const sin = Math.sin((rect.angle ?? 0) * DEG);
  // 포인터 이동량을 박스 자신의 축(회전 전 좌표계)으로 옮긴다.
  const localX = dx * cos + dy * sin;
  const localY = -dx * sin + dy * cos;
  const oldW = (rect.x1 - rect.x0) * width;
  const oldH = (rect.y1 - rect.y0) * height;
  let newW = oldW;
  let newH = oldH;
  let shiftX = 0;
  let shiftY = 0;
  if (handle.includes("e")) {
    newW = Math.max(min * width, oldW + localX);
    shiftX = (newW - oldW) / 2;
  } else if (handle.includes("w")) {
    newW = Math.max(min * width, oldW - localX);
    shiftX = (oldW - newW) / 2;
  }
  if (handle.includes("s")) {
    newH = Math.max(min * height, oldH + localY);
    shiftY = (newH - oldH) / 2;
  } else if (handle.includes("n")) {
    newH = Math.max(min * height, oldH - localY);
    shiftY = (oldH - newH) / 2;
  }
  const center = boxCenter(rect, width, height);
  const cx = center.x + cos * shiftX - sin * shiftY;
  const cy = center.y + sin * shiftX + cos * shiftY;
  return {
    ...rect,
    x0: (cx - newW / 2) / width,
    x1: (cx + newW / 2) / width,
    y0: (cy - newH / 2) / height,
    y1: (cy + newH / 2) / height,
  };
}

// 박스를 크기 조절한다. 각도 0 이면 변이 쪽 경계에서 멈추고, 기울어졌으면 네 꼭짓점이 쪽 안에 있는
// 가장 멀리까지만 움직인다. width, height 는 쪽의 화면 크기(px)이다.
export function resizeBox(
  rect,
  handle,
  dx,
  dy,
  width,
  height,
  min = MIN_CROP_FRACTION,
) {
  if (Math.abs(rect.angle ?? 0) < 1e-9) {
    return resizeRect(rect, handle, dx / width, dy / height, min);
  }
  return largestFeasible(
    (t) => applyRotatedResize(rect, handle, dx * t, dy * t, width, height, min),
    (candidate) => insidePage(candidate, width, height),
  );
}

// fromAngle 에서 toAngle 로 돌리되, 네 꼭짓점이 쪽 안에 머무는 가장 먼 각도까지만 돌린다.
export function rotateBox(rect, fromAngle, toAngle, width, height) {
  const delta = normalizeAngle(toAngle - fromAngle);
  const at = (t) => ({ ...rect, angle: fromAngle + delta * t });
  const result = largestFeasible(at, (candidate) =>
    insidePage(candidate, width, height),
  );
  return normalizeAngle(result.angle);
}

// 박스 방향에 맞는 크기 조절 커서. 핸들의 기준 방향에 각도를 더해 8방향 중 가까운 것을 고른다.
const HANDLE_DIRECTION = { n: 0, ne: 45, e: 90, se: 135, s: 180, sw: 225, w: 270, nw: 315 };
const CURSORS = ["ns-resize", "nesw-resize", "ew-resize", "nwse-resize"];
export function resizeCursor(handle, angle = 0) {
  const direction = (((HANDLE_DIRECTION[handle] + angle) % 180) + 180) % 180;
  return CURSORS[Math.round(direction / 45) % 4];
}

// 기울어진 박스가 하나라도 있을 때 박스들 바깥을 덮는 clip-path. 바깥 사각형과 박스마다의 구멍을
// 한 경로로 이어 even-odd 로 구멍을 뚫는다. 박스끼리 겹치면 겹친 부분이 다시 덮이는 한계가 있다.
export function dimClipPath(rects, width, height) {
  const percent = (value, total) => `${(value / total) * 100}%`;
  const holes = rects.map((rect) => {
    const points = boxCorners(rect, width, height).map(
      ([x, y]) => `${percent(x, width)} ${percent(y, height)}`,
    );
    return [...points, points[0], "0% 0%"].join(", ");
  });
  return `polygon(evenodd, 0% 0%, 100% 0%, 100% 100%, 0% 100%, 0% 0%, ${holes.join(", ")})`;
}
