// 지금 보이는 EPUB 쪽(spine 항목)의 이미지들을 뷰어 상자 기준 화면 좌표로 잰다.
// 책 본문은 iframe 안에서 그려지므로 iframe 의 위치와 이미지의 iframe 안 위치를 더해 구한다.
//
// 돌려주는 값: { count, imageOnly, images }
// - count: 쪽 안의 이미지(<img>, <svg><image>) 수.
// - imageOnly: 글 없이 이미지만 있는 쪽인가.
// - images: 문서 순서대로 [{ index, rect, img }]. index 는 서버가 쓰는 쪽 안 이미지 순번이다.
//   rect: { left, top, width, height }(px, container 기준). 이미지가 뷰어 밖(다른 단·쪽)에 걸쳐 있어
//   박스를 맞출 수 없으면 null. img: 잰 요소(아직 읽는 중이면 load 를 기다려 다시 재는 데 쓴다).
// 쪽을 아직 그리지 못했으면 null 이다.
export function measureCurrentImage(rendition, container, idref) {
  const views = rendition?.manager?.views;
  const list = views?.displayed?.() ?? views?.all?.() ?? [];
  const view = list.find((candidate) => candidate?.section?.idref === idref) ?? list[0];
  const doc = view?.contents?.document;
  if (!doc?.body || !container) return null;
  // <img> 뿐 아니라 <svg><image> 로 이미지를 넣은 쪽(출판사 표지 등)도 이미지로 센다.
  const elements = [...(doc.querySelectorAll?.("img, svg image") ?? doc.images ?? [])];
  const imageOnly = elements.length === 1 && !(doc.body.textContent || "").trim();
  const frame = view.iframe ?? doc.defaultView?.frameElement;
  const containerBox = container.getBoundingClientRect();
  const images = elements.map((img, index) => {
    if (!frame) return { index, rect: null, img };
    const imageBox = img.getBoundingClientRect();
    const frameBox = frame.getBoundingClientRect();
    const rect = {
      left: frameBox.left + imageBox.left - containerBox.left,
      top: frameBox.top + imageBox.top - containerBox.top,
      width: imageBox.width,
      height: imageBox.height,
    };
    const visible =
      rect.width > 0 &&
      rect.height > 0 &&
      rect.left >= -1 &&
      rect.top >= -1 &&
      rect.left + rect.width <= containerBox.width + 1 &&
      rect.top + rect.height <= containerBox.height + 1;
    return { index, rect: visible ? rect : null, img };
  });
  return { count: elements.length, imageOnly, images };
}

// 표지 화면(.epub-cover-page)의 <img>(object-fit: contain)가 실제로 그리는 그림의 위치를 뷰어 기준으로 잰다.
// 상자 안에서 가로·세로 비율을 지켜 가운데 맞춘 영역이다. 이미지를 아직 읽지 못했으면 null.
export function measureCoverImage(img, container) {
  if (!img || !container || !img.naturalWidth || !img.naturalHeight) return null;
  const box = img.getBoundingClientRect();
  const containerBox = container.getBoundingClientRect();
  const scale = Math.min(box.width / img.naturalWidth, box.height / img.naturalHeight);
  const width = img.naturalWidth * scale;
  const height = img.naturalHeight * scale;
  if (!(width > 0 && height > 0)) return null;
  return {
    left: box.left - containerBox.left + (box.width - width) / 2,
    top: box.top - containerBox.top + (box.height - height) / 2,
    width,
    height,
  };
}
