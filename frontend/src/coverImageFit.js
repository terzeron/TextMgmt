// 가로 대비 세로가 이 배수 이상인 첫 이미지는 표지가 아닐 가능성이 커서 가로 폭에 맞춰 보여준다.
export const TALL_COVER_RATIO = 2;

// 표지 <img>의 onLoad에서 호출한다. CSS가 data-tall을 보고 맞춤 방식을 고른다.
export function markTallCover(event) {
  const img = event.currentTarget;
  const tall =
    img.naturalWidth > 0 &&
    img.naturalHeight >= img.naturalWidth * TALL_COVER_RATIO;
  img.dataset.tall = tall ? "true" : "false";
}
