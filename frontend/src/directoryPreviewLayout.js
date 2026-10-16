/**
 * 디렉토리 첫 파일 미리보기(DirectoryPagePreview)의 크기 계산.
 * 컴포넌트 파일에서 함수를 export하면 react-refresh 규칙 경고가 나므로 분리했다.
 */

// 보여 줄 쪽 수와 이미지 높이(CSS px). 높이만 고정하고 폭은 쪽 비율대로 정해 왜곡이 없다.
export const PREVIEW_PAGE_COUNT = 10;
export const PREVIEW_HEIGHT = 600;
// 쪽당 canvas 면적 상한(pdfRenderScale.js 와 같은 값). 높이 600 CSS px 는 dpr 2 에서 세로 쪽이 이 안쪽이다.
const MAX_CANVAS_PIXELS = 1_500_000;

// 높이가 PREVIEW_HEIGHT(CSS px)가 되는 배율. 면적 상한을 넘으면 낮춘다(표시 크기는 CSS 가 정한다).
export function previewScale(baseWidth, baseHeight, dpr) {
  const byHeight = (PREVIEW_HEIGHT * dpr) / baseHeight;
  const byArea = Math.sqrt(MAX_CANVAS_PIXELS / (baseWidth * baseHeight));
  return Math.min(byHeight, byArea);
}
