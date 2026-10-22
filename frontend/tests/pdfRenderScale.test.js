import { describe, it, expect } from "vitest";
import { getFullPageImageSize } from "../src/pdfRenderScale";

const PAINT = 85;
const makePage = (fnArray, argsArray, base = { width: 100, height: 150 }) => ({
  getViewport: () => base,
  getOperatorList: () => Promise.resolve({ fnArray, argsArray }),
});

describe("getFullPageImageSize 경계", () => {
  it("paintOp 가 아닌 연산은 이미지로 세지 않는다", async () => {
    // 다른 연산 두 개와 이미지 연산 한 개가 섞여 있어도 이미지는 1장으로 본다.
    const page = makePage([1, PAINT, 2], [[], ["img", 100, 150], []]);
    expect(await getFullPageImageSize(page, PAINT)).toEqual({ width: 100, height: 150 });
  });

  it("이미지 연산이 없거나 둘 이상이면 null 이다", async () => {
    expect(await getFullPageImageSize(makePage([1], [[]]), PAINT)).toBeNull();
    const two = makePage([PAINT, PAINT], [["a", 100, 150], ["b", 100, 150]]);
    expect(await getFullPageImageSize(two, PAINT)).toBeNull();
  });

  it("이미지 폭이나 높이가 0 이하이거나 쪽 크기가 0 이면 null 이다", async () => {
    expect(await getFullPageImageSize(makePage([PAINT], [["i", 0, 150]]), PAINT)).toBeNull();
    expect(await getFullPageImageSize(makePage([PAINT], [["i", 100, 0]]), PAINT)).toBeNull();
    const zeroBase = makePage([PAINT], [["i", 100, 150]], { width: 0, height: 150 });
    expect(await getFullPageImageSize(zeroBase, PAINT)).toBeNull();
    const zeroHeight = makePage([PAINT], [["i", 100, 150]], { width: 100, height: 0 });
    expect(await getFullPageImageSize(zeroHeight, PAINT)).toBeNull();
  });
});
