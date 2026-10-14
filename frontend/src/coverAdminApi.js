import { getApiUrlPrefix, tryRefreshToken } from "./Common";

// 기존 cookie 인증과 refresh를 유지하면서 viewport 변경 시 요청을 취소한다.
export async function coverAdminRequest(path, { signal, body, image = false } = {}) {
  const url = `${getApiUrlPrefix()}/admin/book-covers${path}`;
  const options = {
    credentials: "include", signal,
    ...(body ? { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) } : {}),
  };
  let response = await fetch(url, options);
  if (response.status === 401 && await tryRefreshToken()) {
    response = await fetch(url, options);
  }
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    throw new Error(typeof data.detail === "string" ? data.detail : "표지 요청을 완료하지 못했습니다");
  }
  if (image) return response.blob();
  const data = await response.json();
  if (data.status !== "success") throw new Error(data.error || "표지 요청을 완료하지 못했습니다");
  return data.result;
}
