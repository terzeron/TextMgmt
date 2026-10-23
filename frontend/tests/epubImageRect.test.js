import { describe, it, expect } from "vitest";
import { measureCoverImage, measureCurrentImage } from "../src/epubImageRect";

const box = (left, top, width, height) => ({ left, top, width, height });
const container = { getBoundingClientRect: () => box(0, 0, 800, 600) };

function view({ images = 1, text = "", img = box(100, 50, 400, 300), idref = "s1", iframe = true } = {}) {
  const doc = document.implementation.createHTMLDocument("p");
  doc.body.textContent = text;
  for (let i = 0; i < images; i += 1) {
    const image = doc.createElement("img");
    image.getBoundingClientRect = () => img;
    doc.body.appendChild(image);
  }
  return {
    section: { idref },
    iframe: iframe ? { getBoundingClientRect: () => box(10, 20, 700, 500) } : undefined,
    contents: { document: doc },
  };
}
const renditionOf = (views, method = "all") => ({ manager: { views: { [method]: () => views } } });

describe("measureCurrentImage", () => {
  it("iframe 위치와 이미지의 iframe 안 위치를 더해 뷰어 기준으로 잰다", () => {
    const result = measureCurrentImage(renditionOf([view()]), container, "s1");
    expect(result).toMatchObject({ count: 1, imageOnly: true });
    expect(result.images[0]).toMatchObject({ index: 0, rect: box(110, 70, 400, 300) });
  });

  it("displayed() 가 있으면 그것을 쓰고, idref 가 맞는 뷰를 고른다", () => {
    const views = [view({ idref: "s0", img: box(0, 0, 10, 10) }), view({ idref: "s1" })];
    const result = measureCurrentImage(renditionOf(views, "displayed"), container, "s1");
    expect(result.images[0].rect.width).toBe(400);
    // 맞는 idref 가 없으면 첫 뷰를 쓴다.
    expect(measureCurrentImage(renditionOf(views), container, "zz").images[0].rect.width).toBe(10);
  });

  it("글이 함께 있으면 imageOnly 가 아니다", () => {
    expect(measureCurrentImage(renditionOf([view({ text: "본문" })]), container, "s1").imageOnly).toBe(false);
  });

  it("이미지가 0장이면 목록이 비고, 여러 장이면 문서 순서대로 index 를 매긴다", () => {
    const none = measureCurrentImage(renditionOf([view({ images: 0 })]), container, "s1");
    expect(none).toMatchObject({ count: 0, imageOnly: false, images: [] });
    const two = measureCurrentImage(renditionOf([view({ images: 2 })]), container, "s1");
    expect(two).toMatchObject({ count: 2, imageOnly: false });
    expect(two.images.map((image) => image.index)).toEqual([0, 1]);
    expect(two.images.every((image) => image.rect)).toBe(true);
  });

  it("iframe 을 찾지 못하면 rect 가 없다", () => {
    const result = measureCurrentImage(renditionOf([view({ iframe: false })]), container, "s1");
    expect(result.images[0].rect).toBeNull();
  });

  it("iframe 요소가 없어도 문서의 frameElement 를 쓴다", () => {
    const v = view({ iframe: false });
    v.contents.document = {
      body: v.contents.document.body,
      images: v.contents.document.images,
      defaultView: { frameElement: { getBoundingClientRect: () => box(10, 20, 700, 500) } },
    };
    expect(measureCurrentImage(renditionOf([v]), container, "s1").images[0].rect.left).toBe(110);
  });

  it("이미지가 뷰어 밖에 있거나 크기가 0이면 보이는 것으로 치지 않는다", () => {
    for (const img of [box(900, 50, 400, 300), box(100, 50, 0, 0), box(-50, 50, 400, 300), box(100, 500, 400, 300)]) {
      expect(measureCurrentImage(renditionOf([view({ img })]), container, "s1").images[0].rect).toBeNull();
    }
  });

  it("쪽이 아직 없거나 컨테이너가 없으면 null 이다", () => {
    expect(measureCurrentImage(renditionOf([]), container, "s1")).toBeNull();
    expect(measureCurrentImage(undefined, container, "s1")).toBeNull();
    expect(measureCurrentImage(renditionOf([view()]), null, "s1")).toBeNull();
    expect(measureCurrentImage({ manager: { views: {} } }, container, "s1")).toBeNull();
  });
});

describe("measureCurrentImage: <svg><image>", () => {
  it("svg 안의 image 도 이미지 한 장으로 센다", () => {
    const doc = document.implementation.createHTMLDocument("p");
    doc.body.innerHTML =
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><image width="10" height="10"/></svg>';
    doc.querySelector("image").getBoundingClientRect = () => box(100, 50, 400, 300);
    const rendition = renditionOf([
      {
        section: { idref: "s1" },
        iframe: { getBoundingClientRect: () => box(10, 20, 700, 500) },
        contents: { document: doc },
      },
    ]);
    const result = measureCurrentImage(rendition, container, "s1");
    expect(result.images[0].rect).toEqual(box(110, 70, 400, 300));
    expect(result.imageOnly).toBe(true);
  });
});

describe("measureCoverImage", () => {
  const coverImg = (natural, bounds) => ({
    naturalWidth: natural[0],
    naturalHeight: natural[1],
    getBoundingClientRect: () => bounds,
  });

  it("object-fit: contain 으로 가운데 맞춘 그림의 영역을 뷰어 기준으로 돌려준다", () => {
    // 600x900 그림이 400x400 상자(뷰어 기준 100,50)에 들어가면 267x400, 가로 가운데
    const rect = measureCoverImage(coverImg([600, 900], box(100, 50, 400, 400)), container);
    expect(rect.height).toBeCloseTo(400);
    expect(rect.width).toBeCloseTo(266.67, 1);
    expect(rect.left).toBeCloseTo(100 + (400 - 266.67) / 2, 1);
    expect(rect.top).toBeCloseTo(50);
  });

  it("그림을 아직 읽지 못했거나 대상이 없으면 null", () => {
    expect(measureCoverImage(coverImg([0, 0], box(0, 0, 10, 10)), container)).toBeNull();
    expect(measureCoverImage(null, container)).toBeNull();
    expect(measureCoverImage(coverImg([5, 5], box(0, 0, 10, 10)), null)).toBeNull();
    expect(measureCoverImage(coverImg([5, 5], box(0, 0, 0, 0)), container)).toBeNull();
  });
});
