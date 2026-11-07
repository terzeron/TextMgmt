import { useCallback, useState } from "react";

const KEY_PREFIX = "tm_viewer_";

function readOption(key, fallback, isValid) {
  try {
    const raw = localStorage.getItem(KEY_PREFIX + key);
    if (raw === null) return fallback;
    const value = JSON.parse(raw);
    return isValid(value) ? value : fallback;
  } catch {
    return fallback;
  }
}

function writeOption(key, value) {
  try {
    localStorage.setItem(KEY_PREFIX + key, JSON.stringify(value));
  } catch {
    // 저장하지 못해도 현재 화면의 옵션 변경은 그대로 유지한다.
  }
}

/**
 * 뷰어 옵션을 브라우저 localStorage에 기억하는 useState.
 * 저장된 값이 없거나 isValid를 통과하지 못하면 fallback을 쓴다.
 * 함수형 갱신(prev => next)도 지원한다.
 */
export function useStoredViewerOption(key, fallback, isValid = () => true) {
  const [value, setValue] = useState(() => readOption(key, fallback, isValid));

  const update = useCallback(
    (next) => {
      setValue((prev) => {
        const resolved = typeof next === "function" ? next(prev) : next;
        writeOption(key, resolved);
        return resolved;
      });
    },
    [key],
  );

  return [value, update];
}
