import { authFetch, getApiUrlPrefix } from "./Common";

// 기존 cookie 인증과 refresh를 유지하면서 viewport 변경 시 요청을 취소한다.
export async function coverAdminRequest(path, { signal, body, image = false } = {}) {
  const url = `${getApiUrlPrefix()}/admin/book-covers${path}`;
  const options = {
    credentials: "include", signal,
    ...(body ? { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) } : {}),
  };
  // 고른 표지를 아직 저장하지 않았을 수 있어 만료돼도 새로고침하지 않는다.
  const response = await authFetch(url, options, "coverAdminApi", { reload: false });
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    throw new Error(typeof data.detail === "string" ? data.detail : "표지 요청을 완료하지 못했습니다");
  }
  if (image) return response.blob();
  const data = await response.json();
  if (data.status !== "success") throw new Error(data.error || "표지 요청을 완료하지 못했습니다");
  return data.result;
}
