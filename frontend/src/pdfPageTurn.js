/**
 * PDF 1장씩 보기의 좌우 쪽 넘김 판정.
 * 스와이프 기준은 EPUB 뷰어(ViewEPUB.jsx)와 같다.
 * 컴포넌트 파일에서 함수를 export하면 react-refresh 규칙 경고가 나므로 분리했다.
 */

// 가로로 이만큼 이상, 세로 이동의 1.5배 이상 움직여야 스와이프로 본다.
export const SWIPE_MIN_DX = 40;
export const SWIPE_RATIO = 1.5;

// 스와이프를 쪽 이동 방향으로 바꾼다. 왼쪽으로 밀면 다음(+1), 오른쪽이면 이전(-1), 스와이프가 아니면 0.
export function swipeDirection(dx, dy) {
  if (Math.abs(dx) < SWIPE_MIN_DX || Math.abs(dx) < Math.abs(dy) * SWIPE_RATIO)
    return 0;
  return dx < 0 ? 1 : -1;
}

// 방향키를 쪽 이동 방향으로 바꾼다. EPUB 뷰어처럼 오른쪽이 다음, 왼쪽이 이전이다.
export function keyDirection(key) {
  if (key === "ArrowRight") return 1;
  if (key === "ArrowLeft") return -1;
  return 0;
}

// 컨테이너 위쪽(툴바 아래)에 가장 가까운 쪽의 index. 쪽이 없으면 -1.
export function currentPageIndex(container, pages, toolbarHeight) {
  const top = container.getBoundingClientRect().top + toolbarHeight;
  let best = -1;
  let bestDist = Infinity;
  pages.forEach((page, index) => {
    const dist = Math.abs(page.getBoundingClientRect().top - top);
    if (dist < bestDist) {
      bestDist = dist;
      best = index;
    }
  });
  return best;
}
