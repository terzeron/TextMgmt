import { getApiUrlPrefix } from "./Common";

const DEDUP_WINDOW_MS = 5000;
const MAX_RECENT_KEYS = 100;
const recentErrors = new Map();

/**
 * 중복 에러 전송 방지 검사 (동일 에러 5초 쿨다운)
 */
function isDuplicateError(key) {
  const now = Date.now();
  const lastTime = recentErrors.get(key);
  if (lastTime && now - lastTime < DEDUP_WINDOW_MS) {
    return true;
  }
  recentErrors.set(key, now);
  // size > MAX_RECENT_KEYS(>0)이 보장되므로 맵은 비어있지 않고 next().value는 항상 정의된다.
  if (recentErrors.size > MAX_RECENT_KEYS) {
    recentErrors.delete(recentErrors.keys().next().value);
  }
  return false;
}

/**
 * 프론트엔드 에러를 백엔드로 전송
 *
 * @param {Object} options
 * @param {'REACT_RENDER_ERROR' | 'WINDOW_ERROR' | 'UNHANDLED_PROMISE' | 'CUSTOM_ERROR'} options.errorType
 * @param {string} options.message
 * @param {string} [options.stack]
 * @param {string} [options.componentStack]
 * @param {string} [options.url]
 * @param {string} [options.userAgent]
 * @param {string} [options.timestamp]
 * @returns {boolean} 전송 시도 여부
 */
export function reportClientError({
  errorType = "CUSTOM_ERROR",
  message = "",
  stack = null,
  componentStack = null,
  url = null,
  userAgent = null,
  timestamp = null,
} = {}) {
  const errorMsg = String(message || "Unknown error");
  const dedupKey = `${errorType}:${errorMsg}:${stack ? String(stack).slice(0, 100) : ""}`;

  if (isDuplicateError(dedupKey)) {
    return false;
  }

  let endpoint = "/logs/client-error";
  try {
    const prefix = getApiUrlPrefix();
    if (prefix) {
      endpoint = `${prefix}/logs/client-error`;
    }
  } catch {
    // getApiUrlPrefix 실패 시 기본 경로 사용
  }

  const payload = {
    error_type: errorType,
    message: errorMsg.slice(0, 2000),
    stack: stack ? String(stack).slice(0, 5000) : null,
    component_stack: componentStack
      ? String(componentStack).slice(0, 5000)
      : null,
    url:
      url ||
      (typeof window !== "undefined" && window.location
        ? window.location.href
        : ""),
    user_agent:
      userAgent ||
      (typeof navigator !== "undefined" ? navigator.userAgent : null),
    timestamp: timestamp || new Date().toISOString(),
  };

  // 1. navigator.sendBeacon 우선 시도 (페이지 언로드 중에도 안전)
  if (
    typeof navigator !== "undefined" &&
    typeof navigator.sendBeacon === "function" &&
    typeof Blob !== "undefined"
  ) {
    try {
      const blob = new Blob([JSON.stringify(payload)], {
        type: "application/json",
      });
      const sent = navigator.sendBeacon(endpoint, blob);
      if (sent) {
        return true;
      }
    } catch {
      // sendBeacon 실패 시 fetch 로 폴백
    }
  }

  // 2. fetch keepalive 폴백
  if (typeof fetch === "function") {
    try {
      fetch(endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
        credentials: "include",
        keepalive: true,
      }).catch(() => {
        // 로깅 실패는 무음 처리 (에러 루프 방지)
      });
      return true;
    } catch {
      return false;
    }
  }

  return false;
}

let _isGlobalLoggingInitialized = false;
let _errorHandler = null;
let _unhandledRejectionHandler = null;

/**
 * 전역 window error 및 unhandledrejection 리스너 등록
 */
export function initGlobalErrorLogging() {
  if (typeof window === "undefined" || _isGlobalLoggingInitialized) {
    return () => {};
  }

  _errorHandler = (event) => {
    try {
      reportClientError({
        errorType: "WINDOW_ERROR",
        message:
          event.message ||
          (event.error && event.error.message) ||
          String(event),
        stack: event.error?.stack,
        url: event.filename || (window.location ? window.location.href : ""),
      });
    } catch {
      // 무음 처리
    }
  };

  _unhandledRejectionHandler = (event) => {
    try {
      const reason = event.reason;
      reportClientError({
        errorType: "UNHANDLED_PROMISE",
        message:
          reason?.message ||
          (typeof reason === "string" ? reason : "Unhandled Promise Rejection"),
        stack: reason?.stack,
      });
    } catch {
      // 무음 처리
    }
  };

  window.addEventListener("error", _errorHandler);
  window.addEventListener("unhandledrejection", _unhandledRejectionHandler);
  _isGlobalLoggingInitialized = true;

  return removeGlobalErrorLogging;
}

/**
 * 전역 에러 리스너 해제 (테스트 및 정리용)
 */
export function removeGlobalErrorLogging() {
  if (typeof window === "undefined" || !_isGlobalLoggingInitialized) {
    return;
  }
  // _isGlobalLoggingInitialized가 true라는 건 initGlobalErrorLogging에서 두 핸들러를
  // 방금 함께 등록했다는 뜻이라, 이 시점엔 둘 다 항상 non-null이다.
  window.removeEventListener("error", _errorHandler);
  _errorHandler = null;
  window.removeEventListener("unhandledrejection", _unhandledRejectionHandler);
  _unhandledRejectionHandler = null;
  _isGlobalLoggingInitialized = false;
}

/**
 * 테스트용 중복 캐시 초기화 함수
 */
export function _resetDedupCacheForTesting() {
  recentErrors.clear();
}
