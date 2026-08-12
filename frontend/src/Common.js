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
let _refreshTimerId = null;

// 선제적 토큰 갱신: 만료 5분 전에 자동으로 refresh
const REFRESH_BUFFER_MS = 5 * 60 * 1000;

// refresh token 은 서버에서 회전(rotation)되므로, 여러 탭이 같은 refresh token 으로
// 동시에 /auth/refresh 를 호출하면 먼저 처리된 요청이 토큰을 회전시킨 뒤 나머지 요청이
// 이미 회전된 토큰을 제출한다. 서버는 이를 재사용 공격으로 보고 패밀리를 폐기해 세션이
// 통째로 풀린다. 탭 내부 single-flight(refreshPromise)만으로는 탭 사이의 경합을 막지
// 못하므로 브라우저 전역 락으로 한 번에 한 탭만 회전하게 하고, 나머지 탭은 그 결과를
// 브로드캐스트로 받아 재사용한다.
const REFRESH_LOCK_NAME = "tm-auth-refresh";
const REFRESH_CHANNEL_NAME = "tm-auth-refresh";
const REFRESH_LOCK_KEY = "tm_auth_refresh_lock";
const REFRESH_RESULT_KEY = "tm_auth_refresh_result";
// 폴백 락의 최대 점유 시간. 홀더가 죽어도 이 시간이 지나면 다른 탭이 회수한다.
const REFRESH_LOCK_TTL_MS = 10_000;
// 락을 잡은 탭의 결과를 기다리는 최대 시간.
const REFRESH_WAIT_TIMEOUT_MS = 10_000;
// BroadcastChannel 은 버퍼링이 없어서 방송 직후에 대기를 시작한 탭은 메시지를 놓친다.
// 그 탭이 타임아웃까지 기다린 뒤 자체 refresh 를 보내면 이미 회전된 토큰을 제출해
// 막으려던 재사용 오판을 그대로 유발하므로, 저장된 결과도 함께 폴링한다.
const REFRESH_POLL_INTERVAL_MS = 150;
// 다수의 탭이 거의 동시에 visible 로 바뀔 때 각 탭이 refresh 를 쏘지 않도록 하는 창.
const VISIBILITY_REFRESH_DEBOUNCE_MS = 3_000;

const NOT_ACQUIRED = Symbol("refresh-lock-not-acquired");

function _scheduleProactiveRefresh(expiresInSec) {
  if (_refreshTimerId) clearTimeout(_refreshTimerId);
  const delayMs = Math.max(expiresInSec * 1000 - REFRESH_BUFFER_MS, 10_000);
  _refreshTimerId = setTimeout(async () => {
    _refreshTimerId = null;
    await tryRefreshToken();
  }, delayMs);
}

export function stopProactiveRefresh() {
  if (_refreshTimerId) {
    clearTimeout(_refreshTimerId);
    _refreshTimerId = null;
  }
}

function _openRefreshChannel() {
  if (typeof BroadcastChannel === "undefined") return null;
  try {
    return new BroadcastChannel(REFRESH_CHANNEL_NAME);
  } catch {
    return null;
  }
}

function _storeRefreshResult(expiresIn) {
  try {
    localStorage.setItem(
      REFRESH_RESULT_KEY,
      JSON.stringify({ at: Date.now(), expiresIn }),
    );
  } catch {
    // localStorage 를 쓸 수 없으면 브로드캐스트만으로 동작한다.
  }
}

function _readStoredRefreshResult() {
  try {
    const parsed = JSON.parse(localStorage.getItem(REFRESH_RESULT_KEY));
    return parsed && Number.isFinite(parsed.at) ? parsed : null;
  } catch {
    return null;
  }
}

function _publishRefreshResult(expiresIn) {
  _storeRefreshResult(expiresIn);
  const channel = _openRefreshChannel();
  if (!channel) return;
  try {
    channel.postMessage({ type: "tm-auth-refreshed", expiresIn });
  } catch {
    // 브로드캐스트 실패는 치명적이지 않다. 대기 탭은 저장된 결과를 폴링해서 본다.
  } finally {
    channel.close();
  }
}

// 락을 잡은 탭이 회전을 끝낼 때까지 기다린다. 결과를 받으면 payload, 실패/타임아웃이면 null.
// `since` 이후에 완료된 회전만 내 것으로 채택한다(오래된 결과 재사용 방지).
function _awaitPeerRefresh(since) {
  return new Promise((resolve) => {
    let settled = false;
    let pollId = null;
    let timerId = null;
    const channel = _openRefreshChannel();
    const finish = (value) => {
      if (settled) return;
      settled = true;
      if (timerId) clearTimeout(timerId);
      if (pollId) clearInterval(pollId);
      if (channel) channel.close();
      resolve(value);
    };
    if (channel) {
      channel.onmessage = (event) => {
        if (event?.data?.type === "tm-auth-refreshed") finish(event.data);
      };
    }
    const checkStored = () => {
      const stored = _readStoredRefreshResult();
      if (stored && stored.at >= since) finish(stored);
    };
    timerId = setTimeout(() => finish(null), REFRESH_WAIT_TIMEOUT_MS);
    pollId = setInterval(checkStored, REFRESH_POLL_INTERVAL_MS);
    // 이미 끝난 회전을 놓쳤을 수 있으므로 즉시 한 번 확인한다.
    checkStored();
  });
}

