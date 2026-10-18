import { useEffect, useState } from "react";

// 최신 탭과 책·만화 탭이 같은 저장 값을 쓰므로 어느 화면에서 바꿔도 같은 보기 모드가 이어진다.
const VIEW_MODE_STORAGE_KEY_PREFIX = "tm_latest_view_mode_";

// 뷰 모드는 탭별 편의 설정이다. 저장소를 쓸 수 없으면 커버 뷰를 사용한다.
function readViewMode(contentType) {
  try {
    return localStorage.getItem(VIEW_MODE_STORAGE_KEY_PREFIX + contentType) ===
      "list"
      ? "list"
      : "cover";
  } catch {
    return "cover";
  }
}

function writeViewMode(contentType, viewMode) {
  try {
    localStorage.setItem(VIEW_MODE_STORAGE_KEY_PREFIX + contentType, viewMode);
  } catch {
    // 저장하지 못해도 현재 화면의 전환은 그대로 유지한다.
  }
}

/** contentType("book" | "comic")별로 목록/커버 보기 모드를 기억하는 상태. */
export function useCoverViewMode(contentType) {
  const [viewMode, setViewMode] = useState(() => readViewMode(contentType));

  // 책↔만화 탭 이동 때 같은 컴포넌트 인스턴스가 재사용될 수 있으므로 탭별 값을 다시 읽는다.
  useEffect(() => {
    setViewMode(readViewMode(contentType));
  }, [contentType]);

  const changeViewMode = (nextViewMode) => {
    setViewMode(nextViewMode);
    writeViewMode(contentType, nextViewMode);
  };

  return [viewMode, changeViewMode];
}
