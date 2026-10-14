/**
 * PDF 뷰어 렌더 배율 계산.
 * 컴포넌트 파일에서 함수를 export하면 react-refresh 규칙 경고가 나므로 분리했다.
 */

// 화면 표시 크기(CSS)를 정하는 기준 배율.
export const DISPLAY_SCALE = 1.2;
// 페이지당 canvas 면적 상한. 그린 canvas는 해제하지 않으므로 100쪽 넘는 만화에서 iPad 메모리가 차지 않게 잡는다.
// (iOS Safari 단일 canvas 상한 16,777,216px보다도 충분히 낮다.)
const MAX_CANVAS_PIXELS = 1_500_000;

// 화면에 실제로 보이는 폭(CSS px)에 맞춰 렌더 배율을 정한다.
// 만화 PDF는 페이지 크기(pt)가 작고 이미지는 고해상도라서, 고정 배율로는 이미지가 크게 줄어든다.
export function computeRenderScale(page, canvas, dpr) {
  const base = page.getViewport({ scale: 1 });
  const floor = DISPLAY_SCALE * dpr;
  const cssWidth = canvas?.clientWidth || 0;
  if (!cssWidth || !base.width || !base.height) return floor;
  const wanted = (cssWidth * dpr) / base.width;
  const areaCap = Math.sqrt(MAX_CANVAS_PIXELS / (base.width * base.height));
  return Math.max(floor, Math.min(wanted, areaCap));
}