// Web Locks 미지원 브라우저용 폴백. localStorage 는 원자적 CAS 를 제공하지 않으므로
// read-then-write 경합이 남지만, 대부분의 중복 요청을 없애는 best-effort 로 충분하다.
function _acquireFallbackLock() {
  try {
    const now = Date.now();
    const heldAt = Number(localStorage.getItem(REFRESH_LOCK_KEY));
    if (
      Number.isFinite(heldAt) &&
      heldAt > 0 &&
      now - heldAt < REFRESH_LOCK_TTL_MS
    ) {
      return false;
    }
    localStorage.setItem(REFRESH_LOCK_KEY, String(now));
    return true;
  } catch {
    // localStorage 를 쓸 수 없으면 조율을 포기하고 진행한다(가용성 우선).
    return true;
  }
}

function _releaseFallbackLock() {
  try {
    localStorage.removeItem(REFRESH_LOCK_KEY);
  } catch {
    // 무시
  }
}

// fn 을 브라우저 전역 락 아래에서 실행한다. 다른 탭이 이미 점유 중이면 NOT_ACQUIRED.
async function _runWithRefreshLock(fn) {
  if (typeof navigator !== "undefined" && navigator.locks?.request) {
    try {
      return await navigator.locks.request(
        REFRESH_LOCK_NAME,
        { ifAvailable: true },
        async (lock) => (lock ? fn() : NOT_ACQUIRED),
      );
    } catch {
      // Web Locks 가 실패하면 조율 없이 진행한다(세션 유지가 우선).
      return await fn();
    }
  }
  if (!_acquireFallbackLock()) return NOT_ACQUIRED;
  try {
    return await fn();
  } finally {
    _releaseFallbackLock();
  }
}

async function _performRefresh() {
  try {
    const res = await fetch(getApiUrlPrefix() + "/auth/refresh", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      credentials: "include",
    });
    if (!res.ok) return false;
    const data = await res.json();
    if (data.expires_in) {
      _scheduleProactiveRefresh(data.expires_in);
    }
    _publishRefreshResult(data.expires_in);
    return true;
  } catch {
    return false;
  }
}

async function _refreshAcrossTabs() {
  const startedAt = Date.now();
  const outcome = await _runWithRefreshLock(_performRefresh);
  if (outcome !== NOT_ACQUIRED) return outcome;

  // 다른 탭이 회전 중이다. 자체 요청을 보내면 회전된 토큰을 재제출하게 되므로
  // 그 탭의 결과를 기다려 그대로 채택한다.
  const shared = await _awaitPeerRefresh(startedAt);
  if (shared) {
    if (shared.expiresIn) _scheduleProactiveRefresh(shared.expiresIn);
    return true;
  }

  // 홀더가 실패했거나 결과가 오지 않았다. 락을 회수해 직접 시도한다.
  const retried = await _runWithRefreshLock(_performRefresh);
  return retried === NOT_ACQUIRED ? false : retried;
}

export async function tryRefreshToken() {
  // 동시 다발 요청이 모두 refresh를 시도하지 않도록 단일 Promise 공유
  if (refreshPromise) return refreshPromise;

  refreshPromise = (async () => {
    try {
      return await _refreshAcrossTabs();
    } finally {
      refreshPromise = null;
    }
  })();

  return refreshPromise;
}

let _lastVisibilityRefreshAt = 0;

// 탭이 다시 보일 때 호출한다. 여러 탭이 한꺼번에 활성화되어도 짧은 창 안에서는
// 한 번만 refresh 를 시도한다.
export function refreshOnVisible() {
  const now = Date.now();
  if (now - _lastVisibilityRefreshAt < VISIBILITY_REFRESH_DEBOUNCE_MS) {
    return Promise.resolve(false);
  }
  _lastVisibilityRefreshAt = now;
  return tryRefreshToken();
}

// 로그인 성공 후 호출하여 선제적 갱신 타이머 시작
export function startProactiveRefresh(expiresInSec) {
  _scheduleProactiveRefresh(expiresInSec);
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

// 열람 뷰어 진입을 서버에 1건 기록한다.
//
// 경로가 유형별로 비대칭인 이유: 책 라우터는 루트에, 만화 라우터는 /comics prefix 에
// 붙어 있어 각각 /books/view-history/{id} 와 /comics/view-history/{id} 가 된다.
// 호출자가 이 차이를 몰라도 되게 여기서 흡수한다.
//
// fire-and-forget: 이력 기록이 실패해도 열람을 막지 않는다.
export function recordBookView(apiPrefix, bookId) {
  if (!bookId) return Promise.resolve(false);
  const path = apiPrefix
    ? `${apiPrefix}/view-history/${bookId}`
    : `/books/view-history/${bookId}`;
  return fetch(getApiUrlPrefix() + path, {
    method: "POST",
    headers: getAuthHeaders(),
    credentials: "include",
  })
    .then((res) => res.ok)
    .catch(() => false);
}

export const ROOT_DIRECTORY = "$$rootdir$$";
