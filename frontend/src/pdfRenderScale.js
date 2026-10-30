/**
 * PDF 뷰어 렌더 배율 계산.
 * 컴포넌트 파일에서 함수를 export하면 react-refresh 규칙 경고가 나므로 분리했다.
 */

// 화면 표시 크기(CSS)를 정하는 기준 배율.
export const DISPLAY_SCALE = 1.2;
// 페이지당 canvas 면적 상한(픽셀). 비트맵은 쪽당 4바이트 x 면적이라 3M 이면 12MB 다.
// 보이는 쪽과 남겨 두는 쪽을 합쳐 10장쯤 살아 있어도 120MB 안에 든다. iOS Safari 는 canvas 메모리가
// 수백 MB 를 넘으면 탭을 강제로 다시 불러온다(실측: dpr 3, 쪽당 17M px 가 9장 쌓인 183M px 에서 크래시).
// 휴대폰 화면 폭(약 1170px)에 맞춘 한 쪽이 약 2M px 라서 이 상한에서도 선명하다.
const MAX_CANVAS_PIXELS = 3_000_000;
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
  // 면적 상한이 기본 배율(floor)보다 우선한다. floor 가 우선이면 큰 쪽에서 화면보다 훨씬 큰 비트맵을 만든다.
  // 쪽 크기가 비정상적으로 작은 PDF 는 areaCap 이 매우 커서 영향이 없고, 짧은 변은 minScale 이 지킨다.
  return Math.max(minScale, Math.min(Math.max(floor, wanted), areaCap));
}

// 쪽 전체를 이미지 한 장으로 채운 PDF(만화·스캔본)의 이미지 픽셀 크기를 돌려준다. 아니면 null.
// 이런 PDF 의 쪽 크기(pt)는 만든 도구가 임의로 정한 값이라(예: 448px 이미지를 300dpi 로 107.52pt) 그대로 쓰면
// 원본 이미지보다 훨씬 작게 보인다. 이미지 픽셀 폭을 기준 표시 폭으로 삼으면 원래 크기로 보인다.
// paintOp 는 pdfjs.OPS.paintImageXObject 이고 인자는 [objId, width, height]다.
const ASPECT_TOLERANCE = 0.02;
export async function getFullPageImageSize(page, paintOp) {
  try {
    if (typeof page?.getOperatorList !== "function" || paintOp === undefined) return null;
    const { fnArray, argsArray } = await page.getOperatorList();
    const images = [];
    fnArray.forEach((fn, i) => {
      if (fn === paintOp) images.push(argsArray[i]);
    });
    if (images.length !== 1) return null;
    const [, width, height] = images[0];
    const base = page.getViewport({ scale: 1 });
    if (!(width > 0 && height > 0 && base.width > 0 && base.height > 0)) return null;
    const imageAspect = height / width;
    const pageAspect = base.height / base.width;
    if (Math.abs(imageAspect - pageAspect) / pageAspect > ASPECT_TOLERANCE) return null;
    return { width, height };
  } catch {
    return null;
  }
}
