import { afterEach, describe, expect, it, vi } from "vitest";
import { coverAdminRequest } from "../src/coverAdminApi";

// 갱신·만료 처리는 Common.authFetch 테스트가 맡는다. 여기서는 전역 fetch 로 넘기기만 한다.
const { authFetch } = vi.hoisted(() => ({ authFetch: vi.fn((url, options) => fetch(url, options)) }));
vi.mock("../src/Common", () => ({ getApiUrlPrefix: () => "https://api.example.org", authFetch }));
afterEach(() => { vi.unstubAllGlobals(); authFetch.mockClear(); });

describe("표지 관리 API", () => {
  it("cookie 인증과 AbortSignal을 유지해 JSON을 조회한다", async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({ status: "success", result: { total: 2 } }) });
    vi.stubGlobal("fetch", fetch);
    const signal = new AbortController().signal;
    expect(await coverAdminRequest("/missing", { signal })).toEqual({ total: 2 });
    expect(fetch).toHaveBeenCalledWith("https://api.example.org/admin/book-covers/missing", { credentials: "include", signal });
  });

  it("POST body를 JSON으로 보낸다", async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ status: "success", result: true }) });
    vi.stubGlobal("fetch", fetch);
    const body = { file_path: "한글.epub", revision: "r1" };
    expect(await coverAdminRequest("/cache/clear", { body })).toBe(true);
    expect(fetch.mock.calls[0][1]).toMatchObject({ method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  });

  it("이미지 응답을 Blob으로 받는다", async () => {
    const blob = new Blob(["cover"]);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, blob: async () => blob }));
    expect(await coverAdminRequest("/candidate-image/a", { image: true })).toBe(blob);
  });

  it("만료돼도 고른 표지를 잃지 않도록 새로고침 없이 authFetch 로 요청한다", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: async () => ({ status: "success", result: "saved" }) }));
    expect(await coverAdminRequest("/apply", { body: { revision: "r1" } })).toBe("saved");
    expect(authFetch).toHaveBeenCalledWith("https://api.example.org/admin/book-covers/apply", expect.objectContaining({ method: "POST" }), "coverAdminApi", { reload: false });
  });

  it("세션 만료 401 은 서버 사유로 실패한다", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ status: 401, ok: false, json: async () => ({ detail: "로그인이 필요합니다" }) }));
    await expect(coverAdminRequest("/missing")).rejects.toThrow("로그인이 필요합니다");
  });

  it.each([{}, { detail: [] }])("문자열이 아닌 HTTP 오류는 기본 문구를 쓴다: %j", async (data) => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, json: async () => data }));
    await expect(coverAdminRequest("/missing")).rejects.toThrow("표지 요청을 완료하지 못했습니다");
  });

  it("HTML 오류 응답도 기본 문구를 쓴다", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, json: async () => { throw new Error("HTML"); } }));
    await expect(coverAdminRequest("/missing")).rejects.toThrow("표지 요청을 완료하지 못했습니다");
  });

  it.each(["서점 오류", undefined])("HTTP 성공의 실패 envelope를 처리한다: %s", async (error) => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: async () => ({ status: "error", error }) }));
    await expect(coverAdminRequest("/missing")).rejects.toThrow(error || "표지 요청을 완료하지 못했습니다");
  });

  it("취소와 네트워크 오류를 호출자에게 전달한다", async () => {
    const error = new DOMException("취소", "AbortError");
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(error));
    await expect(coverAdminRequest("/missing")).rejects.toBe(error);
  });
});
