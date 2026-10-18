// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

import {
  MAX_UPLOAD_BYTES,
  blobToBase64,
  requestInsertPreview,
} from "../src/insertPreview";

vi.mock("../src/Common", () => ({
  getApiUrlPrefix: () => "http://localhost:8000",
}));

const JPEG = new Blob(["jpeg"], { type: "image/jpeg" });
const okResponse = () => ({ ok: true, status: 200, blob: () => Promise.resolve(JPEG) });
const textResponse = (status, text) => ({
  ok: false,
  status,
  text: () => Promise.resolve(text),
});

let originalFetch;
beforeEach(() => {
  originalFetch = globalThis.fetch;
  globalThis.fetch = vi.fn(() => Promise.resolve(okResponse()));
});
afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("requestInsertPreview", () => {
  it("URL 은 insert-preview 에 JSON 으로 보낸다", async () => {
    const blob = await requestInsertPreview("", 7, "https://img.example.com/a.png");

    expect(blob).toBe(JPEG);
    const [url, init] = globalThis.fetch.mock.calls[0];
    expect(url).toBe("http://localhost:8000/pdf-pages/7/insert-preview");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers).toEqual({ "Content-Type": "application/json" });
    expect(JSON.parse(init.body)).toEqual({ url: "https://img.example.com/a.png" });
  });

  it("파일은 insert-preview-upload 에 multipart 로 보내며 Content-Type 은 브라우저가 정하게 둔다", async () => {
    const file = new File(["x"], "photo.jpg", { type: "image/jpeg" });

    const blob = await requestInsertPreview("/api", 7, file);

    expect(blob).toBe(JPEG);
    const [url, init] = globalThis.fetch.mock.calls[0];
    expect(url).toBe("http://localhost:8000/api/pdf-pages/7/insert-preview-upload");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers).toBeUndefined(); // boundary 가 빠지지 않도록 직접 지정하지 않는다
    expect(init.body).toBeInstanceOf(FormData);
    expect(init.body.get("file")).toBe(file);
  });

  it("한도를 넘는 파일은 올리기 전에 거절한다", async () => {
    const big = new File(["x"], "big.jpg");
    Object.defineProperty(big, "size", { value: MAX_UPLOAD_BYTES + 1 });

    await expect(requestInsertPreview("", 7, big)).rejects.toThrow(
      "이미지는 15MB 이하여야 합니다",
    );
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it("한도와 같은 크기의 파일은 올린다", async () => {
    const exact = new File(["x"], "exact.jpg");
    Object.defineProperty(exact, "size", { value: MAX_UPLOAD_BYTES });

    await requestInsertPreview("", 7, exact);

    expect(globalThis.fetch).toHaveBeenCalledTimes(1);
  });

  it("400 은 서버가 보낸 사유를 그대로 던진다", async () => {
    globalThis.fetch.mockResolvedValue(textResponse(400, " 이미지 형식을 읽을 수 없습니다 "));

    await expect(
      requestInsertPreview("", 7, new File(["x"], "a.txt")),
    ).rejects.toThrow("이미지 형식을 읽을 수 없습니다");
  });

  it("400 에 본문이 없거나 다른 오류면 상태 코드로 설명한다", async () => {
    globalThis.fetch.mockResolvedValue(textResponse(400, ""));
    await expect(requestInsertPreview("", 7, "https://a.example/a.png")).rejects.toThrow(
      "HTTP 400",
    );

    globalThis.fetch.mockResolvedValue(textResponse(502, "bad gateway"));
    await expect(requestInsertPreview("", 7, "https://a.example/a.png")).rejects.toThrow(
      "HTTP 502 - bad gateway",
    );
  });
});

describe("blobToBase64", () => {
  it("data URL 의 접두어 없이 base64 만 돌려준다", async () => {
    expect(await blobToBase64(new Blob(["jpeg"]))).toBe("anBlZw==");
  });

  it("읽기에 실패하면 사유를 던진다", async () => {
    const original = globalThis.FileReader;
    globalThis.FileReader = class {
      readAsDataURL() {
        this.onerror();
      }
    };
    try {
      await expect(blobToBase64(new Blob(["x"]))).rejects.toThrow(
        "이미지를 읽지 못했습니다",
      );
    } finally {
      globalThis.FileReader = original;
    }
  });
});
