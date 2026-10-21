// 가로 대비 세로가 이 배수 이상인 첫 이미지는 표지가 아닐 가능성이 커서 가로 폭에 맞춰 보여준다.
export const TALL_COVER_RATIO = 2;

// 서버가 만드는 썸네일 폭(backend COVER_THUMBNAIL_WIDTHS)의 1x/2x 두 단계. 카드 폭 상한(--cover-card-max, 150px)에 맞춘다.
export const COVER_THUMB_WIDTHS = [150, 300];

// 썸네일 URL을 1x/2x srcSet으로 펼친다. 브라우저가 화면 배율(DPR)에 맞는 쪽만 받는다.
// src는 srcSet을 못 읽는 환경용이며 이전 화질(큰 쪽)을 유지한다.
export function coverThumbProps(url) {
  const [small, large] = COVER_THUMB_WIDTHS.map(
    (width) => `${url}${url.includes("?") ? "&" : "?"}w=${width}`,
  );
  return { src: large, srcSet: `${small} 1x, ${large} 2x` };
}

// 표지 <img>의 onLoad에서 호출한다. CSS가 data-tall을 보고 맞춤 방식을 고른다.
export function markTallCover(event) {
  const img = event.currentTarget;
  const tall =
    img.naturalWidth > 0 &&
    img.naturalHeight >= img.naturalWidth * TALL_COVER_RATIO;
  img.dataset.tall = tall ? "true" : "false";
}
