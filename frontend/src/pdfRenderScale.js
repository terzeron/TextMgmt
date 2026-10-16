/**
 * PDF 뷰어 렌더 배율 계산.
 * 컴포넌트 파일에서 함수를 export하면 react-refresh 규칙 경고가 나므로 분리했다.
 */

// 화면 표시 크기(CSS)를 정하는 기준 배율.
export const DISPLAY_SCALE = 1.2;
// 페이지당 canvas 면적 상한. 그린 canvas는 해제하지 않으므로 100쪽 넘는 만화에서 iPad 메모리가 차지 않게 잡는다.
// (iOS Safari 단일 canvas 상한 16,777,216px보다도 충분히 낮다.)
const MAX_CANVAS_PIXELS = 1_500_000;
// canvas 짧은 변의 최소 픽셀 수. 이보다 작으면 쪽이 보이지 않는다.
const MIN_CANVAS_SIDE = 100;

// 화면에 실제로 보이는 폭(CSS px)에 맞춰 렌더 배율을 정한다.
// 만화 PDF는 페이지 크기(pt)가 작고 이미지는 고해상도라서, 고정 배율로는 이미지가 크게 줄어든다.
export function computeRenderScale(page, canvas, dpr) {
  const base = page.getViewport({ scale: 1 });
  const floor = DISPLAY_SCALE * dpr;
  const cssWidth = canvas?.clientWidth || 0;
  if (!base.width || !base.height) return floor;
  // 쪽 크기가 비정상적으로 작은 PDF(0.09pt 등)는 floor 배율이면 canvas 가 0x0 이 된다.
  // 화면 폭을 아직 모를 때도 짧은 변이 MIN_CANVAS_SIDE 픽셀은 되도록 배율을 올린다.
  const minScale = MIN_CANVAS_SIDE / Math.min(base.width, base.height);
  if (!cssWidth) return Math.max(floor, minScale);
  const wanted = (cssWidth * dpr) / base.width;
  const areaCap = Math.sqrt(MAX_CANVAS_PIXELS / (base.width * base.height));
  return Math.max(floor, Math.min(wanted, areaCap));
}
