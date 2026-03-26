import { str } from "crc-32";
import { DateTime } from "luxon";

const GOLDEN_RATIO_CONJUGATE = 0.6180339887498949; // 1/φ

// R1 시퀀스: 황금비 기반 equidistributed 수열
export const quasiRandomHue = (index) => {
  return (index * GOLDEN_RATIO_CONJUGATE) % 1.0;
};

// Van der Corput 시퀀스 (대안 준난수열)
export const vanDerCorput = (n, base = 2) => {
  let result = 0;
  let denom = 1;
  while (n > 0) {
    denom *= base;
    result += (n % base) / denom;
    n = Math.floor(n / base);
  }
  return result;
};

export function getApiUrlPrefix() {
  const api_url_prefix =
    window.__ENV__?.["VITE_API_URL_PREFIX"] ||
    import.meta.env.VITE_API_URL_PREFIX;
  if (!api_url_prefix) {
    console.log("The environment variable VITE_API_URL_PREFIX is not set.");
  }
  return api_url_prefix;
}

function getAuthHeaders(includeContentType = false) {
  const headers = {};
  if (includeContentType) {
    headers["Content-Type"] = "application/json";
  }
  return headers;
}

function clearAuthState() {
  // HttpOnly 쿠키는 JS에서 직접 제거 불가
}

let refreshPromise = null;

export async function tryRefreshToken() {
  // 동시 다발 요청이 모두 refresh를 시도하지 않도록 단일 Promise 공유
  if (refreshPromise) return refreshPromise;

  refreshPromise = (async () => {
    try {
      const res = await fetch(getApiUrlPrefix() + "/auth/refresh", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        credentials: "include",
      });
      if (!res.ok) return false;
      return true;
    } catch {
      return false;
    } finally {
      refreshPromise = null;
    }
  })();

  return refreshPromise;
}

export function handleFetchErrors(response) {
  if (!response.ok && response.status !== 401) {
    throw Error(response.statusText);
  }
  return response;
}

export const getRandomLightColor = (key) => {
  let index = str(key + "_saltstring") % (256 * 256 * 256);
  if (index < 0) index = -index;
  const hue = Math.round(quasiRandomHue(index) * 360);
  return `hsl(${hue}, 55%, 90%)`;
};

export const getRandomMediumColor = (key) => {
  let index = str(key + "_saltstring") % (256 * 256 * 256);
  if (index < 0) index = -index;
  const hue = Math.round(quasiRandomHue(index) * 360);
  return `hsl(${hue}, 50%, 55%)`;
};

function buildFetchOptions(method, payload) {
  return payload
    ? {
        method,
        headers: getAuthHeaders(true),
        body: JSON.stringify(payload),
        credentials: "include",
      }
    : { method, headers: getAuthHeaders(), credentials: "include" };
}

function processResponse(response, type) {
  if (type === "JSON") return response.json();
  if (type === "TEXT") return response.text();
  return response.blob();
}

function processData(data, type, resolve, reject) {
  if (type === "JSON") {
    if (data["status"] === "success") {
      if (typeof data["result"] === "object") {
        if (data["last_modified_time"]) {
          data["result"]["last_modified_time"] = DateTime.fromISO(
            data["last_modified_time"],
          )
            .setZone("local")
            .toFormat("MM-dd HH:mm");
        }
        if (data["last_responded_time"]) {
          data["result"]["last_responded_time"] = DateTime.fromISO(
            data["last_responded_time"],
          )
            .setZone("local")
            .toFormat("MM-dd HH:mm");
        }
      }
      if (data["warning"]) {
        resolve({ result: data["result"], warning: data["warning"] });
      } else {
        resolve(data["result"]);
      }
    } else {
      reject(data["error"]);
    }
  } else {
    resolve(data);
  }
}

const apiReq = (url, method, payload, type, resolve, reject, final) => {
  const fullUrl = getApiUrlPrefix() + url;
  fetch(fullUrl, buildFetchOptions(method, payload))
    .then(handleFetchErrors)
    .then(async (response) => {
      if (response.status === 401) {
        const refreshed = await tryRefreshToken();
        if (refreshed) {
          // 새 토큰으로 재시도
          const retry = await fetch(
            fullUrl,
            buildFetchOptions(method, payload),
          );
          if (!retry.ok) {
            clearAuthState();
            window.location.reload();
            throw Error("Authentication required");
          }
          return processResponse(retry, type);
        }
        clearAuthState();
        window.location.reload();
        throw Error("Authentication required");
      }
      return processResponse(response, type);
    })
    .then((data) => processData(data, type, resolve, reject))
    .catch((error) => {
      if (reject) reject(error);
    })
    .finally(() => {
      if (final) final();
    });
};

export const jsonGetReq = (url, payload, resolve, reject, final) =>
  apiReq(url, "GET", payload, "JSON", resolve, reject, final);
export const jsonPostReq = (url, payload, resolve, reject, final) =>
  apiReq(url, "POST", payload, "JSON", resolve, reject, final);
export const jsonPutReq = (url, payload, resolve, reject, final) =>
  apiReq(url, "PUT", payload, "JSON", resolve, reject, final);
export const jsonDeleteReq = (url, payload, resolve, reject, final) =>
  apiReq(url, "DELETE", payload, "JSON", resolve, reject, final);
export const textGetReq = (url, payload, resolve, reject, final) =>
  apiReq(url, "GET", payload, "TEXT", resolve, reject, final);
export const blobGetReq = (url, payload, resolve, reject, final) =>
  apiReq(url, "GET", payload, "BLOB", resolve, reject, final);

// 원본 JSON 응답을 그대로 반환 (status 체크 없이, 내부 API용)
export const rawJsonGetReq = (url, resolve, reject, final) => {
  const fullUrl = getApiUrlPrefix() + url;
  fetch(fullUrl, { headers: getAuthHeaders(), credentials: "include" })
    .then(handleFetchErrors)
    .then(async (response) => {
      if (response.status === 401) {
        const refreshed = await tryRefreshToken();
        if (refreshed) {
          const retry = await fetch(fullUrl, {
            headers: getAuthHeaders(),
            credentials: "include",
          });
          if (!retry.ok) {
            clearAuthState();
            window.location.reload();
            throw Error("Authentication required");
          }
          return retry.json();
        }
        clearAuthState();
        window.location.reload();
        throw Error("Authentication required");
      }
      return response.json();
    })
    .then((data) => resolve && resolve(data))
    .catch((error) => reject && reject(error))
    .finally(() => final && final());
};

// 외부 API 호출용 (prefix 없이, 원본 JSON 응답 반환)
export const externalJsonGetReq = (url, resolve, reject, final) => {
  fetch(url)
    .then(handleFetchErrors)
    .then((response) => response.json())
    .then((data) => resolve && resolve(data))
    .catch((error) => reject && reject(error))
    .finally(() => final && final());
};

export const ROOT_DIRECTORY = "$$rootdir$$";
