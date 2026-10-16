/**
 * 디렉토리 첫 파일 미리보기(DirectoryPagePreview)의 크기 계산.
 * 컴포넌트 파일에서 함수를 export하면 react-refresh 규칙 경고가 나므로 분리했다.
 */

// 보여 줄 쪽 수와 이미지 높이(CSS px). 높이만 고정하고 폭은 쪽 비율대로 정해 왜곡이 없다.
export const PREVIEW_PAGE_COUNT = 10;
export const PREVIEW_HEIGHT = 900;
// 쪽당 canvas 면적 상한. 10장뿐이라 뷰어(1.5M)보다 넉넉하게 잡아, 세로 쪽을 dpr 2 로 높이 900 CSS px 에
// 그려도(1350x1800 = 2.43M) 흐려지지 않게 한다. 10장 합계는 약 26M 픽셀(104MB) 이내다.
const MAX_CANVAS_PIXELS = 2_600_000;

// 높이가 PREVIEW_HEIGHT(CSS px)가 되는 배율. 면적 상한을 넘으면 낮춘다(표시 크기는 CSS 가 정한다).
export function previewScale(baseWidth, baseHeight, dpr) {
  const byHeight = (PREVIEW_HEIGHT * dpr) / baseHeight;
  const byArea = Math.sqrt(MAX_CANVAS_PIXELS / (baseWidth * baseHeight));
  return Math.min(byHeight, byArea);
}
